//! Paints the colours highlight helpers sent over the colours bash's own
//! syntax gives the line.

use std::ops::Range;

use crate::args::{Arg, CommandArgs};
use crate::colors::{ColorSet, Colors};
use crate::lexer::{Kind, Span, merge};
use crate::mode_server::protocol::Reply;

/// The line's colours with the helpers' replies painted in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Painted {
    /// Bash's spans with the helpers' spans over them, sorted and not
    /// overlapping.
    pub spans: Vec<Span>,
    /// The typed bytes of each argument a helper coloured (the bytes its
    /// text came from), for the `script` style: sorted, not overlapping,
    /// and never an argument's quote marks.
    pub script: Vec<Range<usize>>,
    /// The bytes whose kind colour a helper gave, for commands with colours
    /// of their own, with the index of those colours: sorted, not
    /// overlapping.
    pub span_sets: Vec<(Range<usize>, usize)>,
    /// The bytes of `script` that belong to commands with colours of their
    /// own, with the index of those colours: sorted, not overlapping.
    pub script_sets: Vec<(Range<usize>, usize)>,
    /// The error to show: the bytes of the line it points at (`None` when it
    /// has no place there), and `NAME: MESSAGE`.
    pub error: Option<(Option<Range<usize>>, String)>,
}

/// A command a mode server answered for, with the index of its mode's own
/// colours in the sets `with_sets` returns, if it has any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answered {
    pub command: CommandArgs,
    pub reply: Reply,
    pub set: Option<usize>,
}

/// Each of `found` (the mode a command uses, the command, and the reply its
/// mode server sent) with the index of the mode's own colours, and those
/// colours: one whole `Colors` per mode that has some (`set_of` gives them
/// by mode name), `base` (`inkline-colors`) with the mode's colours on top,
/// in line order.
pub fn with_sets(
    found: Vec<(String, CommandArgs, Reply)>,
    base: &Colors,
    set_of: impl Fn(&str) -> Option<ColorSet>,
) -> (Vec<Answered>, Vec<Colors>) {
    let mut modes: Vec<String> = Vec::new();
    let mut sets: Vec<Colors> = Vec::new();
    let mut answered = Vec::with_capacity(found.len());
    for (mode, command, reply) in found {
        let set = match modes.iter().position(|m| *m == mode) {
            Some(i) => Some(i),
            None => set_of(&mode).map(|own| {
                modes.push(mode.clone());
                sets.push(base.layered(&own));
                sets.len() - 1
            }),
        };
        answered.push(Answered {
            command,
            reply,
            set,
        });
    }
    (answered, sets)
}

/// Paints `found`, each command with the reply its helper sent for it, in
/// line order, over `bash`, the line's own spans (`line_len` bytes long).
///
/// A helper's span goes over bash's colours on the typed bytes it maps to,
/// except where bash has a variable (`$x` inside a double-quoted script
/// stays bash's) and on quote marks, which a `raw` argument's bytes still
/// hold: a helper never colours those. The error is the first one, in line
/// order, that is not inside a `raw` argument: bash will change that text
/// before the program sees it. A command with colours of its own has its
/// bytes noted in `span_sets` and `script_sets`; bash's own colours inside
/// its arguments are not.
pub fn paint(line_len: usize, bash: &[Span], found: &[Answered]) -> Painted {
    let mut labels: Vec<Option<Kind>> = vec![None; line_len];
    for span in bash {
        labels[span.start.min(line_len)..span.end.min(line_len)].fill(Some(span.kind));
    }
    let variable: Vec<bool> = labels.iter().map(|l| *l == Some(Kind::Variable)).collect();
    let mut script: Vec<Range<usize>> = Vec::new();
    // The colour set each byte's kind colour and script style come from.
    let mut span_set: Vec<Option<usize>> = vec![None; line_len];
    let mut script_set: Vec<Option<usize>> = vec![None; line_len];
    for Answered {
        command,
        reply,
        set,
    } in found
    {
        let mut coloured = vec![false; command.args.len()];
        for span in &reply.spans {
            let Some(arg) = command.args.get(span.arg) else {
                continue;
            };
            if span.end > arg.text.len() {
                continue;
            }
            coloured[span.arg] = true;
            for byte in typed_bytes(arg, span.start, span.end) {
                if byte < line_len && !variable[byte] {
                    labels[byte] = Some(span.kind);
                    span_set[byte] = *set;
                }
            }
        }
        for (arg, _) in command.args.iter().zip(coloured).filter(|(_, c)| *c) {
            for byte in typed_bytes(arg, 0, arg.text.len()) {
                script.push(byte..byte + 1);
                if byte < line_len {
                    script_set[byte] = *set;
                }
            }
        }
    }
    Painted {
        spans: merge(&labels),
        script: joined(script),
        span_sets: runs(&span_set),
        script_sets: runs(&script_set),
        error: found.iter().find_map(|a| error(&a.command, &a.reply)),
    }
}

/// The runs of neighbouring bytes with the same set in `sets`, as sorted
/// ranges with that set; bytes with none are left out.
fn runs(sets: &[Option<usize>]) -> Vec<(Range<usize>, usize)> {
    let mut out: Vec<(Range<usize>, usize)> = Vec::new();
    for (i, set) in sets.iter().enumerate() {
        let Some(set) = *set else {
            continue;
        };
        match out.last_mut() {
            Some((range, last)) if range.end == i && *last == set => range.end = i + 1,
            _ => out.push((i..i + 1, set)),
        }
    }
    out
}

/// The error of `reply` to show, with the bytes of the line it points at;
/// `None` when it has none or it is inside a `raw` argument.
fn error(command: &CommandArgs, reply: &Reply) -> Option<(Option<Range<usize>>, String)> {
    let e = reply.error.as_ref()?;
    let range = match e.place {
        Some((i, start, end)) => {
            let arg = command.args.get(i)?;
            if arg.raw {
                return None;
            }
            error_range(arg, start, end)
        }
        None => None,
    };
    Some((range, format!("{}: {}", command.name, e.message)))
}

/// The bytes of the line that bytes `start..end` of `arg` were typed as,
/// from the first to the last. An empty range stands for the character
/// after it, or for the argument's last character when it is at the end;
/// an empty argument has no character, so `None`.
fn error_range(arg: &Arg, start: usize, end: usize) -> Option<Range<usize>> {
    let text = &arg.text;
    let len = text.len();
    let (start, end) = if start < end && end <= len {
        (start, end)
    } else if start < len {
        let next = (start + 1..=len).find(|&i| text.is_char_boundary(i))?;
        (start, next)
    } else {
        let last = (0..len).rev().find(|&i| text.is_char_boundary(i))?;
        (last, len)
    };
    Some(arg.map[start]..arg.map[end - 1] + 1)
}

/// The bytes of the line that bytes `start..end` of `arg` were typed as,
/// leaving out its quote marks.
fn typed_bytes(arg: &Arg, start: usize, end: usize) -> impl Iterator<Item = usize> + '_ {
    arg.map[start..end]
        .iter()
        .copied()
        .filter(|b| arg.quotes.binary_search(b).is_err())
}

/// `ranges` sorted, with the ones that overlap or touch joined.
fn joined(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|r| r.start);
    let mut out: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match out.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => out.push(range),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args;
    use crate::lexer::Lexer;
    use crate::mode_server::protocol::{ReplyError, ReplySpan};
    use Kind::{Command, Keyword, Number, Variable};

    /// The `csvm` commands on `line` and bash's spans of it.
    fn parse(line: &str) -> (Vec<CommandArgs>, Vec<Span>) {
        let mut lexer = Lexer::new();
        let tree = lexer.tree(line).unwrap();
        let commands = args::commands(&tree, line, |name| name == "csvm");
        (commands, lexer.spans(line, |_| true))
    }

    fn span(arg: usize, start: usize, end: usize, kind: Kind) -> ReplySpan {
        ReplySpan {
            arg,
            start,
            end,
            kind,
        }
    }

    fn reply(spans: Vec<ReplySpan>, error: Option<ReplyError>) -> Reply {
        Reply { spans, error }
    }

    fn err(place: Option<(usize, usize, usize)>, message: &str) -> Option<ReplyError> {
        Some(ReplyError {
            place,
            message: message.to_owned(),
        })
    }

    /// Paints `line` with one reply per `csvm` command on it, none with
    /// colours of its own.
    fn painted(line: &str, replies: Vec<Reply>) -> Painted {
        let (commands, bash) = parse(line);
        assert_eq!(commands.len(), replies.len(), "{line}");
        let found: Vec<Answered> = commands
            .into_iter()
            .zip(replies)
            .map(|(command, reply)| Answered {
                command,
                reply,
                set: None,
            })
            .collect();
        paint(line.len(), &bash, &found)
    }

    /// Each span as its kind and the text it covers.
    fn labels<'a>(line: &'a str, p: &Painted) -> Vec<(Kind, &'a str)> {
        p.spans
            .iter()
            .map(|s| (s.kind, &line[s.start..s.end]))
            .collect()
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn a_single_quoted_argument() {
        let line = "csvm 'ab cd' x";
        let p = painted(
            line,
            vec![reply(
                vec![span(1, 0, 2, Keyword), span(1, 3, 5, Number)],
                None,
            )],
        );
        assert_eq!(
            labels(line, &p),
            [
                (Command, "csvm"),
                (Kind::String, "'"),
                (Keyword, "ab"),
                (Kind::String, " "),
                (Number, "cd"),
                (Kind::String, "'"),
            ]
        );
        assert_eq!(p.script, [6..11], "not the quote marks, nor `x`");
        assert_eq!(p.error, None);
    }

    #[test]
    fn a_double_quoted_argument_maps_back_past_the_backslashes() {
        let line = r#"csvm "a \"b\" 1""#;
        // The helper got `a "b" 1`.
        let p = painted(
            line,
            vec![reply(
                vec![span(1, 2, 5, Keyword), span(1, 6, 7, Number)],
                None,
            )],
        );
        assert_eq!(
            labels(line, &p),
            [
                (Command, "csvm"),
                (Kind::String, "\"a \\"),
                (Keyword, "\"b"),
                (Kind::String, "\\"),
                (Keyword, "\""),
                (Kind::String, " "),
                (Number, "1"),
                (Kind::String, "\""),
            ]
        );
        assert_eq!(p.script, [6..8, 9..11, 12..15]);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn bash_variables_keep_their_colour() {
        let line = r#"csvm "a $x 1""#;
        // A raw argument: the helper got the text as typed.
        let p = painted(line, vec![reply(vec![span(1, 0, 13 - 5, Keyword)], None)]);
        assert_eq!(
            labels(line, &p),
            [
                (Command, "csvm"),
                (Kind::String, "\""),
                (Keyword, "a "),
                (Variable, "$x"),
                (Keyword, " 1"),
                (Kind::String, "\""),
            ],
            "the quote marks are never the helper's"
        );
        assert_eq!(p.script, [6..12], "nor the script's");
    }

    #[test]
    fn only_arguments_with_a_span_get_the_script_style() {
        let line = "csvm 'a' 'b' | csvm 'c'";
        let p = painted(
            line,
            vec![
                reply(vec![span(2, 0, 1, Number)], None),
                reply(vec![span(1, 0, 1, Number)], None),
            ],
        );
        assert_eq!(p.script, [10..11, 21..22]);
    }

    #[test]
    fn the_first_error_not_in_a_raw_argument() {
        let line = r#"csvm "$x" | csvm 'ab'"#;
        let p = painted(
            line,
            vec![
                reply(vec![], err(Some((1, 0, 2)), "in raw")),
                reply(vec![], err(Some((1, 0, 2)), "unknown")),
            ],
        );
        assert_eq!(p.error, Some((Some(18..20), "csvm: unknown".to_owned())));
        assert!(p.script.is_empty(), "an error is not a span");
    }

    #[test]
    fn an_error_with_no_place() {
        let line = "csvm 'a' | csvm 'b'";
        let p = painted(
            line,
            vec![
                reply(vec![], err(None, "no place")),
                reply(vec![], err(Some((1, 0, 1)), "later")),
            ],
        );
        assert_eq!(p.error, Some((None, "csvm: no place".to_owned())));
    }

    #[test]
    fn an_empty_error_range_underlines_a_character() {
        let error = |line: &str, start: usize| {
            let p = painted(line, vec![reply(vec![], err(Some((1, start, start)), "e"))]);
            p.error.unwrap().0
        };
        assert_eq!(error("csvm 'ab'", 0), Some(6..7), "the character after it");
        assert_eq!(
            error("csvm 'ab'", 2),
            Some(7..8),
            "the last one, at the end"
        );
        assert_eq!(error("csvm 'aé'", 3), Some(7..9), "all of its bytes");
        assert_eq!(error("csvm 'éa'", 0), Some(6..8));
        assert_eq!(error("csvm ''", 0), None, "an empty argument has none");
    }

    #[test]
    fn an_error_range_maps_to_the_typed_bytes() {
        let line = r#"csvm "a\"b""#;
        // The helper got `a"b`; the error covers `"b`, not the backslash.
        let p = painted(line, vec![reply(vec![], err(Some((1, 1, 3)), "e"))]);
        assert_eq!(p.error, Some((Some(8..10), "csvm: e".to_owned())));
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn a_commands_colour_set_goes_on_its_own_bytes() {
        let line = r#"csvm "a $x" | csvm 'b'"#;
        let (commands, bash) = parse(line);
        // A raw argument: offsets count from its first `"`; `a $x` is 1..5.
        let found = vec![
            Answered {
                command: commands[0].clone(),
                reply: reply(vec![span(1, 1, 5, Keyword)], None),
                set: Some(0),
            },
            Answered {
                command: commands[1].clone(),
                reply: reply(vec![span(1, 0, 1, Number)], None),
                set: None,
            },
        ];
        let p = paint(line.len(), &bash, &found);
        assert_eq!(p.span_sets, [(6..8, 0)], "not bash's `$x`");
        assert_eq!(p.script_sets, [(6..10, 0)], "the whole script, `$x` too");
        assert_eq!(p.script, [6..10, 20..21]);
    }

    #[test]
    fn each_mode_with_colours_gets_one_set() {
        let line = "csvm 'a' | other 'b' | c 'c'";
        let mut lexer = Lexer::new();
        let tree = lexer.tree(line).unwrap();
        let commands = args::commands(&tree, line, |n| matches!(n, "csvm" | "other" | "c"));
        let found: Vec<_> = commands
            .into_iter()
            .map(|c| {
                let mode = if c.name == "other" {
                    "other-mode"
                } else {
                    "csvm-mode"
                };
                (mode.to_owned(), c, reply(vec![], None))
            })
            .collect();
        let own = ColorSet::from_entries(&[("command".to_owned(), "35".to_owned())]).unwrap();
        let base = Colors::default();
        let (answered, sets) = with_sets(found, &base, |mode| {
            (mode == "csvm-mode").then(|| own.clone())
        });
        let got: Vec<Option<usize>> = answered.iter().map(|a| a.set).collect();
        assert_eq!(got, [Some(0), None, Some(0)], "csvm and c share one set");
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].sgr(Kind::Command), "35");
        assert_eq!(sets[0].sgr(Kind::String), base.sgr(Kind::String));
    }
}
