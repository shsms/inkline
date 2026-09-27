//! Completion items from a mode server, placed on the line: their byte
//! ranges mapped from the argument's text to the line, and their text
//! quoted so that bash still gives the program exactly that text.

use crate::args::{Arg, unquote};
use crate::menu::{Item, Source};
use crate::mode_server::protocol::ReplyItem;

/// The quoting at a place in an argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Quoting {
    None,
    Single,
    Double,
}

/// The items of `items` (for the argument `arg` of `line`, with the cursor
/// at line byte `point`) as menu items: each range mapped to the line, and
/// each text quoted for the quoting there. An end of a range at the
/// cursor's offset maps to the cursor. A range that starts on an escaped
/// character starts at its backslash, so the backslash goes with it.
///
/// An item is left out when its range starts or ends inside a character
/// of the line; when it ends at a cursor that sits between a backslash and
/// the character it escapes; when it starts and ends in different quoting
/// or holds a quote mark; when `quoted` cannot write its text there; or
/// when the line with the item in place would not give the program exactly
/// the item's text (see `reads_back`). In a `raw` argument the text goes in
/// as it is.
pub fn place(line: &str, point: usize, arg: &Arg, items: &[ReplyItem]) -> Vec<Item> {
    let offset = arg.offset_at(point);
    items
        .iter()
        .filter_map(|item| {
            let start = if item.start == offset {
                point
            } else {
                *arg.map.get(item.start)?
            };
            let end = if item.end == offset {
                point
            } else {
                arg.map.get(item.end.checked_sub(1)?)? + 1
            };
            if start > end || !line.is_char_boundary(start) || !line.is_char_boundary(end) {
                return None;
            }
            let (start, text) = if arg.raw {
                (start, item.text.clone())
            } else {
                let start = with_backslash(line, arg, start);
                // Only an end at the cursor can come right after a removed
                // backslash. Ending there would put the item between the
                // backslash and its character, and ending before the
                // backslash would not reach the cursor.
                if with_backslash(line, arg, end) != end {
                    return None;
                }
                let quoting = quoting_at(line, arg, start);
                if quoting != quoting_at(line, arg, end)
                    || arg.quotes.iter().any(|&q| start <= q && q < end)
                {
                    return None;
                }
                let text = quoted(quoting, &item.text)?;
                if !reads_back(line, arg, item, (start, end), &text)
                    || starts_history(line, start, quoting, &text)
                {
                    return None;
                }
                (start, text)
            };
            Some(Item {
                text,
                start,
                end,
                source: Source::Mode,
                note: item.note.clone(),
            })
        })
        .collect()
}

/// Whether bash, reading `line` with `text` in place of its bytes
/// `start..end`, gives the program the text of `arg` with `item` in place:
/// the argument read again is not `raw`, its text is exactly that, and it
/// is not a word bash takes as part of a redirection (`2` or `{fd}` right
/// before a typed `<` or `>`). The text around the item can change how
/// bash reads it, as a typed `{}` around `a,b`, or a typed `~` after `a=`.
fn reads_back(
    line: &str,
    arg: &Arg,
    item: &ReplyItem,
    (start, end): (usize, usize),
    text: &str,
) -> bool {
    let (Some(before), Some(after)) = (arg.text.get(..item.start), arg.text.get(item.end..)) else {
        return false;
    };
    let wanted = format!("{before}{}{after}", item.text);
    // The argument as typed, a backslash that escapes its first character
    // included.
    let from = with_backslash(line, arg, arg.start().unwrap_or(start)).min(start);
    let to = arg.end().unwrap_or(end).max(end);
    let placed = format!("{}{text}{}", &line[..start], &line[end..]);
    let to = to - (end - start) + text.len();
    let again = unquote(&placed, from..to);
    !again.raw && again.text == wanted && !redirects(&placed[from..to], &placed[to..])
}

/// Whether bash reads `word`, typed right before `rest`, as the number or
/// `{NAME}` of a redirection: `2>out`, `{fd}<in`. A backslash-newline
/// between them is joined away first.
fn redirects(word: &str, rest: &str) -> bool {
    let number = !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit());
    let name = word
        .strip_prefix('{')
        .and_then(|w| w.strip_suffix('}'))
        .is_some_and(|name| {
            name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    (number || name) && rest.trim_start_matches("\\\n").starts_with(['<', '>'])
}

/// Whether a `!` typed right before line byte `start` would, with `text`
/// after it, start a history expansion: the `!` has no backslash before it
/// and is not inside single quotes, and `text` does not start with `=` or,
/// inside double quotes, with a blank.
fn starts_history(line: &str, start: usize, quoting: Quoting, text: &str) -> bool {
    let bytes = line.as_bytes();
    let Some(bang) = start.checked_sub(1) else {
        return false;
    };
    let escaped = bang.checked_sub(1).is_some_and(|b| bytes[b] == b'\\');
    let quiet =
        text.starts_with('=') || quoting == Quoting::Double && text.starts_with([' ', '\t']);
    bytes[bang] == b'!' && quoting != Quoting::Single && !escaped && !quiet
}

/// `pos`, or the byte before it when that is a backslash bash drops: the
/// backslash that escapes the character at `pos`. A backslash right before
/// one of `arg`'s characters is part of the same word, so it is one of
/// `arg`'s bytes, dropped when it is neither kept nor a quote mark.
fn with_backslash(line: &str, arg: &Arg, pos: usize) -> usize {
    let Some(before) = pos.checked_sub(1) else {
        return pos;
    };
    let dropped = line.as_bytes().get(before) == Some(&b'\\')
        && arg.map.binary_search(&before).is_err()
        && arg.quotes.binary_search(&before).is_err();
    if dropped { before } else { pos }
}

/// The quoting at line byte `pos` of `arg`: inside the pair of quote marks
/// `(open, close)` that has `open < pos <= close` (an unclosed one runs to
/// the end of the line), else none. `unquote` notes quote marks in order,
/// each opening mark followed by its closing one.
fn quoting_at(line: &str, arg: &Arg, pos: usize) -> Quoting {
    for pair in arg.quotes.chunks(2) {
        let open = pair[0];
        let close = pair.get(1).copied().unwrap_or(usize::MAX);
        if open < pos && pos <= close {
            return match line.as_bytes().get(open) {
                Some(b'\'') => Quoting::Single,
                Some(_) => Quoting::Double,
                None => Quoting::None,
            };
        }
    }
    Quoting::None
}

/// `text` written so that bash gives it back unchanged in `quoting`: as it
/// is inside single quotes (None when it holds `'`); with a backslash
/// before `"`, `\`, `$` and `` ` `` inside double quotes (None when it
/// holds `!`, which history expansion would change and no quoting inside
/// double quotes keeps); with a backslash before a blank and each
/// character bash treats specially outside quotes.
fn quoted(quoting: Quoting, text: &str) -> Option<String> {
    let special: &[char] = match quoting {
        Quoting::Single => return (!text.contains('\'')).then(|| text.to_owned()),
        Quoting::Double if text.contains('!') => return None,
        Quoting::Double => &['"', '\\', '$', '`'],
        Quoting::None => &[
            ' ', '\t', '\'', '"', '\\', '$', '`', '|', '&', ';', '(', ')', '<', '>', '*', '?', '[',
            ']', '#', '~', '{', '}', '!',
        ],
    };
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if special.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::unquote;
    use crate::mode_server::protocol::ReplyItem;

    fn reply(start: usize, end: usize, text: &str) -> ReplyItem {
        ReplyItem {
            start,
            end,
            text: text.into(),
            note: Some("n".into()),
        }
    }

    /// The line after taking each placed item, for the argument typed over
    /// `arg_range` of `line`, with the cursor at `point`.
    fn taken(
        line: &str,
        arg_range: std::ops::Range<usize>,
        point: usize,
        items: &[ReplyItem],
    ) -> Vec<String> {
        let arg = unquote(line, arg_range);
        place(line, point, &arg, items)
            .iter()
            .map(|i| format!("{}{}{}", &line[..i.start], i.text, &line[i.end..]))
            .collect()
    }

    #[test]
    fn inside_single_quotes_an_item_goes_in_as_it_is() {
        // csvm 'sort am' data.csv ; the argument is bytes 5..14, its text
        // "sort am".
        let line = "csvm 'sort am' data.csv";
        let got = taken(
            line,
            5..14,
            13,
            &[
                reply(5, 7, "amount"),
                reply(5, 7, "first name"),
                reply(5, 7, "it's"),
            ],
        );
        assert_eq!(
            got,
            [
                "csvm 'sort amount' data.csv",
                "csvm 'sort first name' data.csv"
            ]
        );
    }

    #[test]
    fn inside_double_quotes_the_special_characters_are_escaped() {
        let line = r#"csvm "sort am" x"#;
        let got = taken(
            line,
            5..14,
            13,
            &[reply(5, 7, r#"a"b$c`d\e"#), reply(5, 7, "it's")],
        );
        assert_eq!(
            got,
            [r#"csvm "sort a\"b\$c\`d\\e" x"#, r#"csvm "sort it's" x"#]
        );
    }

    #[test]
    fn outside_quotes_the_special_characters_are_escaped() {
        // ls am|  (argument 3..5)
        let line = "ls am";
        let got = taken(
            line,
            3..5,
            5,
            &[reply(0, 2, "first name"), reply(0, 2, "it's")],
        );
        assert_eq!(got, [r"ls first\ name", r"ls it\'s"]);
    }

    #[test]
    fn an_item_that_crosses_a_quote_mark_is_left_out() {
        // csvm so'rt a'  : text "sort a", cursor after "sort a" (line byte 12)
        let line = "csvm so'rt a' x";
        let arg = unquote(line, 5..13);
        // Replaces the whole text, which starts outside the quotes and ends
        // inside.
        let placed = place(line, 12, &arg, &[reply(0, 6, "sort b")]);
        assert!(placed.is_empty());
    }

    #[test]
    fn an_item_that_holds_a_quote_mark_is_left_out() {
        // 'a''b' : both ends are inside single quotes, with quote marks
        // between them.
        let line = "x 'a''b'";
        let arg = unquote(line, 2..8);
        let placed = place(line, 7, &arg, &[reply(0, 2, "ab")]);
        assert!(placed.is_empty());
    }

    #[test]
    fn an_empty_word_at_the_cursor_is_an_insertion() {
        let line = "csvm 'sort ' x";
        let arg = unquote(line, 5..12);
        // The cursor is before the closing quote, at byte 11.
        let placed = place(line, 11, &arg, &[reply(5, 5, "amount")]);
        assert_eq!(
            (placed[0].start, placed[0].end, placed[0].text.as_str()),
            (11, 11, "amount")
        );
        assert_eq!(placed[0].source, crate::menu::Source::Mode);
        assert_eq!(placed[0].note.as_deref(), Some("n"));
    }

    #[test]
    fn a_raw_argument_takes_the_item_as_it_is() {
        let line = r#"csvm "sort $x am" f"#;
        let arg = unquote(line, 5..17);
        assert!(arg.raw);
        // Offsets count the typed text, quote marks included: `am` is 9..11.
        let placed = place(line, 16, &arg, &[reply(9, 11, "amount")]);
        assert_eq!(
            (placed[0].start, placed[0].end, placed[0].text.as_str()),
            (14, 16, "amount")
        );
    }

    #[test]
    fn a_range_on_an_escaped_character_starts_at_its_backslash() {
        // ls \$fo| : the program gets `$fo`; the backslash is replaced too,
        // so it is not left before the item's own.
        assert_eq!(
            taken(r"ls \$fo", 3..7, 7, &[reply(0, 3, "$foo")]),
            [r"ls \$foo"]
        );
        // echo "a\$fo|" : the same inside double quotes.
        assert_eq!(
            taken(r#"echo "a\$fo""#, 5..12, 11, &[reply(1, 4, "$foo")]),
            [r#"echo "a\$foo""#]
        );
        // ls a\|$ : the cursor between the backslash and its `$`. An item
        // that reaches the cursor would split the two, so there is none.
        assert!(taken(r"ls a\$", 3..6, 5, &[reply(1, 1, "x"), reply(0, 1, "b")]).is_empty());
    }

    #[test]
    fn an_item_after_a_typed_bang_is_left_out() {
        // csvm a!| : `a!x` would recall the last command starting with x.
        assert!(taken("csvm a!", 5..7, 7, &[reply(2, 2, "x")]).is_empty());
        assert!(taken(r#"csvm "a!""#, 5..9, 8, &[reply(2, 2, "x")]).is_empty());
        // Outside quotes a blank goes in escaped, and `!\` expands too.
        assert!(taken("csvm a!", 5..7, 7, &[reply(2, 2, " x")]).is_empty());
        // `!=` and, inside double quotes, `! ` do not expand; an escaped
        // `!` and one inside single quotes are kept as they are.
        assert_eq!(
            taken("csvm a!", 5..7, 7, &[reply(2, 2, "=x")]),
            ["csvm a!=x"]
        );
        assert_eq!(
            taken(r#"csvm "a!""#, 5..9, 8, &[reply(2, 2, " x")]),
            [r#"csvm "a! x""#]
        );
        assert_eq!(
            taken(r"csvm a\!", 5..8, 8, &[reply(2, 2, "x")]),
            [r"csvm a\!x"]
        );
        // History expansion skips a `!` after any backslash, one bash keeps
        // too.
        assert_eq!(
            taken(r#"csvm "a\!""#, 5..10, 9, &[reply(3, 3, "x")]),
            [r#"csvm "a\!x""#]
        );
        assert_eq!(
            taken("csvm 'a!'", 5..9, 8, &[reply(2, 2, "x")]),
            ["csvm 'a!x'"]
        );
    }

    #[test]
    fn an_item_that_would_start_an_expansion_is_left_out() {
        // csvm {|} : `a,b` or `1..3` between typed braces is a brace
        // expansion.
        assert!(
            taken(
                "csvm {}",
                5..7,
                6,
                &[reply(1, 1, "a,b"), reply(1, 1, "1..3")]
            )
            .is_empty()
        );
        // csvm a|~/x : `a=` before a typed `~` makes it expand.
        assert!(taken("csvm a~/x", 5..9, 6, &[reply(0, 1, "a=")]).is_empty());
        // csvm "a\|b" : after the kept `\`, the item's `\"` would read as
        // `\\` and a closing quote.
        assert!(taken(r#"csvm "a\b""#, 5..10, 8, &[reply(2, 2, "\"")]).is_empty());
        // csvm a|>out : `2` before a typed `>` is a redirection, and so is
        // `{ab}`.
        assert!(taken("csvm a>out", 5..6, 6, &[reply(0, 1, "2")]).is_empty());
        assert!(taken("csvm {a-b}>f", 5..10, 8, &[reply(1, 4, "ab")]).is_empty());
        assert!(taken("csvm a\\\n>out", 5..6, 6, &[reply(0, 1, "2")]).is_empty());
        // Neither happens for an item that bash reads as it is.
        assert_eq!(taken("csvm {}", 5..7, 6, &[reply(1, 1, "a")]), ["csvm {a}"]);
        assert_eq!(
            taken("csvm a~/x", 5..9, 6, &[reply(0, 1, "ab")]),
            ["csvm ab~/x"]
        );
    }

    #[test]
    fn a_range_inside_a_character_is_left_out() {
        // ls é| : `é` is line bytes 3 and 4, and offset 1 is inside it.
        let line = "ls é";
        let got = taken(
            line,
            3..5,
            5,
            &[reply(1, 2, "x"), reply(0, 1, "x"), reply(0, 2, "ab")],
        );
        assert_eq!(got, ["ls ab"]);
    }

    #[test]
    fn inside_double_quotes_an_item_with_a_bang_is_left_out() {
        let line = r#"csvm "sort am" x"#;
        let got = taken(
            line,
            5..14,
            13,
            &[reply(5, 7, "a!b"), reply(5, 7, "amount")],
        );
        assert_eq!(got, [r#"csvm "sort amount" x"#]);
    }
}
