//! Decides what bash would make of a command if it were entered now:
//! unfinished (bash would ask for another line), wrong (a syntax error) or
//! fine. tree-sitter-bash parses the command; the checks here cover where its
//! error recovery differs from bash. `tests/data/syntax-cases.txt` records
//! what bash does with thousands of commands, and the tests hold these rules
//! to it.

mod scan;

use std::ops::Range;

use tree_sitter::{Node, Parser, Tree};

/// What bash would make of a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Fine,
    /// Bash would ask for another line.
    Unfinished,
    /// Bash would report a syntax error near the token at these bytes of the
    /// command, read with a newline after it as bash reads it.
    Wrong(Range<usize>),
}

pub struct Checker {
    parser: Parser,
}

impl Checker {
    pub fn new() -> Checker {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_bash::LANGUAGE.into())
            .expect("tree-sitter-bash is built for this tree-sitter version");
        Checker { parser }
    }

    /// What bash would make of `text` if it were entered now. With `extglob`
    /// on, the `(` of a pattern such as `@(a|b)` is not an error.
    pub fn check(&mut self, text: &str, extglob: bool) -> Status {
        // Bash reads the line with its newline.
        let src0 = format!("{text}\n");
        // A lexical scan finds here-documents (their end lines must match
        // exactly), open quotes, and backquoted commands (bash parses those
        // only when it runs them). Bodies and backquoted text are blanked.
        let hd = scan::scan(&src0);
        if hd.open {
            return Status::Unfinished;
        }
        let mut bytes = src0.into_bytes();
        blank(&mut bytes, &hd.blank);
        blank(&mut bytes, &hd.backquoted);
        for r in &hd.words {
            bytes[r.clone()].fill(b'x');
        }
        for r in &hd.backquoted {
            if !r.is_empty() {
                bytes[r.start] = b':';
            }
        }
        // tree-sitter-bash does not know extended patterns; a word of the
        // same length, on the same lines, parses as bash reads them with
        // extglob on.
        if extglob {
            for r in &hd.patterns {
                for byte in &mut bytes[r.clone()] {
                    if *byte != b'\n' {
                        *byte = b'x';
                    }
                }
            }
        }
        let mut src =
            String::from_utf8(bytes).expect("blanking replaces every byte of a character");
        let Some(mut tree) = self.parser.parse(&src, None) else {
            return Status::Fine;
        };
        // Blank `time`, `time -p`, `coproc` and `!`, which tree-sitter reads
        // as command names (or gets wrong before `{`), and `;;&`/`;&` become
        // `;; `/`;;`, which tree-sitter gets wrong before `esac`.
        for _ in 0..4 {
            let mut hide = Vec::new();
            let mut swap = Vec::new();
            let mut bare_coproc = None;
            let mut to_colon = Vec::new();
            walk(tree.root_node(), &mut |n| {
                // Bash runs a command of only assignments and redirections
                // (`x=1 >f`); tree-sitter wants a command name after them and
                // reports it missing, or takes the next line's first word or
                // the command after `;` as it. The first assignment's name
                // becomes `:`, so its value is an argument.
                if n.kind() == "command"
                    && let Some(name) = n.child_by_field_name("name")
                    && let Some(before) = name.prev_sibling()
                    && (name.child(0).is_some_and(|w| w.is_missing())
                        || before.is_error()
                        || src[before.end_byte()..name.start_byte()].contains('\n'))
                {
                    let mut c = n.walk();
                    let kids: Vec<Node> = n
                        .children(&mut c)
                        .take_while(|k| k.id() != name.id())
                        .collect();
                    let assign = kids.iter().find(|k| k.kind() == "variable_assignment");
                    if let Some(a) = assign
                        && kids.iter().any(|k| k.kind().ends_with("_redirect"))
                    {
                        let value = a.child_by_field_name("value");
                        to_colon
                            .push(a.start_byte()..value.map_or(a.end_byte(), |v| v.start_byte()));
                        if let Some(v) = value
                            && v.kind() == "array"
                        {
                            let mut c = v.walk();
                            hide.extend(
                                v.children(&mut c)
                                    .filter(|p| ["(", ")"].contains(&p.kind()) && !p.is_missing())
                                    .map(|p| p.byte_range()),
                            );
                        }
                    }
                }
                if n.child_count() == 0
                    && n.kind() == "!"
                    && n.parent()
                        .map(|p| p.kind() == "negated_command")
                        .unwrap_or(false)
                {
                    hide.push(n.byte_range());
                }
                if n.child_count() == 0 && (n.kind() == ";;&" || n.kind() == ";&") {
                    swap.push(n.byte_range());
                }
                if n.kind() == "command"
                    && let Some(name) = n.child_by_field_name("name")
                {
                    if n.child(0).map(|c| c.id()) != Some(name.id()) {
                        return;
                    }
                    let t = &src[name.byte_range()];
                    if t == "time" || t == "coproc" {
                        if n.named_child_count() == 1 && t == "coproc" {
                            bare_coproc = Some(name.byte_range());
                            return;
                        }
                        let mut end = name.end_byte();
                        if t == "coproc"
                            && let (Some(a), Some(b)) = (
                                name.next_named_sibling(),
                                name.next_named_sibling()
                                    .and_then(|a| a.next_named_sibling()),
                            )
                            && src[b.byte_range()].starts_with(['{', '('])
                        {
                            end = a.end_byte();
                        }
                        if t == "time"
                            && let Some(a) = name.next_named_sibling()
                            && &src[a.byte_range()] == "-p"
                        {
                            end = a.end_byte();
                        }
                        hide.push(name.start_byte()..end);
                    }
                }
            });
            if let Some(r) = bare_coproc {
                return Status::Wrong(r);
            }
            if hide.is_empty() && swap.is_empty() && to_colon.is_empty() {
                break;
            }
            let mut bytes = src.into_bytes();
            blank(&mut bytes, &hide);
            blank(&mut bytes, &to_colon);
            for r in &to_colon {
                bytes[r.start] = b':';
            }
            for r in swap {
                bytes[r.start..r.end].copy_from_slice(if r.len() == 3 { b";; " } else { b";;" });
            }
            src = String::from_utf8(bytes).expect("blanking replaces every byte of a character");
            let Some(next) = self.parser.parse(&src, None) else {
                return Status::Fine;
            };
            tree = next;
        }
        let v = analyse(&tree, &src, hd.open_brace);
        // An open quote at the end: whatever follows it is quoted text. A
        // backslash at the end asks for the next line.
        if let Some(q) = hd.open_quote.or(hd.continued) {
            match &v {
                Status::Wrong(r) if r.start < q => {}
                _ => return Status::Unfinished,
            }
        }
        v
    }
}

impl Default for Checker {
    fn default() -> Checker {
        Checker::new()
    }
}

/// The word around `range` in `text`, for the underline: widened over the
/// characters next to it up to a blank or an operator, and cut at the end of
/// `text`. An operator such as `)` or `;;` stays as it is.
pub fn word_around(text: &str, range: Range<usize>) -> Range<usize> {
    let bytes = text.as_bytes();
    let end = range.end.min(text.len());
    let start = range.start.min(end);
    let is_break = |b: u8| b.is_ascii_whitespace() || b";&|<>()".contains(&b);
    if bytes[start..end].iter().all(|&b| is_break(b)) {
        return start..end;
    }
    let start = bytes[..start]
        .iter()
        .rposition(|&b| is_break(b))
        .map_or(0, |i| i + 1);
    let end = bytes[end..]
        .iter()
        .position(|&b| is_break(b))
        .map_or(text.len(), |i| end + i);
    start..end
}

/// Whether a line starting after `before` is part of a here-document's body:
/// `before` has a `<<` whose end line has not come yet.
pub fn heredoc_open(before: &str) -> bool {
    scan::scan(&format!("{before}\n")).open
}

fn walk<'a>(n: Node<'a>, f: &mut impl FnMut(Node<'a>)) {
    f(n);
    let mut c = n.walk();
    for ch in n.children(&mut c) {
        walk(ch, f);
    }
}

/// Leaves in order. Zero-width ERROR leaves are kept.
fn leaves<'a>(n: Node<'a>) -> Vec<Node<'a>> {
    let mut v = Vec::new();
    walk(n, &mut |x| {
        if x.child_count() == 0 {
            v.push(x)
        }
    });
    v
}

/// Whether `l` is inside a complete, error-free compound command.
fn in_complete(l: Node) -> bool {
    const KINDS: &[&str] = &[
        "for_statement",
        "c_style_for_statement",
        "while_statement",
        "if_statement",
        "case_statement",
        "compound_statement",
        "subshell",
        "function_definition",
        "command_substitution",
        "test_command",
        "arithmetic_expansion",
    ];
    let mut p = l.parent();
    while let Some(x) = p {
        if x.is_error() {
            return false;
        }
        if KINDS.contains(&x.kind()) && !x.has_error() {
            return true;
        }
        p = x.parent();
    }
    false
}

/// End of the last byte that is code: not whitespace, not a comment.
fn code_end(tree: &Tree, src: &str) -> usize {
    let mut end = 0;
    for l in leaves(tree.root_node()) {
        if l.kind() == "comment" || l.is_missing() {
            continue;
        }
        let t = &src[l.byte_range()];
        let trimmed = t.trim_end();
        if !trimmed.is_empty() {
            end = end.max(l.start_byte() + trimmed.len());
        } else if l.is_error() {
            end = end.max(l.start_byte());
        }
    }
    end
}

fn problems<'a>(tree: &'a Tree) -> Vec<Node<'a>> {
    let mut v = Vec::new();
    walk(tree.root_node(), &mut |x| {
        if x.is_error() || x.is_missing() {
            v.push(x)
        }
    });
    v
}

const CLOSERS: &[&str] = &[
    "then", "else", "elif", "fi", "do", "done", "esac", "}", "in", ";;",
];
const REDIRS: &[&str] = &[
    ">", "<", ">>", ">&", "<&", "&>", "&>>", "<<", "<<-", "<<<", ">|", "<>", ">&-", "<&-",
];
const SEPS: &[&str] = &[";", "&", "|", "&&", "||", "|&", ";;", ";&", ";;&"];

/// Replaces the bytes in `ranges` with spaces, keeping newlines.
fn blank(src: &mut [u8], ranges: &[Range<usize>]) {
    for r in ranges {
        for k in r.clone() {
            if src[k] != b'\n' {
                src[k] = b' ';
            }
        }
    }
}

/// The kind of a leaf, or the text of a childless ERROR.
fn lk<'a>(l: Node<'a>, src: &'a str) -> &'a str {
    if l.is_error() {
        src[l.byte_range()].trim()
    } else {
        l.kind()
    }
}

const OPENERS: &[&str] = &[
    "{", "(", "$(", "then", "do", "else", "if", "elif", "while", "until", "!", "|", "&&", "||",
    "|&",
];

const COND_BINOPS: &[&str] = &["==", "=", "!=", "=~", "<", ">"];

/// The words of an open `[[` after its last `[[`, `&&`, `||`, `!` or `(`:
/// Some(range) when a newline there is a syntax error to bash.
fn cond_tail(ls: &[Node], src: &str) -> Option<Range<usize>> {
    let k: Vec<&str> = ls.iter().map(|l| lk(*l, src)).collect();
    // A `(` glued to the word before it is part of a pattern or regex.
    let from = (0..k.len())
        .rposition(|i| {
            ["[[", "&&", "||", "!"].contains(&k[i])
                || (k[i] == "(" && (i == 0 || ls[i - 1].end_byte() != ls[i].start_byte()))
        })
        .map(|p| p + 1)
        .unwrap_or(0);
    let last = ls.len() - 1;
    if ls[last].is_error() && src[ls[last].byte_range()].starts_with(['"', '\'']) {
        return None;
    }
    // An open `$(`, `${` or `(` in the tail: bash is still reading it.
    let mut depth = 0i32;
    for x in &k[from..] {
        match *x {
            "$(" | "${" | "(" | "$((" | "((" => depth += 1,
            ")" | "}" | "))" => depth -= 1,
            _ => {}
        }
    }
    if depth > 0 {
        return None;
    }
    // Group leaves into words: a gap with a blank starts a new word.
    let mut words: Vec<(bool, usize)> = Vec::new(); // (is operator, leaf index)
    for i in from..ls.len() {
        let unary_word = k[i].len() == 2
            && src[ls[i].byte_range()].starts_with('-')
            && (i == from || ls[i - 1].end_byte() < ls[i].start_byte());
        let op = COND_BINOPS.contains(&k[i])
            || ls[i].kind() == "test_operator"
            || (ls[i].kind() == "word" && unary_word);
        let glued = i > from
            && ls[i - 1].end_byte() == ls[i].start_byte()
            && !op
            && !words.last().map(|w| w.0).unwrap_or(true);
        if !glued {
            words.push((op, i));
        }
    }
    let shape: Vec<bool> = words.iter().map(|w| w.0).collect();
    // A single `]` word at the end (tree-sitter may leave it out of the
    // leaves).
    let rest = &src[ls[last].end_byte()..];
    let rest = rest[..rest.find('\n').unwrap_or(rest.len())].trim();
    let spaced = last > 0 && ls[last - 1].end_byte() < ls[last].start_byte();
    if (k[last] == "]" && spaced) || rest == "]" {
        let at = ls[last].end_byte() + src[ls[last].end_byte()..].find(']').unwrap_or(0);
        return Some(if rest == "]" {
            at..at + 1
        } else {
            ls[last].byte_range()
        });
    }
    let complete =
        matches!(shape.as_slice(), [] | [true, false] | [false, true, false]) || shape.len() > 3;
    if complete {
        None
    } else {
        Some(ls[last].byte_range())
    }
}

/// Checks the leaves of an ERROR that reaches the end for words that need
/// more on the same line: `for`, `case`, `select` or `function` last on it,
/// `for (`, a `for ((` closed by a single `)`, `case WORD` not followed by
/// `in`, and an open `[[` that a newline cannot continue. Some(range) is a
/// syntax error there.
fn same_line_error(n: Node, ls: &[Node], src: &str) -> Option<Range<usize>> {
    let k: Vec<&str> = ls.iter().map(|l| lk(*l, src)).collect();
    let last = ls.len() - 1;
    // `for`, `case`, `select`, `function` last on the line.
    if ["for", "case", "select", "function"].contains(&k[last]) {
        return Some(ls[last].byte_range());
    }
    for i in 0..k.len() {
        // `for (` ; `for ((...)` closed by one `)`.
        if k[i] == "for" && k.get(i + 1) == Some(&"(") {
            return Some(ls[i + 1].byte_range());
        }
        if k[i] == "for" && k.get(i + 1) == Some(&"((") {
            let j = (i + 2..k.len())
                .find(|j| [")", "))", "do"].contains(&k[*j]))
                .filter(|j| k[*j] == ")");
            if j == Some(last) && k[last - 1] != ")" {
                return Some(ls[last].byte_range());
            }
        }
    }
    // `case WORD X`: X must be `in`.
    let mut c = n.walk();
    let kids: Vec<Node> = n.children(&mut c).collect();
    for i in 0..kids.len() {
        if kids[i].kind() == "case" && i + 2 < kids.len() && kids[i + 2].kind() != "in" {
            return Some(kids[i + 2].byte_range());
        }
    }
    // The last `[[` still open: a newline is fine only after `&&`, `||`,
    // `!`, `(` or `[[`, or inside an open quote.
    if let Some(p) = k.iter().rposition(|x| *x == "[[")
        && !k[p..].contains(&"]]")
    {
        return cond_tail(&ls[p..], src);
    }
    None
}

/// Checks the leaves of an ERROR that reaches the end: Some(range) is a
/// syntax error there, None means the command is unfinished.
fn trailing_error(ls: &[Node], before: Option<Node>, src: &str) -> Option<Range<usize>> {
    let k: Vec<&str> = ls.iter().map(|l| lk(*l, src)).collect();
    let bk = before.map(|b| lk(b, src)).unwrap_or("");
    let end = ls.last().map(|l| l.end_byte()).unwrap_or(0);
    let eol = end..end;
    // A lone keyword that needs a word on the same line.
    if k == ["case"] || k == ["function"] || k == ["for"] || k == ["select"] {
        return Some(ls[0].byte_range());
    }
    // A leading separator: after nothing or another separator it is an
    // error; a lone trailing `|`, `&&`, `||` asks for more.
    if SEPS.contains(&k[0]) {
        if bk.is_empty() || SEPS.contains(&bk) || ls.len() > 1 && [";", "&"].contains(&k[1]) {
            return Some(ls[0].byte_range());
        }
        if k.len() == 1 {
            return if ["|", "&&", "||", "|&"].contains(&k[0]) {
                None
            } else {
                Some(ls[0].byte_range())
            };
        }
        return trailing_error(&ls[1..], Some(ls[0]), src);
    }
    if [")", "}", "fi", "done", "esac", "then", "else", "elif", "do"].contains(&k[0]) {
        return Some(ls[0].byte_range());
    }
    if k == ["("] && !(bk.is_empty() || SEPS.contains(&bk) || OPENERS.contains(&bk)) {
        return Some(ls[0].byte_range());
    }
    if REDIRS.contains(k.last().unwrap()) && !k.contains(&"((") && !k.contains(&"$((") {
        return Some(ls[ls.len() - 1].byte_range());
    }
    for i in 0..k.len() {
        let line_start = src[..ls[i].start_byte()]
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .trim()
            .is_empty();
        if line_start
            && [";", "&", "|", "&&", "||", "|&"].contains(&k[i])
            && (i > 0 || !bk.is_empty())
        {
            return Some(ls[i].byte_range());
        }
    }
    for i in 1..k.len() {
        let adjacent = !src[ls[i - 1].end_byte()..ls[i].start_byte()].contains('\n');
        // `;` or `&` right after an opener or separator.
        if [";", "&", ";;"].contains(&k[i])
            && (SEPS.contains(&k[i - 1]) || OPENERS.contains(&k[i - 1]))
            && adjacent
        {
            return Some(ls[i].byte_range());
        }
        // A body that is empty: `then fi`, `do done`, `else fi`.
        if ["then", "do", "else"].contains(&k[i - 1])
            && ["fi", "done", "else", "elif", "esac", "}"].contains(&k[i])
        {
            return Some(ls[i].byte_range());
        }
    }
    // `for NAME in WORDS ;` must be followed by `do`.
    for i in 0..k.len() {
        if (k[i] == "for" || k[i] == "select") && k.get(i + 2) == Some(&"in") {
            let mut j = i + 3;
            while j < k.len()
                && !SEPS.contains(&k[j])
                && !k[j].is_empty()
                && !src[ls[j - 1].end_byte()..ls[j].start_byte()].contains('\n')
            {
                j += 1;
            }
            if j < k.len() && (k[j] == ";" || k[j].is_empty()) {
                j += 1;
            }
            if j < k.len() && k[j] != "do" {
                return Some(ls[j].byte_range());
            }
        }
    }
    // Inside `[[`: a newline is fine only where a new term can start or
    // after `&&`/`||`.
    if k[0] == "[[" {
        return cond_tail(ls, src);
    }
    // Case patterns: after `in` or `;;` a pattern must end with `)` on the
    // same line; `)` right after `in`/`;;` has no pattern.
    let case_at = k.iter().rposition(|x| *x == "case");
    let in_case = case_at.map(|c| k[c..].contains(&"in")).unwrap_or(false);
    for i in case_at.unwrap_or(k.len())..k.len() {
        if in_case && i > 0 && [";;", "in"].contains(&k[i - 1]) && k[i] == ")" {
            return Some(ls[i].byte_range());
        }
    }
    if in_case
        && let Some(pos) = k
            .iter()
            .rposition(|x| [";;", "in", ";&", ";;&"].contains(x))
            .filter(|p| *p > case_at.unwrap())
        && (pos + 1 < k.len() && !k[pos + 1..].contains(&")") && k[pos] != "in"
            || (k[pos] == "in" && pos + 1 < k.len() && !k[pos + 1..].contains(&")")))
    {
        return Some(eol);
    }
    // Bracket and keyword pairs: a closer that matches nothing open.
    let mut stack: Vec<&str> = Vec::new();
    for i in 0..k.len() {
        if in_complete(ls[i]) {
            continue;
        }
        let top = stack.last().copied();
        match k[i] {
            "(" if top == Some("esac") => {}
            "(" | "$(" | "<(" | ">(" => stack.push(")"),
            "((" | "$((" => {
                stack.push(")");
                stack.push(")");
            }
            "{" | "${" => stack.push("}"),
            "[[" => stack.push("]]"),
            "if" => stack.push("fi"),
            "case" => stack.push("esac"),
            "do" => stack.push("done"),
            ")" if top == Some("esac") => {}
            "))" => {
                if stack.ends_with(&[")", ")"]) {
                    stack.truncate(stack.len() - 2);
                } else {
                    return Some(ls[i].byte_range());
                }
            }
            c @ (")" | "}" | "]]" | "fi" | "esac" | "done") => {
                if top == Some(c) {
                    stack.pop();
                } else {
                    return Some(ls[i].byte_range());
                }
            }
            _ => {}
        }
    }
    None
}

fn analyse(tree: &Tree, src: &str, open_brace: bool) -> Status {
    let end = code_end(tree, src);
    let all = leaves(tree.root_node());
    let code: Vec<Node> = all
        .iter()
        .copied()
        .filter(|l| l.kind() != "comment" && !l.is_missing())
        .collect();
    let code: Vec<Node> = code
        .into_iter()
        .filter(|l| l.end_byte() > l.start_byte())
        .collect();
    let prev_code = |pos: usize| {
        let k = code.partition_point(|l| l.end_byte() <= pos);
        (k > 0).then(|| code[k - 1])
    };
    let next_code = |pos: usize| {
        code.get(code.partition_point(|l| l.start_byte() < pos))
            .copied()
    };

    let mut wrong: Vec<Range<usize>> = Vec::new();
    let mut unfinished = false;

    // Reserved words where bash expects a command; an empty `{ }`; `{` glued
    // to the next word; `;;` outside `case`; an empty body (`then fi`,
    // `do done`).
    walk(tree.root_node(), &mut |n| {
        if n.kind() == "command"
            && let Some(name) = n.child_by_field_name("name")
            && n.child(0).map(|c| c.id()) == Some(name.id())
            && CLOSERS.contains(&&src[name.byte_range()])
        {
            wrong.push(name.byte_range());
        }
        if n.kind() == "compound_statement" && !n.has_error() {
            if n.named_child_count() == 0 {
                wrong.push(n.child(n.child_count() - 1).unwrap().byte_range());
            } else if let Some(open) = n.child(0) {
                let after = src.as_bytes().get(open.end_byte()).copied().unwrap_or(b' ');
                if open.kind() == "{" && !b" \t\n".contains(&after) {
                    wrong.push(n.child(n.child_count() - 1).unwrap().byte_range());
                }
            }
        }
        if n.child_count() == 0 && [";;", ";&", ";;&"].contains(&n.kind()) {
            let in_case = n
                .parent()
                .map(|p| p.kind() == "case_item" || p.kind() == "case_statement")
                .unwrap_or(false);
            if !in_case && !n.parent().map(|p| p.is_error()).unwrap_or(false) {
                wrong.push(n.byte_range());
            }
        }
        if n.child_count() == 0
            && ["then", "do", "else"].contains(&n.kind())
            && !n.is_missing()
            && let Some(nx) = next_code(n.end_byte())
            && ["fi", "done", "else", "elif"].contains(&nx.kind())
            && !nx.parent().map(|p| p.is_error()).unwrap_or(false)
        {
            wrong.push(nx.byte_range());
        }
        // `[[ a ==` newline `b ]]`: a newline right after a binary operator.
        if n.kind() == "binary_expression"
            && n.parent()
                .map(|p| p.kind() == "test_command")
                .unwrap_or(false)
            && let Some(op) = n.child_by_field_name("operator")
            && let Some(nx) = next_code(op.end_byte())
            && src[op.end_byte()..nx.start_byte()].contains('\n')
            && !["&&", "||"].contains(&op.kind())
        {
            wrong.push(op.byte_range());
        }
    });
    // `esac` right after a word is an argument to bash, so the case is still
    // open.
    for l in &all {
        if l.kind() == "esac"
            && !l.is_missing()
            && let Some(pv) = prev_code(l.start_byte())
        {
            let gap = &src[pv.end_byte()..l.start_byte()];
            let sep = [";", ";;", ";&", ";;&", "&", "in", ")", "("].contains(&pv.kind())
                || gap.contains('\n');
            if !sep {
                unfinished = true;
            }
        }
    }

    for n in problems(tree) {
        let at_end = if n.is_missing() {
            n.start_byte() >= end
        } else {
            n.end_byte() >= end
        };
        if n.is_missing() {
            let before = prev_code(n.start_byte());
            let bk = before.map(|b| b.kind()).unwrap_or("");
            // A `}` of `${` that tree-sitter closed early: the lexical scan
            // says whether it is really open.
            if n.kind() == "}" && n.parent().map(|p| p.kind() == "expansion").unwrap_or(false) {
                unfinished |= open_brace;
                continue;
            }
            if n.kind() == ";" && ["}", ")", "fi", "done", "esac"].contains(&bk) {
                continue;
            }
            if !at_end {
                // Underline the token after the gap.
                let r = next_code(n.start_byte())
                    .map(|x| x.byte_range())
                    .unwrap_or(n.start_byte()..n.start_byte());
                wrong.push(r);
                continue;
            }
            match n.kind() {
                // `[` is a plain command to bash, and a lone `$` is literal.
                "]" | "special_variable_name" => {}
                "word" if REDIRS.contains(&bk) => wrong.push(before.unwrap().byte_range()),
                "]]" if n
                    .parent()
                    .map(|p| {
                        p.named_child_count() == 1
                            && !p.named_child(0).unwrap().kind().ends_with("expression")
                    })
                    .unwrap_or(false) =>
                {
                    wrong.push(
                        before
                            .map(|b| b.byte_range())
                            .unwrap_or(n.start_byte()..n.start_byte()),
                    )
                }
                "]]" if bk == "]]" || src[..n.start_byte()].trim_end().ends_with("]]") => wrong
                    .push(
                        before
                            .map(|b| b.byte_range())
                            .unwrap_or(n.start_byte()..n.start_byte()),
                    ),
                _ => unfinished = true,
            }
            continue;
        }
        // ERROR
        if n.parent().map(|p| p.is_error()).unwrap_or(false) {
            continue; // judged with the outer ERROR
        }
        let ls: Vec<Node> = leaves(n)
            .into_iter()
            .filter(|l| l.kind() != "comment" && !l.is_missing())
            .collect();
        let text = src[n.byte_range()].trim();
        if text == "$" {
            continue;
        }
        if ls.is_empty() {
            if at_end {
                unfinished = true;
            }
            continue;
        }
        if !at_end {
            // Tree-sitter skipped these tokens. Bash names the first token it
            // cannot take: an operator the ERROR starts with, or else the
            // token after the skipped words.
            let fk = lk(ls[0], src);
            let op = text != "()"
                && (SEPS.contains(&fk)
                    || REDIRS.contains(&fk)
                    || [
                        ")", "}", "(", "fi", "done", "esac", "then", "else", "elif", "do", "in",
                    ]
                    .contains(&fk));
            let r = if op {
                ls[0].byte_range()
            } else {
                next_code(n.end_byte())
                    .map(|x| x.byte_range())
                    .unwrap_or(ls[0].byte_range())
            };
            wrong.push(r);
            continue;
        }
        // An ERROR reaching the end.
        if text == "!" || lk(ls[0], src) == "[" || (text.starts_with("$((") && text.ends_with("))"))
        {
            continue; // bash accepts these; any error comes when it runs
        }
        if let Some(pv) = prev_code(n.start_byte()) {
            let closes = [")", "}", "fi", "done", "esac", "]]", "))"].contains(&pv.kind())
                && pv
                    .parent()
                    .map(|p| {
                        [
                            "compound_statement",
                            "subshell",
                            "if_statement",
                            "do_group",
                            "case_statement",
                            "test_command",
                            "arithmetic_expansion",
                        ]
                        .contains(&p.kind())
                    })
                    .unwrap_or(false);
            if closes
                && ls[0].kind() == "word"
                && !src[pv.end_byte()..ls[0].start_byte()].contains('\n')
            {
                wrong.push(ls[0].byte_range());
                continue;
            }
        }
        if let Some(r) = same_line_error(n, &ls, src) {
            wrong.push(r);
            continue;
        }
        match trailing_error(&ls, prev_code(n.start_byte()), src) {
            Some(r) => wrong.push(r),
            None => unfinished = true,
        }
    }
    if let Some(r) = wrong.iter().min_by_key(|r| r.start) {
        return Status::Wrong(r.clone());
    }
    if unfinished {
        return Status::Unfinished;
    }
    Status::Fine
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_underline_covers_the_whole_word() {
        assert_eq!(word_around("echo $x )", 8..9), 8..9);
        assert_eq!(word_around("echo $x", 6..7), 5..7);
        assert_eq!(word_around("echo a; fi", 8..10), 8..10);
        assert_eq!(word_around("x;;y", 1..3), 1..3);
        assert_eq!(word_around("ls ", 3..4), 3..3);
        assert_eq!(word_around("echo 日本x", 11..12), 5..12);
    }

    #[test]
    fn lines_inside_a_here_document() {
        assert!(heredoc_open("cat <<EOF"));
        assert!(heredoc_open("cat <<EOF\nhi"));
        assert!(heredoc_open("cat <<-'END' | sort"));
        assert!(!heredoc_open("cat <<EOF\nhi\nEOF"));
        assert!(!heredoc_open("cat <<<x"));
        assert!(!heredoc_open("echo hi"));
    }

    /// One line of `tests/data/syntax-cases.txt`.
    struct Case {
        text: String,
        /// Bash 5.2's answer with extglob off, then on.
        bash: [String; 2],
        /// The answer these rules give with extglob off, then on.
        want: [String; 2],
    }

    fn cases() -> Vec<Case> {
        include_str!("../tests/data/syntax-cases.txt")
            .lines()
            .filter(|line| !line.starts_with('#'))
            .map(|line| {
                let (columns, text) = line.split_once('\t').expect("a tab before the text");
                let c: Vec<&str> = columns.split(' ').collect();
                Case {
                    text: unescape(text),
                    bash: [c[0].to_owned(), c[1].to_owned()],
                    want: [c[3].to_owned(), c[4].to_owned()],
                }
            })
            .collect()
    }

    /// Undoes the file's `\\`, `\n` and `\t` escapes.
    fn unescape(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            match (c, c == '\\') {
                (_, true) => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => out.push('\\'),
                },
                (c, false) => out.push(c),
            }
        }
        out
    }

    /// `status` in the file's notation.
    fn notation(status: &Status) -> String {
        match status {
            Status::Fine => "F".to_owned(),
            Status::Unfinished => "U".to_owned(),
            Status::Wrong(r) => format!("W:{}-{}", r.start, r.end),
        }
    }

    /// The cases whose answer differs from the recorded one.
    fn differing(extglob: bool) -> Vec<String> {
        let column = usize::from(extglob);
        let mut checker = Checker::new();
        cases()
            .into_iter()
            .filter_map(|case| {
                let got = notation(&checker.check(&case.text, extglob));
                let want = &case.want[column];
                (got != *want).then(|| format!("{:?}: want {want}, got {got}", case.text))
            })
            .collect()
    }

    #[test]
    fn recorded_answers_with_extglob_off() {
        let differ = differing(false);
        assert!(
            differ.is_empty(),
            "{} differ:\n{}",
            differ.len(),
            differ.join("\n")
        );
    }

    #[test]
    fn recorded_answers_with_extglob_on() {
        let differ = differing(true);
        assert!(
            differ.is_empty(),
            "{} differ:\n{}",
            differ.len(),
            differ.join("\n")
        );
    }

    /// Enter adds a line only where bash would ask for one: no recorded answer
    /// says Unfinished where bash runs the command.
    #[test]
    fn never_unfinished_where_bash_runs_the_command() {
        for case in cases() {
            for column in 0..2 {
                if case.bash[column] == "F" {
                    assert_ne!(case.want[column], "U", "{:?}", case.text);
                }
            }
        }
    }

    #[test]
    fn examples() {
        let mut c = Checker::new();
        assert_eq!(c.check("echo hi", false), Status::Fine);
        assert_eq!(c.check("for x in a; do", false), Status::Unfinished);
        assert_eq!(c.check("echo 'abc", false), Status::Unfinished);
        assert_eq!(c.check("cat <<EOF\nhi", false), Status::Unfinished);
        assert_eq!(c.check("cat <<EOF\nhi\nEOF", false), Status::Fine);
        assert_eq!(c.check("ls |", false), Status::Unfinished);
        assert_eq!(c.check("echo a \\", false), Status::Unfinished);
        assert_eq!(c.check("echo )", false), Status::Wrong(5..6));
        assert_eq!(c.check("echo a; fi", false), Status::Wrong(8..10));
        assert_eq!(c.check("for", false), Status::Wrong(0..3));
        assert_eq!(
            c.check("for x in a b; do\n    echo $x\ndone", false),
            Status::Fine
        );
        assert_eq!(c.check("echo @(a|b)", false), Status::Wrong(6..7));
        assert_eq!(c.check("echo @(a|b)", true), Status::Fine);
    }

    /// Bash with extglob on runs these; with it off, each `(` is an error.
    #[test]
    fn extended_patterns() {
        let mut c = Checker::new();
        for (text, off) in [
            ("ls @(a|b)*", Status::Wrong(4..5)),
            // tree-sitter reads `@()` as the start of a function definition.
            ("echo @()", Status::Unfinished),
            ("cp !(x) dest/", Status::Wrong(4..5)),
            ("mv *.@(jpg|png) dir/", Status::Wrong(6..7)),
            ("echo !(a).txt", Status::Wrong(6..7)),
            ("echo @(a)b", Status::Wrong(6..7)),
            ("cat @(a) x", Status::Wrong(5..6)),
            ("echo a @(x) c", Status::Wrong(8..9)),
        ] {
            assert_eq!(c.check(text, true), Status::Fine, "{text:?}");
            assert_eq!(c.check(text, false), off, "{text:?}");
        }
    }

    /// A quoted `(` or `)` inside an extended pattern does not open or close
    /// it.
    #[test]
    fn quoted_parens_in_patterns() {
        let mut c = Checker::new();
        for text in [
            "echo @(')') x",
            "echo @(\")\") x",
            "echo @(a')'b) x",
            "echo @(a|\"b)\") x",
            "echo @($')') x",
            "rm -- @(*')'*) x",
            "ls @(*')') && echo ok",
            "ls @(*\"(\"*) x",
            "ls !(*' ('*) x",
        ] {
            assert_eq!(c.check(text, true), Status::Fine, "{text:?}");
        }
    }

    /// An escaped `$` does not start a `$'…'` quote, so a backslash in the
    /// `'…'` after it is plain text.
    #[test]
    fn escaped_dollar_before_quote() {
        let mut c = Checker::new();
        for extglob in [false, true] {
            assert_eq!(c.check("echo \\$'\\' x", extglob), Status::Fine);
            assert_eq!(c.check("echo \\\\$'\\'' x", extglob), Status::Fine);
        }
        assert_eq!(c.check("echo @(\\$'\\') x", true), Status::Fine);
    }

    /// Inside arithmetic, `*(` is a product, not a pattern.
    #[test]
    fn arithmetic_is_not_a_pattern() {
        let mut c = Checker::new();
        for text in [
            "echo $((2*(3+4)))",
            "(( x = 2*(3) ))",
            "for ((i=0; i<2*(3); i++)); do :; done",
        ] {
            for extglob in [false, true] {
                assert_eq!(c.check(text, extglob), Status::Fine, "{text:?} {extglob}");
            }
        }
    }

    /// A command of only assignments and redirections runs: tree-sitter
    /// wants a command name after them.
    #[test]
    fn assignments_with_a_redirection() {
        let mut c = Checker::new();
        for text in [
            "out=$(git rev-parse HEAD) 2>/dev/null",
            "x=1 >f",
            "x=1 &>/dev/null",
            "x=1 <<< a",
            "x=( a ) >f",
            ">f x=1",
            "x=1 >f; echo",
            "x=1 >f && echo",
            "if true; then\n    x=1 >/dev/null\nfi",
        ] {
            assert_eq!(c.check(text, false), Status::Fine, "{text:?}");
        }
        assert_eq!(c.check("x=1 >f\nfi", false), Status::Wrong(7..9));
    }
}
