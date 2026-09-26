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

/// How well an item matched, lower first: every item that starts with the
/// typed text, then the others by the length of their tightest match and
/// then by where it starts (in characters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Prefix,
    Gapped { span: usize, start: usize },
}

/// Whether the characters of `typed` appear in `text` in order.
fn in_order(typed: &str, text: &str) -> bool {
    let mut rest = text.chars();
    typed.chars().all(|c| rest.any(|t| t == c))
}

/// The tightest place `typed` (not empty) matches in `text`, in order with
/// gaps: its length and start, in characters. The earliest wins a tie.
///
/// Each round matches `typed` forward from `from` to the earliest end it can
/// reach, then back from that end to the latest start that still matches:
/// no start in between gives a shorter match. The next round starts after
/// that start, so the time grows with the text's length times `typed`'s,
/// not with the square of the text's length.
fn tightest(typed: &[char], text: &[char]) -> Option<(usize, usize)> {
    typed.first()?;
    let mut best: Option<(usize, usize)> = None;
    let mut from = 0;
    loop {
        let mut want = 0;
        let mut end = from;
        while want < typed.len() && end < text.len() {
            if text[end] == typed[want] {
                want += 1;
            }
            end += 1;
        }
        if want < typed.len() {
            // No later start can match all of `typed` either.
            break;
        }
        let mut start = end;
        while want > 0 {
            start -= 1;
            if text[start] == typed[want - 1] {
                want -= 1;
            }
        }
        let span = end - start;
        if best.is_none_or(|(s, _)| span < s) {
            best = Some((span, start));
        }
        from = start + 1;
    }
    best
}

/// How `typed` matches `text` under `style`, or None for no match.
fn rank(style: Style, typed: &str, text: &str) -> Option<Rank> {
    if text.starts_with(typed) {
        return Some(Rank::Prefix);
    }
    match style {
        Style::Prefix => None,
        Style::Fuzzy => {
            let typed: Vec<char> = typed.chars().collect();
            let text: Vec<char> = text.chars().collect();
            tightest(&typed, &text).map(|(span, start)| Rank::Gapped { span, start })
        }
    }
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

/// `line` with `item` taken in.
fn applied(line: &str, item: &Item) -> String {
    format!("{}{}{}", &line[..item.start], item.text, &line[item.end..])
}

/// The items of one source that match, in the order `style` gives: those
/// that start with the typed text in list order, then the others.
fn ordered(line: &str, point: usize, style: Style, items: Vec<Item>) -> Vec<Item> {
    let mut ranked: Vec<(Rank, Item)> = items
        .into_iter()
        .filter(|i| i.start <= point && point <= i.end && i.end <= line.len())
        .filter_map(|i| Some((rank(style, &line[i.start..point], &i.text)?, i)))
        .collect();
    // A stable sort keeps list order among equal ranks.
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, item)| item).collect()
}

/// The menu's items for `line` with the cursor at byte `point`: `history`,
/// then `whole` (the suggestion hook's line), then `words` (the completion
/// hook's items), each source matched and ordered under `style`. An item
/// that cannot be drawn, that would leave the line as it is, or that gives
/// the same line as an item before it is left out.
pub fn assemble(
    line: &str,
    point: usize,
    style: Style,
    history: Vec<Item>,
    whole: Option<Item>,
    words: Vec<Item>,
) -> Vec<Item> {
    let mut seen = HashSet::from([line.to_owned()]);
    let mut items = Vec::new();
    let groups = [
        ordered(line, point, style, history),
        ordered(line, point, style, whole.into_iter().collect()),
        ordered(line, point, style, words),
    ];
    for item in groups.into_iter().flatten() {
        if drawable(&item.text) && seen.insert(applied(line, &item)) {
            items.push(item);
        }
    }
    items
}

/// The grey text for `item`: the rest of it after the typed text. Only when
/// the cursor is at the end of the line, the item ends at the cursor, and it
/// starts with the typed text and is longer.
pub fn grey<'a>(line: &str, point: usize, item: &'a Item) -> Option<&'a str> {
    if point != line.len() || item.end != point || item.start > point {
        return None;
    }
    let rest = item.text.strip_prefix(&line[item.start..point])?;
    (!rest.is_empty()).then_some(rest)
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

    fn word(text: &str, start: usize, end: usize) -> Item {
        Item {
            text: text.to_owned(),
            start,
            end,
            source: Source::Lisp,
        }
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

    #[test]
    fn prefix_matching_is_case_sensitive() {
        let items = assemble(
            "x Sw",
            4,
            Style::Prefix,
            vec![],
            None,
            vec![word("switch", 2, 4), word("Swap", 2, 4)],
        );
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["Swap"]);
    }

    #[test]
    fn an_empty_typed_text_matches_every_item() {
        let items = assemble(
            "git ",
            4,
            Style::Prefix,
            vec![],
            None,
            vec![word("status", 4, 4), word("stash", 4, 4)],
        );
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn fuzzy_puts_prefixes_first_then_the_tightest() {
        let items = assemble(
            "sw",
            2,
            Style::Fuzzy,
            vec![],
            None,
            vec![
                word("show-switch", 0, 2),
                word("s-w", 0, 2),
                word("swap", 0, 2),
                word("abs--w", 0, 2),
                word("switch", 0, 2),
                word("xyz", 0, 2),
            ],
        );
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        // Prefixes in list order; then the tightest: span 2 ("show-switch"
        // has "sw" at 5), span 3 ("s-w"), span 4 ("abs--w").
        assert_eq!(texts, ["swap", "switch", "show-switch", "s-w", "abs--w"]);
    }

    #[test]
    fn the_tightest_match_may_overlap_an_earlier_one() {
        let chars = |s: &str| s.chars().collect::<Vec<char>>();
        let t = |typed: &str, text: &str| tightest(&chars(typed), &chars(text));
        // "abxac" matches "abc" from 0; "acbc" from 3 is shorter.
        assert_eq!(t("abc", "abxacbc"), Some((4, 3)));
        // The earliest wins a tie.
        assert_eq!(t("ab", "abxab"), Some((2, 0)));
        assert_eq!(t("ab", "ba"), None);
    }

    #[test]
    fn sources_keep_their_order_and_repeats_go() {
        let hist = vec![Item {
            text: "git status".into(),
            start: 0,
            end: 6,
            source: Source::History,
        }];
        let whole = Some(Item {
            text: "git status".into(),
            start: 0,
            end: 6,
            source: Source::Lisp,
        });
        let words = vec![word("status", 4, 6), word("stash", 4, 6)];
        let items = assemble("git st", 6, Style::Prefix, hist, whole, words);
        let texts: Vec<(&str, char)> = items
            .iter()
            .map(|i| (i.text.as_str(), i.source.letter()))
            .collect();
        // "status" gives the same line as the history item.
        assert_eq!(texts, [("git status", 'h'), ("stash", 'l')]);
    }

    #[test]
    fn items_that_change_nothing_or_cannot_be_drawn_go() {
        let items = assemble(
            "ab",
            2,
            Style::Prefix,
            vec![],
            None,
            vec![word("ab", 0, 2), word("ab\x07c", 0, 2), word("abc", 0, 2)],
        );
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["abc"]);
    }

    #[test]
    fn grey_is_the_rest_after_the_typed_text() {
        let item = word("status", 4, 6);
        assert_eq!(grey("git st", 6, &item), Some("atus"));
        // Not at the end of the line, or the item does not end at the cursor.
        assert_eq!(grey("git st x", 6, &item), None);
        assert_eq!(grey("git st", 6, &word("status", 4, 5)), None);
        // A fuzzy match has no grey text; nothing left to add has none.
        assert_eq!(grey("git st", 6, &word("sxt", 4, 6)), None);
        assert_eq!(grey("git st", 6, &word("st", 4, 6)), None);
    }
}
