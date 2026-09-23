//! Labels the pieces of a command line (command words, options, strings, …)
//! using tree-sitter-bash.

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
}

/// A labelled piece of the line, as byte offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
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
        painter.paint(tree.root_node(), None);
        merge(&labels)
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
    fn paint(&mut self, node: Node, parent: Option<&str>) {
        if let Some(kind) = self.kind_of(node, parent) {
            self.labels[node.byte_range()].fill(Some(kind));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.paint(child, Some(node.kind()));
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

    fn kind_of(&mut self, node: Node, parent: Option<&str>) -> Option<Kind> {
        if !node.is_named() {
            let token = node.kind();
            return if KEYWORDS.contains(&token) {
                Some(Kind::Keyword)
            } else if DECLARATIONS.contains(&token) {
                Some(self.command_kind(token))
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

/// Turns per-byte labels into spans, joining neighbours with the same label.
fn merge(labels: &[Option<Kind>]) -> Vec<Span> {
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
                (Operator, "|"),
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
                (Operator, ";"),
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
        assert_eq!(labels("ls |"), [(Command, "ls"), (Operator, "|")]);
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
                (Operator, ";"),
                (Keyword, "do"),
                (Command, "echo"),
                (Variable, "$i"),
            ]
        );
        assert_eq!(labels("echo \\"), [(Command, "echo")]);
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
}
