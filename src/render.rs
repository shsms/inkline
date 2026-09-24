//! Builds the bytes that repaint readline's line in colour and draw the grey
//! suggestion after it and a message under it.
//!
//! The output saves the cursor, moves to where the line starts, rewrites the
//! same characters readline drew with colours, and restores the cursor.

use std::io::Write;
use std::ops::Range;

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
    /// The most lines of a multi-line suggestion to show.
    pub suggestion_lines: usize,
    /// The bytes of `line` to underline as a syntax error.
    pub error: Option<Range<usize>>,
    /// Text to show on the row after the line's last row. While it shows,
    /// a suggestion takes one row.
    pub message: Option<&'a str>,
    pub rows: usize,
    pub cols: usize,
}

pub struct Output {
    pub bytes: Vec<u8>,
    /// The column the suggestion starts at, if one was drawn.
    pub suggestion_col: Option<usize>,
    /// How many rows below the cursor the message is, if one was drawn.
    pub message_rows: Option<usize>,
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

    // The message needs a row below the line, without pushing the line's
    // first row off the screen.
    let message = repaint
        .message
        .map(|text| fit(&expand_tabs(text, 0), cols.saturating_sub(1)).to_owned())
        .filter(|text| !text.is_empty() && end.0 + 2 <= repaint.rows);
    let suggestion_lines = if message.is_some() {
        1
    } else {
        repaint.suggestion_lines
    };

    let mut suggestion_col = None;
    if let Some(suggestion) = repaint
        .suggestion
        .filter(|_| repaint.point == repaint.line.len())
    {
        let rows = suggestion_rows(suggestion, cursor.1, end.0, suggestion_lines, repaint);
        if rows.iter().any(|row| !row.is_empty()) {
            let _ = write!(out, "\x1b[{}m", repaint.colors.suggestion());
            for (i, row) in rows.iter().enumerate() {
                if i > 0 {
                    out.extend_from_slice(b"\r\n");
                }
                out.extend_from_slice(row.as_bytes());
            }
            out.extend_from_slice(b"\x1b[0m");
            match rows.len() - 1 {
                0 => out.extend_from_slice(b"\x1b8"),
                // A line feed may have scrolled the screen, which leaves the
                // saved cursor position one row off: go back by rows instead.
                below => {
                    let _ = write!(out, "\x1b[{below}A\x1b[{}G", cursor.1 + 1);
                }
            }
            suggestion_col = Some(cursor.1);
        }
    }

    let mut message_rows = None;
    if let Some(text) = message {
        // A line feed from the last row scrolls the screen as needed; the
        // cursor then goes back up by rows, as after a multi-row suggestion.
        let below = end.0 - cursor.0 + 1;
        if below > 1 {
            let _ = write!(out, "\x1b[{}B", below - 1);
        }
        let _ = write!(out, "\r\n{text}\x1b[K\x1b[{below}A\x1b[{}G", cursor.1 + 1);
        message_rows = Some(below);
    }
    Some(Output {
        bytes: out,
        suggestion_col,
        message_rows,
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
    let mut style: (Option<Kind>, bool) = (None, false);
    let mut spans = repaint.spans.iter().peekable();
    for (i, c) in repaint.line.char_indices() {
        while spans.next_if(|s| s.end <= i).is_some() {}
        let kind = spans.peek().filter(|s| s.start <= i).map(|s| s.kind);
        let want = (kind, repaint.error.as_ref().is_some_and(|e| e.contains(&i)));
        if want != style {
            if style != (None, false) {
                out.extend_from_slice(b"\x1b[0m");
            }
            if let Some(kind) = want.0 {
                let _ = write!(out, "\x1b[{}m", repaint.colors.sgr(kind));
            }
            if want.1 {
                out.extend_from_slice(repaint.colors.error().as_bytes());
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
    if style != (None, false) {
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

/// The rows of grey text to draw for `suggestion`, the first starting at
/// column `col` on the command's last row `last_row`: at most `max_lines`
/// lines, only as many as fit on the screen with the command's rows above,
/// each cut to the screen's width. When lines are left out, the last row
/// says how many.
fn suggestion_rows(
    suggestion: &str,
    col: usize,
    last_row: usize,
    max_lines: usize,
    repaint: &Repaint,
) -> Vec<String> {
    let cols = repaint.cols;
    let lines: Vec<&str> = suggestion.split('\n').collect();
    let room = repaint.rows.saturating_sub(last_row).max(1);
    let shown = lines.len().min(max_lines.max(1)).min(room);
    let start_of = |i: usize| if i == 0 { col } else { 0 };
    // Keep the last column free so the terminal never wraps.
    let mut rows: Vec<String> = lines[..shown]
        .iter()
        .enumerate()
        .map(|(i, line)| {
            fit(
                &expand_tabs(line, start_of(i)),
                cols.saturating_sub(start_of(i) + 1),
            )
            .to_owned()
        })
        .collect();
    let hidden = lines.len() - shown;
    if hidden > 0
        && let Some(last) = rows.last_mut()
    {
        let used: usize =
            start_of(shown - 1) + last.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>();
        let note = format!(
            " … {hidden} more line{}",
            if hidden == 1 { "" } else { "s" }
        );
        last.push_str(fit(&note, cols.saturating_sub(used + 1)));
    }
    rows
}

/// `text` with each tab replaced by the spaces up to the next multiple of 8
/// columns, counting from `col`.
fn expand_tabs(text: &str, col: usize) -> String {
    let mut out = String::new();
    let mut col = col;
    for c in text.chars() {
        if c == '\t' {
            let spaces = 8 - col % 8;
            out.extend(std::iter::repeat_n(' ', spaces));
            col += spaces;
        } else {
            out.push(c);
            col += c.width().unwrap_or(0);
        }
    }
    out
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
            suggestion_lines: 5,
            error: None,
            message: None,
            rows: 24,
            cols: 80,
        }
    }

    fn text(out: &Output) -> String {
        String::from_utf8(out.bytes.clone()).unwrap()
    }

    #[test]
    fn underlines_the_error() {
        let colors = Colors::parse("error=4");
        let out = build(&Repaint {
            error: Some(5..6),
            ..repaint("echo )", 6, &[], &colors)
        })
        .unwrap();
        assert_eq!(text(&out), "\x1b7\r\x1b[2Cecho \x1b[4m)\x1b[0m\x1b8");
    }

    #[test]
    fn the_underline_goes_on_top_of_the_colour() {
        let colors = Colors::parse("error=4");
        let spans = [Span {
            start: 0,
            end: 2,
            kind: Kind::Command,
        }];
        let out = build(&Repaint {
            error: Some(0..2),
            ..repaint("fi x", 4, &spans, &colors)
        })
        .unwrap();
        assert_eq!(text(&out), "\x1b7\r\x1b[2C\x1b[32m\x1b[4mfi\x1b[0m x\x1b8");
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
    fn a_multi_line_suggestion_goes_back_up() {
        let colors = Colors::default();
        let out = build(&Repaint {
            suggestion: Some(" a b; do\n    echo\ndone"),
            ..repaint("for x in", 8, &[], &colors)
        })
        .unwrap();
        assert!(
            text(&out).ends_with("\x1b8\x1b[90m a b; do\r\n    echo\r\ndone\x1b[0m\x1b[2A\x1b[11G"),
            "{:?}",
            text(&out)
        );
        assert_eq!(out.suggestion_col, Some(10));
    }

    #[test]
    fn suggestion_lines_are_limited() {
        let colors = Colors::default();
        let out = build(&Repaint {
            suggestion: Some("1\n2\n3\n4"),
            suggestion_lines: 2,
            ..repaint("x", 1, &[], &colors)
        })
        .unwrap();
        assert!(text(&out).ends_with("\x1b[90m1\r\n2 … 2 more lines\x1b[0m\x1b[1A\x1b[4G"));
    }

    #[test]
    fn suggestion_rows_fit_on_the_screen() {
        let colors = Colors::default();
        let out = build(&Repaint {
            suggestion: Some("1\n2\n3"),
            rows: 1,
            ..repaint("x", 1, &[], &colors)
        })
        .unwrap();
        assert!(text(&out).ends_with("\x1b[90m1 … 2 more lines\x1b[0m\x1b8"));
    }

    #[test]
    fn tabs_in_a_suggestion_become_spaces() {
        let colors = Colors::default();
        let out = build(&Repaint {
            suggestion: Some("\tx"),
            ..repaint("echo", 4, &[], &colors)
        })
        .unwrap();
        assert!(text(&out).ends_with("\x1b[90m  x\x1b[0m\x1b8"));
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
    #[test]
    fn a_message_goes_on_the_row_below() {
        let colors = Colors::default();
        let out = build(&Repaint {
            message: Some("hello"),
            ..repaint("ab", 1, &[], &colors)
        })
        .unwrap();
        assert!(
            text(&out).ends_with("ab\x1b8\r\nhello\x1b[K\x1b[1A\x1b[4G"),
            "{:?}",
            text(&out)
        );
        assert_eq!(out.message_rows, Some(1));
        assert_eq!(out.suggestion_col, None);
    }

    #[test]
    fn a_message_leaves_the_suggestion_one_row() {
        let colors = Colors::default();
        let out = build(&Repaint {
            suggestion: Some(" a b; do\n    echo\ndone"),
            message: Some("hello"),
            ..repaint("for x in", 8, &[], &colors)
        })
        .unwrap();
        assert!(
            text(&out).ends_with(
                "\x1b8\x1b[90m a b; do … 2 more lines\x1b[0m\x1b8\r\nhello\x1b[K\x1b[1A\x1b[11G"
            ),
            "{:?}",
            text(&out)
        );
        assert_eq!(out.suggestion_col, Some(10));
        assert_eq!(out.message_rows, Some(1));
    }

    #[test]
    fn a_message_goes_below_the_last_row_of_a_wrapped_line() {
        let colors = Colors::default();
        let out = build(&Repaint {
            cols: 10,
            message: Some("hello"),
            ..repaint("echo 12345678", 0, &[], &colors)
        })
        .unwrap();
        assert!(
            text(&out).ends_with("\x1b8\x1b[1B\r\nhello\x1b[K\x1b[2A\x1b[3G"),
            "{:?}",
            text(&out)
        );
        assert_eq!(out.message_rows, Some(2));
    }

    #[test]
    fn a_message_is_one_row_cut_to_the_screen() {
        let colors = Colors::default();
        let out = build(&Repaint {
            cols: 10,
            message: Some("0123456789abc"),
            ..repaint("ab", 2, &[], &colors)
        })
        .unwrap();
        assert!(
            text(&out).contains("\r\n012345678\x1b[K"),
            "{:?}",
            text(&out)
        );
        let out = build(&Repaint {
            message: Some("one\ttab\x1b[31m"),
            ..repaint("ab", 2, &[], &colors)
        })
        .unwrap();
        assert!(
            text(&out).contains("\r\none     tab\x1b[K"),
            "{:?}",
            text(&out)
        );
    }

    #[test]
    fn no_message_without_a_row_for_it() {
        let colors = Colors::default();
        let line = "x".repeat(15);
        let out = build(&Repaint {
            rows: 2,
            cols: 10,
            message: Some("hello"),
            ..repaint(&line, 15, &[], &colors)
        })
        .unwrap();
        assert!(!text(&out).contains("hello"));
        assert_eq!(out.message_rows, None);
        let out = build(&Repaint {
            rows: 3,
            cols: 10,
            message: Some("hello"),
            ..repaint(&line, 15, &[], &colors)
        })
        .unwrap();
        assert_eq!(out.message_rows, Some(1));
    }
}
