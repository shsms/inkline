//! Where the cursor goes in a command that spans several lines.

use std::ops::Range;

use unicode_width::UnicodeWidthChar;

/// The start of the line `point` is on.
pub fn line_start(text: &str, point: usize) -> usize {
    text[..point].rfind('\n').map_or(0, |i| i + 1)
}

/// The end of the line `point` is on: its newline, or the end of `text`.
pub fn line_end(text: &str, point: usize) -> usize {
    text[point..].find('\n').map_or(text.len(), |i| point + i)
}

/// The screen column of `point`, counting from where its line starts on
/// screen: after the prompt (`prompt_width` columns) on the first line, at the
/// left edge on the others. Tabs go to the next multiple of 8, as readline
/// draws them. A line longer than the screen is counted as one long row.
pub fn column(text: &str, point: usize, prompt_width: usize) -> usize {
    let start = line_start(text, point);
    let offset = if start == 0 { prompt_width } else { 0 };
    text[start..point].chars().fold(offset, advance)
}

/// How many columns `text` takes from the left edge.
pub fn width(text: &str) -> usize {
    text.chars().fold(0, advance)
}

fn advance(col: usize, c: char) -> usize {
    if c == '\t' {
        col + 8 - col % 8
    } else {
        col + c.width().unwrap_or(0)
    }
}

/// The point on the line starting at `start` that is at column `goal`, or
/// before the character that covers it; the line's end if it is shorter.
fn at_column(text: &str, start: usize, goal: usize, prompt_width: usize) -> usize {
    let end = line_end(text, start);
    let mut col = if start == 0 { prompt_width } else { 0 };
    for (i, c) in text[start..end].char_indices() {
        let next = advance(col, c);
        if next > goal {
            return start + i;
        }
        col = next;
    }
    end
}

/// The point one line up, at column `goal`, or None on the first line.
pub fn up(text: &str, point: usize, goal: usize, prompt_width: usize) -> Option<usize> {
    let start = line_start(text, point);
    (start > 0).then(|| at_column(text, line_start(text, start - 1), goal, prompt_width))
}

/// The point one line down, at column `goal`, or None on the last line.
pub fn down(text: &str, point: usize, goal: usize, prompt_width: usize) -> Option<usize> {
    let end = line_end(text, point);
    (end < text.len()).then(|| at_column(text, end + 1, goal, prompt_width))
}

/// What `C-k` kills: to the end of the line, or at its end, the newline.
pub fn kill_forward(text: &str, point: usize) -> Range<usize> {
    let end = line_end(text, point);
    if end == point && end < text.len() {
        point..point + 1
    } else {
        point..end
    }
}

/// What `C-u` kills: back to the start of the line, or at its start, the
/// newline before it.
pub fn kill_backward(text: &str, point: usize) -> Range<usize> {
    let start = line_start(text, point);
    if start == point && start > 0 {
        start - 1..point
    } else {
        start..point
    }
}

/// The spaces and tabs around `point` on its line, which a new line at
/// `point` drops. A blank right after a backslash is part of a word, so it
/// stays.
pub fn blanks_around(text: &str, point: usize) -> Range<usize> {
    let before = &text[line_start(text, point)..point];
    let kept = before.trim_end_matches([' ', '\t']);
    let mut start = point - (before.len() - kept.len());
    let backslashes = kept.bytes().rev().take_while(|&b| b == b'\\').count();
    if start < point && backslashes % 2 == 1 {
        start += 1;
    }
    let after = &text[point..line_end(text, point)];
    let end = point + (after.len() - after.trim_start_matches([' ', '\t']).len());
    start..end
}

/// `text` with `begin` put in front of every line; with `toggle`, taken off
/// every line instead when they all start with it.
pub fn comment(text: &str, begin: &str, toggle: bool) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let strip = toggle && lines.iter().all(|l| l.starts_with(begin));
    lines
        .iter()
        .map(|l| {
            if strip {
                l[begin.len()..].to_owned()
            } else {
                format!("{begin}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "echo abcdefghij\nx\necho 12345678";

    #[test]
    fn width_from_the_left_edge() {
        assert_eq!(width(""), 0);
        assert_eq!(width("    "), 4);
        assert_eq!(width("\t"), 8);
        assert_eq!(width("  \t "), 9);
    }

    #[test]
    fn line_edges() {
        assert_eq!(line_start(TEXT, 16), 16);
        assert_eq!(line_start(TEXT, 17), 16);
        assert_eq!(line_end(TEXT, 16), 17);
        assert_eq!(line_end(TEXT, 18), TEXT.len());
        assert_eq!(line_start("ls", 2), 0);
    }

    #[test]
    fn columns_count_the_prompt_on_the_first_line_only() {
        assert_eq!(column(TEXT, 4, 2), 6);
        assert_eq!(column(TEXT, 22, 2), 4);
        assert_eq!(column("a\tb", 2, 2), 8);
        assert_eq!(column("日本", 6, 0), 4);
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let end = TEXT.len();
        // Column 13 on the last line; the middle line is shorter.
        assert_eq!(up(TEXT, end, 13, 2), Some(17));
        assert_eq!(up(TEXT, 17, 13, 2), Some(11));
        assert_eq!(up(TEXT, 11, 13, 2), None);
        assert_eq!(down(TEXT, 11, 13, 2), Some(17));
        assert_eq!(down(TEXT, 17, 13, 2), Some(31));
        assert_eq!(down(TEXT, 31, 13, 2), None);
    }

    #[test]
    fn a_column_inside_a_wide_character_goes_before_it() {
        assert_eq!(up("日本\nabc", 7, 3, 0), Some(3));
    }

    #[test]
    fn kills_stop_at_the_line_or_take_the_newline() {
        assert_eq!(kill_forward("ab\ncd", 1), 1..2);
        assert_eq!(kill_forward("ab\ncd", 2), 2..3);
        assert_eq!(kill_forward("ab\ncd", 5), 5..5);
        assert_eq!(kill_backward("ab\ncd", 4), 3..4);
        assert_eq!(kill_backward("ab\ncd", 3), 2..3);
        assert_eq!(kill_backward("ab\ncd", 0), 0..0);
    }

    #[test]
    fn blanks_around_the_point_on_its_line() {
        assert_eq!(blanks_around("then   \n  x", 5), 4..7);
        assert_eq!(blanks_around("a  \t b", 3), 1..5);
        assert_eq!(blanks_around("a\n  b", 2), 2..4);
        assert_eq!(blanks_around("ab", 1), 1..1);
        assert_eq!(blanks_around("a\\  ", 4), 3..4);
        assert_eq!(blanks_around("a\\\\  ", 5), 3..5);
    }

    #[test]
    fn comment_every_line() {
        assert_eq!(comment("a\nb", "#", false), "#a\n#b");
        assert_eq!(comment("#a\n#b", "#", true), "a\nb");
        assert_eq!(comment("#a\nb", "#", true), "##a\n#b");
        assert_eq!(comment("#a\n#b", "#", false), "##a\n##b");
    }
}
