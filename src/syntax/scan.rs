//! A lexical scan for what tree-sitter-bash gets wrong: here-documents, open
//! quotes, a trailing backslash, backquoted commands and extended patterns.

use std::ops::Range;

pub(super) struct Scan {
    pub(super) open: bool,
    /// Where a quote (', ", $', `) that is still open at the end starts.
    pub(super) open_quote: Option<usize>,
    /// A `${` is still open at the end.
    pub(super) open_brace: bool,
    /// The text ends with a backslash that continues the line.
    pub(super) continued: Option<usize>,
    /// Backquoted commands: the text between the backquotes.
    pub(super) backquoted: Vec<Range<usize>>,
    /// Bytes to blank: bodies with their end lines, and the second `<` (and
    /// `-`) of each operator so `<<EOF` reads as the redirect `< EOF`.
    pub(super) blank: Vec<Range<usize>>,
    /// Closed extended patterns such as `@(a|b)`, outside arithmetic. With
    /// extglob on, bash reads each as part of a word.
    pub(super) patterns: Vec<Range<usize>>,
    /// Words tree-sitter-bash reads as broken: empty command and process
    /// substitutions (`$()`, `$( )`, `<()`, `>()`), an empty arithmetic
    /// expansion (`$(())`) and a `{}` word. Bash takes each as a word.
    pub(super) words: Vec<Range<usize>>,
}

/// Scans `src` for here-documents (`<<` and `<<-`, not `<<<`, with bodies up
/// to an exact end line), quotes still open at the end, a trailing backslash,
/// backquoted commands and extended patterns.
pub(super) fn scan(src: &str) -> Scan {
    #[derive(Clone, Copy, PartialEq)]
    enum Ctx {
        Code(u32), // open `(` count inside this level
        Dq(usize), // start of the double quote
        Brace,     // ${ ... }
    }
    let b = src.as_bytes();
    let mut blank = Vec::new();
    let mut backquoted = Vec::new();
    let mut pending: Vec<(String, bool)> = Vec::new();
    let mut stack = vec![Ctx::Code(0)];
    let mut patterns = Vec::new();
    let mut words = Vec::new();
    // The end of an empty substitution whose opening `(` is at `open`.
    let empty_after = |open: usize| {
        let close = open
            + 1
            + b[open + 1..]
                .iter()
                .take_while(|c| b" \t\n".contains(c))
                .count();
        (b.get(close) == Some(&b')')).then_some(close + 1)
    };
    // Open `((` and `$((`: the level of `stack` and its `(` count at which
    // each ends.
    let mut arith: Vec<(usize, u32)> = Vec::new();
    let mut i = 0;
    let done = |blank, backquoted, patterns, words, open, open_quote, continued| Scan {
        open,
        open_quote,
        blank,
        backquoted,
        continued,
        open_brace: false,
        patterns,
        words,
    };
    while i < b.len() {
        let c = b[i];
        let top = *stack.last().unwrap();
        let next = b.get(i + 1).copied();
        // Shared by every context.
        if c == b'\\' {
            if i + 2 == b.len() && next == Some(b'\n') {
                let q = stack
                    .iter()
                    .find_map(|x| if let Ctx::Dq(p) = x { Some(*p) } else { None });
                return done(blank, backquoted, patterns, words, false, q, Some(i));
            }
            i += 2;
            continue;
        }
        if c == b'`' {
            // Bash reads a backquoted command only when it runs it.
            let mut j = i + 1;
            while j < b.len() && b[j] != b'`' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            if j >= b.len() {
                return done(blank, backquoted, patterns, words, false, Some(i), None);
            }
            backquoted.push(i + 1..j);
            i = j + 1;
            continue;
        }
        if c == b'$' && next == Some(b'(') {
            // `$()`, and the empty arithmetic `$(())`.
            let arith_end = if b.get(i + 2) == Some(&b'(') {
                empty_after(i + 2)
                    .filter(|&e| b.get(e) == Some(&b')'))
                    .map(|e| e + 1)
            } else {
                None
            };
            if let Some(end) = arith_end.or_else(|| empty_after(i + 1)) {
                words.push(i..end);
                i = end;
                continue;
            }
            stack.push(Ctx::Code(0));
            if b.get(i + 2) == Some(&b'(') {
                arith.push((stack.len() - 1, 0));
            }
            i += 2;
            continue;
        }
        if c == b'$' && next == Some(b'{') {
            stack.push(Ctx::Brace);
            i += 2;
            continue;
        }
        match top {
            Ctx::Dq(_) => {
                if c == b'"' {
                    stack.pop();
                }
                i += 1;
                continue;
            }
            Ctx::Brace if c == b'}' => {
                stack.pop();
                i += 1;
                continue;
            }
            _ => {}
        }
        match c {
            b'\'' => {
                let start = if ansi_c_dollar(b, i) { i - 1 } else { i };
                let ansi = start < i && top != Ctx::Brace;
                let mut j = i + 1;
                while j < b.len() && b[j] != b'\'' {
                    j += if ansi && b[j] == b'\\' { 2 } else { 1 };
                }
                if j >= b.len() {
                    return done(blank, backquoted, patterns, words, false, Some(start), None);
                }
                i = j + 1;
                continue;
            }
            b'"' => {
                let start = if i > 0 && b[i - 1] == b'$' { i - 1 } else { i };
                stack.push(Ctx::Dq(start));
            }
            b'#' if top != Ctx::Brace && (i == 0 || is_meta(b[i - 1])) => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'?' | b'*' | b'+' | b'@' | b'!'
                if next == Some(b'(') && matches!(top, Ctx::Code(_)) && arith.is_empty() =>
            {
                // A newline would start a here-document body.
                let mut depth = 0;
                let mut j = i + 1;
                while j < b.len() && (b[j] != b'\n' || pending.is_empty()) {
                    match b[j] {
                        b'\\' => j += 1,
                        // Quoted parens neither open nor close the pattern.
                        quote @ (b'\'' | b'"') => {
                            let escapes = quote == b'"' || ansi_c_dollar(b, j);
                            j += 1;
                            while j < b.len() && b[j] != quote {
                                j += if escapes && b[j] == b'\\' { 2 } else { 1 };
                            }
                        }
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                patterns.push(i..j + 1);
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
            }
            b'<' | b'>'
                if next == Some(b'(')
                    && !(i > 0 && b"<>&".contains(&b[i - 1]))
                    && let Some(end) = empty_after(i + 1) =>
            {
                words.push(i..end);
                i = end;
                continue;
            }
            b'{' if next == Some(b'}')
                && top != Ctx::Brace
                && (i == 0 || is_meta(b[i - 1]))
                && !after_function_name(b, i) =>
            {
                words.push(i..i + 2);
                i += 2;
                continue;
            }
            b'(' => {
                let level = stack.len() - 1;
                if let Some(Ctx::Code(n)) = stack.last_mut() {
                    if next == Some(b'(') {
                        arith.push((level, *n));
                    }
                    *n += 1;
                }
            }
            b')' => {
                let depth = stack.len();
                match stack.last_mut() {
                    Some(Ctx::Code(n)) if *n > 0 => {
                        *n -= 1;
                        let at = (depth - 1, *n);
                        while arith.last() == Some(&at) {
                            arith.pop();
                        }
                    }
                    Some(Ctx::Code(_)) if depth > 1 => {
                        stack.pop();
                        arith.retain(|&(level, _)| level < stack.len());
                    }
                    _ => {}
                }
            }
            // `<<` is a shift inside arithmetic, but a here-document inside a
            // command substitution nested in it.
            b'<' if top != Ctx::Brace
                && arith
                    .last()
                    .is_none_or(|&(level, _)| level + 1 < stack.len())
                && next == Some(b'<')
                && b.get(i + 2) != Some(&b'<') =>
            {
                let dash = b.get(i + 2) == Some(&b'-');
                let op = i + 1..i + 2 + dash as usize;
                let mut j = i + 2 + dash as usize;
                while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                    j += 1;
                }
                let mut delim = String::new();
                let (mut q1, mut q2) = (false, false);
                while j < b.len() && (q1 || q2 || !is_meta(b[j])) {
                    match b[j] {
                        b'\'' if !q2 => q1 = !q1,
                        b'"' if !q1 => q2 = !q2,
                        b'\\' if !q1 && j + 1 < b.len() => {
                            j += 1;
                            delim.push(b[j] as char);
                        }
                        ch => delim.push(ch as char),
                    }
                    j += 1;
                }
                if !delim.is_empty() {
                    blank.push(op);
                    pending.push((delim, dash));
                }
                i = j;
                continue;
            }
            b'<' if next == Some(b'<') => {
                i += 3;
                continue;
            }
            b'\n' if !pending.is_empty() => {
                // Bodies start on the next line, one after another.
                let mut pos = i + 1;
                for (delim, dash) in pending.drain(..) {
                    let start = pos;
                    let mut found = false;
                    while pos < b.len() {
                        let eol = src[pos..].find('\n').map(|k| pos + k).unwrap_or(b.len());
                        let mut line = &src[pos..eol];
                        if dash {
                            line = line.trim_start_matches('\t');
                        }
                        let nx = (eol + 1).min(b.len());
                        if line == delim {
                            blank.push(start..eol);
                            pos = nx;
                            found = true;
                            break;
                        }
                        pos = nx;
                        if eol == b.len() {
                            break;
                        }
                    }
                    if !found {
                        return done(blank, backquoted, patterns, words, true, None, None);
                    }
                }
                i = pos;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    let q = stack
        .iter()
        .find_map(|x| if let Ctx::Dq(p) = x { Some(*p) } else { None });
    let mut h = done(
        blank,
        backquoted,
        patterns,
        words,
        !pending.is_empty(),
        q,
        None,
    );
    h.open_brace = stack.contains(&Ctx::Brace);
    h
}

/// Whether bash ends a word at `c`: a blank, a newline or an operator byte.
fn is_meta(c: u8) -> bool {
    b" \t\n;&|()<>".contains(&c)
}

/// Where the run of bytes before `end` that all pass `f` starts.
fn run_start(b: &[u8], end: usize, f: impl Fn(u8) -> bool) -> usize {
    end - b[..end].iter().rev().take_while(|&&c| f(c)).count()
}

/// Whether `at` follows `function NAME` where `function` starts a command:
/// bash then wants a compound command and takes a `{}` word as a syntax error.
fn after_function_name(b: &[u8], at: usize) -> bool {
    let blank = |c| b" \t\n".contains(&c);
    let name_end = run_start(b, at, blank);
    let name = run_start(b, name_end, |c| !is_meta(c));
    let keyword_end = run_start(b, name, blank);
    name < name_end
        && keyword_end < name
        && b[..keyword_end].ends_with(b"function")
        && (keyword_end == 8 || b[keyword_end - 9] == b'{' || is_meta(b[keyword_end - 9]))
        && starts_command(b, keyword_end - 8)
}

/// Whether a word starting at `at` is where a command starts: at the start of
/// the text, after a newline or an operator, or after a keyword such as `if`
/// or `!` that itself starts a command.
fn starts_command(b: &[u8], mut at: usize) -> bool {
    loop {
        let end = run_start(b, at, |c| b" \t".contains(&c));
        if end == 0 || b"\n;&|({".contains(&b[end - 1]) {
            return true;
        }
        let word = run_start(b, end, |c| !is_meta(c));
        if !matches!(
            &b[word..end],
            b"!" | b"time"
                | b"coproc"
                | b"if"
                | b"then"
                | b"else"
                | b"elif"
                | b"do"
                | b"while"
                | b"until"
                | b"{"
        ) {
            return false;
        }
        at = word;
    }
}

/// Whether the `'` at `quote` starts a `$'…'` quote: a `$` comes right before
/// it and is not escaped by an odd number of backslashes.
fn ansi_c_dollar(b: &[u8], quote: usize) -> bool {
    quote > 0
        && b[quote - 1] == b'$'
        && b[..quote - 1]
            .iter()
            .rev()
            .take_while(|&&c| c == b'\\')
            .count()
            % 2
            == 0
}
