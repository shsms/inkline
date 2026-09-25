//! A registered command's arguments, as the program will receive them: quote
//! marks and escaping backslashes removed, with a map back to the bytes each
//! kept byte was typed as.

use std::ops::Range;

use tree_sitter::{Node, Tree};

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

/// A `command` node's name and arguments, in the order they were typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandArgs {
    pub name: String,
    /// `args[0]` is the command name.
    pub args: Vec<Arg>,
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

/// Every `command` node in `tree`, in line order, whose name unquotes to a
/// registered, non-raw name: pipelines, lists, `$( … )` and compound
/// commands are all walked. The arguments are read from the line text by
/// `command_words`, starting right after the name, not from the tree, whose
/// words for a command change shape under redirects and error recovery.
/// Redirects and leading `variable_assignment`s are not arguments. A
/// command whose words `command_words` cannot tell for sure (one holds a
/// `case` or a heredoc inside `$(…)`, `<(…)` or `>(…)`) is left out, and so
/// is a command inside backquotes, or after backquotes the tree places
/// wrongly (see `Passed::lost`). `time` (with an optional lone `-p`) and
/// `coproc` are skipped as prefixes: the words after them are the name and
/// arguments instead.
pub fn commands(tree: &Tree, line: &str, registered: impl Fn(&str) -> bool) -> Vec<CommandArgs> {
    let line = Line {
        text: line,
        chars: line.char_indices().collect(),
    };
    let mut found = Vec::new();
    visit(
        tree.root_node(),
        &line,
        &registered,
        false,
        &mut Passed::default(),
        &mut found,
    );
    found
}

/// The line `commands` reads, and its characters with their byte offsets,
/// made once for the whole line.
struct Line<'a> {
    text: &'a str,
    chars: Vec<(usize, char)>,
}

/// What `visit` has passed so far on the line.
#[derive(Default)]
struct Passed {
    /// The backquotes passed, for the ones tree-sitter's error recovery
    /// leaves outside a backquoted substitution.
    backquotes: usize,
    /// Set where tree-sitter's idea of the backquotes after it is wrong: at
    /// a `` $` `` inside backquotes that does not open a substitution in the
    /// tree (bash reads it as a `$` and the closing backquote), and
    /// at a closing backquote tree-sitter adds that is not on the line,
    /// before its end.
    lost: bool,
}

/// Adds the registered commands in `node` to `found`, leaving out those
/// inside backquotes: bash removes some of the backslashes inside before it
/// reads the commands there, and the words of one would run on past the
/// closing backquote. `in_backquotes` says `node` is inside a backquoted
/// substitution.
fn visit(
    node: Node,
    line: &Line,
    registered: &impl Fn(&str) -> bool,
    in_backquotes: bool,
    passed: &mut Passed,
    found: &mut Vec<CommandArgs>,
) {
    // tree-sitter reads `` $` `` as one token that opens a backquoted
    // substitution.
    let backquote = |node: Node| matches!(node.kind(), "`" | "$`");
    let inside = in_backquotes || passed.backquotes % 2 == 1;
    let substitution =
        node.kind() == "command_substitution" && node.child(0).is_some_and(backquote);
    // A `` $` `` that does not open the substitution it is in.
    let loose_dollar = node.kind() == "$`"
        && !node.parent().is_some_and(|p| {
            p.kind() == "command_substitution" && p.child(0).is_some_and(|c| c.id() == node.id())
        });
    let added = backquote(node) && node.is_missing() && node.start_byte() < line.text.len();
    if (inside && loose_dollar) || added {
        passed.lost = true;
    }
    if backquote(node) {
        passed.backquotes += 1;
    }
    if node.kind() == "command"
        && !(inside || passed.lost)
        && let Some(command) = command_args(node, line, registered)
    {
        found.push(command);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(
            child,
            line,
            registered,
            in_backquotes || substitution,
            passed,
            found,
        );
    }
}

fn command_args(
    node: Node,
    line: &Line,
    registered: &impl Fn(&str) -> bool,
) -> Option<CommandArgs> {
    let name_range = node.child_by_field_name("name")?.byte_range();
    let mut name = unquote(line.text, name_range.clone());
    // Only a prefix needs the words to find the name: every other command
    // is known to be left out before they are read.
    let prefix = matches!(name.text.as_str(), "time" | "coproc");
    if name.raw || !(prefix || registered(&name.text)) {
        return None;
    }
    let mut rest = command_words(line, name_range.end)?;
    let line = line.text;

    while !name.raw && matches!(name.text.as_str(), "time" | "coproc") {
        let mut words = rest.into_iter();
        if name.text == "time"
            && let Some(first) = words.as_slice().first().cloned()
        {
            let flag = unquote(line, first);
            if !flag.raw && flag.text == "-p" {
                words.next();
            }
        }
        name = unquote(line, words.next()?);
        rest = words.collect();
    }

    if name.raw || !registered(&name.text) {
        return None;
    }
    let mut args = vec![name.clone()];
    for word in rest {
        args.push(unquote(line, word));
    }
    Some(CommandArgs {
        name: name.text,
        args,
    })
}

/// The byte ranges of a simple command's words in `line`, starting at byte
/// `start` (the position right after its name, or after a prefix like
/// `time`), or `None` when they cannot be told for sure (see
/// `Scanner::unsure`).
///
/// Reads bash's quoting (single and double quotes, `$'…'`, a backslash
/// outside quotes) and nested `$(…)`, `$((…))`, `${…}`, backquote and
/// process substitutions (so a `)`, `;` or quote mark inside one does not
/// end anything here); a backslash-newline between words joins the lines;
/// redirections are skipped along with the word each takes (see
/// `Scanner::redirect_end`). The words stop at the first unquoted control
/// operator — a newline, `;`, `&`, `|`, a bare `(` (a syntax error) or a
/// `)` closing an enclosing group — at a `#` that starts a word (a comment
/// to the end of the line), or at the end of the line. A heredoc's body is
/// on the following lines, so it is never reached: the command's words
/// already ended at the newline before it. This does not look inside
/// substitutions for further commands; `commands` finds those separately,
/// by walking the tree.
fn command_words(line: &Line, start: usize) -> Option<Vec<Range<usize>>> {
    let chars = &line.chars;
    let byte_at = |k: usize| chars.get(k).map_or(line.text.len(), |&(pos, _)| pos);
    let mut scan = Scanner {
        chars,
        unsure: false,
    };
    let mut words = Vec::new();
    let mut i = chars.partition_point(|&(pos, _)| pos < start);
    // Every pass either stops or moves `i` forward: `blanks_end`,
    // `redirect_end` and `word_end` never return less than they were given,
    // and the pass stops unless one of the last two moved.
    loop {
        i = scan.blanks_end(i);
        let Some(ch) = scan.at(i) else {
            break;
        };
        if ch == '#' {
            break;
        }
        if let Some(after) = scan.redirect_end(i) {
            i = after;
            continue;
        }
        if matches!(ch, '\n' | ';' | '&' | '|' | '(' | ')') {
            break;
        }
        let end = scan.word_end(i);
        if end <= i {
            break;
        }
        words.push(byte_at(i)..byte_at(end));
        i = end;
    }
    (!scan.unsure).then_some(words)
}

/// Reads bash words out of a line's characters. Every method takes an index
/// into `chars` and returns one at or past it (possibly past the end, for
/// a quote or group the line leaves open), and each of their loops moves
/// forward on every pass, so a scan always ends.
struct Scanner<'a> {
    chars: &'a [(usize, char)],
    /// Set when a `$(…)`, `<(…)` or `>(…)` holds the word `case` or a
    /// heredoc, so the words around it are not known for sure. The patterns
    /// of a `case` end in a `)` of their own (`case x in a) …`), which only a
    /// parser of `case` could tell from the `)` that closes the
    /// substitution; a heredoc's body is not command text.
    unsure: bool,
}

impl Scanner<'_> {
    fn at(&self, i: usize) -> Option<char> {
        self.chars.get(i).map(|&(_, c)| c)
    }

    /// The index past any blanks, and backslash-newlines (which join two
    /// lines into one), starting at `i`.
    fn blanks_end(&self, mut i: usize) -> usize {
        loop {
            match (self.at(i), self.at(i + 1)) {
                (Some(' ' | '\t'), _) => i += 1,
                (Some('\\'), Some('\n')) => i += 2,
                _ => return i,
            }
        }
    }

    /// If `chars[i..]` starts a redirection, the index just past it (its
    /// operator, and the word it takes, if any); `None` if it does not.
    ///
    /// A redirection is an optional prefix — digits (a file descriptor) or
    /// a `{name}` — directly before one of the operators `<`, `>`, `>>`,
    /// `>|`, `<>`, `&>`, `&>>`, `>&`, `<&`, `<<<`, `<<` or `<<-`; a `<(` or
    /// `>(` is a process substitution instead, a word. `<<`/`<<-` take the
    /// next word as the heredoc's delimiter; the others take it as their
    /// target — except `>&`/`<&` immediately followed by (optional digits
    /// then) `-`, such as `>&-` or `>&2-`, which take no word: the target
    /// is already part of the operator.
    fn redirect_end(&mut self, i: usize) -> Option<usize> {
        let op_start = self.prefix_end(i);
        let has_prefix = op_start != i;

        let (op_end, no_target) = match (
            self.at(op_start),
            self.at(op_start + 1),
            self.at(op_start + 2),
        ) {
            (Some('<' | '>'), Some('('), _) => return None, // <( >(
            (Some('<'), Some('<'), Some('<')) => (op_start + 3, false), // <<<
            (Some('<'), Some('<'), Some('-')) => (op_start + 3, false), // <<-
            (Some('<'), Some('<'), _) => (op_start + 2, false), // <<
            (Some('>'), Some('>'), _) => (op_start + 2, false), // >>
            (Some('>'), Some('|'), _) => (op_start + 2, false), // >|
            (Some('<'), Some('>'), _) => (op_start + 2, false), // <>
            (Some('&'), Some('>'), Some('>')) if !has_prefix => (op_start + 3, false), // &>>
            (Some('&'), Some('>'), _) if !has_prefix => (op_start + 2, false), // &>
            (Some('>' | '<'), Some('&'), _) => match self.close_spec_end(op_start + 2) {
                Some(end) => (end, true),
                None => (op_start + 2, false),
            },
            (Some('<' | '>'), _, _) => (op_start + 1, false),
            _ => return None,
        };

        if no_target {
            return Some(op_end);
        }
        let target = self.blanks_end(op_end);
        let takes_word = match (self.at(target), self.at(target + 1)) {
            (Some('<' | '>'), Some('(')) => true,
            (None | Some('\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>' | '#'), _) => false,
            _ => true,
        };
        Some(if takes_word {
            self.word_end(target)
        } else {
            op_end
        })
    }

    /// The index just past an optional redirection prefix at `i`: a run of
    /// digits, or a `{name}` (letters, digits, `_`), with neither requiring
    /// what follows it to actually be a redirect operator — the caller
    /// checks that. Returns `i` when there is no such prefix.
    fn prefix_end(&self, i: usize) -> usize {
        let mut k = i;
        while self.at(k).is_some_and(|c| c.is_ascii_digit()) {
            k += 1;
        }
        if k > i {
            return k;
        }
        if self.at(i) == Some('{') {
            let name_start = i + 1;
            let mut k = name_start;
            while self
                .at(k)
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                k += 1;
            }
            if k > name_start && self.at(k) == Some('}') {
                return k + 1;
            }
        }
        i
    }

    /// Whether `chars[i..]` is (optional digits then) `-`, the `>&`/`<&`
    /// "close" form that takes no target word; if so, the index just past
    /// it.
    fn close_spec_end(&self, i: usize) -> Option<usize> {
        let mut k = i;
        while self.at(k).is_some_and(|c| c.is_ascii_digit()) {
            k += 1;
        }
        (self.at(k) == Some('-')).then_some(k + 1)
    }

    /// The index just past the word starting at `i`, stopping (without
    /// consuming) at the first character outside a quote, substitution or
    /// group that is whitespace or one of bash's control/redirect
    /// characters: `;`, `&`, `|`, `)`, `<`, `>`. A `<(` or `>(` is a process
    /// substitution, read to its balanced `)`; a `(` after the word's first
    /// character (an extglob group such as `@(a|b)`, or `x=(…)`) is read
    /// with its balanced group. The word is empty only when `start`
    /// already points at one of the stopping characters or at a `(`.
    fn word_end(&mut self, start: usize) -> usize {
        let mut i = start;
        while let Some(ch) = self.at(i) {
            i = match ch {
                ' ' | '\t' | '\n' | ';' | '&' | '|' | ')' => break,
                '<' | '>' if self.at(i + 1) == Some('(') => self.group_end(i + 2, '(', ')', true),
                '<' | '>' => break,
                '(' if i > start => self.group_end(i + 1, '(', ')', false),
                '(' => break,
                '\'' => self.single_quoted_end(i + 1),
                '"' => self.double_quoted_end(i + 1),
                '\\' => i + 2,
                '$' => self.dollar_end(i + 1, true),
                '`' => self.backquoted_end(i + 1),
                _ => i + 1,
            };
        }
        i
    }

    /// The index just past the closing `'` of a single-quoted string whose
    /// contents start at `i` (nothing inside it is special).
    fn single_quoted_end(&self, mut i: usize) -> usize {
        while self.at(i).is_some_and(|c| c != '\'') {
            i += 1;
        }
        i + 1
    }

    /// The index just past the closing `'` of a `$'…'` string whose
    /// contents start at `i`: a backslash keeps its next character, a `'`
    /// included, from ending the string.
    fn ansi_c_quoted_end(&self, mut i: usize) -> usize {
        while let Some(ch) = self.at(i) {
            match ch {
                '\'' => return i + 1,
                '\\' => i += 2,
                _ => i += 1,
            }
        }
        i
    }

    /// The index just past the closing `"` of a double-quoted string whose
    /// contents start at `i`: a backslash keeps its next character from
    /// ending the string, and a nested `$(…)`, `${…}`, `$((…))` or
    /// backquote substitution is skipped whole. `$'` is not special here.
    fn double_quoted_end(&mut self, mut i: usize) -> usize {
        while let Some(ch) = self.at(i) {
            i = match ch {
                '"' => return i + 1,
                '\\' => i + 2,
                '$' => self.dollar_end(i + 1, false),
                '`' => self.backquoted_end(i + 1),
                _ => i + 1,
            };
        }
        i
    }

    /// The index just past whatever follows a `$` starting at `i`: a
    /// `$(…)` or `$((…))` (both are just balanced parentheses), a `${…}`
    /// (balanced braces), a `$'…'` when `ansi_c` (outside double quotes),
    /// or, for a plain `$name`/`$$`/…, `i` unchanged — the name's own
    /// characters are not special and the caller's loop moves over them.
    fn dollar_end(&mut self, i: usize, ansi_c: bool) -> usize {
        match self.at(i) {
            // `$((…))` is arithmetic, not commands.
            Some('(') => {
                let arithmetic = self.at(i + 1) == Some('(');
                self.group_end(i + 1, '(', ')', !arithmetic)
            }
            Some('{') => self.group_end(i + 1, '{', '}', false),
            Some('\'') if ansi_c => self.ansi_c_quoted_end(i + 1),
            _ => i,
        }
    }

    /// The index just past the `close` that balances an `open` just before
    /// `i`: quotes, substitutions and further `open`/`close` pairs inside
    /// are skipped whole, so a stray `)` or `}` in them does not end this
    /// early. `holds_commands` says the group is a command substitution
    /// or process substitution: a `#` that starts a word in one starts a
    /// comment to the end of the line, and the word `case` or a heredoc
    /// (`<<`, but not `<<<`) sets `unsure`.
    fn group_end(&mut self, mut i: usize, open: char, close: char, holds_commands: bool) -> usize {
        let mut depth = 1u32;
        while let Some(ch) = self.at(i) {
            if holds_commands && self.starts_case(i) {
                self.unsure = true;
            }
            i = match ch {
                c if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                    i + 1
                }
                c if c == open => {
                    depth += 1;
                    i + 1
                }
                '#' if holds_commands && self.starts_word(i) => self.comment_end(i),
                '<' if holds_commands && self.at(i + 1) == Some('<') => {
                    if self.at(i + 2) == Some('<') {
                        i + 3
                    } else {
                        self.unsure = true;
                        i + 2
                    }
                }
                '\'' => self.single_quoted_end(i + 1),
                '"' => self.double_quoted_end(i + 1),
                '\\' => i + 2,
                '$' => self.dollar_end(i + 1, true),
                '`' => self.backquoted_end(i + 1),
                _ => i + 1,
            };
        }
        i
    }

    /// Whether a word inside a group starts at `i`: the character before it
    /// is a blank, a newline or one of `;`, `&`, `|`, `(`.
    fn starts_word(&self, i: usize) -> bool {
        let before = i.checked_sub(1).and_then(|k| self.at(k));
        matches!(before, Some(' ' | '\t' | '\n' | ';' | '&' | '|' | '('))
    }

    /// Whether the word `case` starts at `i`, followed by a blank, a newline
    /// or the end.
    fn starts_case(&self, i: usize) -> bool {
        let word = self.chars.get(i..i + 4).unwrap_or_default();
        self.starts_word(i)
            && word.iter().map(|&(_, c)| c).eq("case".chars())
            && matches!(self.at(i + 4), None | Some(' ' | '\t' | '\n'))
    }

    /// The index of the newline (or the end) that ends a comment starting
    /// at `i`.
    fn comment_end(&self, mut i: usize) -> usize {
        while self.at(i).is_some_and(|c| c != '\n') {
            i += 1;
        }
        i
    }

    /// The index just past the closing `` ` `` of a backquoted command
    /// substitution whose contents start at `i`. Until that backquote, bash
    /// reads only a backslash as special: quotes and `$(` inside are read
    /// once it runs the command.
    fn backquoted_end(&self, mut i: usize) -> usize {
        while let Some(ch) = self.at(i) {
            i = match ch {
                '`' => return i + 1,
                '\\' => i + 2,
                _ => i + 1,
            };
        }
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

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

    #[test]
    fn registered_commands_anywhere_on_the_line() {
        let line = "X=1 csvm 'select a' <in >out | head; echo $(csvm \"b\") && ls";
        let tree = Lexer::new().tree(line).unwrap();
        let found = commands(&tree, line, |n| n == "csvm");
        let texts: Vec<Vec<&str>> = found
            .iter()
            .map(|c| c.args.iter().map(|a| a.text.as_str()).collect())
            .collect();
        assert_eq!(texts, [vec!["csvm", "select a"], vec!["csvm", "b"]]);
        let line = "'csvm' x; c\\svm y; $c z";
        let tree = Lexer::new().tree(line).unwrap();
        let names: Vec<String> = commands(&tree, line, |n| n == "csvm")
            .into_iter()
            .map(|c| c.args[1].text.clone())
            .collect();
        assert_eq!(names, ["x", "y"]);
    }

    /// Words tree-sitter-bash parses as an extra `destination` of a redirect
    /// (`csvm >out a b`: `a` and `b` follow the destination) are really
    /// arguments — bash passes them all: `bash -c 'printf "<%s>\n" a
    /// 2>/dev/null b c'` prints `<a> <b> <c>`.
    #[test]
    fn words_after_a_redirect_are_arguments() {
        for (line, want) in [
            ("csvm a 2>/dev/null b >out c", vec!["csvm", "a", "b", "c"]),
            ("csvm >out 'select a' b", vec!["csvm", "select a", "b"]),
        ] {
            let tree = Lexer::new().tree(line).unwrap();
            let found = commands(&tree, line, |n| n == "csvm");
            let texts: Vec<&str> = found[0].args.iter().map(|a| a.text.as_str()).collect();
            assert_eq!(texts, want, "{line}");
        }
    }

    /// `<&-`/`>&-` close a descriptor and take no destination; a second bare
    /// word after one lands in an `ERROR` node instead of `destination`
    /// (tree-sitter-bash's recovery from the ambiguity), but bash still
    /// passes both words: `bash -c 'printf "<%s>\n" a <&- b'` prints `<a>
    /// <b>`.
    #[test]
    fn words_split_across_an_error_node_are_arguments() {
        for line in ["csvm >&- a b", "csvm <&- a b", "csvm 2>&- a b"] {
            let tree = Lexer::new().tree(line).unwrap();
            let found = commands(&tree, line, |n| n == "csvm");
            let texts: Vec<&str> = found[0].args.iter().map(|a| a.text.as_str()).collect();
            assert_eq!(texts, ["csvm", "a", "b"], "{line}");
        }
    }

    /// Words after a heredoc's delimiter are the heredoc_redirect's own
    /// `argument` field children; bash still passes them to the command:
    /// `bash -c 'printf "<%s>\n" x <<EOF a b\nhi\nEOF'` prints `<x> <a>
    /// <b>`.
    #[test]
    fn words_after_a_heredoc_start_are_arguments() {
        for (line, want) in [
            ("csvm <<EOF a b\nhi\nEOF", vec!["csvm", "a", "b"]),
            ("csvm x <<EOF a\nhi\nEOF", vec!["csvm", "x", "a"]),
        ] {
            let tree = Lexer::new().tree(line).unwrap();
            let found = commands(&tree, line, |n| n == "csvm");
            let texts: Vec<&str> = found[0].args.iter().map(|a| a.text.as_str()).collect();
            assert_eq!(texts, want, "{line}");
        }
    }

    /// `time` (skipping a lone `-p`) and `coproc` are prefixes: the word
    /// after them is the program's own name.
    #[test]
    fn time_and_coproc_are_prefixes() {
        for line in ["time csvm a", "time -p csvm a", "coproc csvm a"] {
            let tree = Lexer::new().tree(line).unwrap();
            let found = commands(&tree, line, |n| n == "csvm");
            let texts: Vec<&str> = found[0].args.iter().map(|a| a.text.as_str()).collect();
            assert_eq!(texts, ["csvm", "a"], "{line}");
        }
    }

    /// A command's words are found in the line text (`command_words`), not
    /// by trusting whatever shape tree-sitter-bash's own error recovery
    /// gives a redirect or an unterminated heredoc on that call: an
    /// unquoted control operator always ends the command's own words, and
    /// `2>&- a b >out c` never drops `b` the way tree-sitter's `ERROR`
    /// recovery once did.
    #[test]
    fn words_come_from_the_line_not_the_tree() {
        for (line, want) in [
            ("csvm <<EOF a; ls", vec!["csvm", "a"]),
            ("csvm <<'EOF' a | wc", vec!["csvm", "a"]),
            ("csvm <<EOF a >out", vec!["csvm", "a"]),
            ("csvm 2>&- a b >out c", vec!["csvm", "a", "b", "c"]),
        ] {
            let tree = Lexer::new().tree(line).unwrap();
            let found = commands(&tree, line, |n| n == "csvm");
            assert_eq!(found.len(), 1, "{line}");
            let texts: Vec<&str> = found[0].args.iter().map(|a| a.text.as_str()).collect();
            assert_eq!(texts, want, "{line}");
        }
    }

    /// A `$(…)` argument is skipped whole while scanning for the command's
    /// words, so a `)` or quote mark inside it (`$(echo ')')`) does not end
    /// the command early; `unquote` still marks it raw. The same nesting
    /// lets a registered command inside a substitution (`echo $(csvm a)`)
    /// still be found, with its own words ending at the substitution's
    /// closing `)`.
    #[test]
    fn substitutions_are_skipped_whole() {
        let line = "csvm $(echo ')') x";
        let tree = Lexer::new().tree(line).unwrap();
        let found = commands(&tree, line, |n| n == "csvm");
        assert_eq!(found.len(), 1);
        let texts: Vec<&str> = found[0].args.iter().map(|a| a.text.as_str()).collect();
        assert_eq!(texts, ["csvm", "$(echo ')')", "x"]);
        assert!(found[0].args[1].raw);

        let line = "echo $(csvm a)";
        let tree = Lexer::new().tree(line).unwrap();
        let found = commands(&tree, line, |n| n == "csvm");
        let texts: Vec<&str> = found[0].args.iter().map(|a| a.text.as_str()).collect();
        assert_eq!(texts, ["csvm", "a"]);
    }

    /// The words of every registered `csvm` command on `line`, as the program
    /// will receive them.
    fn csvm_words(line: &str) -> Vec<Vec<String>> {
        let tree = Lexer::new().tree(line).unwrap();
        commands(&tree, line, |n| n == "csvm")
            .into_iter()
            .map(|c| c.args.into_iter().map(|a| a.text).collect())
            .collect()
    }

    /// A process substitution is a word (bash passes `/dev/fd/63` for it,
    /// with any text typed around it), and a `(` inside a word (an extglob
    /// group) is read along with its balanced group. A bare `(` where a word
    /// should start ends the command: bash rejects `printf x (` and
    /// `printf x=(1 2)` as syntax errors; the second is kept as one word,
    /// the way bash reads it before it rejects it.
    #[test]
    fn parentheses_are_read_as_part_of_a_word() {
        for (line, want) in [
            ("csvm <(ls) x", vec!["csvm", "<(ls)", "x"]),
            ("csvm >(tee y) x", vec!["csvm", ">(tee y)", "x"]),
            ("csvm a<(true)b c", vec!["csvm", "a<(true)b", "c"]),
            ("csvm < <(ls) x", vec!["csvm", "x"]),
            ("csvm @(a|b) x", vec!["csvm", "@(a|b)", "x"]),
            ("csvm !(a b) x", vec!["csvm", "!(a b)", "x"]),
            ("csvm +(a) x", vec!["csvm", "+(a)", "x"]),
            ("csvm *(a) x", vec!["csvm", "*(a)", "x"]),
            ("csvm ?(a) x", vec!["csvm", "?(a)", "x"]),
            ("csvm x=(1 2)", vec!["csvm", "x=(1 2)"]),
            ("csvm a (", vec!["csvm", "a"]),
            ("csvm a (b) c", vec!["csvm", "a"]),
        ] {
            assert_eq!(csvm_words(line), [want], "{line}");
        }
        let line = "csvm <(ls) x";
        let tree = Lexer::new().tree(line).unwrap();
        assert!(commands(&tree, line, |n| n == "csvm")[0].args[1].raw);
    }

    /// `&>` and `&>>` are redirections, not the `&` that ends a command:
    /// `bash -c 'printf "<%s>\n" a &>f b'` writes `<a> <b>` to `f`. A digit
    /// before `&>` is a word of its own (`a 2&>f b` passes `a 2 b`).
    #[test]
    fn ampersand_redirections_are_skipped() {
        for (line, want) in [
            ("csvm a &>/dev/null b", vec!["csvm", "a", "b"]),
            ("csvm a &>>log b", vec!["csvm", "a", "b"]),
            ("csvm a 2&>log b", vec!["csvm", "a", "2", "b"]),
            ("csvm a &&", vec!["csvm", "a"]),
        ] {
            assert_eq!(csvm_words(line), [want], "{line}");
        }
    }

    /// A `#` that starts a word starts a comment that runs to the end of the
    /// line; inside a word it is an ordinary character: `bash -c 'printf
    /// "<%s>\n" a #b c'` prints `<a>`, and `a#b` prints `<a#b>`.
    #[test]
    fn a_hash_starting_a_word_starts_a_comment() {
        for (line, want) in [
            ("csvm a # comment", vec!["csvm", "a"]),
            ("csvm a #b c", vec!["csvm", "a"]),
            ("csvm a >out #b c", vec!["csvm", "a"]),
            ("csvm a#b c", vec!["csvm", "a#b", "c"]),
            ("csvm <(true)#x c", vec!["csvm", "<(true)#x", "c"]),
        ] {
            assert_eq!(csvm_words(line), [want], "{line}");
        }
    }

    /// A backslash-newline between words joins the lines; it is not an
    /// empty argument.
    #[test]
    fn a_backslash_newline_between_words_continues_the_line() {
        for (line, want) in [
            ("csvm a \\\n b", vec!["csvm", "a", "b"]),
            ("csvm a \\\nb", vec!["csvm", "a", "b"]),
            ("csvm a \\\n#b c", vec!["csvm", "a"]),
        ] {
            assert_eq!(csvm_words(line), [want], "{line}");
        }
    }

    /// Inside `$'…'` a backslash escapes the next character, so `\'` does
    /// not end it: `bash -c 'printf "<%s>\n" $'"'"'a\'"'"'b'"'"' c'` prints
    /// `<a'b> <c>`. Inside double quotes `$'` is not special.
    #[test]
    fn ansi_c_quoting_keeps_an_escaped_quote() {
        for (line, want) in [
            ("csvm $'a\\'b' c", vec!["csvm", "$'a\\'b'", "c"]),
            ("csvm a$'\\''b c", vec!["csvm", "a$'\\''b", "c"]),
            ("csvm \"$'a\" b", vec!["csvm", "\"$'a\"", "b"]),
            (
                "csvm $(echo $'\\')') z",
                vec!["csvm", "$(echo $'\\')')", "z"],
            ),
        ] {
            assert_eq!(csvm_words(line), [want], "{line}");
        }
        let line = "csvm $'a\\'b' c";
        let tree = Lexer::new().tree(line).unwrap();
        assert!(commands(&tree, line, |n| n == "csvm")[0].args[1].raw);
    }

    /// Inside `$(…)` a `#` that starts a word starts a comment, which hides
    /// a `)`: `bash -c $'printf "<%s>" $(echo a # )\n) b'` prints `<a><b>`.
    #[test]
    fn a_comment_inside_a_substitution_hides_its_text() {
        for (line, want) in [
            (
                "csvm $(echo a # )\n) b",
                vec!["csvm", "$(echo a # )\n)", "b"],
            ),
            ("csvm $(echo a#) b", vec!["csvm", "$(echo a#)", "b"]),
        ] {
            assert_eq!(csvm_words(line), [want], "{line}");
        }
    }

    /// A `case` inside `$(…)` has patterns ending in an unmatched `)`, which
    /// only a parser of `case` could tell from the `)` that ends the
    /// substitution; such a command is left out rather than given the wrong
    /// words. A registered command inside the `case` is still found.
    #[test]
    fn a_case_inside_a_substitution_leaves_the_command_out() {
        assert!(csvm_words("csvm $(case x in a) echo;; esac) b").is_empty());
        assert!(csvm_words("csvm <(case x in a) echo;; esac) b").is_empty());
        assert_eq!(
            csvm_words("echo $(case x in a) csvm y;; esac)"),
            [vec!["csvm", "y"]]
        );
    }

    /// A command inside backquotes is left out: bash removes some of the
    /// backslashes inside before it reads the command, and the command's
    /// words would run on past the closing backquote.
    #[test]
    fn a_command_inside_backquotes_is_left_out() {
        for line in [
            "echo `csvm 'select a'` | grep x",
            "echo \"`csvm a`\" x",
            "echo $(echo `csvm a` y) x",
            "echo `echo $(csvm a)` x",
            "echo `time csvm a` x",
            "echo `csvm <<EOF a` y",
            "echo `csvm a",
            "echo `&& csvm a` b",
            "echo `|| csvm a` b",
            "echo `&& csvm a",
            "echo `; csvm a",
            "echo `ls \\`x\\`; csvm a` b",
        ] {
            assert!(csvm_words(line).is_empty(), "{line}");
        }
        assert_eq!(csvm_words("echo `ls` && csvm a"), [vec!["csvm", "a"]]);
        assert!(csvm_words("echo $`; csvm a` b").is_empty());
        // Inside backquotes, `` $` `` is a `$` and the closing backquote.
        for line in [
            "echo `echo $` csvm a",
            "echo `echo $`csvm a` b` c",
            "echo $`echo $` csvm a",
            "echo $(a `b) $`date`; csvm c",
            "echo $(a `b) ; csvm c",
        ] {
            assert!(csvm_words(line).is_empty(), "{line}");
        }
        assert_eq!(csvm_words("echo $`date` ; csvm a"), [vec!["csvm", "a"]]);
        // Inside backquotes, bash reads only a backslash as special.
        assert_eq!(
            csvm_words("csvm 'q' `ls # don't` x; ls"),
            [vec!["csvm", "q", "`ls # don't`", "x"]]
        );
        assert_eq!(
            csvm_words("csvm \"a `b\" c` d\" e"),
            [vec!["csvm", "\"a `b\" c` d\"", "e"]]
        );
        assert_eq!(csvm_words("csvm `ls $(` x"), [vec!["csvm", "`ls $(`", "x"]]);
    }

    /// A heredoc inside `$(…)`, `<(…)` or `>(…)` has a body that is not
    /// command text, so the words around it are not known for sure; `<<<`
    /// and a shift inside `$((…))` are not heredocs.
    #[test]
    fn a_heredoc_inside_a_substitution_leaves_the_command_out() {
        assert!(csvm_words("csvm $(cat <<EOF\ndon't\nEOF\n) b; ls").is_empty());
        assert!(csvm_words("csvm <(cat <<EOF\n)\nEOF\n) b").is_empty());
        assert_eq!(
            csvm_words("csvm $(cat <<<x) $((1<<2)) b"),
            [vec!["csvm", "$(cat <<<x)", "$((1<<2))", "b"]]
        );
    }

    /// Finding the words looks at each part of the line only a few times,
    /// so a very long line is quick, whether its commands are registered or
    /// not.
    #[test]
    fn a_long_line_of_commands_is_quick() {
        use std::time::{Duration, Instant};
        for (line, want) in [
            ("ls a; ".repeat(20_000), 0),
            ("csvm a; ".repeat(20_000), 20_000),
        ] {
            let tree = Lexer::new().tree(&line).unwrap();
            let began = Instant::now();
            let found = commands(&tree, &line, |n| n == "csvm");
            let took = began.elapsed();
            assert_eq!(found.len(), want);
            assert!(took < Duration::from_millis(500), "took {took:?}");
        }
    }

    /// Partly typed lines are the normal input: every prefix of every tricky
    /// line must be scanned to the end, without stalling, and every word's
    /// map must point inside the line.
    #[test]
    fn no_prefix_of_a_tricky_line_stalls_the_scan() {
        let lines = [
            "csvm <(ls) x",
            "csvm >(tee y) x",
            "csvm < <(ls) x",
            "csvm x=(1 2)",
            "csvm @(a|b) x",
            "csvm !(a) +(b) *(c) ?(d) x",
            "csvm (",
            "csvm a (b) c",
            "csvm a &>/dev/null b",
            "csvm a &>>log b",
            "csvm a 2&>log b",
            "csvm a # comment",
            "csvm a #b c",
            "csvm a#b",
            "csvm a \\\n b",
            "csvm $'a\\'b' c",
            "csvm $(case x in a) echo;; esac) b",
            "csvm \"a $(b 'c' \")\" d",
            "csvm <<'E' a | x",
            "csvm ${a:-}} b",
            "X=1 csvm 'select a' <in >out | head; echo $(csvm \"b\") && ls",
            "'csvm' x; c\\svm y; $c z",
            "csvm a 2>/dev/null b >out c",
            "csvm >out 'select a' b",
            "csvm >&- a b",
            "csvm <&- a b",
            "csvm 2>&- a b",
            "csvm <<EOF a b\nhi\nEOF",
            "csvm x <<EOF a\nhi\nEOF",
            "time -p csvm a",
            "coproc csvm a",
            "csvm <<EOF a; ls",
            "csvm <<'EOF' a | wc",
            "csvm <<EOF a >out",
            "csvm 2>&- a b >out c",
            "csvm $(echo ')') x",
            "echo $(csvm a)",
            "csvm {x}>f `a \\` b` 'é' \"é$((1+(2)))\" ${x/\\}/}",
            "csvm $( ( a ) ) \\",
            "csvm $(echo a # )\n) b",
        ];
        for line in lines {
            for (end, _) in line.char_indices().chain([(line.len(), ' ')]) {
                let prefix = &line[..end];
                let tree = Lexer::new().tree(prefix).unwrap();
                for command in commands(&tree, prefix, |_| true) {
                    for arg in command.args {
                        assert!(arg.map.iter().all(|&b| b < prefix.len()), "{prefix:?}");
                    }
                }
            }
        }
    }
}
