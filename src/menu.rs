//! The completion menu's items: where they come from, which of them match
//! what was typed, and in what order they are listed.

use std::collections::HashSet;

/// Where an item came from, shown as a letter at the start of its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    History,
    Lisp,
}

impl Source {
    pub fn letter(self) -> char {
        match self {
            Source::History => 'h',
            Source::Lisp => 'l',
        }
    }
}

/// A possible completion: `text` replaces bytes `start..end` of the line.
/// The typed text it is matched against is the line from `start` to the
/// cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub source: Source,
}

/// How the typed text matches an item (`inkline-completion-style`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// The item starts with the typed text.
    #[default]
    Prefix,
    /// The typed text's characters appear in the item in order, with gaps.
    Fuzzy,
}

/// The most history items gathered for one line that start with it, and in
/// the fuzzy style also the most that match it with gaps.
pub const HISTORY_LIMIT: usize = 50;

/// Whether `text` can be drawn: it has no control character other than a
/// newline or a tab.
pub fn drawable(text: &str) -> bool {
    !text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
}

/// Whether the characters of `typed` appear in `text` in order.
fn in_order(typed: &str, text: &str) -> bool {
    let mut rest = text.chars();
    typed.chars().all(|c| rest.any(|t| t == c))
}

/// Gathers history items for `line`: offered entries come newest first.
pub struct HistoryGather {
    line: String,
    style: Style,
    items: Vec<Item>,
    seen: HashSet<String>,
    /// How many of `items` start with the line.
    prefixed: usize,
}

impl HistoryGather {
    pub fn new(line: &str, style: Style) -> HistoryGather {
        HistoryGather {
            line: line.to_owned(),
            style,
            items: Vec::new(),
            seen: HashSet::new(),
            prefixed: 0,
        }
    }

    /// Takes `entry` when it matches the whole line and is not the line
    /// itself, an entry already taken, or one that cannot be drawn. Entries
    /// that start with the line and those that match with gaps each stop
    /// being taken at `HISTORY_LIMIT`, so gapped matches never crowd out an
    /// older entry that starts with the line. True once `HISTORY_LIMIT`
    /// entries that start with the line are taken: the scan can stop.
    pub fn offer(&mut self, entry: &str) -> bool {
        let prefixed = entry.starts_with(&self.line);
        let matches = match self.style {
            Style::Prefix => prefixed,
            Style::Fuzzy => {
                prefixed
                    || (self.items.len() - self.prefixed < HISTORY_LIMIT
                        && in_order(&self.line, entry))
            }
        };
        if matches && entry != self.line && !self.seen.contains(entry) && drawable(entry) {
            self.prefixed += usize::from(prefixed);
            self.seen.insert(entry.to_owned());
            self.items.push(Item {
                text: entry.to_owned(),
                start: 0,
                end: self.line.len(),
                source: Source::History,
            });
        }
        self.prefixed >= HISTORY_LIMIT
    }

    pub fn into_items(self) -> Vec<Item> {
        self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(line: &str, style: Style, entries: &[&str]) -> Vec<String> {
        let mut gather = HistoryGather::new(line, style);
        for entry in entries {
            if gather.offer(entry) {
                break;
            }
        }
        gather.into_items().into_iter().map(|i| i.text).collect()
    }

    #[test]
    fn history_is_newest_first_without_repeats_or_the_line() {
        // Entries come newest first, as `ffi::history_find_map` gives them.
        let found = history(
            "git st",
            Style::Prefix,
            &["git status", "ls", "git stash", "git status", "git st"],
        );
        assert_eq!(found, ["git status", "git stash"]);
    }

    #[test]
    fn history_skips_entries_that_cannot_be_drawn() {
        let found = history(
            "echo",
            Style::Prefix,
            &["echo \x1b[31m", "echo\thi", "echo a\nb"],
        );
        assert_eq!(found, ["echo\thi", "echo a\nb"]);
    }

    #[test]
    fn history_stops_at_the_limit() {
        let entries: Vec<String> = (0..80).map(|i| format!("x{i}")).collect();
        let refs: Vec<&str> = entries.iter().map(String::as_str).collect();
        let mut gather = HistoryGather::new("x", Style::Prefix);
        let mut offered = 0;
        for entry in &refs {
            offered += 1;
            if gather.offer(entry) {
                break;
            }
        }
        assert_eq!(offered, HISTORY_LIMIT);
        assert_eq!(gather.into_items().len(), HISTORY_LIMIT);
    }

    #[test]
    fn a_history_item_replaces_the_whole_line() {
        let mut gather = HistoryGather::new("git st", Style::Prefix);
        gather.offer("git status");
        assert_eq!(
            gather.into_items(),
            [Item {
                text: "git status".into(),
                start: 0,
                end: 6,
                source: Source::History
            }]
        );
    }

    /// Newer entries that match only with gaps do not crowd out an older one
    /// that starts with the line.
    #[test]
    fn fuzzy_history_keeps_room_for_entries_that_start_with_the_line() {
        let mut entries: Vec<String> = (0..80).map(|i| format!("l-{i}-s")).collect();
        entries.push("ls -la".to_owned());
        let refs: Vec<&str> = entries.iter().map(String::as_str).collect();
        let found = history("ls", Style::Fuzzy, &refs);
        assert_eq!(found.len(), HISTORY_LIMIT + 1);
        assert_eq!(found.last().map(String::as_str), Some("ls -la"));
    }

    #[test]
    fn fuzzy_history_takes_letters_in_order() {
        let found = history("gst", Style::Fuzzy, &["git status", "gist", "ls"]);
        assert_eq!(found, ["git status", "gist"]);
    }
}
