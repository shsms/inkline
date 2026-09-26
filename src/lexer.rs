//! Labels the pieces of a command line (command words, options, strings, …)
//! using tree-sitter-bash.

use std::ops::Range;

use tree_sitter::{Node, Parser};

/// What a piece of the line is. The order matters: `colors` indexes a table by
/// `kind as usize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Command,
    /// A command word that names nothing bash can run.
    Unknown,
    Keyword,
    Option,
    String,
    Variable,
    Operator,
    Comment,
    /// A numeric literal. Only a mode server produces this (never the
    /// lexer itself).
    Number,
    /// A function name. Only a mode server produces this (never the
    /// lexer itself).
    Function,
    /// A mark between parts, such as a pipeline's `|`. Drawn with the
    /// `operator` colour unless `separator` is set.
    Separator,
}

impl Kind {
    /// How many kinds there are: the length of a table indexed by
    /// `Kind as usize`.
    pub const COUNT: usize = Kind::Separator as usize + 1;
}

/// A labelled piece of the line, as byte offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

/// Where a position in the line is, for the pairing rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    Code,
    Comment,
    /// Inside a string opened with this quote.
    Quoted(char),
}

const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "while", "until", "do", "done", "case",
    "esac", "function", "select", "[[", "]]", "!",
];

/// Command words that tree-sitter-bash parses as keywords. Bash has no
/// `unsetenv`; the grammar treats it like `unset`.
const DECLARATIONS: &[&str] = &[
    "export", "declare", "local", "readonly", "typeset", "unset", "unsetenv",
];

const OPERATORS: &[&str] = &[
    "|", "|&", "&&", "||", ";", ";;", "&", ">", ">>", "<", "<<", "<<-", "<<<", ">&", "<&", "&>",
    "&>>", ">|", "$(", "<(", ">(", "`",
];

pub struct Lexer {
    parser: Parser,
}

impl Lexer {
    pub fn new() -> Lexer {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_bash::LANGUAGE.into())
            .expect("tree-sitter-bash is built for this tree-sitter version");
        Lexer { parser }
    }

    /// The labelled pieces of `line`, in order. Bytes that belong to no piece
    /// (spaces, plain arguments) are left out. A command name is `Unknown` when
    /// `known` returns false for its whole text.
    pub fn spans(&mut self, line: &str, mut known: impl FnMut(&str) -> bool) -> Vec<Span> {
        let Some(tree) = self.parser.parse(line, None) else {
            return Vec::new();
        };
        let mut labels = vec![None; line.len()];
        let mut painter = Painter {
            line,
            known: &mut known,
            labels: &mut labels,
        };
        painter.paint(tree.root_node(), None, None);
        merge(&labels)
    }

    /// Parses `line` and returns the tree, or `None` if tree-sitter refuses
    /// to parse it at all.
    pub fn tree(&mut self, line: &str) -> Option<tree_sitter::Tree> {
        self.parser.parse(line, None)
    }

    /// Whether the cursor at byte `pos` is in code, a comment or a string.
    pub fn context_at(&mut self, line: &str, pos: usize) -> Context {
        let Some(tree) = self.parser.parse(line, None) else {
            return Context::Code;
        };
        // The byte before the cursor finds the string or comment the cursor
        // sits at the end of.
        let mut node = tree
            .root_node()
            .descendant_for_byte_range(pos.saturating_sub(1), pos);
        while let Some(n) = node {
            // An unfinished string or substitution also contains the position
            // just past its last byte.
            let inside =
                n.start_byte() < pos && (pos < n.end_byte() || n.has_error() || n.is_error());
            let text = &line[n.byte_range()];
            match n.kind() {
                "command_substitution" | "process_substitution" if inside => return Context::Code,
                "comment" if n.start_byte() < pos => return Context::Comment,
                "string" | "translated_string" if inside => return Context::Quoted('"'),
                "raw_string" | "ansi_c_string" if inside => return Context::Quoted('\''),
                "ERROR" if inside && text.starts_with(['\'', '"']) => {
                    return Context::Quoted(text.chars().next().unwrap_or('"'));
                }
                _ => {}
            }
            node = n.parent();
        }
        Context::Code
    }
}

impl Default for Lexer {
    fn default() -> Lexer {
        Lexer::new()
    }
}

struct Painter<'a> {
    line: &'a str,
    known: &'a mut dyn FnMut(&str) -> bool,
    /// One label per byte of `line`.
    labels: &'a mut [Option<Kind>],
}

impl Painter<'_> {
    /// Labels the bytes of `node`, then lets its children override that, so
    /// the innermost label wins (a `$HOME` inside a string is a variable).
    /// `parent` is the kind of the node's parent; looking it up with
    /// `Node::parent` would search down from the root again.
    /// `arithmetic` is where the `((…))` of an arithmetic `for` around this
    /// node is on the line, also while the loop is still being typed and
    /// parses as an error. It is `None` inside `$(…)`, `` `…` ``, `<(…)` or
    /// `>(…)`, whose `;`s end commands.
    fn paint(&mut self, node: Node, parent: Option<&str>, mut arithmetic: Option<Range<usize>>) {
        let in_arithmetic = arithmetic
            .as_ref()
            .is_some_and(|r| r.contains(&node.start_byte()));
        if let Some(kind) = self.kind_of(node, parent, in_arithmetic) {
            self.labels[node.byte_range()].fill(Some(kind));
        }
        let mut cursor = node.walk();
        let mut after_for = false;
        for child in node.children(&mut cursor) {
            // Any other `((` is two `(` that start subshells.
            if after_for && child.kind() == "((" {
                arithmetic = Some(child.start_byte()..brackets_end(self.line, child.end_byte()));
            }
            after_for = child.kind() == "for";
            // A command inside `$(…)`, `` `…` ``, `<(…)` or `>(…)` has its
            // own `;`s.
            let inner = match child.kind() {
                "command_substitution" | "process_substitution" => None,
                _ => arithmetic.clone(),
            };
            self.paint(child, Some(node.kind()), inner);
        }
    }

    /// `Unknown` when `known` says `name` names nothing bash can run.
    fn command_kind(&mut self, name: &str) -> Kind {
        if (self.known)(name) {
            Kind::Command
        } else {
            Kind::Unknown
        }
    }

    fn kind_of(&mut self, node: Node, parent: Option<&str>, in_arithmetic: bool) -> Option<Kind> {
        if !node.is_named() {
            let token = node.kind();
            return if KEYWORDS.contains(&token) {
                Some(Kind::Keyword)
            } else if DECLARATIONS.contains(&token) {
                Some(self.command_kind(token))
            } else if separates_commands(token, parent, in_arithmetic) {
                Some(Kind::Separator)
            } else if OPERATORS.contains(&token)
                || (token == ")"
                    && matches!(
                        parent,
                        Some("command_substitution" | "process_substitution")
                    ))
            {
                Some(Kind::Operator)
            } else {
                None
            };
        }
        let line = self.line;
        let text = &line[node.byte_range()];
        match node.kind() {
            "command_name" => Some(self.command_kind(text)),
            "string" | "raw_string" | "ansi_c_string" | "translated_string" | "heredoc_body"
            | "heredoc_start" => Some(Kind::String),
            "simple_expansion" | "expansion" | "variable_name" => Some(Kind::Variable),
            "comment" => Some(Kind::Comment),
            "test_operator" => Some(Kind::Option),
            "file_descriptor" => Some(Kind::Operator),
            "word" if parent == Some("command") && text.starts_with('-') => Some(Kind::Option),
            // An unclosed single quote parses as an error, not a string.
            "ERROR" if text.starts_with(['\'', '"']) => Some(Kind::String),
            _ => None,
        }
    }
}

/// Where the `((…))` whose `((` ends at `from` closes on `line`: just after
/// its `))`, or at the end of the line when it is not closed yet. The
/// brackets in the text are counted, not the tree's nodes, because an error
/// parse can split a `))` into two `)` or put it inside another node.
/// Brackets in quotes or after a backslash do not count.
fn brackets_end(line: &str, from: usize) -> usize {
    let mut open = 2;
    let mut quote = None;
    let mut escaped = false;
    for (i, byte) in line.bytes().enumerate().skip(from) {
        if escaped {
            escaped = false;
            continue;
        }
        // Inside '…' nothing is special; inside "…" only \ and ".
        match (quote, byte) {
            (Some(b'\''), b'\'') => quote = None,
            (Some(b'\''), _) => {}
            (_, b'\\') => escaped = true,
            (Some(_), b'"') => quote = None,
            (Some(_), _) => {}
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'(') => open += 1,
            (None, b')') => {
                open -= 1;
                if open == 0 {
                    return i + 1;
                }
            }
            (None, _) => {}
        }
    }
    line.len()
}

/// Whether `token`, whose parent is `parent`, stands between two commands:
/// a pipeline's `|` or `|&`, or a `;` other than the two inside an
/// arithmetic `for ((…; …; …))` (`in_arithmetic`, as for `paint`). A `|`
/// between `case` patterns has a `case_item` parent.
fn separates_commands(token: &str, parent: Option<&str>, in_arithmetic: bool) -> bool {
    match token {
        "|" | "|&" => parent == Some("pipeline"),
        ";" => !in_arithmetic,
        _ => false,
    }
}

/// Turns per-byte labels into spans, joining neighbours with the same label.
pub fn merge(labels: &[Option<Kind>]) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let Some(kind) = *label else { continue };
        match spans.last_mut() {
            Some(last) if last.end == i && last.kind == kind => last.end = i + 1,
            _ => spans.push(Span {
                start: i,
                end: i + 1,
                kind,
            }),
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use Kind::*;

    fn labels(line: &str) -> Vec<(Kind, &str)> {
        Lexer::new()
            .spans(line, |_| true)
            .into_iter()
            .map(|s| (s.kind, &line[s.start..s.end]))
            .collect()
    }

    #[test]
    fn labels_a_full_command_line() {
        assert_eq!(
            labels("ls -la \"foo $HOME\" | grep x > out 2>&1 # hi"),
            [
                (Command, "ls"),
                (Option, "-la"),
                (String, "\"foo "),
                (Variable, "$HOME"),
                (String, "\""),
                (Separator, "|"),
                (Command, "grep"),
                (Operator, ">"),
                (Operator, "2>&"),
                (Comment, "# hi"),
            ]
        );
    }

    #[test]
    fn labels_keywords_and_test_options() {
        assert_eq!(
            labels("if [[ -f x ]]; then"),
            [
                (Keyword, "if"),
                (Keyword, "[["),
                (Option, "-f"),
                (Keyword, "]]"),
                (Separator, ";"),
                (Keyword, "then"),
            ]
        );
    }

    #[test]
    fn labels_assignments_and_declarations() {
        assert_eq!(
            labels("FOO=1 cmd --opt=val"),
            [(Variable, "FOO"), (Command, "cmd"), (Option, "--opt=val")]
        );
        assert_eq!(labels("export A=1"), [(Command, "export"), (Variable, "A")]);
    }

    #[test]
    fn labels_half_typed_lines() {
        assert_eq!(
            labels("echo \"unterminated $x"),
            [
                (Command, "echo"),
                (String, "\"unterminated "),
                (Variable, "$x")
            ]
        );
        assert_eq!(labels("echo 'raw"), [(Command, "echo"), (String, "'raw")]);
        assert_eq!(labels("ls |"), [(Command, "ls"), (Separator, "|")]);
        assert_eq!(
            labels("echo $(date"),
            [(Command, "echo"), (Operator, "$("), (Command, "date")]
        );
        assert_eq!(
            labels("for i in 1 2; do echo $i"),
            [
                (Keyword, "for"),
                (Variable, "i"),
                (Keyword, "in"),
                (Separator, ";"),
                (Keyword, "do"),
                (Command, "echo"),
                (Variable, "$i"),
            ]
        );
        assert_eq!(labels("echo \\"), [(Command, "echo")]);
    }

    #[test]
    fn pipes_and_semicolons_between_commands_are_separators() {
        assert_eq!(
            labels("ls | wc"),
            [(Command, "ls"), (Separator, "|"), (Command, "wc")]
        );
        assert_eq!(
            labels("ls |& wc"),
            [(Command, "ls"), (Separator, "|&"), (Command, "wc")]
        );
        assert_eq!(
            labels("a; b"),
            [(Command, "a"), (Separator, ";"), (Command, "b")]
        );
        assert_eq!(
            labels("echo $(a; b | c)"),
            [
                (Command, "echo"),
                (Operator, "$("),
                (Command, "a"),
                (Separator, ";"),
                (Command, "b"),
                (Separator, "|"),
                (Command, "c"),
                (Operator, ")"),
            ]
        );
    }

    /// The kinds of the `;`s on `line`, in order.
    fn semicolons(line: &str) -> Vec<Kind> {
        labels(line)
            .into_iter()
            .filter(|&(_, text)| text == ";")
            .map(|(kind, _)| kind)
            .collect()
    }

    /// Only a `for`'s `((` holds operators. Where an error parse splits its
    /// `))` or puts part of the loop in another node, the brackets in the
    /// text still say where it ends.
    #[test]
    fn a_semicolon_after_the_brackets_close_is_a_separator() {
        assert_eq!(
            semicolons("for ((a; b; c)); d"),
            [Operator, Operator, Separator]
        );
        assert_eq!(
            semicolons("((echo a; echo b); echo c)"),
            [Separator, Separator],
            "`((` as two `(` that start subshells"
        );
        assert_eq!(
            semicolons("for ((i=$(a; b) ; i<3; i++)); do c; d; done"),
            [
                Separator, Operator, Operator, Separator, Separator, Separator
            ],
            "a command inside the loop's brackets"
        );
        assert_eq!(
            semicolons("for ((i=0; i<\"(\"; i++)); do a; b; done"),
            [Operator, Operator, Separator, Separator, Separator],
            "a bracket in quotes"
        );
        assert_eq!(
            semicolons("for ((i=0; i<'('; i++)); do a; done"),
            [Operator, Operator, Separator, Separator],
            "a bracket in single quotes"
        );
        assert_eq!(
            semicolons("for ((i=\\(; i<3; i++)); do a; b; done"),
            [Operator, Operator, Separator, Separator, Separator],
            "a bracket after a backslash"
        );
        assert_eq!(
            semicolons("for ((i=`a;b` ; i<3; i++)); do c; done"),
            [Separator, Operator, Operator, Separator, Separator],
            "a command in backquotes"
        );
        assert_eq!(
            semicolons("for ((i=0;\ni<3; i++)); do a; b; done"),
            [Operator, Operator, Separator, Separator, Separator],
            "the `;`s after a loop's `((…))` split over two lines"
        );
    }

    #[test]
    fn other_marks_between_commands_stay_operators() {
        assert_eq!(
            labels("a && b"),
            [(Command, "a"), (Operator, "&&"), (Command, "b")]
        );
        assert_eq!(
            labels("a || b"),
            [(Command, "a"), (Operator, "||"), (Command, "b")]
        );
        assert_eq!(labels("a &"), [(Command, "a"), (Operator, "&")]);
    }

    /// A `|` between the patterns of a `case` item, `;;`, the `;`s inside
    /// an arithmetic `for ((…))` and a `|` or `;` in a string or comment
    /// separate no commands.
    #[test]
    fn case_patterns_and_quoted_marks_are_not_separators() {
        assert_eq!(
            labels("case x in a|b) ;; esac"),
            [
                (Keyword, "case"),
                (Keyword, "in"),
                (Operator, "|"),
                (Operator, ";;"),
                (Keyword, "esac"),
            ]
        );
        assert_eq!(
            labels("for ((i=0; i<3; i++)); do :; done"),
            [
                (Keyword, "for"),
                (Variable, "i"),
                (Operator, ";"),
                (Operator, "<"),
                (Operator, ";"),
                (Separator, ";"),
                (Keyword, "do"),
                (Command, ":"),
                (Separator, ";"),
                (Keyword, "done"),
            ]
        );
        assert_eq!(
            labels("for ((i=0; i<3"),
            [
                (Keyword, "for"),
                (Variable, "i"),
                (Operator, ";"),
                (Operator, "<"),
            ],
            "a loop still being typed"
        );
        assert_eq!(
            labels("for ((i=0; i<3; i++))"),
            [
                (Keyword, "for"),
                (Variable, "i"),
                (Operator, ";"),
                (Operator, "<"),
                (Operator, ";"),
            ],
            "a loop without its body yet"
        );
        assert_eq!(
            labels("for ((i=(1); i<3"),
            [
                (Keyword, "for"),
                (Variable, "i"),
                (Operator, ";"),
                (Operator, "<"),
            ],
            "brackets inside the loop's `((…))`"
        );
        assert_eq!(
            labels("echo 'a|b;c' # d; e | f"),
            [
                (Command, "echo"),
                (String, "'a|b;c'"),
                (Comment, "# d; e | f")
            ]
        );
    }

    #[test]
    fn command_names_are_checked_whole() {
        let mut seen = Vec::new();
        let spans = Lexer::new().spans("l's' x | nope", |word| {
            seen.push(word.to_string());
            word != "nope"
        });
        assert_eq!(seen, ["l's'", "nope"]);
        assert_eq!(spans.last().map(|s| s.kind), Some(Unknown));
    }

    /// tree-sitter-bash parses `unsetenv` like `unset`, but bash has no such
    /// builtin.
    #[test]
    fn unsetenv_is_checked_like_a_command_name() {
        let spans = Lexer::new().spans("unsetenv FOO", |word| word != "unsetenv");
        assert_eq!(spans.first().map(|s| s.kind), Some(Unknown));
        let spans = Lexer::new().spans("unset FOO", |word| word != "unsetenv");
        assert_eq!(spans.first().map(|s| s.kind), Some(Command));
    }

    #[test]
    fn byte_offsets_survive_wide_characters() {
        assert_eq!(
            labels("echo 日本 \"x\""),
            [(Command, "echo"), (String, "\"x\"")]
        );
    }

    #[test]
    fn context_inside_and_after_strings() {
        let mut lexer = Lexer::new();
        let cases = [
            ("echo \"abc", 9, Context::Quoted('"')),
            ("echo \"abc\"", 9, Context::Quoted('"')),
            ("echo \"abc\"", 10, Context::Code),
            ("echo 'raw", 9, Context::Quoted('\'')),
            ("echo 'ab'", 8, Context::Quoted('\'')),
            ("echo 'ab'", 9, Context::Code),
            ("echo \"\"", 6, Context::Quoted('"')),
            ("echo ''", 6, Context::Quoted('\'')),
            ("echo \"$x", 8, Context::Quoted('"')),
        ];
        for (line, pos, want) in cases {
            assert_eq!(lexer.context_at(line, pos), want, "{line:?} at {pos}");
        }
    }

    #[test]
    fn context_in_comments_and_substitutions() {
        let mut lexer = Lexer::new();
        let cases = [
            ("ls # note", 9, Context::Comment),
            ("ls # note", 3, Context::Code),
            ("echo $(ls", 9, Context::Code),
            ("echo `ls", 8, Context::Code),
            ("echo (", 6, Context::Code),
            ("ls", 2, Context::Code),
            ("", 0, Context::Code),
        ];
        for (line, pos, want) in cases {
            assert_eq!(lexer.context_at(line, pos), want, "{line:?} at {pos}");
        }
    }
}
