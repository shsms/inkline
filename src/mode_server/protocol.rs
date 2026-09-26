//! Version 1 of the line protocol inkline speaks with a highlight helper: the
//! requests inkline writes (colours, and with the `indent` feature,
//! indentation), and the replies a helper writes back. All numbers are
//! decimal ASCII; lengths and offsets count bytes.

use std::collections::BTreeMap;

use crate::lexer::Kind;

/// The first line a helper must print, before any words naming the extra
/// requests it can answer.
pub const VERSION_LINE: &str = "inkline-mode 1";

/// Writes a request: `:request ID`, `:cwd LEN` + bytes, one `:arg` per
/// argument (argument 0 is the command name), then `:done`. Each of `args`
/// is the argument's text and whether it still holds something bash will
/// expand (`false` for the argument bash would actually pass the program).
pub fn request(id: u64, cwd: &[u8], args: &[(bool, String)]) -> Vec<u8> {
    let mut out = format!(":request {id}\n").into_bytes();
    write_args(&mut out, cwd, args);
    out.extend_from_slice(b":done\n");
    out
}

/// Writes an indent request: as `request`, headed `:indent ID`, with
/// `:at ARG OFFSET` before `:done`. `at` is where the new line breaks: an
/// argument's index and a byte offset in its text.
pub fn indent_request(id: u64, cwd: &[u8], args: &[(bool, String)], at: (usize, usize)) -> Vec<u8> {
    let mut out = format!(":indent {id}\n").into_bytes();
    write_args(&mut out, cwd, args);
    out.extend_from_slice(format!(":at {} {}\n", at.0, at.1).as_bytes());
    out.extend_from_slice(b":done\n");
    out
}

/// Writes `:cwd` and one `:arg` per argument.
fn write_args(out: &mut Vec<u8>, cwd: &[u8], args: &[(bool, String)]) {
    write_block(out, "cwd", cwd);
    for (raw, text) in args {
        let head = if *raw { "arg raw" } else { "arg final" };
        write_block(out, head, text.as_bytes());
    }
}

/// Writes `:HEAD LEN\nBYTES\n`.
fn write_block(out: &mut Vec<u8>, head: &str, bytes: &[u8]) {
    out.extend_from_slice(format!(":{head} {}\n", bytes.len()).as_bytes());
    out.extend_from_slice(bytes);
    out.push(b'\n');
}

/// The result of reading one protocol message from a buffer that may not yet
/// hold all of it.
pub enum Read<T> {
    /// The buffer ends before the message does.
    Incomplete,
    /// The message, and the number of bytes of the buffer it used.
    Done(T, usize),
    /// The buffer holds something the protocol forbids.
    Bad(String),
}

impl<T> Read<T> {
    /// The same read, with the message made into another by `f`.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Read<U> {
        match self {
            Read::Incomplete => Read::Incomplete,
            Read::Done(message, used) => Read::Done(f(message), used),
            Read::Bad(reason) => Read::Bad(reason),
        }
    }
}

/// Reads the helper's first line: `Done` with the feature words after
/// `inkline-mode 1` (empty for the bare line, and consuming that many
/// bytes); `Bad("not a mode server")` for any other first line.
pub fn version(buf: &[u8]) -> Read<Vec<String>> {
    let Some(nl) = buf.iter().position(|&b| b == b'\n') else {
        return Read::Incomplete;
    };
    let used = nl + 1;
    let bad = || Read::Bad("not a mode server".to_owned());
    let Ok(line) = std::str::from_utf8(&buf[..nl]) else {
        return bad();
    };
    let Some(rest) = line.strip_prefix(VERSION_LINE) else {
        return bad();
    };
    if rest.is_empty() {
        return Read::Done(Vec::new(), used);
    }
    let Some(words) = rest.strip_prefix(' ') else {
        return bad();
    };
    Read::Done(words.split(' ').map(str::to_owned).collect(), used)
}

/// A helper's whole reply to one request: the spans it kept and the one
/// error it may have raised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub spans: Vec<ReplySpan>,
    pub error: Option<ReplyError>,
}

/// A `:span` line kept from a reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplySpan {
    /// The index of the argument this span is inside.
    pub arg: usize,
    /// A byte range in that argument's bytes: `start < end`.
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

/// A `:error` line kept from a reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyError {
    /// The argument and byte range the error points at, `(arg, start, end)`;
    /// `None` for `:error - - - MESSAGE` and for a range this reply's
    /// arguments cannot place.
    pub place: Option<(usize, usize, usize)>,
    pub message: String,
}

/// Reads a whole reply: zero or more `:span` lines, at most one `:error`
/// line, then `:end ID`. `lens` are the byte lengths of the request's
/// arguments, for the range checks. Reads whole lines only (up to `\n`);
/// `Incomplete` until `:end` is read.
///
/// For a buffer that grows between calls: `seen` is where the calls before
/// on the same buffer stopped looking, 0 at first. The lines are parsed only
/// once the buffer holds one that settles the reply (see `settled`); until
/// then each call looks only at the lines that came since, so a reply that
/// comes a little at a time is read in time that grows with its length.
pub fn reply(buf: &[u8], id: u64, lens: &[usize], seen: &mut usize) -> Read<Reply> {
    if !settled(buf, seen) {
        return Read::Incomplete;
    }
    let mut spans: Vec<ReplySpan> = Vec::new();
    // The spans kept, by argument and start, with their ends: the spans of
    // one argument never overlap, so a new span can only overlap the one
    // that starts last before its end.
    let mut kept: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut error: Option<ReplyError> = None;
    let mut pos = 0;
    loop {
        let Some((line, keyword, rest)) = next_line(buf, &mut pos) else {
            return Read::Incomplete;
        };
        match keyword {
            b":span" => match parse_span(rest, lens, &kept) {
                Ok(Some(span)) => {
                    kept.insert((span.arg, span.start), span.end);
                    spans.push(span);
                }
                Ok(None) => {}
                Err(()) => return Read::Bad(bad_reply(line)),
            },
            b":error" => {
                if error.is_some() {
                    return Read::Bad(bad_reply(line));
                }
                match parse_error(rest, lens) {
                    Ok(e) => error = Some(e),
                    Err(()) => return Read::Bad(bad_reply(line)),
                }
            }
            b":end" => {
                return match parse_number::<u64>(rest) {
                    Ok(got) if got == id => Read::Done(Reply { spans, error }, pos),
                    _ => Read::Bad(bad_reply(line)),
                };
            }
            _ if keyword.starts_with(b":") => {}
            _ => return Read::Bad(bad_reply(line)),
        }
    }
}

/// Reads the next whole line from `buf` at `*pos`: the line itself, and it
/// split at the first space into keyword and rest (rest is empty when the
/// line has no space, so a known keyword still fails to parse its missing
/// fields). Advances `*pos` past the line; `None` when the buffer does not
/// hold a whole line yet.
fn next_line<'a>(buf: &'a [u8], pos: &mut usize) -> Option<(&'a [u8], &'a [u8], &'a [u8])> {
    let nl = buf[*pos..].iter().position(|&b| b == b'\n')?;
    let line = &buf[*pos..*pos + nl];
    *pos += nl + 1;
    let (keyword, rest) = match line.iter().position(|&b| b == b' ') {
        Some(space) => (&line[..space], &line[space + 1..]),
        None => (line, &b""[..]),
    };
    Some((line, keyword, rest))
}

/// Whether `buf` holds a line that settles a reply: `:end`, or a line that
/// does not start with `:`, which is a failure. `seen` is the start of the
/// first line not looked at yet; it moves past each line that does not
/// settle the reply.
fn settled(buf: &[u8], seen: &mut usize) -> bool {
    while let Some(nl) = buf[*seen..].iter().position(|&b| b == b'\n') {
        let line = &buf[*seen..*seen + nl];
        if !line.starts_with(b":") || line == b":end" || line.starts_with(b":end ") {
            return true;
        }
        *seen += nl + 1;
    }
    false
}

/// `ARG START END KIND`: `Err` when a field fails to parse; `Ok(None)` when
/// the span is ignored (unknown `KIND`, `ARG` out of range, offsets not
/// `START < END <= LEN`, or an overlap with a span in `kept`, the spans
/// already kept by argument and start, with their ends); `Ok(Some(_))`
/// otherwise.
fn parse_span(
    rest: &[u8],
    lens: &[usize],
    kept: &BTreeMap<(usize, usize), usize>,
) -> Result<Option<ReplySpan>, ()> {
    let fields: Vec<&[u8]> = rest.split(|&b| b == b' ').collect();
    let [arg, start, end, kind] = fields[..] else {
        return Err(());
    };
    let arg = parse_number(arg)?;
    let start = parse_number(start)?;
    let end = parse_number(end)?;
    let Some(kind) = std::str::from_utf8(kind).ok().and_then(kind_named) else {
        return Ok(None);
    };
    if arg >= lens.len() || !(start < end && end <= lens[arg]) {
        return Ok(None);
    }
    let overlaps = kept
        .range(..(arg, end))
        .next_back()
        .is_some_and(|(&(kept_arg, _), &kept_end)| kept_arg == arg && start < kept_end);
    if overlaps {
        return Ok(None);
    }
    Ok(Some(ReplySpan {
        arg,
        start,
        end,
        kind,
    }))
}

/// `ARG START END MESSAGE` or `- - - MESSAGE`. `Err` when `ARG`, `START` or
/// `END` are given but fail to parse; otherwise `Ok`, with `place` cleared
/// when the offsets fall outside the argument or `START > END`.
fn parse_error(rest: &[u8], lens: &[usize]) -> Result<ReplyError, ()> {
    let mut fields = Vec::with_capacity(3);
    let mut at = 0;
    for _ in 0..3 {
        let space = rest[at..].iter().position(|&b| b == b' ').ok_or(())?;
        fields.push(&rest[at..at + space]);
        at += space + 1;
    }
    let message = String::from_utf8_lossy(&rest[at..]).into_owned();
    let place = if fields == [b"-".as_slice(), b"-", b"-"] {
        None
    } else {
        let arg = parse_number(fields[0])?;
        let start = parse_number(fields[1])?;
        let end = parse_number(fields[2])?;
        (arg < lens.len() && start <= end && end <= lens[arg]).then_some((arg, start, end))
    };
    Ok(ReplyError { place, message })
}

/// The depths an indent reply gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Depths {
    /// The depth of the new line.
    pub new: usize,
    /// The depth of the line the cursor is on, the one being split.
    pub current: usize,
}

/// Reads a whole indent reply: at most one `:depth NEW CURRENT` line, then
/// `:end ID`; other lines starting with `:` are ignored. `Done(None, _)`
/// for a reply with no `:depth`: the helper cannot tell. `seen` is as for
/// `reply`.
pub fn indent_reply(buf: &[u8], id: u64, seen: &mut usize) -> Read<Option<Depths>> {
    if !settled(buf, seen) {
        return Read::Incomplete;
    }
    let mut depths: Option<Depths> = None;
    let mut pos = 0;
    loop {
        let Some((line, keyword, rest)) = next_line(buf, &mut pos) else {
            return Read::Incomplete;
        };
        match keyword {
            b":depth" => {
                if depths.is_some() {
                    return Read::Bad(bad_reply(line));
                }
                match parse_depths(rest) {
                    Ok(d) => depths = Some(d),
                    Err(()) => return Read::Bad(bad_reply(line)),
                }
            }
            b":end" => {
                return match parse_number::<u64>(rest) {
                    Ok(got) if got == id => Read::Done(depths, pos),
                    _ => Read::Bad(bad_reply(line)),
                };
            }
            _ if keyword.starts_with(b":") => {}
            _ => return Read::Bad(bad_reply(line)),
        }
    }
}

/// `NEW CURRENT`: two numbers in decimal digits, one space apart.
fn parse_depths(rest: &[u8]) -> Result<Depths, ()> {
    let fields: Vec<&[u8]> = rest.split(|&b| b == b' ').collect();
    let [new, current] = fields[..] else {
        return Err(());
    };
    Ok(Depths {
        new: parse_number(new)?,
        current: parse_number(current)?,
    })
}

/// A number written only in decimal digits, without a sign.
fn parse_number<T: std::str::FromStr>(bytes: &[u8]) -> Result<T, ()> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return Err(());
    }
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or(())
}

/// `bad reply: "LINE"`, `LINE` cut to 40 bytes.
fn bad_reply(line: &[u8]) -> String {
    let cut = &line[..line.len().min(40)];
    format!("bad reply: {:?}", String::from_utf8_lossy(cut))
}

/// The `Kind` named by one of the protocol's nine kind names (`command
/// keyword option operator string number variable function comment`).
pub fn kind_named(name: &str) -> Option<Kind> {
    Some(match name {
        "command" => Kind::Command,
        "keyword" => Kind::Keyword,
        "option" => Kind::Option,
        "operator" => Kind::Operator,
        "string" => Kind::String,
        "number" => Kind::Number,
        "variable" => Kind::Variable,
        "function" => Kind::Function,
        "comment" => Kind::Comment,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_follow_the_protocol() {
        let args = [
            (false, "csvm".to_owned()),
            (false, "a\nb".to_owned()),
            (true, "$HOME/x".to_owned()),
            (false, String::new()),
        ];
        assert_eq!(
            String::from_utf8(request(7, b"/tmp", &args)).unwrap(),
            ":request 7\n:cwd 4\n/tmp\n:arg final 4\ncsvm\n:arg final 3\na\nb\n\
             :arg raw 7\n$HOME/x\n:arg final 0\n\n:done\n"
        );
    }

    #[test]
    fn indent_requests_follow_the_protocol() {
        let args = [(false, "csvm".to_owned()), (false, "a\n{b".to_owned())];
        assert_eq!(
            String::from_utf8(indent_request(8, b"/tmp", &args, (1, 3))).unwrap(),
            ":indent 8\n:cwd 4\n/tmp\n:arg final 4\ncsvm\n:arg final 4\na\n{b\n:at 1 3\n:done\n"
        );
    }

    fn depths(buf: &[u8]) -> Option<Depths> {
        match indent_reply(buf, 3, &mut 0) {
            Read::Done(d, used) => {
                assert_eq!(used, buf.len());
                d
            }
            Read::Incomplete => panic!("incomplete"),
            Read::Bad(e) => panic!("{e}"),
        }
    }

    #[test]
    fn indent_replies() {
        assert_eq!(
            depths(b":depth 2 1\n:end 3\n"),
            Some(Depths { new: 2, current: 1 })
        );
        assert_eq!(depths(b":end 3\n"), None, "cannot tell");
        assert_eq!(
            depths(b":hint x\n:span 1 0 1 number\n:depth 0 0\n:error - - - e\n:end 3\n"),
            Some(Depths { new: 0, current: 0 }),
            "other lines starting with `:` are ignored"
        );
        for buf in [&b":depth 1 0\n"[..], b":depth 1 0\n:en", b":depth x 0\n"] {
            assert!(matches!(indent_reply(buf, 3, &mut 0), Read::Incomplete));
        }
    }

    #[test]
    fn broken_indent_replies() {
        let bad = |buf: &[u8]| match indent_reply(buf, 3, &mut 0) {
            Read::Bad(e) => e,
            _ => panic!("not bad: {:?}", String::from_utf8_lossy(buf)),
        };
        for (buf, line) in [
            (&b":depth 1\n:end 3\n"[..], ":depth 1"),
            (b":depth 1 2 3\n:end 3\n", ":depth 1 2 3"),
            (b":depth\n:end 3\n", ":depth"),
            (b":depth -1 0\n:end 3\n", ":depth -1 0"),
            (b":depth +1 0\n:end 3\n", ":depth +1 0"),
            (b":depth x 0\n:end 3\n", ":depth x 0"),
            (b":depth 1  0\n:end 3\n", ":depth 1  0"),
            (
                b":depth 99999999999999999999999 0\n:end 3\n",
                ":depth 99999999999999999999999 0",
            ),
            (b":depth 1 0\n:depth 1 0\n:end 3\n", ":depth 1 0"),
            (b":end 4\n", ":end 4"),
            (b"hello\n", "hello"),
        ] {
            assert_eq!(bad(buf), format!("bad reply: {line:?}"));
        }
    }

    #[test]
    fn the_version_line() {
        assert!(matches!(version(b"inkline-mo"), Read::Incomplete));
        assert!(matches!(version(b"inkline-mode 1\n:x"), Read::Done(f, 15) if f.is_empty()));
        assert!(matches!(
            version(b"inkline-mode 1 indent later\n"),
            Read::Done(f, 28) if f == ["indent", "later"]
        ));
        for bad in [
            &b"hello\n"[..],
            b"inkline-mode 2\n",
            b"inkline-mode 1x\n",
            b"inkline-mode 10\n",
            b"inkline-highlight 1\n",
        ] {
            assert!(matches!(version(bad), Read::Bad(e) if e == "not a mode server"));
        }
    }

    fn done(buf: &[u8], lens: &[usize]) -> Reply {
        match reply(buf, 3, lens, &mut 0) {
            Read::Done(r, used) => {
                assert_eq!(used, buf.len());
                r
            }
            Read::Incomplete => panic!("incomplete"),
            Read::Bad(e) => panic!("{e}"),
        }
    }

    #[test]
    fn replies_keep_good_spans_and_the_error() {
        let r = done(
            b":span 1 0 6 command\n:span 1 7 9 variable\n:error 1 0 6 unknown command 'selct'\n:end 3\n",
            &[4, 20],
        );
        assert_eq!(
            r.spans,
            [
                ReplySpan {
                    arg: 1,
                    start: 0,
                    end: 6,
                    kind: Kind::Command
                },
                ReplySpan {
                    arg: 1,
                    start: 7,
                    end: 9,
                    kind: Kind::Variable
                },
            ]
        );
        assert_eq!(
            r.error,
            Some(ReplyError {
                place: Some((1, 0, 6)),
                message: "unknown command 'selct'".into()
            })
        );
    }

    #[test]
    fn replies_ignore_what_a_later_version_may_add() {
        let r = done(
            b":span 1 0 2 sparkle\n:span 1 0 2 number\n:span 1 1 3 string\n:span 5 0 1 string\n\
              :span 1 3 99 string\n:hint something\n:error - - - no place here\n:end 3\n",
            &[4, 10],
        );
        assert_eq!(
            r.spans,
            [ReplySpan {
                arg: 1,
                start: 0,
                end: 2,
                kind: Kind::Number
            }]
        );
        assert_eq!(
            r.error,
            Some(ReplyError {
                place: None,
                message: "no place here".into()
            })
        );
    }

    /// A kind that is not UTF-8 is still a kind inkline does not know.
    #[test]
    fn a_kind_that_is_not_utf8_is_ignored() {
        let r = done(b":span 1 0 2 \xffx\n:span 1 0 2 number\n:end 3\n", &[4, 4]);
        assert_eq!(r.spans.len(), 1);
        assert_eq!(r.spans[0].kind, Kind::Number);
    }

    #[test]
    fn overlaps_are_found_in_any_order() {
        let r = done(
            b":span 1 5 7 string\n:span 2 0 9 number\n:span 1 0 5 number\n\
              :span 1 6 8 string\n:span 1 7 9 string\n:span 1 4 6 string\n\
              :span 1 0 9 command\n:span 0 0 4 command\n:end 3\n",
            &[4, 10, 10],
        );
        let kept: Vec<(usize, usize, usize)> =
            r.spans.iter().map(|s| (s.arg, s.start, s.end)).collect();
        assert_eq!(
            kept,
            [(1, 5, 7), (2, 0, 9), (1, 0, 5), (1, 7, 9), (0, 0, 4)]
        );
    }

    /// Many spans are read in time that grows with their number: the
    /// overlap check does not look at every span kept before.
    #[test]
    fn many_spans_are_read_quickly() {
        use std::time::{Duration, Instant};
        let count = 50_000;
        let mut buf = Vec::new();
        for i in 0..count {
            buf.extend_from_slice(format!(":span 1 {} {} string\n", 2 * i, 2 * i + 1).as_bytes());
        }
        buf.extend_from_slice(b":end 3\n");
        let began = Instant::now();
        let r = done(&buf, &[4, 2 * count]);
        let took = began.elapsed();
        assert_eq!(r.spans.len(), count);
        assert!(took < Duration::from_millis(500), "took {took:?}");
    }

    /// A reply read again each time more of it comes, a line at a time, is
    /// read in time that grows with its length: its lines are only parsed
    /// once it is whole.
    #[test]
    fn a_reply_that_comes_a_line_at_a_time_is_read_quickly() {
        use std::time::{Duration, Instant};
        let count = 5_000;
        let mut buf = Vec::new();
        let mut seen = 0;
        let began = Instant::now();
        for i in 0..count {
            buf.extend_from_slice(format!(":span 1 {} {} string\n", 2 * i, 2 * i + 1).as_bytes());
            assert!(matches!(
                reply(&buf, 3, &[4, 2 * count], &mut seen),
                Read::Incomplete
            ));
        }
        buf.extend_from_slice(b":end 3\n");
        let got = reply(&buf, 3, &[4, 2 * count], &mut seen);
        let took = began.elapsed();
        assert!(matches!(got, Read::Done(r, _) if r.spans.len() == count));
        assert!(took < Duration::from_millis(500), "took {took:?}");
    }

    #[test]
    fn broken_replies() {
        let bad = |buf: &[u8]| match reply(buf, 3, &[4, 4], &mut 0) {
            Read::Bad(e) => e,
            _ => panic!("not bad"),
        };
        assert_eq!(bad(b":span 1 0 2 command\nhello\n"), "bad reply: \"hello\"");
        assert_eq!(
            bad(b":span 1 x 2 command\n:end 3\n"),
            "bad reply: \":span 1 x 2 command\""
        );
        assert_eq!(bad(b":end 4\n"), "bad reply: \":end 4\"");
        assert_eq!(bad(b":end +3\n"), "bad reply: \":end +3\"");
        assert_eq!(
            bad(b":span +1 0 2 command\n:end 3\n"),
            "bad reply: \":span +1 0 2 command\""
        );
        assert_eq!(bad(b":end\n"), "bad reply: \":end\"");
        assert_eq!(
            bad(b":error - - - a\n:error - - - b\n:end 3\n"),
            "bad reply: \":error - - - b\""
        );
        // A known line whose fields do not parse is found once the reply
        // is whole.
        for buf in [&b":span 1 0 2 command\n:en"[..], b":span 1 x 2 command\n"] {
            assert!(matches!(reply(buf, 3, &[4, 4], &mut 0), Read::Incomplete));
        }
    }
}
