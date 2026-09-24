//! Builds the bytes that repaint readline's line in colour and draw the grey
//! suggestion after it.
//!
//! The output saves the cursor, moves to where the line starts, rewrites the
//! same characters readline drew with colours, and restores the cursor.

use std::io::Write;

use unicode_width::UnicodeWidthChar;

use crate::colors::Colors;
use crate::lexer::{Kind, Span};

/// Everything `build` needs to repaint the line once.
pub struct Repaint<'a> {
    /// Columns used by the last line of the prompt.
    pub prompt_width: usize,
    pub line: &'a str,
    /// Cursor position in `line`, in bytes.
    pub point: usize,
    pub spans: &'a [Span],
    pub colors: &'a Colors,
    /// Text to show after the line; drawn only when the cursor is at the end.
    pub suggestion: Option<&'a str>,
    pub rows: usize,
    pub cols: usize,
}

pub struct Output {
    pub bytes: Vec<u8>,
    /// The column the suggestion starts at, if one was drawn.
    pub suggestion_col: Option<usize>,
}

/// Columns used by the last line of `prompt`, skipping the parts between `\001`
/// and `\002`, which readline treats as invisible. None when a visible part has
/// control characters: readline counts each byte of an escape sequence left
/// outside `\[ \]` as a column, the terminal does not, so where the line starts
/// is unknown.
pub fn prompt_width(prompt: &[u8]) -> Option<usize> {
    let last = prompt.rsplit(|&b| b == b'\n').next().unwrap_or(&[]);
    let mut visible = Vec::new();
    let mut hidden = false;
    for &b in last {
        match b {
            1 => hidden = true,
            2 => hidden = false,
            _ if !hidden => visible.push(b),
            _ => {}
        }
    }
    let visible = String::from_utf8_lossy(&visible);
    if visible.chars().any(char::is_control) {
        return None;
    }
    Some(visible.chars().map(|c| c.width().unwrap_or(0)).sum())
}

/// The bytes to write, or None when readline's own drawing should be left
/// alone: control characters other than a newline or a tab (drawn as `^X`), a
/// cursor inside a character (readline works in bytes outside UTF-8 locales),
/// or a line taller than the screen.
pub fn build(repaint: &Repaint) -> Option<Output> {
    let cols = repaint.cols;
    if cols == 0
        || !repaint.line.is_char_boundary(repaint.point)
        || repaint
            .line
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return None;
    }
    let start = position(repaint.prompt_width, "", cols);
    let cursor = position(repaint.prompt_width, &repaint.line[..repaint.point], cols);
    let end = position(repaint.prompt_width, repaint.line, cols);
    if end.0 >= repaint.rows {
        return None;
    }

    let mut out = b"\x1b7".to_vec();
    if cursor.0 > start.0 {
        let _ = write!(out, "\x1b[{}A", cursor.0 - start.0);
    }
    out.push(b'\r');
    if start.1 > 0 {
        let _ = write!(out, "\x1b[{}C", start.1);
    }
    paint_line(&mut out, repaint, start);
    out.extend_from_slice(b"\x1b8");

    let mut suggestion_col = None;
    if let Some(suggestion) = repaint
        .suggestion
        .filter(|_| repaint.point == repaint.line.len())
    {
        // Keep the last column free so the terminal never wraps.
        let shown = fit(suggestion, cols.saturating_sub(cursor.1 + 1));
        if !shown.is_empty() {
            let _ = write!(
                out,
                "\x1b[{}m{shown}\x1b[0m\x1b8",
                repaint.colors.suggestion()
            );
            suggestion_col = Some(cursor.1);
        }
    }
    Some(Output {
        bytes: out,
        suggestion_col,
    })
}

/// Row and column after `text`, starting `prompt_width` cells into the first
/// row, where readline puts each character: see `advance`.
fn position(prompt_width: usize, text: &str, cols: usize) -> (usize, usize) {
    let start = (prompt_width / cols, prompt_width % cols);
    text.chars().fold(start, |at, c| advance(at, c, cols))
}

/// How many screen rows `text` takes after a prompt `prompt_width` columns
/// wide.
pub fn rows(prompt_width: usize, text: &str, cols: usize) -> usize {
    if cols == 0 {
        return 1;
    }
    position(prompt_width, text, cols).0 + 1
}

/// Where readline puts the next character after drawing `c` at `at`. A
/// newline starts the next row. A tab takes the spaces up to the next
/// multiple of 8 columns, continuing on the next row past the edge. A wide
/// character that does not fit moves to the next row, and filling the last
/// column moves to the next row.
fn advance((row, col): (usize, usize), c: char, cols: usize) -> (usize, usize) {
    match c {
        '\n' => (row + 1, 0),
        '\t' => (0..8 - col % 8).fold((row, col), |(r, c), _| step(r, c + 1, cols)),
        _ => {
            let width = c.width().unwrap_or(0);
            let (row, col) = if col + width > cols {
                (row + 1, 0)
            } else {
                (row, col)
            };
            step(row, col + width, cols)
        }
    }
}

/// `row`, `col`, or the start of the next row when `col` reached the edge.
fn step(row: usize, col: usize, cols: usize) -> (usize, usize) {
    if col == cols {
        (row + 1, 0)
    } else {
        (row, col)
    }
}

/// Writes the line in colour, starting at `start`, the screen position of its
/// first character. Later rows are reached with explicit cursor moves, never
/// by writing a newline, so each character lands where readline put it.
fn paint_line(out: &mut Vec<u8>, repaint: &Repaint, start: (usize, usize)) {
    let cols = repaint.cols;
    let mut at = start;
    let mut cursor_row = start.0;
    let mut style: Option<Kind> = None;
    let mut spans = repaint.spans.iter().peekable();
    for (i, c) in repaint.line.char_indices() {
        while spans.next_if(|s| s.end <= i).is_some() {}
        let want = spans.peek().filter(|s| s.start <= i).map(|s| s.kind);
        if want != style {
            if style.is_some() {
                out.extend_from_slice(b"\x1b[0m");
            }
            if let Some(kind) = want {
                let _ = write!(out, "\x1b[{}m", repaint.colors.sgr(kind));
            }
            style = want;
        }
        let next = advance(at, c, cols);
        match c {
            '\n' => {}
            '\t' => {
                let mut cell = at;
                for _ in 0..8 - at.1 % 8 {
                    move_down(out, &mut cursor_row, cell);
                    out.push(b' ');
                    cell = step(cell.0, cell.1 + 1, cols);
                }
            }
            _ => {
                let width = c.width().unwrap_or(0);
                let cell = if at.1 + width > cols {
                    (at.0 + 1, 0)
                } else {
                    at
                };
                move_down(out, &mut cursor_row, cell);
                let mut buf = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        at = next;
    }
    if style.is_some() {
        out.extend_from_slice(b"\x1b[0m");
    }
}

/// Moves the terminal's cursor from row `*cursor_row` down to `cell`, when
/// `cell` is on a later row.
fn move_down(out: &mut Vec<u8>, cursor_row: &mut usize, (row, col): (usize, usize)) {
    if row > *cursor_row {
        let _ = write!(out, "\r\x1b[{}B", row - *cursor_row);
        if col > 0 {
            let _ = write!(out, "\x1b[{col}C");
        }
        *cursor_row = row;
    }
}

/// The longest start of `text` that fits in `room` columns and has no control
/// characters.
fn fit(text: &str, room: usize) -> &str {
    let mut used = 0;
    for (i, c) in text.char_indices() {
        let width = c.width().unwrap_or(0);
        if c.is_control() || used + width > room {
            return &text[..i];
        }
        used += width;
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Kind;

    fn repaint<'a>(
        line: &'a str,
        point: usize,
        spans: &'a [Span],
        colors: &'a Colors,
    ) -> Repaint<'a> {
        Repaint {
            prompt_width: 2,
            line,
            point,
            spans,
            colors,
            suggestion: None,
            rows: 24,
            cols: 80,
        }
    }

    fn text(out: &Output) -> String {
        String::from_utf8(out.bytes.clone()).unwrap()
    }

    #[test]
    fn prompt_width_skips_invisible_parts() {
        assert_eq!(prompt_width(b"$ "), Some(2));
        assert_eq!(
            prompt_width(b"\x01\x1b[32m\x02user\x01\x1b[0m\x02$ "),
            Some(6)
        );
        assert_eq!(prompt_width(b"top line\n> "), Some(2));
        assert_eq!(prompt_width("日本$ ".as_bytes()), Some(6));
    }

    /// Readline counts escape sequences outside `\\[ \\]` as visible, so
    /// inkline cannot tell where the line starts.
    #[test]
    fn prompt_width_unknown_with_visible_control_characters() {
        assert_eq!(prompt_width(b"\x1b[32m$ \x1b[0m"), None);
    }

    #[test]
    fn cursor_inside_a_character_is_left_to_readline() {
        let colors = Colors::default();
        assert!(build(&repaint("echo \u{e9}", 6, &[], &colors)).is_none());
    }

    #[test]
    fn repaints_the_line_in_colour() {
        let colors = Colors::default();
        let spans = [
            Span {
                start: 0,
                end: 2,
                kind: Kind::Command,
            },
            Span {
                start: 3,
                end: 5,
                kind: Kind::Option,
            },
        ];
        let out = build(&repaint("ls -l", 5, &spans, &colors)).unwrap();
        assert_eq!(
            text(&out),
            "\x1b7\r\x1b[2C\x1b[32mls\x1b[0m \x1b[36m-l\x1b[0m\x1b8"
        );
        assert_eq!(out.suggestion_col, None);
    }

    #[test]
    fn draws_the_suggestion_after_the_cursor() {
        let colors = Colors::default();
        let spans = [Span {
            start: 0,
            end: 3,
            kind: Kind::Command,
        }];
        let out = build(&Repaint {
            suggestion: Some("atus"),
            ..repaint("git st", 6, &spans, &colors)
        })
        .unwrap();
        assert_eq!(
            text(&out),
            "\x1b7\r\x1b[2C\x1b[32mgit\x1b[0m st\x1b8\x1b[90matus\x1b[0m\x1b8"
        );
        assert_eq!(out.suggestion_col, Some(8));
    }

    #[test]
    fn no_suggestion_unless_the_cursor_is_at_the_end() {
        let colors = Colors::default();
        let out = build(&Repaint {
            suggestion: Some("atus"),
            ..repaint("git st", 3, &[], &colors)
        })
        .unwrap();
        assert!(!text(&out).contains("atus"));
        assert_eq!(out.suggestion_col, None);
    }

    #[test]
    fn suggestion_stops_before_the_last_column() {
        let colors = Colors::default();
        let f = Repaint {
            suggestion: Some("defghijk"),
            cols: 10,
            ..repaint("abc", 3, &[], &colors)
        };
        let out = build(&f).unwrap();
        assert!(text(&out).ends_with("\x1b[90mdefg\x1b[0m\x1b8"));
        assert_eq!(out.suggestion_col, Some(5));

        let f = Repaint {
            suggestion: Some("xyz"),
            cols: 10,
            ..repaint("abcdefg", 7, &[], &colors)
        };
        let out = build(&f).unwrap();
        assert!(!text(&out).contains("\x1b[90m"));
        assert_eq!(out.suggestion_col, None);
    }

    #[test]
    fn moves_up_to_the_first_row_of_a_wrapped_line() {
        let colors = Colors::default();
        let out = build(&Repaint {
            cols: 10,
            ..repaint("echo 12345678", 13, &[], &colors)
        })
        .unwrap();
        assert!(text(&out).starts_with("\x1b7\x1b[1A\r\x1b[2C"));
        let out = build(&Repaint {
            cols: 10,
            ..repaint("echo 12345678", 0, &[], &colors)
        })
        .unwrap();
        assert!(text(&out).starts_with("\x1b7\r\x1b[2C"));
        // Filling the last column puts the cursor on the next row.
        let out = build(&Repaint {
            cols: 10,
            ..repaint("12345678", 8, &[], &colors)
        })
        .unwrap();
        assert!(text(&out).starts_with("\x1b7\x1b[1A"));
    }

    #[test]
    fn wide_characters_wrap_like_the_terminal() {
        let colors = Colors::default();
        // 2 + 2 + 2 + 2 columns fit in 9; the fourth wide character does not
        // fit in the one column left, so it starts the next row.
        let line = "日本語日本";
        let out = build(&Repaint {
            cols: 9,
            ..repaint(line, line.len(), &[], &colors)
        })
        .unwrap();
        assert!(text(&out).starts_with("\x1b7\x1b[1A"));
        let line = "日本語";
        let out = build(&Repaint {
            cols: 9,
            ..repaint(line, line.len(), &[], &colors)
        })
        .unwrap();
        assert!(text(&out).starts_with("\x1b7\r"));
    }

    #[test]
    fn control_characters_are_left_to_readline() {
        let colors = Colors::default();
        assert!(build(&repaint("a\u{1}b", 3, &[], &colors)).is_none());
        assert!(build(&repaint("a\u{1b}b", 3, &[], &colors)).is_none());
    }

    #[test]
    fn a_newline_starts_the_next_row() {
        let colors = Colors::default();
        let out = build(&repaint("ls\necho", 7, &[], &colors)).unwrap();
        assert_eq!(text(&out), "\x1b7\x1b[1A\r\x1b[2Cls\r\x1b[1Becho\x1b8");
    }

    #[test]
    fn tabs_are_drawn_as_spaces() {
        let colors = Colors::default();
        // The prompt takes 2 columns: `a` ends at 3, the tab fills to 8.
        let out = build(&repaint("a\tb", 3, &[], &colors)).unwrap();
        assert_eq!(text(&out), "\x1b7\r\x1b[2Ca     b\x1b8");
    }

    #[test]
    fn a_tab_past_the_edge_continues_on_the_next_row() {
        let colors = Colors::default();
        let out = build(&Repaint {
            cols: 10,
            ..repaint("abcdef\tx", 8, &[], &colors)
        })
        .unwrap();
        assert!(text(&out).contains("abcdef  \r\x1b[1B      x"));
    }

    #[test]
    fn a_newline_after_a_full_row_leaves_a_blank_row() {
        assert_eq!(position(2, "12345678\nx", 10), (2, 1));
        assert_eq!(rows(2, "12345678\nx", 10), 3);
        assert_eq!(rows(2, "ls", 10), 1);
    }

    #[test]
    fn wrapped_rows_are_reached_with_explicit_moves() {
        let colors = Colors::default();
        let out = build(&Repaint {
            cols: 10,
            ..repaint("echo 12345678", 13, &[], &colors)
        })
        .unwrap();
        assert_eq!(
            text(&out),
            "\x1b7\x1b[1A\r\x1b[2Cecho 123\r\x1b[1B45678\x1b8"
        );
    }

    #[test]
    fn lines_taller_than_the_screen_are_left_to_readline() {
        let colors = Colors::default();
        let line = "x".repeat(25);
        assert!(
            build(&Repaint {
                rows: 2,
                cols: 10,
                ..repaint(&line, 25, &[], &colors)
            })
            .is_none()
        );
        assert!(
            build(&Repaint {
                rows: 3,
                cols: 10,
                ..repaint(&line, 25, &[], &colors)
            })
            .is_some()
        );
    }
}
