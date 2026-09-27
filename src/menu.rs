//! The completion menu's items: where they come from, which of them match
//! what was typed, and in what order they are listed.

use std::collections::HashSet;

pub mod mode;

/// Where an item came from, shown as a letter at the start of its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    History,
    Lisp,
    Mode,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::History, Source::Lisp, Source::Mode];

    pub fn letter(self) -> char {
        match self {
            Source::History => 'h',
            Source::Lisp => 'l',
            Source::Mode => 'm',
        }
    }

    /// Its name in `inkline-menu-sources`.
    pub fn name(self) -> &'static str {
        match self {
            Source::History => "history",
            Source::Lisp => "lisp",
            Source::Mode => "mode",
        }
    }

    pub fn named(name: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|s| s.name() == name)
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
    /// A few words saying what the item is, shown after it; it takes no
    /// part in matching.
    pub note: Option<String>,
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

/// How items are matched: the style and whether case counts
/// (`inkline-completion-ignore-case`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Matching {
    pub style: Style,
    pub ignore_case: bool,
}

impl Matching {
    /// Whether `a` and `b` match as characters.
    fn same(self, a: char, b: char) -> bool {
        a == b || (self.ignore_case && a.to_lowercase().eq(b.to_lowercase()))
    }

    /// Whether `text` starts with `typed`: `Prefix` in the typed case,
    /// `OtherCasePrefix` only when case is ignored.
    fn starts(self, typed: &str, text: &str) -> Option<Rank> {
        if text.starts_with(typed) {
            return Some(Rank::Prefix);
        }
        if !self.ignore_case {
            return None;
        }
        let mut rest = text.chars();
        typed
            .chars()
            .all(|c| rest.next().is_some_and(|t| self.same(t, c)))
            .then_some(Rank::OtherCasePrefix)
    }

    /// Whether the characters of `typed` appear in `text` in order.
    fn in_order(self, typed: &str, text: &str) -> bool {
        let mut rest = text.chars();
        typed.chars().all(|c| rest.any(|t| self.same(t, c)))
    }
}

/// The most history items gathered for one line of each kind: those that
/// start with it in the same case, those that start with it in another case,
/// and those that match it with gaps.
pub const HISTORY_LIMIT: usize = 50;

/// Whether `text` can be drawn: it has no control character other than a
/// newline or a tab.
pub fn drawable(text: &str) -> bool {
    !text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
}

/// How well an item matched, lower first: every item that starts with the typed
/// text, then those that start with it in another case, then the others by the
/// length of their tightest match and then by where it starts (in characters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Prefix,
    OtherCasePrefix,
    Gapped { span: usize, start: usize },
}

/// The tightest place `typed` (not empty) matches in `text`, in order with
/// gaps: its length and start, in characters. The earliest wins a tie.
///
/// Each round matches `typed` forward from `from` to the earliest end it can
/// reach, then back from that end to the latest start that still matches:
/// no start in between gives a shorter match. The next round starts after
/// that start, so the time grows with the text's length times `typed`'s,
/// not with the square of the text's length.
fn tightest(how: Matching, typed: &[char], text: &[char]) -> Option<(usize, usize)> {
    typed.first()?;
    let mut best: Option<(usize, usize)> = None;
    let mut from = 0;
    loop {
        let mut want = 0;
        let mut end = from;
        while want < typed.len() && end < text.len() {
            if how.same(text[end], typed[want]) {
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
            if how.same(text[start], typed[want - 1]) {
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

/// How `typed` matches `text`, or None for no match.
fn rank(how: Matching, typed: &str, text: &str) -> Option<Rank> {
    if let Some(rank) = how.starts(typed, text) {
        return Some(rank);
    }
    match how.style {
        Style::Prefix => None,
        Style::Fuzzy => {
            let typed: Vec<char> = typed.chars().collect();
            let text: Vec<char> = text.chars().collect();
            tightest(how, &typed, &text).map(|(span, start)| Rank::Gapped { span, start })
        }
    }
}

/// How `typed` matches `item`, or None for no match. A mode server's item whose
/// text starts with a quote mark (`` ` ``, `'` or `"`) also matches against the
/// text after that mark, so `fi` finds `` `first name` ``: starting with the
/// typed text there ranks as starting with it, and in the fuzzy style a match
/// with gaps there ranks by where it falls after the mark. The better of the
/// two ranks counts.
fn item_rank(how: Matching, typed: &str, item: &Item) -> Option<Rank> {
    let whole = rank(how, typed, &item.text);
    if item.source != Source::Mode || whole == Some(Rank::Prefix) {
        return whole;
    }
    let Some(after) = item.text.strip_prefix(['`', '\'', '"']) else {
        return whole;
    };
    whole.into_iter().chain(rank(how, typed, after)).min()
}

/// Gathers history items for `line`: offered entries come newest first.
pub struct HistoryGather {
    line: String,
    how: Matching,
    items: Vec<Item>,
    seen: HashSet<String>,
    /// How many of `items` are of each `Kind`.
    taken: [usize; 3],
}

/// How a history entry matches the line.
#[derive(Clone, Copy)]
enum Kind {
    /// It starts with the line in the same case.
    SameCase,
    /// It starts with the line in another case.
    OtherCase,
    /// It does not start with the line: it is taken only when it matches
    /// with gaps, in the fuzzy style.
    Gapped,
}

impl HistoryGather {
    pub fn new(line: &str, how: Matching) -> HistoryGather {
        HistoryGather {
            line: line.to_owned(),
            how,
            items: Vec::new(),
            seen: HashSet::new(),
            taken: [0; 3],
        }
    }

    /// Takes `entry` when it matches the whole line and is not the line
    /// itself, an entry already taken, or one that cannot be drawn. Each
    /// `Kind` of entry stops being taken at `HISTORY_LIMIT`, so a kind that
    /// ranks lower never crowds out an older entry of a kind that ranks
    /// higher. True once `HISTORY_LIMIT` entries that start with the line in
    /// the same case are taken: the scan can stop.
    pub fn offer(&mut self, entry: &str) -> bool {
        let kind = match self.how.starts(&self.line, entry) {
            Some(Rank::Prefix) => Kind::SameCase,
            Some(Rank::OtherCasePrefix) => Kind::OtherCase,
            Some(Rank::Gapped { .. }) | None => Kind::Gapped,
        };
        let room = self.taken[kind as usize] < HISTORY_LIMIT;
        let matches = room
            && match (kind, self.how.style) {
                (Kind::SameCase | Kind::OtherCase, _) => true,
                (Kind::Gapped, Style::Prefix) => false,
                (Kind::Gapped, Style::Fuzzy) => self.how.in_order(&self.line, entry),
            };
        if matches && entry != self.line && !self.seen.contains(entry) && drawable(entry) {
            self.taken[kind as usize] += 1;
            self.seen.insert(entry.to_owned());
            self.items.push(Item {
                text: entry.to_owned(),
                start: 0,
                end: self.line.len(),
                source: Source::History,
                note: None,
            });
        }
        self.taken[Kind::SameCase as usize] >= HISTORY_LIMIT
    }

    pub fn into_items(self) -> Vec<Item> {
        self.items
    }
}

/// `line` with `item` taken in.
fn applied(line: &str, item: &Item) -> String {
    format!("{}{}{}", &line[..item.start], item.text, &line[item.end..])
}

/// The items of one source that match, in the order `how` gives: those
/// that start with the typed text in list order, then the others. See
/// [`item_rank`] for a mode server's quoted items.
fn ordered(line: &str, point: usize, how: Matching, items: Vec<Item>) -> Vec<Item> {
    let mut ranked: Vec<(Rank, Item)> = items
        .into_iter()
        .filter(|i| i.start <= point && point <= i.end && i.end <= line.len())
        .filter_map(|i| Some((item_rank(how, &line[i.start..point], &i)?, i)))
        .collect();
    // A stable sort keeps list order among equal ranks.
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, item)| item).collect()
}

/// The menu's items for `line` with the cursor at byte `point`: `history`,
/// then `whole` (the suggestion hook's line), then `mode` (a mode server's
/// items), then `words` (the completion hook's items), each source matched
/// and ordered under `how`. An item that cannot be drawn, that would leave
/// the line as it is, or that gives the same line as an item before it is
/// left out.
pub fn assemble(
    line: &str,
    point: usize,
    how: Matching,
    history: Vec<Item>,
    whole: Option<Item>,
    mode: Vec<Item>,
    words: Vec<Item>,
) -> Vec<Item> {
    let mut seen = HashSet::from([line.to_owned()]);
    let mut items = Vec::new();
    let groups = [
        ordered(line, point, how, history),
        ordered(line, point, how, whole.into_iter().collect()),
        ordered(line, point, how, mode),
        ordered(line, point, how, words),
    ];
    for item in groups.into_iter().flatten() {
        if drawable(&item.text) && seen.insert(applied(line, &item)) {
            items.push(item);
        }
    }
    items
}

/// Which items the menu lists: those from `sources` of which at least
/// `min_chars` characters are typed (`inkline-menu-sources` and
/// `inkline-menu-min-chars`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub sources: Vec<Source>,
    pub min_chars: usize,
}

impl Listed {
    /// Whether the menu for `line` with the cursor at `point` lists `item`.
    pub fn lists(&self, line: &str, point: usize, item: &Item) -> bool {
        self.sources.contains(&item.source)
            && line
                .get(item.start..point)
                .is_some_and(|typed| typed.chars().count() >= self.min_chars)
    }
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

/// The menu for one line and cursor: its items, the picked row, and
/// whether the last draw showed it.
#[derive(Clone, Debug)]
pub struct Menu {
    pub line: String,
    pub point: usize,
    /// The items the menu lists.
    pub items: Vec<Item>,
    /// The top item of all those it was made from, listed or not: the grey
    /// text's item when none is picked.
    pub top: Option<Item>,
    /// The picked item's index; None until `C-n` or `C-p`.
    pub picked: Option<usize>,
    /// Whether the last draw in plain editing put the menu on screen.
    pub shown: bool,
    /// Whether it was made while Lisp ran; the Lisp hooks give no items
    /// then.
    pub lisp_ran: bool,
    /// Whether a mode server was asked for items for this line and cursor
    /// and had not answered when the menu was made.
    pub mode_waiting: bool,
}

impl Menu {
    pub fn new(line: &str, point: usize, items: Vec<Item>) -> Menu {
        Menu {
            line: line.to_owned(),
            point,
            top: items.first().cloned(),
            items,
            picked: None,
            shown: false,
            lisp_ran: false,
            mode_waiting: false,
        }
    }

    /// Whether this menu was made for `line` with the cursor at `point`.
    pub fn is_for(&self, line: &str, point: usize) -> bool {
        self.point == point && self.line == line
    }

    /// Moves the pick `count` rows down, or up when `down` is false (a
    /// negative count goes the other way), wrapping at either end. With no
    /// pick yet, the first step down picks the top item and the first step
    /// up the bottom one.
    pub fn step(&mut self, down: bool, count: i64) {
        let total = self.items.len() as i64;
        if total == 0 {
            return;
        }
        let delta = if down { count } else { -count };
        let from = match self.picked {
            Some(i) => i as i64,
            None if delta >= 0 => -1,
            None => total,
        };
        self.picked = Some((from + delta).rem_euclid(total) as usize);
    }

    pub fn picked_item(&self) -> Option<&Item> {
        self.items.get(self.picked?)
    }

    /// The item the grey text comes from: the picked one, else `top`.
    pub fn grey_item(&self) -> Option<&Item> {
        self.picked_item().or(self.top.as_ref())
    }
}

/// Which items a menu shows: `count` items from `first`, and a last row
/// saying `more` items are not shown (0 for no such row).
#[derive(Debug, PartialEq, Eq)]
pub struct Window {
    pub first: usize,
    pub count: usize,
    pub more: usize,
}

/// The window of a menu of `total` items with row `highlighted`
/// highlighted, in at most `rows` rows. With more items than rows, the last
/// row counts the ones not shown, and the items start at the top until the
/// highlighted row passes the last item row; it then stays on that row. A
/// single row shows the highlighted item (or the top item) and no count.
pub fn window(total: usize, highlighted: Option<usize>, rows: usize) -> Window {
    if rows == 0 || total == 0 {
        return Window {
            first: 0,
            count: 0,
            more: 0,
        };
    }
    if total <= rows {
        return Window {
            first: 0,
            count: total,
            more: 0,
        };
    }
    if rows == 1 {
        return Window {
            first: highlighted.unwrap_or(0),
            count: 1,
            more: 0,
        };
    }
    let count = rows - 1;
    let first = match highlighted {
        Some(p) if p >= count => p + 1 - count,
        _ => 0,
    };
    Window {
        first,
        count,
        more: total - count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: Matching = Matching {
        style: Style::Prefix,
        ignore_case: false,
    };
    const FUZZY: Matching = Matching {
        style: Style::Fuzzy,
        ignore_case: false,
    };

    fn history(line: &str, how: Matching, entries: &[&str]) -> Vec<String> {
        let mut gather = HistoryGather::new(line, how);
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
            note: None,
        }
    }

    #[test]
    fn history_is_newest_first_without_repeats_or_the_line() {
        // Entries come newest first, as `ffi::history_find_map` gives them.
        let found = history(
            "git st",
            PREFIX,
            &["git status", "ls", "git stash", "git status", "git st"],
        );
        assert_eq!(found, ["git status", "git stash"]);
    }

    #[test]
    fn history_skips_entries_that_cannot_be_drawn() {
        let found = history("echo", PREFIX, &["echo \x1b[31m", "echo\thi", "echo a\nb"]);
        assert_eq!(found, ["echo\thi", "echo a\nb"]);
    }

    #[test]
    fn history_stops_at_the_limit() {
        let entries: Vec<String> = (0..80).map(|i| format!("x{i}")).collect();
        let refs: Vec<&str> = entries.iter().map(String::as_str).collect();
        let mut gather = HistoryGather::new("x", PREFIX);
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
        let mut gather = HistoryGather::new("git st", PREFIX);
        gather.offer("git status");
        assert_eq!(
            gather.into_items(),
            [Item {
                text: "git status".into(),
                start: 0,
                end: 6,
                source: Source::History,
                note: None,
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
        let found = history("ls", FUZZY, &refs);
        assert_eq!(found.len(), HISTORY_LIMIT + 1);
        assert_eq!(found.last().map(String::as_str), Some("ls -la"));
    }

    #[test]
    fn fuzzy_history_takes_letters_in_order() {
        let found = history("gst", FUZZY, &["git status", "gist", "ls"]);
        assert_eq!(found, ["git status", "gist"]);
    }

    #[test]
    fn prefix_matching_is_case_sensitive() {
        let items = assemble(
            "x Sw",
            4,
            PREFIX,
            vec![],
            None,
            vec![],
            vec![word("switch", 2, 4), word("Swap", 2, 4)],
        );
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["Swap"]);
    }

    const IGNORE_CASE: Matching = Matching {
        style: Style::Prefix,
        ignore_case: true,
    };

    #[test]
    fn ignoring_case_ranks_items_of_the_typed_case_first() {
        let items = assemble(
            "x sw",
            4,
            IGNORE_CASE,
            vec![],
            None,
            vec![],
            vec![word("Swap", 2, 4), word("switch", 2, 4), word("SHOW", 2, 4)],
        );
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["switch", "Swap"]);
    }

    #[test]
    fn fuzzy_can_ignore_case() {
        let fuzzy = Matching {
            style: Style::Fuzzy,
            ignore_case: true,
        };
        let items = assemble(
            "gst",
            3,
            fuzzy,
            vec![],
            None,
            vec![],
            vec![word("Git STatus", 0, 3), word("ls", 0, 3)],
        );
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["Git STatus"]);
    }

    #[test]
    fn history_can_ignore_case() {
        let found = history("GIT st", IGNORE_CASE, &["git status", "ls", "GIT stash"]);
        assert_eq!(found, ["git status", "GIT stash"]);
        let fuzzy = Matching {
            style: Style::Fuzzy,
            ignore_case: true,
        };
        assert_eq!(history("gst", fuzzy, &["Git STatus", "ls"]), ["Git STatus"]);
    }

    /// Each kind of match counts toward its own limit, so entries in
    /// another case never crowd out an older entry in the same case, and
    /// entries with gaps never crowd out one in another case.
    #[test]
    fn each_kind_of_history_match_has_its_own_limit() {
        let mut entries: Vec<String> = (0..80).map(|i| format!("GIT s{i}")).collect();
        entries.push("git status".to_owned());
        let refs: Vec<&str> = entries.iter().map(String::as_str).collect();
        let found = history("git s", IGNORE_CASE, &refs);
        assert_eq!(found.len(), HISTORY_LIMIT + 1);
        assert_eq!(found.last().map(String::as_str), Some("git status"));
        let fuzzy = Matching {
            style: Style::Fuzzy,
            ignore_case: true,
        };
        let mut entries: Vec<String> = (0..80).map(|i| format!("g x s t {i}")).collect();
        entries.push("GST older".to_owned());
        let refs: Vec<&str> = entries.iter().map(String::as_str).collect();
        let found = history("gst", fuzzy, &refs);
        assert_eq!(found.len(), HISTORY_LIMIT + 1);
        assert_eq!(found.last().map(String::as_str), Some("GST older"));
    }

    /// Adding the rest of an item of another case would leave the line with the
    /// typed case followed by the item's, so no grey text shows.
    #[test]
    fn no_grey_text_for_an_item_of_another_case() {
        assert_eq!(grey("x Sw", 4, &word("switch", 2, 4)), None);
        assert_eq!(grey("x Sw", 4, &word("Swap", 2, 4)), Some("ap"));
    }

    #[test]
    fn an_empty_typed_text_matches_every_item() {
        let items = assemble(
            "git ",
            4,
            PREFIX,
            vec![],
            None,
            vec![],
            vec![word("status", 4, 4), word("stash", 4, 4)],
        );
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn fuzzy_puts_prefixes_first_then_the_tightest() {
        let items = assemble(
            "sw",
            2,
            FUZZY,
            vec![],
            None,
            vec![],
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
        let t = |typed: &str, text: &str| tightest(PREFIX, &chars(typed), &chars(text));
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
            note: None,
        }];
        let whole = Some(Item {
            text: "git status".into(),
            start: 0,
            end: 6,
            source: Source::Lisp,
            note: None,
        });
        let words = vec![word("status", 4, 6), word("stash", 4, 6)];
        let items = assemble("git st", 6, PREFIX, hist, whole, vec![], words);
        let texts: Vec<(&str, char)> = items
            .iter()
            .map(|i| (i.text.as_str(), i.source.letter()))
            .collect();
        // "status" gives the same line as the history item.
        assert_eq!(texts, [("git status", 'h'), ("stash", 'l')]);
    }

    #[test]
    fn mode_items_come_between_the_whole_line_and_lisp_words() {
        let hist = vec![Item {
            source: Source::History,
            ..word("git status", 0, 6)
        }];
        let whole = Some(word("git stage", 0, 6));
        let mode = vec![Item {
            note: Some("column".into()),
            source: Source::Mode,
            ..word("stash", 4, 6)
        }];
        let words = vec![word("stack", 4, 6), word("stash", 4, 6)];
        let items = assemble("git st", 6, PREFIX, hist, whole, mode, words);
        let got: Vec<(&str, char)> = items
            .iter()
            .map(|i| (i.text.as_str(), i.source.letter()))
            .collect();
        assert_eq!(
            got,
            [
                ("git status", 'h'),
                ("git stage", 'l'),
                ("stash", 'm'),
                ("stack", 'l')
            ]
        );
    }

    #[test]
    fn a_note_takes_no_part_in_matching_or_the_repeat_check() {
        let noted = |text: &str, note: &str| Item {
            note: Some(note.into()),
            source: Source::Mode,
            ..word(text, 4, 6)
        };
        // "st" is in the note but not the text; the second "stash" repeats
        // the first, whatever its note says.
        let mode = vec![
            noted("stash", "a"),
            noted("add", "first"),
            noted("stash", "b"),
        ];
        let items = assemble("git st", 6, PREFIX, vec![], None, mode, vec![]);
        let got: Vec<(&str, Option<&str>)> = items
            .iter()
            .map(|i| (i.text.as_str(), i.note.as_deref()))
            .collect();
        assert_eq!(got, [("stash", Some("a"))]);
    }

    fn mode(text: &str, start: usize, end: usize) -> Item {
        Item {
            source: Source::Mode,
            ..word(text, start, end)
        }
    }

    #[test]
    fn a_quoted_mode_item_matches_after_its_quote_mark() {
        let texts = |line: &str, style, mode: Vec<Item>, words: Vec<Item>| -> Vec<String> {
            let point = line.len();
            let items = assemble(line, point, style, vec![], None, mode, words);
            items.into_iter().map(|i| i.text).collect()
        };
        // `fi` finds each quoted name, and ranks it with the plain ones that
        // start with `fi`, in list order.
        let mode_items = vec![
            mode("`first name`", 5, 7),
            mode("file", 5, 7),
            mode("'fig'", 5, 7),
            mode("\"fin\"", 5, 7),
            mode("`other`", 5, 7),
        ];
        assert_eq!(
            texts("sort fi", PREFIX, mode_items.clone(), vec![]),
            ["`first name`", "file", "'fig'", "\"fin\""]
        );
        // The typed quote mark still matches as it is.
        assert_eq!(
            texts("sort `fi", PREFIX, vec![mode("`first name`", 5, 8)], vec![]),
            ["`first name`"]
        );
        // In the fuzzy style the text after the mark counts too: a prefix
        // there comes before a gapped match.
        assert_eq!(
            texts(
                "sort fi",
                FUZZY,
                vec![mode("xfxi", 5, 7), mode("`first name`", 5, 7)],
                vec![]
            ),
            ["`first name`", "xfxi"]
        );
        assert_eq!(
            texts("sort fn", FUZZY, vec![mode("`first name`", 5, 7)], vec![]),
            ["`first name`"]
        );
        // Only a mode server's items: a quoted Lisp word must start with the
        // typed text.
        assert!(texts("sort fi", PREFIX, vec![], vec![word("`first`", 5, 7)]).is_empty());
    }

    #[test]
    fn a_quoted_match_has_no_grey_text() {
        assert_eq!(grey("sort fi", 7, &mode("`first name`", 5, 7)), None);
        assert_eq!(
            grey("sort `fi", 8, &mode("`first name`", 5, 8)),
            Some("rst name`")
        );
    }

    #[test]
    fn items_that_change_nothing_or_cannot_be_drawn_go() {
        let items = assemble(
            "ab",
            2,
            PREFIX,
            vec![],
            None,
            vec![],
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

    fn menu_of(n: usize) -> Menu {
        let items = (0..n).map(|i| word(&format!("w{i}"), 0, 0)).collect();
        Menu::new("", 0, items)
    }

    #[test]
    fn stepping_down_starts_at_the_top_and_wraps() {
        let mut m = menu_of(3);
        assert_eq!(m.picked, None);
        m.step(true, 1);
        assert_eq!(m.picked, Some(0));
        m.step(true, 2);
        assert_eq!(m.picked, Some(2));
        m.step(true, 1);
        assert_eq!(m.picked, Some(0));
    }

    #[test]
    fn stepping_up_starts_at_the_bottom_and_wraps() {
        let mut m = menu_of(3);
        m.step(false, 1);
        assert_eq!(m.picked, Some(2));
        m.step(false, 3);
        assert_eq!(m.picked, Some(2));
        m.step(false, 2);
        assert_eq!(m.picked, Some(0));
        // A negative count goes the other way.
        m.step(false, -1);
        assert_eq!(m.picked, Some(1));
    }

    #[test]
    fn stepping_an_empty_menu_picks_nothing() {
        let mut m = menu_of(0);
        m.step(true, 1);
        assert_eq!(m.picked, None);
    }

    #[test]
    fn the_grey_item_is_the_pick_or_the_top() {
        let mut m = menu_of(2);
        assert_eq!(m.grey_item().map(|i| i.text.as_str()), Some("w0"));
        assert!(m.picked_item().is_none());
        m.step(false, 1);
        assert_eq!(m.grey_item().map(|i| i.text.as_str()), Some("w1"));
        assert_eq!(m.picked_item().map(|i| i.text.as_str()), Some("w1"));
    }

    /// The menu may list fewer items than it was made from; with no pick, the
    /// grey text still comes from the top item of them all.
    #[test]
    fn the_grey_item_comes_from_every_item() {
        let mut m = menu_of(3);
        m.items.retain(|i| i.text != "w0");
        assert_eq!(m.grey_item().map(|i| i.text.as_str()), Some("w0"));
        m.step(true, 1);
        assert_eq!(m.grey_item().map(|i| i.text.as_str()), Some("w1"));
    }

    #[test]
    fn sources_have_names() {
        for source in [Source::History, Source::Lisp, Source::Mode] {
            assert_eq!(Source::named(source.name()), Some(source));
        }
        assert_eq!(Source::named("bash"), None);
    }

    #[test]
    fn the_menu_lists_items_from_its_sources_with_enough_typed() {
        let listed = Listed {
            sources: vec![Source::Lisp],
            min_chars: 2,
        };
        let history = Item {
            source: Source::History,
            ..word("git status", 0, 6)
        };
        assert!(!listed.lists("git st", 6, &history));
        assert!(listed.lists("git st", 6, &word("stash", 4, 6)));
        assert!(!listed.lists("git s", 5, &word("stash", 4, 5)));
        // Characters, not bytes.
        assert!(!listed.lists("git é", 6, &word("été", 4, 6)));
    }

    #[test]
    fn a_menu_is_for_one_line_and_cursor() {
        let m = Menu::new("ab", 2, vec![]);
        assert!(m.is_for("ab", 2));
        assert!(!m.is_for("ab", 1));
        assert!(!m.is_for("abc", 2));
    }

    #[test]
    fn the_window_shows_what_fits() {
        let w = |total, picked, rows| {
            let w = window(total, picked, rows);
            (w.first, w.count, w.more)
        };
        // Everything fits.
        assert_eq!(w(3, None, 8), (0, 3, 0));
        // Too many: the last row counts the rest.
        assert_eq!(w(10, None, 4), (0, 3, 7));
        // The pick stays on the last item row once it passes it.
        assert_eq!(w(10, Some(2), 4), (0, 3, 7));
        assert_eq!(w(10, Some(5), 4), (3, 3, 7));
        assert_eq!(w(10, Some(9), 4), (7, 3, 7));
        // One row: the pick or the top item, no count row.
        assert_eq!(w(10, None, 1), (0, 1, 0));
        assert_eq!(w(10, Some(6), 1), (6, 1, 0));
        // No room, or nothing to show.
        assert_eq!(w(10, None, 0), (0, 0, 0));
        assert_eq!(w(0, None, 5), (0, 0, 0));
    }
}
