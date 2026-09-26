//! Where the lines of a multi-line command start: the indentation for a new
//! line, closing words that move a line back out, and the indentation inside
//! a program's script from the depths its helper gives.

use crate::mode_server::protocol::Depths;

/// Words that close a block. A line starting with one moves back one step.
const CLOSERS: &[&str] = &["done", "fi", "esac", "}", ")", "else", "elif"];

/// Operators that continue a command on the next line.
const CONTINUERS: &[&str] = &["|", "|&", "&&", "||", "\\"];

/// The indentation for a new line inserted at `point`: that of the line the
/// cursor is on, one step more after a word that opens a block or an
/// operator that continues the command, one step less after the last line of
/// a continued command or a `;;` that ends a case's body.
pub fn for_new_line(text: &str, point: usize, step: usize) -> String {
    let start = text[..point].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[start..point];
    let previous = start
        .checked_sub(1)
        .map(|end| &text[text[..end].rfind('\n').map_or(0, |i| i + 1)..end]);
    let continued = previous.is_some_and(|p| continues(&tokens(p)));
    let toks = tokens(line);
    let base = indentation(line);
    let deeper = || format!("{base}{}", " ".repeat(step));
    if continues(&toks) {
        if continued { base.to_owned() } else { deeper() }
    } else if opens(&toks) {
        deeper()
    } else if (toks.last().is_some_and(|t| t.text == ";;") && !has_pattern(&toks)) || continued {
        shallower(base, step)
    } else {
        base.to_owned()
    }
}

/// How many bytes to remove from the start of `line` because it starts with
/// a word that closes a block: up to one step of spaces, or one tab.
pub fn outdent(line: &str, step: usize) -> usize {
    let toks = tokens(line);
    let Some(first) = toks.first() else { return 0 };
    if !first.command || !CLOSERS.contains(&first.text) {
        return 0;
    }
    let indent = indentation(line);
    if indent.starts_with('\t') {
        1
    } else {
        indent.bytes().take_while(|&b| b == b' ').count().min(step)
    }
}

/// The spaces and tabs `line` starts with.
pub fn indentation(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// The most nesting levels a helper's depth counts for: a larger depth is
/// taken as this, so a wrong reply cannot make a huge line.
pub const MOST_DEPTH: usize = 20;

/// The indentation of a line at `depth` inside a program's script: `base`,
/// the indentation of the line the command's name is on, then one `step`
/// of spaces for being inside the script and one per level of `depth`.
pub fn for_depth(base: &str, depth: usize, step: usize) -> String {
    format!("{base}{}", " ".repeat((1 + depth.min(MOST_DEPTH)) * step))
}

/// How a new line inside a program's script is indented.
#[derive(Debug, PartialEq, Eq)]
pub struct ScriptLine {
    /// The indentation the cursor's line moves out to, when it moves.
    pub moved_out: Option<String>,
    /// The new line's indentation.
    pub new_line: String,
}

/// How a new line at `point` in `text` is indented, inside a script whose
/// command's name is on the line that starts at `command_line` and whose
/// argument starts on the line that starts at `first_line` (the script's
/// first line). With the helper's `depths`, the new line gets `NEW`'s
/// indentation; the cursor's line moves out to `CURRENT`'s when the cursor
/// is past its first non-blank character, it is not the script's first
/// line, and that is less far in than it is now (a line is never pushed
/// further in). Without depths nothing moves, and the new line gets depth
/// 0's indentation when the cursor is on the script's first line, and the
/// cursor's line's own indentation below it.
pub fn script_line(
    text: &str,
    point: usize,
    command_line: usize,
    first_line: usize,
    depths: Option<Depths>,
    step: usize,
) -> ScriptLine {
    let start = crate::lines::line_start(text, point);
    let present = indentation(&text[start..]);
    let base = indentation(&text[command_line..]);
    let Some(depths) = depths else {
        let new_line = if start == first_line {
            for_depth(base, 0, step)
        } else {
            present.to_owned()
        };
        return ScriptLine {
            moved_out: None,
            new_line,
        };
    };
    let current = for_depth(base, depths.current, step);
    let moves = start != first_line
        && point > start + present.len()
        && crate::lines::width(&current) < crate::lines::width(present);
    ScriptLine {
        moved_out: moves.then_some(current),
        new_line: for_depth(base, depths.new, step),
    }
}

/// `base` less one step: up to `step` spaces, or one tab, from its end.
fn shallower(base: &str, step: usize) -> String {
    if let Some(stripped) = base.strip_suffix('\t') {
        return stripped.to_owned();
    }
    let spaces = base.len() - base.trim_end_matches(' ').len();
    base[..base.len() - spaces.min(step)].to_owned()
}

/// A word or operator of a line, and whether it is where a command starts
/// (so a keyword counts as one).
struct Token<'a> {
    text: &'a str,
    command: bool,
}

/// Whether the line's last token continues the command on the next line.
fn continues(toks: &[Token]) -> bool {
    toks.last().is_some_and(|t| CONTINUERS.contains(&t.text))
}

/// Whether the line's last token opens a block.
fn opens(toks: &[Token]) -> bool {
    let Some(last) = toks.last() else {
        return false;
    };
    (last.command && ["do", "then", "else"].contains(&last.text))
        || ["{", "("].contains(&last.text)
        || (last.text == "in" && toks.iter().any(|t| t.command && t.text == "case"))
        || (last.text == ")" && has_pattern(toks))
}

/// Whether the line has a `)` with no `(` before it on the line: the end of
/// a case pattern such as `a|b)`. The `)`s that start the line close lists or
/// subshells, since bash wants a pattern's `)` on the pattern's line.
fn has_pattern(toks: &[Token]) -> bool {
    let leading = toks.iter().take_while(|t| t.text == ")").count();
    let mut depth = 0i32;
    for t in &toks[leading..] {
        match t.text {
            "(" => depth += 1,
            ")" if depth == 0 => return true,
            ")" => depth -= 1,
            _ => {}
        }
    }
    false
}

/// The words and operators of `line` outside quotes and comments. `$(`,
/// `<(` and `>(` count as `(`. A backslash at the end is a token of its own.
fn tokens(line: &str) -> Vec<Token<'_>> {
    const STARTS_COMMAND: &[&str] = &[
        ";", "&", "&&", "||", "|", "|&", "(", "{", "do", "then", "else", "elif", "if", "while",
        "until", "!",
    ];
    let bytes = line.as_bytes();
    let mut toks: Vec<Token> = Vec::new();
    let mut i = 0;
    let mut command = true;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b' ' || b == b'\t' {
            i += 1;
            continue;
        }
        if b == b'#' {
            break;
        }
        let start = i;
        let op = ["&&", "||", "|&", ";;", "$(", "<(", ">("]
            .iter()
            .find(|op| line[i..].starts_with(**op))
            .copied()
            .or_else(|| {
                [";", "&", "|", "(", ")"]
                    .iter()
                    .find(|op| line[i..].starts_with(**op))
                    .copied()
            });
        let text = if let Some(op) = op {
            i += op.len();
            if op.ends_with('(') { "(" } else { op }
        } else if b == b'\\' && i + 1 == bytes.len() {
            i += 1;
            "\\"
        } else {
            while i < bytes.len() && !b" \t;&|()".contains(&bytes[i]) {
                match bytes[i] {
                    b'\\' => i += 2,
                    q @ (b'\'' | b'"') => {
                        i += 1;
                        while i < bytes.len() && bytes[i] != q {
                            i += if q == b'"' && bytes[i] == b'\\' { 2 } else { 1 };
                        }
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
            // A backslash or quote can skip into the middle of a character.
            i = i.min(bytes.len());
            while !line.is_char_boundary(i) {
                i += 1;
            }
            &line[start..i]
        };
        toks.push(Token { text, command });
        command = STARTS_COMMAND.contains(&text);
    }
    toks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(text: &str) -> String {
        for_new_line(text, text.len(), 4)
    }

    fn depths(new: usize, current: usize) -> Option<Depths> {
        Some(Depths { new, current })
    }

    #[test]
    fn depths_become_spaces() {
        assert_eq!(for_depth("", 0, 2), "  ");
        assert_eq!(for_depth("", 2, 2), "      ");
        assert_eq!(
            for_depth("    ", 0, 4),
            "        ",
            "from the command's line"
        );
        assert_eq!(for_depth("\t", 1, 2), "\t    ", "a tab stays");
    }

    #[test]
    fn for_depth_is_capped() {
        assert_eq!(for_depth("", 100_000, 2).len(), (1 + MOST_DEPTH) * 2);
    }

    #[test]
    fn the_new_line_gets_the_new_depth() {
        let text = "  csvm \"head";
        let got = script_line(text, text.len(), 0, 0, depths(0, 0), 2);
        assert_eq!(
            got,
            ScriptLine {
                moved_out: None,
                new_line: "    ".into()
            }
        );
    }

    #[test]
    fn a_closing_line_moves_out() {
        let text = "csvm \"a (\n    b\n    ) c";
        let got = script_line(text, text.len(), 0, 0, depths(0, 0), 2);
        assert_eq!(
            got,
            ScriptLine {
                moved_out: Some("  ".into()),
                new_line: "  ".into()
            }
        );
    }

    #[test]
    fn a_line_is_never_pushed_in() {
        let text = "csvm \"a\n  b";
        let got = script_line(text, text.len(), 0, 0, depths(3, 3), 2);
        assert_eq!(got.moved_out, None);
        assert_eq!(got.new_line, "        ");
    }

    #[test]
    fn a_cursor_in_the_indentation_moves_nothing() {
        let text = "csvm \"a (\n    ) c";
        let got = script_line(text, 12, 0, 0, depths(0, 0), 2);
        assert_eq!(got.moved_out, None);
    }

    #[test]
    fn without_depths_the_cursors_line_is_copied() {
        let text = "csvm \"a\n   b";
        let got = script_line(text, text.len(), 0, 0, None, 2);
        assert_eq!(
            got,
            ScriptLine {
                moved_out: None,
                new_line: "   ".into()
            }
        );
        let text = "  x\n  csvm \"a\n b";
        let got = script_line(text, text.len(), 4, 4, None, 2);
        assert_eq!(
            got.new_line, " ",
            "below the command's line, even less far in"
        );
    }

    #[test]
    fn without_depths_the_first_line_is_one_step_in() {
        let got = script_line("  csvm \"a", 9, 0, 0, None, 2);
        assert_eq!(
            got,
            ScriptLine {
                moved_out: None,
                new_line: "    ".into()
            }
        );
        let text = "if x; then\n\tcsvm \"a";
        let got = script_line(text, text.len(), 11, 11, None, 2);
        assert_eq!(got.new_line, "\t  ", "from the command's own line");
    }

    /// A script whose quote opens on a line below the command's name.
    const BELOW: &str = "  csvm \\\n      \"head x";

    #[test]
    fn the_scripts_first_line_is_where_its_argument_starts() {
        let first = BELOW.find('\n').unwrap() + 1;
        let got = script_line(BELOW, BELOW.len(), 0, first, depths(0, 0), 2);
        assert_eq!(
            got,
            ScriptLine {
                moved_out: None,
                new_line: "    ".into()
            },
            "never moved, and depths count from the command's line"
        );
        let got = script_line(BELOW, BELOW.len(), 0, first, None, 2);
        assert_eq!(
            got,
            ScriptLine {
                moved_out: None,
                new_line: "    ".into()
            },
            "without depths, one step in from the command's line"
        );
    }

    #[test]
    fn after_a_block_opener() {
        assert_eq!(new("for x in a b; do"), "    ");
        assert_eq!(new("if true; then"), "    ");
        assert_eq!(new("if true\nthen"), "    ");
        assert_eq!(new("if a; then\n    b\nelse"), "    ");
        assert_eq!(new("f() {"), "    ");
        assert_eq!(new("x=("), "    ");
        assert_eq!(new("echo $("), "    ");
        assert_eq!(new("\tfor x in a; do"), "\t    ");
    }

    #[test]
    fn inside_a_block() {
        assert_eq!(new("for x in a; do\n    echo $x"), "    ");
        assert_eq!(new("echo do"), "");
        assert_eq!(new("echo 'do"), "");
        assert_eq!(new("ls # then"), "");
    }

    #[test]
    fn case_items() {
        assert_eq!(new("case $x in"), "    ");
        assert_eq!(new("case $x in\n    a|b)"), "        ");
        assert_eq!(new("case $x in\n    a)\n        echo a;;"), "    ");
        assert_eq!(new("case $x in\n    a) echo a;;"), "    ");
    }

    #[test]
    fn continued_commands() {
        assert_eq!(new("ls |"), "    ");
        assert_eq!(new("ls |\n    grep x |"), "    ");
        assert_eq!(new("if x; then\n    ls |\n        grep x"), "    ");
        assert_eq!(new("echo a \\"), "    ");
        assert_eq!(new("true &&"), "    ");
    }

    #[test]
    fn after_a_closing_parenthesis() {
        assert_eq!(new("if x; then\n    a=(\n        x\n    )"), "    ");
        assert_eq!(new("if x; then\n    (\n        cd /\n    )"), "    ");
        assert_eq!(new("if x; then\n    x=$(\n        ls\n    )"), "    ");
        assert_eq!(new("if x; then\n\ta=(\n\t    x\n\t)"), "\t");
        assert_eq!(new("case $x in\n    *)"), "        ");
        assert_eq!(new("if x; then\n    y=$((\n        1\n    ))"), "    ");
        assert_eq!(new("case $x in\n    a) echo\n    ) ;; b)"), "        ");
    }

    #[test]
    fn at_the_cursor_and_with_other_steps() {
        assert_eq!(for_new_line("for x in a; do\n    echo", 14, 4), "    ");
        assert_eq!(for_new_line("for x in a; do", 14, 2), "  ");
    }

    #[test]
    fn closing_words_move_out() {
        assert_eq!(outdent("    done", 4), 4);
        assert_eq!(outdent("  fi", 4), 2);
        assert_eq!(outdent("    esac", 2), 2);
        assert_eq!(outdent("\t}", 4), 1);
        assert_eq!(outdent("    elif x; then", 4), 4);
        assert_eq!(outdent("done", 4), 0);
        assert_eq!(outdent("    echo done", 4), 0);
        assert_eq!(outdent("    then", 4), 0);
        assert_eq!(outdent("    donex", 4), 0);
    }

    #[test]
    fn escapes_before_wide_characters() {
        assert_eq!(new("echo \\日本 do"), "");
        assert_eq!(new("echo \"\\日\" x; do"), "    ");
    }
}
