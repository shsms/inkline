//! Works out the suggestion from a history entry, and how much of it each
//! accept command takes.

/// The part of `entry` after `line`, if `entry` starts with a non-empty `line`
/// and has more before its first control character other than a newline or a
/// tab: an escape sequence would not show as one column.
pub fn rest<'a>(line: &str, entry: &'a str) -> Option<&'a str> {
    if line.is_empty() {
        return None;
    }
    let rest = entry.strip_prefix(line)?;
    let end = rest
        .find(|c: char| c.is_control() && c != '\n' && c != '\t')
        .unwrap_or(rest.len());
    let rest = &rest[..end];
    (!rest.is_empty()).then_some(rest)
}

/// The most lines of a suggestion to show: the value of
/// `INKLINE_SUGGESTION_LINES`, or 5.
pub fn line_limit(value: Option<&str>) -> usize {
    value
        .and_then(|v| v.trim().parse().ok())
        .filter(|&n: &usize| n >= 1)
        .unwrap_or(5)
}

/// The first `count` characters of `suggestion`.
pub fn chars(suggestion: &str, count: usize) -> &str {
    let end = suggestion
        .char_indices()
        .nth(count)
        .map_or(suggestion.len(), |(i, _)| i);
    &suggestion[..end]
}

/// The first `count` words of `suggestion`, where a word is what readline's
/// `forward-word` moves over: letters and digits.
pub fn words(suggestion: &str, count: usize) -> &str {
    let mut end = 0;
    for _ in 0..count {
        let rest = &suggestion[end..];
        let word_start = rest.find(char::is_alphanumeric).unwrap_or(rest.len());
        let word = &rest[word_start..];
        let word_len = word
            .find(|c: char| !c.is_alphanumeric())
            .unwrap_or(word.len());
        end += word_start + word_len;
    }
    &suggestion[..end]
}

/// All of `suggestion`, whatever the count.
pub fn all(suggestion: &str, _count: usize) -> &str {
    suggestion
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_of_a_matching_entry() {
        assert_eq!(rest("git st", "git status"), Some("atus"));
        assert_eq!(rest("git st", "git stash"), Some("ash"));
        assert_eq!(rest("git st", "ls"), None);
        assert_eq!(rest("git status", "git status"), None);
        assert_eq!(rest("", "git status"), None);
    }

    #[test]
    fn rest_keeps_newlines_and_tabs() {
        assert_eq!(
            rest("for", "for i in 1\ndo echo $i; done"),
            Some(" i in 1\ndo echo $i; done")
        );
        assert_eq!(rest("for i in 1", "for i in 1\ndone"), Some("\ndone"));
        assert_eq!(rest("echo", "echo a\tb"), Some(" a\tb"));
        assert_eq!(rest("echo", "echo\u{1b}[m"), None);
    }

    #[test]
    fn line_limit_from_the_setting() {
        assert_eq!(line_limit(None), 5);
        assert_eq!(line_limit(Some("3")), 3);
        assert_eq!(line_limit(Some("0")), 5);
        assert_eq!(line_limit(Some("many")), 5);
    }

    #[test]
    fn chars_counts_characters_not_bytes() {
        assert_eq!(chars("atus", 2), "at");
        assert_eq!(chars("日本", 1), "日");
        assert_eq!(chars("ab", 5), "ab");
    }

    #[test]
    fn words_like_forward_word() {
        assert_eq!(words("atus --short", 1), "atus");
        assert_eq!(words("atus --short", 2), "atus --short");
        assert_eq!(words(" --short x", 1), " --short");
        assert_eq!(words("a_b", 1), "a");
        assert_eq!(words("", 1), "");
    }

    #[test]
    fn all_ignores_the_count() {
        assert_eq!(all("atus --short", 1), "atus --short");
    }
}
