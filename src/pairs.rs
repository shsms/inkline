//! The rules for inserting and deleting bracket and quote pairs.

use crate::lexer::Context;

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Do what the key normally does.
    Fallback,
    /// Insert both characters and put the cursor between them.
    InsertPair(char, char),
    /// Move over the next character instead of inserting another one.
    Skip,
    /// Delete the characters on both sides of the cursor.
    DeletePair,
}

fn closer(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' | '\'' | '`' => Some(open),
        _ => None,
    }
}

fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '`')
}

/// The characters before and after the cursor.
fn around(line: &str, point: usize) -> (Option<char>, Option<char>) {
    (
        line[..point].chars().next_back(),
        line[point..].chars().next(),
    )
}

/// What typing the opening character `typed` should do.
pub fn open(
    line: &str,
    point: usize,
    typed: char,
    explicit_count: bool,
    context: Context,
) -> Action {
    let Some(close) = closer(typed) else {
        return Action::Fallback;
    };
    let (prev, next) = around(line, point);
    if explicit_count {
        return Action::Fallback;
    }
    if next == Some(typed) && closes_here(line, point, typed, context) {
        return Action::Skip;
    }
    if next.is_some_and(|c| c.is_alphanumeric() || c == '_' || is_quote(c)) {
        return Action::Fallback;
    }
    if is_quote(typed) && prev.is_some_and(char::is_alphanumeric) {
        return Action::Fallback;
    }
    if prev == Some('\\') {
        return Action::Fallback;
    }
    match context {
        Context::Code => Action::InsertPair(typed, close),
        Context::Comment | Context::Quoted(_) => Action::Fallback,
    }
}

/// Whether the quote at `point` closes the string or substitution the cursor is
/// in, rather than opening a new one after it.
fn closes_here(line: &str, point: usize, typed: char, context: Context) -> bool {
    match typed {
        '`' => line[..point].matches('`').count() % 2 == 1,
        '"' | '\'' => context == Context::Quoted(typed),
        _ => false,
    }
}

/// What typing the closing bracket `typed` should do.
pub fn close(line: &str, point: usize, typed: char, explicit_count: bool) -> Action {
    if !explicit_count && matches!(typed, ')' | ']' | '}') && around(line, point).1 == Some(typed) {
        Action::Skip
    } else {
        Action::Fallback
    }
}

/// What Backspace should do.
pub fn backspace(line: &str, point: usize, explicit_count: bool) -> Action {
    match around(line, point) {
        (Some(prev), Some(next)) if !explicit_count && closer(prev) == Some(next) => {
            Action::DeletePair
        }
        _ => Action::Fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Context::{Code, Comment, Quoted};

    #[test]
    fn pairs_in_code() {
        assert_eq!(
            open("echo ", 5, '(', false, Code),
            Action::InsertPair('(', ')')
        );
        assert_eq!(
            open("echo ", 5, '[', false, Code),
            Action::InsertPair('[', ']')
        );
        assert_eq!(
            open("echo ", 5, '{', false, Code),
            Action::InsertPair('{', '}')
        );
        assert_eq!(
            open("echo ", 5, '"', false, Code),
            Action::InsertPair('"', '"')
        );
        assert_eq!(
            open("echo ", 5, '\'', false, Code),
            Action::InsertPair('\'', '\'')
        );
        assert_eq!(
            open("echo ", 5, '`', false, Code),
            Action::InsertPair('`', '`')
        );
    }

    #[test]
    fn plain_insert_cases() {
        assert_eq!(
            open("echo ", 5, '(', true, Code),
            Action::Fallback,
            "count prefix"
        );
        assert_eq!(
            open("echo x", 5, '(', false, Code),
            Action::Fallback,
            "before a word"
        );
        assert_eq!(
            open("echo _x", 5, '[', false, Code),
            Action::Fallback,
            "before an underscore"
        );
        assert_eq!(
            open("echo don", 8, '\'', false, Code),
            Action::Fallback,
            "apostrophe"
        );
        assert_eq!(
            open("echo \\", 6, '(', false, Code),
            Action::Fallback,
            "escaped"
        );
        assert_eq!(
            open("ls # ", 5, '(', false, Comment),
            Action::Fallback,
            "comment"
        );
        assert_eq!(
            open("echo \"a", 7, '(', false, Quoted('"')),
            Action::Fallback,
            "in a string"
        );
        assert_eq!(
            open("echo \"a ", 8, '\'', false, Quoted('"')),
            Action::Fallback,
            "other quote"
        );
        assert_eq!(
            open("echo \"a", 7, '"', false, Quoted('"')),
            Action::Fallback,
            "closing quote"
        );
        assert_eq!(
            open("x", 1, 'a', false, Code),
            Action::Fallback,
            "not an opener"
        );
    }

    #[test]
    fn quote_before_an_existing_string_is_plain() {
        assert_eq!(open("echo \"foo\"", 5, '"', false, Code), Action::Fallback);
        assert_eq!(open("echo 'x'", 5, '\'', false, Code), Action::Fallback);
        assert_eq!(open("echo `ls`", 5, '`', false, Code), Action::Fallback);
    }

    #[test]
    fn backtick_types_over_the_closing_backtick() {
        assert_eq!(open("echo `ls`", 8, '`', false, Code), Action::Skip);
    }

    #[test]
    fn quotes_type_over_their_closing_quote() {
        assert_eq!(open("echo \"a\"", 7, '"', false, Quoted('"')), Action::Skip);
        assert_eq!(open("echo ''", 6, '\'', false, Quoted('\'')), Action::Skip);
    }

    #[test]
    fn closers_type_over() {
        assert_eq!(close("echo ()", 6, ')', false), Action::Skip);
        assert_eq!(close("echo (", 6, ')', false), Action::Fallback);
        assert_eq!(close("echo ()", 6, ')', true), Action::Fallback);
        assert_eq!(close("echo (]", 6, ')', false), Action::Fallback);
    }

    #[test]
    fn backspace_deletes_an_empty_pair() {
        assert_eq!(backspace("echo ()", 6, false), Action::DeletePair);
        assert_eq!(backspace("echo \"\"", 6, false), Action::DeletePair);
        assert_eq!(backspace("echo (a)", 7, false), Action::Fallback);
        assert_eq!(backspace("echo ()", 6, true), Action::Fallback);
        assert_eq!(backspace("", 0, false), Action::Fallback);
    }
}
