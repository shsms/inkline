//! A registered command's arguments, as the program will receive them: quote
//! marks and escaping backslashes removed, with a map back to the bytes each
//! kept byte was typed as.

use std::ops::Range;

/// An argument after quote removal, with a map back to the line it was typed
/// on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arg {
    /// The bytes the program will receive.
    pub text: String,
    /// Set when the argument holds an expansion or a glob: `text` is the
    /// typed text unchanged, and quote removal was not attempted.
    pub raw: bool,
    /// `map[i]` is the byte of the line that byte `i` of `text` came from.
    /// The identity map when `raw`.
    pub map: Vec<usize>,
    /// The bytes of the line that are the quote marks bash removes, in
    /// order, for either kind of argument.
    pub quotes: Vec<usize>,
}

/// Which quotes, if any, `unquote` is inside as it walks the argument.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

/// Removes quotes and escaping backslashes from `line[range]` the way bash
/// does, and says whether the result holds an expansion or a glob (`raw`).
///
/// Single quotes keep everything inside as is. Double quotes keep
/// everything inside, except that a backslash before `$`, `` ` ``, `"`, `\`
/// or a newline is dropped (the newline goes with it). Outside quotes, a
/// backslash drops itself and keeps the next character; backslash-newline
/// drops both.
///
/// The result is `raw` — returned as the typed text, unchanged, with the
/// identity map — when, outside single quotes, it holds an unescaped `$` or
/// a backquote; or, outside all quotes, an unescaped glob (`*`, `?`, `[`) or
/// extglob pattern (`@(`, `+(`, `!(`), a leading `~` (or one right after the
/// first unquoted `=` of a word shaped like an assignment, or after an
/// unquoted `:` that follows it, e.g. `a=~/x`, `PATH=a:~/b` — but not
/// `a=b=~/x` or an escaped `:`), a process substitution (`<(`/`>(`), or a
/// `{…}` holding an unquoted `,` or `..` between its own `{` and `}`, at any
/// nesting depth.
pub fn unquote(line: &str, range: Range<usize>) -> Arg {
    let slice = &line[range.clone()];
    let base = range.start;
    let chars: Vec<(usize, char)> = slice.char_indices().collect();
    let assignment_looks = looks_like_assignment(&chars);

    let mut text = Vec::new();
    let mut map = Vec::new();
    let mut quotes = Vec::new();
    let mut raw = false;
    let mut quote = Quote::None;
    // One entry per unquoted `{` still open, innermost last: whether an
    // unquoted `,` or `..` was seen since it opened. A `}` pops its own
    // entry, so `{{a,b}` is raw (from the inner, matched pair) even though
    // the outer `{` never closes.
    let mut brace_stack: Vec<bool> = Vec::new();
    // Whether the character just placed in `text` was the assignment's
    // first unquoted `=`, or an unquoted `:` after it — the one spot after
    // which bash also tilde-expands. `seen_first_eq` keeps a later `=` from
    // counting.
    let mut tilde_marker = false;
    let mut seen_first_eq = false;

    let mut i = 0;
    while i < chars.len() {
        let (off, ch) = chars[i];
        let abs = base + off;
        // Whether *this* character is the tilde-expansion marker (the
        // assignment's first `=`, or a `:` after it) for the *next*
        // character's `~` check. Defaults to "no": every branch below that
        // does not explicitly set it — a quote mark, a quoted character, an
        // escape — clears it, so a `~` right after one of those (`a=''~/x`,
        // `a=\:~/x`) is not treated as following the marker.
        let mut next_tilde_marker = false;
        match quote {
            Quote::Single => {
                if ch == '\'' {
                    quotes.push(abs);
                    quote = Quote::None;
                } else {
                    push(&mut text, &mut map, ch, abs);
                }
                i += 1;
            }
            Quote::Double => {
                if ch == '"' {
                    quotes.push(abs);
                    quote = Quote::None;
                    i += 1;
                } else if ch == '\\' {
                    match chars.get(i + 1) {
                        Some(&(_, '\n')) => i += 2,
                        Some(&(noff, next @ ('$' | '`' | '"' | '\\'))) => {
                            push(&mut text, &mut map, next, base + noff);
                            i += 2;
                        }
                        _ => {
                            push(&mut text, &mut map, ch, abs);
                            i += 1;
                        }
                    }
                } else {
                    if ch == '$' || ch == '`' {
                        raw = true;
                    }
                    push(&mut text, &mut map, ch, abs);
                    i += 1;
                }
            }
            Quote::None => {
                if ch == '\'' {
                    quotes.push(abs);
                    quote = Quote::Single;
                    i += 1;
                } else if ch == '"' {
                    quotes.push(abs);
                    quote = Quote::Double;
                    i += 1;
                } else if ch == '\\' {
                    match chars.get(i + 1) {
                        Some(&(_, '\n')) => i += 2,
                        Some(&(noff, next)) => {
                            push(&mut text, &mut map, next, base + noff);
                            i += 2;
                        }
                        None => {
                            push(&mut text, &mut map, ch, abs);
                            i += 1;
                        }
                    }
                } else {
                    let next = chars.get(i + 1).map(|&(_, c)| c);
                    if ch == '$' || ch == '`' {
                        raw = true;
                    }
                    if ch == '*' || ch == '?' || ch == '[' {
                        raw = true;
                    }
                    if matches!(ch, '@' | '+' | '!') && next == Some('(') {
                        raw = true;
                    }
                    if ch == '~' && (off == 0 || tilde_marker) {
                        raw = true;
                    }
                    if matches!(ch, '<' | '>') && next == Some('(') {
                        raw = true;
                    }
                    let dotdot = ch == '.' && next == Some('.');
                    if ch == '{' {
                        brace_stack.push(false);
                    } else if ch == '}' {
                        if brace_stack.pop() == Some(true) {
                            raw = true;
                        }
                    } else if (ch == ',' || dotdot)
                        && let Some(marker) = brace_stack.last_mut()
                    {
                        *marker = true;
                    }

                    next_tilde_marker = assignment_looks
                        && ((ch == '=' && !seen_first_eq) || (ch == ':' && seen_first_eq));
                    seen_first_eq = seen_first_eq || (assignment_looks && ch == '=');

                    push(&mut text, &mut map, ch, abs);
                    i += 1;
                }
            }
        }
        tilde_marker = next_tilde_marker;
    }

    if raw {
        return Arg {
            text: slice.to_string(),
            raw: true,
            map: (base..base + slice.len()).collect(),
            quotes,
        };
    }
    Arg {
        text: String::from_utf8(text).expect("only whole chars were pushed"),
        raw: false,
        map,
        quotes,
    }
}

/// Appends `ch`'s bytes to `text`, and `abs` (the byte of the line the first
/// of them came from) plus each further byte's offset to `map`.
fn push(text: &mut Vec<u8>, map: &mut Vec<usize>, ch: char, abs: usize) {
    let mut buf = [0u8; 4];
    for (k, b) in ch.encode_utf8(&mut buf).bytes().enumerate() {
        text.push(b);
        map.push(abs + k);
    }
}

/// Whether `chars` starts with a shell identifier (a letter or `_`, then
/// letters, digits or `_`) followed by `=`, the shape bash requires before
/// it will tilde-expand after that `=` or after a later `:`.
fn looks_like_assignment(chars: &[(usize, char)]) -> bool {
    let mut chars = chars.iter().map(|&(_, c)| c);
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    for c in chars {
        match c {
            '=' => return true,
            c if c.is_ascii_alphanumeric() || c == '_' => {}
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn un(line: &str) -> Arg {
        unquote(line, 0..line.len())
    }

    #[test]
    fn quotes_come_off_and_the_map_points_back() {
        let a = un("'a b'");
        assert_eq!((a.text.as_str(), a.raw), ("a b", false));
        assert_eq!(a.map, [1, 2, 3]);
        let a = un(r#""x\"y\\z\q""#);
        assert_eq!(a.text, r#"x"y\z\q"#);
        assert_eq!(a.map, [1, 3, 4, 6, 7, 8, 9]);
        let a = un(r#"'a'"b"c\ d"#);
        assert_eq!(a.text, "abc d");
        assert_eq!(a.map, [1, 4, 6, 8, 9]);
        let a = un("\"a\\\nb\"");
        assert_eq!(
            (a.text.as_str(), a.map.as_slice()),
            ("ab", [1usize, 4].as_slice())
        );
        let a = un("'é|x'");
        assert_eq!(a.text, "é|x");
        assert_eq!(a.map, [1, 2, 3, 4]);
    }

    #[test]
    fn the_quote_marks_are_noted() {
        assert_eq!(un(r#"'a'"b"c\ d"#).quotes, [0, 2, 3, 5]);
        assert_eq!(un(r#""a\"b""#).quotes, [0, 5], "not an escaped one");
        assert_eq!(un(r#""a $x" 'b'"#).quotes, [0, 5, 7, 9], "raw too");
        assert!(un("a").quotes.is_empty());
    }

    #[test]
    fn expansions_make_an_argument_raw() {
        for typed in [
            "\"a $x\"",
            "$x",
            "`date`",
            "\"$(date)\"",
            "$'a'",
            "*.csv",
            "a?",
            "[ab]",
            "~/x",
            "a{b,c}",
            "{1..3}",
            "{a,{b}}",
            "{{a,b}",
            "x{y{a,b}",
            "a=~/x",
            "PATH=a:~/b",
            "@(a|b)",
            "+(a)",
            "!(a)",
        ] {
            let a = un(typed);
            assert!(a.raw, "{typed}");
            assert_eq!(a.text, typed);
            assert_eq!(a.map, (0..typed.len()).collect::<Vec<_>>());
        }
        for typed in [
            "'$x'",
            "'*'",
            "\\$x",
            "a~b",
            "{a}",
            "\"*\"",
            "a=b~c",
            "!x",
            "@x",
            "+x",
            "a=b=~/x",
            "a=b\\:~/x",
            "a=\\:~/x",
            "a=''~/x",
            "a=\"b\"~/x",
            "a=\\x~/y",
        ] {
            assert!(!un(typed).raw, "{typed}");
        }
    }
}
