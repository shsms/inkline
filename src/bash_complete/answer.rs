//! The answer a copy of the shell sends for one request: bash's matches,
//! each as a replayed Tab put it on the line, in fields that each end with
//! a NUL byte.

/// The most matches an answer holds; bash's other matches are left out.
pub const MOST: usize = 1000;

/// The most bytes of an answer that are read.
pub const MOST_BYTES: usize = 4 << 20;

/// One match placed on the line: `text` replaces the line from byte
/// `start` to `extra` bytes after the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub extra: usize,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    pub matches: Vec<Match>,
    /// Whether bash had more than `MOST` matches.
    pub cut: bool,
}

/// What `decode` made of the bytes read so far.
#[derive(Debug, PartialEq, Eq)]
pub enum Decoded {
    Done(Answer),
    /// Not all of the answer has come yet.
    More,
    Bad,
}

/// The answer as the copy writes it: for each match `i`, its start, its
/// extra and its text; then `e` and `1` or `0` for `cut`.
pub fn encode(answer: &Answer) -> Vec<u8> {
    let mut out = Vec::new();
    for m in &answer.matches {
        field(&mut out, b"i");
        field(&mut out, m.start.to_string().as_bytes());
        field(&mut out, m.extra.to_string().as_bytes());
        field(&mut out, m.text.as_bytes());
    }
    field(&mut out, b"e");
    field(&mut out, if answer.cut { b"1" } else { b"0" });
    out
}

fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(bytes);
    out.push(0);
}

/// Reads an answer from the bytes the copy has written so far. A field is
/// whole once its NUL has come; the end mark and `cut` must be the last
/// fields.
pub fn decode(bytes: &[u8]) -> Decoded {
    let mut fields: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    // After the last NUL: a field still being written, empty when the bytes
    // end with a NUL.
    let unfinished = fields.pop().unwrap_or_default();
    let mut fields = fields.into_iter();
    let mut answer = Answer::default();
    loop {
        match fields.next() {
            None => return Decoded::More,
            Some(&[b'i']) => {
                let (Some(start), Some(extra), Some(text)) =
                    (fields.next(), fields.next(), fields.next())
                else {
                    return Decoded::More;
                };
                let (Some(start), Some(extra), Ok(text)) =
                    (number(start), number(extra), std::str::from_utf8(text))
                else {
                    return Decoded::Bad;
                };
                if answer.matches.len() == MOST {
                    return Decoded::Bad;
                }
                answer.matches.push(Match {
                    start,
                    extra,
                    text: text.to_owned(),
                });
            }
            Some(&[b'e']) => {
                let Some(cut) = fields.next() else {
                    return Decoded::More;
                };
                answer.cut = match cut {
                    [b'0'] => false,
                    [b'1'] => true,
                    _ => return Decoded::Bad,
                };
                return if fields.next().is_none() && unfinished.is_empty() {
                    Decoded::Done(answer)
                } else {
                    Decoded::Bad
                };
            }
            Some(_) => return Decoded::Bad,
        }
    }
}

fn number(field: &[u8]) -> Option<usize> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

/// The match a replayed Tab made. `old` is the line before it, with the
/// cursor at `point` and the word starting at `word_start`; `new` is the
/// line readline left. The bytes that differ are the match: it starts at
/// the word, or before it when readline rewrote the quote the word opened,
/// and ends at the cursor, or after it when readline replaced a closing
/// quote there. Both ends fall on whole characters. None when `new` is not
/// UTF-8, or the cursor or word start is not on `old`: past the end, inside
/// a character, or the word starting after the cursor.
pub fn placed(old: &str, point: usize, word_start: usize, new: &[u8]) -> Option<Match> {
    let new = std::str::from_utf8(new).ok()?;
    if point > old.len()
        || word_start > point
        || !old.is_char_boundary(point)
        || !old.is_char_boundary(word_start)
    {
        return None;
    }
    let (o, n) = (old.as_bytes(), new.as_bytes());
    let same_start = o.iter().zip(n).take_while(|(a, b)| a == b).count();
    let mut start = same_start.min(word_start);
    while !old.is_char_boundary(start) {
        start -= 1;
    }
    let room = (o.len() - point).min(n.len() - start);
    let mut same_end = o
        .iter()
        .rev()
        .zip(n.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(room);
    // The bytes kept at the end are the same in both lines, so a character
    // boundary in one is one in the other.
    while !old.is_char_boundary(o.len() - same_end) {
        same_end -= 1;
    }
    Some(Match {
        start,
        extra: o.len() - same_end - point,
        text: new[start..n.len() - same_end].to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(start: usize, extra: usize, text: &str) -> Match {
        Match {
            start,
            extra,
            text: text.to_owned(),
        }
    }

    fn sample() -> Answer {
        Answer {
            matches: vec![
                m(4, 0, "my\\ file.txt "),
                m(4, 1, "my file\" "),
                m(0, 0, ""),
            ],
            cut: true,
        }
    }

    #[test]
    fn an_answer_reads_back_as_written() {
        assert_eq!(decode(&encode(&sample())), Decoded::Done(sample()));
        let empty = Answer::default();
        assert_eq!(decode(&encode(&empty)), Decoded::Done(empty));
    }

    #[test]
    fn a_part_of_an_answer_wants_more() {
        let bytes = encode(&sample());
        for n in 0..bytes.len() {
            assert_eq!(decode(&bytes[..n]), Decoded::More, "{n} bytes");
        }
    }

    #[test]
    fn a_bad_answer_is_bad() {
        let bad: [&[u8]; 6] = [
            b"x\0",
            b"i\0a\x000\0t\0e\x000\0",
            b"i\x001\x000\0\xff\0e\x000\0",
            b"e\x002\0",
            b"e\x000\0i\0",
            b"e\x000\0x",
        ];
        for bytes in bad {
            assert_eq!(decode(bytes), Decoded::Bad, "{bytes:?}");
        }
    }

    #[test]
    fn more_than_the_most_matches_is_bad() {
        let answer = Answer {
            matches: vec![m(0, 0, "a"); MOST + 1],
            cut: false,
        };
        assert_eq!(decode(&encode(&answer)), Decoded::Bad);
    }

    #[test]
    fn placed_takes_the_word_and_what_readline_added() {
        assert_eq!(
            placed("cat my", 6, 4, b"cat my\\ file.txt "),
            Some(m(4, 0, "my\\ file.txt "))
        );
    }

    /// `git sw` and `git switch ` share `git sw`, more than the text before
    /// the word: the item still starts at the word.
    #[test]
    fn placed_starts_at_the_word_when_the_match_repeats_it() {
        assert_eq!(
            placed("git sw", 6, 4, b"git switch "),
            Some(m(4, 0, "switch "))
        );
    }

    /// readline may rewrite the quote the word opened.
    #[test]
    fn placed_takes_in_an_opening_quote_readline_rewrote() {
        assert_eq!(
            placed("ls \"x", 5, 4, b"ls 'x y' "),
            Some(m(3, 0, "'x y' "))
        );
    }

    /// Pairing put `"` after the cursor; readline replaces it with its own.
    #[test]
    fn placed_replaces_a_closing_quote_after_the_cursor() {
        assert_eq!(
            placed("ls \"my f\"", 8, 4, b"ls \"my file\" "),
            Some(m(4, 1, "my file\" "))
        );
    }

    #[test]
    fn placed_keeps_the_text_after_the_cursor() {
        assert_eq!(
            placed("gg sw rest", 5, 3, b"gg switch  rest"),
            Some(m(3, 0, "switch "))
        );
    }

    #[test]
    fn placed_with_no_change_gives_the_typed_text() {
        assert_eq!(placed("ls ab", 5, 3, b"ls ab"), Some(m(3, 0, "ab")));
    }

    /// `é` and `è` share their first byte: the item starts at the whole
    /// character.
    #[test]
    fn placed_keeps_whole_characters() {
        assert_eq!(placed("xé", 3, 3, "xèa".as_bytes()), Some(m(1, 0, "èa")));
    }

    #[test]
    fn placed_refuses_a_line_that_is_not_utf8() {
        assert_eq!(placed("ls a", 4, 3, b"ls a\xff "), None);
    }

    #[test]
    fn placed_refuses_places_off_the_line() {
        assert_eq!(placed("ls", 3, 0, b"ls"), None);
        assert_eq!(placed("ls ab", 3, 4, b"ls ab"), None);
        assert_eq!(placed("é", 1, 0, b"x"), None);
        assert_eq!(placed("é", 2, 1, b"x"), None);
        // Any result will do, as long as there is no panic.
        let _ = placed("ls abc", 6, 3, b"ls");
        let _ = placed("é", 2, 2, "é".as_bytes());
    }
}
