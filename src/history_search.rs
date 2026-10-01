//! The prefix and substring searches of inkline's Up and Down keys: which
//! history entry the next match is. Reading history and moving to the match
//! are the keys' own (`hooks::multiline`).

use std::ffi::c_int;

use memchr::memmem::Finder;

/// How a search matches an entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// The entry starts with the text, as readline's
    /// `history-search-backward`.
    Prefix,
    /// The entry holds the text anywhere.
    Substring,
}

/// A prefix or substring search that an Up or Down key started.
pub struct HistorySearch {
    /// How the search matches an entry.
    pub kind: Kind,
    /// The text to find: the text before the cursor when the search
    /// started.
    pub text: Vec<u8>,
    /// The line the search started from.
    pub line: Vec<u8>,
    /// The cursor on that line.
    pub point: usize,
    /// The history place the search started from: past the newest entry,
    /// or the place of the entry on the line, one an earlier search found
    /// or, for the prefix search, one brought back by walking.
    pub start: c_int,
    /// The history place past the newest entry the search finds: `start`
    /// for the substring search, which finds only older entries, and past
    /// the newest history entry for the prefix search, which finds older
    /// and newer ones, as readline's prefix search does.
    pub end: c_int,
    /// Whether an earlier search found the line at `start`.
    pub start_found: bool,
    /// The history place of the current match, or `start` while the line
    /// is the one the search started from.
    pub at: c_int,
    /// The text of the last match the search found, as readline's
    /// `prev_line_found`: none until the search has found a match, and
    /// again once Down gives back the line it started from. The next key
    /// passes over matches that repeat it.
    pub last_found: Option<Vec<u8>>,
    /// Whether the run of Up and Down keys that ran this search goes on
    /// with it: a further key of the run, of the same kind, continues it.
    pub going_on: bool,
}

impl HistorySearch {
    /// The history place of the line this search found, if the line is
    /// one it found (`last_found`), the entry it started from included, or
    /// the entry an earlier search found that it started on.
    pub fn found_at(&self) -> Option<c_int> {
        (self.last_found.is_some() || self.start_found).then_some(self.at)
    }

    /// Whether the search started on a line past every entry it finds,
    /// a typed line or, for the substring search, the entry an earlier
    /// search found. Such a search passes over entries that repeat that
    /// line, and Down past its newest match gives the line back. A prefix
    /// search that started on an entry inside history goes through history
    /// in order, that entry and entries that repeat it included, as
    /// readline's does.
    pub fn started_past_its_entries(&self) -> bool {
        self.start == self.end
    }

    /// Whether Down that finds no match gives back the line the search
    /// started from: only for a search that started past its entries
    /// (`started_past_its_entries`) and has left that line. Otherwise the
    /// search finds nothing and the line stays.
    pub fn gives_back_start(&self) -> bool {
        self.started_past_its_entries() && self.at != self.start
    }

    /// The place of the match `count` matches older (`up`) or newer than
    /// the current one, older than `end`, reading entries with `entry`. An
    /// entry that matches is passed over when it repeats the last match
    /// found (`last_found`, none until the search has found a match), as
    /// readline passes over a match that repeats the last one, and, for a
    /// search that started past its entries, when it repeats that line. A
    /// key that finds fewer matches goes to the last it finds; a search
    /// that started inside history goes to a match it passed over after
    /// that instead, as readline's does. None when the key goes nowhere: it
    /// finds no match, or Down runs past the newest match of a search that
    /// gives back its start (`gives_back_start`).
    pub fn next_match<'h>(
        &self,
        up: bool,
        count: c_int,
        ignore_case: bool,
        entry: impl Fn(c_int) -> Option<&'h [u8]>,
    ) -> Option<c_int> {
        let wanted = SearchText::new(&self.text, ignore_case);
        let past = self.started_past_its_entries();
        let mut found = None;
        let mut seen = None;
        let mut last = self.last_found.as_deref();
        let mut place = self.at;
        let mut left = count;
        while left > 0 {
            place += if up { -1 } else { 1 };
            if place < 0 || place >= self.end {
                break;
            }
            let Some(text) = entry(place) else { continue };
            if (past && text == self.line) || !wanted.matches(text, self.kind) {
                continue;
            }
            seen = Some(place);
            if last == Some(text) {
                continue;
            }
            found = Some(place);
            last = Some(text);
            left -= 1;
        }
        if !past {
            seen
        } else if up || left == 0 || !self.gives_back_start() {
            found
        } else {
            None
        }
    }

    /// Where the text ends in `line`, a line that starts with it, as the
    /// prefix search matches it: after as many characters as the text has
    /// when both are UTF-8, since a character may have a lowercase form of
    /// another length when case is ignored, and else after as many bytes.
    pub fn prefix_end(&self, line: &[u8]) -> usize {
        if let (Ok(text), Ok(line)) = (std::str::from_utf8(&self.text), std::str::from_utf8(line)) {
            return line
                .char_indices()
                .nth(text.chars().count())
                .map_or(line.len(), |(i, _)| i);
        }
        self.text.len().min(line.len())
    }
}

/// The text a search looks for, with what finds it made once, not for
/// every entry.
enum SearchText<'a> {
    /// Found byte for byte.
    Bytes(Finder<'a>),
    /// Case ignored and the text UTF-8: its lowercase form, found in an
    /// entry's lowercase form, or the text itself, found ignoring the case
    /// of ASCII letters in an entry that is not UTF-8.
    Lowercase {
        finder: Finder<'static>,
        text: &'a [u8],
    },
    /// Case ignored and the text not UTF-8: found ignoring the case of
    /// ASCII letters.
    Ascii(&'a [u8]),
}

impl<'a> SearchText<'a> {
    fn new(text: &'a [u8], ignore_case: bool) -> Self {
        if !ignore_case {
            return SearchText::Bytes(Finder::new(text));
        }
        match std::str::from_utf8(text) {
            Ok(utf8) => SearchText::Lowercase {
                finder: Finder::new(lowercase(utf8).as_bytes()).into_owned(),
                text,
            },
            Err(_) => SearchText::Ascii(text),
        }
    }

    /// Whether `entry` starts with the text (`Kind::Prefix`) or holds it
    /// (`Kind::Substring`): byte for byte, or ignoring case as readline's
    /// `search-ignore-case` does for both. With case ignored, Unicode
    /// letters are folded one to one, like `towlower` in a UTF-8 locale,
    /// when both are UTF-8, and only ASCII letters are otherwise.
    fn matches(&self, entry: &[u8], kind: Kind) -> bool {
        match kind {
            Kind::Prefix => match self {
                SearchText::Bytes(finder) => entry.starts_with(finder.needle()),
                SearchText::Lowercase { finder, text } => match std::str::from_utf8(entry) {
                    // Only as much of the entry is folded as the text is long.
                    Ok(entry) => lowercase_bytes(entry)
                        .take(finder.needle().len())
                        .eq(finder.needle().iter().copied()),
                    Err(_) => matches_ignoring_ascii_case(entry, text, kind),
                },
                SearchText::Ascii(text) => matches_ignoring_ascii_case(entry, text, kind),
            },
            Kind::Substring => match self {
                SearchText::Bytes(finder) => finder.find(entry).is_some(),
                SearchText::Lowercase { finder, text } => match std::str::from_utf8(entry) {
                    Ok(entry) => finder.find(lowercase(entry).as_bytes()).is_some(),
                    Err(_) => matches_ignoring_ascii_case(entry, text, kind),
                },
                SearchText::Ascii(text) => matches_ignoring_ascii_case(entry, text, kind),
            },
        }
    }
}

/// Whether `entry` starts with or holds `text`, as `kind` says, ignoring
/// the case of ASCII letters.
fn matches_ignoring_ascii_case(entry: &[u8], text: &[u8], kind: Kind) -> bool {
    match kind {
        Kind::Prefix => entry
            .get(..text.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(text)),
        Kind::Substring => {
            text.is_empty()
                || entry
                    .windows(text.len())
                    .any(|w| w.eq_ignore_ascii_case(text))
        }
    }
}

/// `text` in lowercase, one letter to one letter as `towlower` folds case:
/// each letter becomes the first of its lowercase form (`İ` becomes `i`),
/// with no rule that looks at the letters around, such as Greek's final
/// sigma.
fn lowercase(text: &str) -> String {
    text.chars().map(lowercase_char).collect()
}

/// The bytes of `text` in lowercase, as `lowercase` makes it, one letter at
/// a time.
fn lowercase_bytes(text: &str) -> impl Iterator<Item = u8> + '_ {
    text.chars().flat_map(|c| {
        let mut bytes = [0; 4];
        let len = lowercase_char(c).encode_utf8(&mut bytes).len();
        bytes.into_iter().take(len)
    })
}

/// `c` in lowercase, as `lowercase` folds it.
fn lowercase_char(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A substring search for `text` started from `line` past the newest
    /// of `history`'s entries.
    fn search(text: &str, line: &str, history: &[&str]) -> HistorySearch {
        let start = history.len() as c_int;
        HistorySearch {
            kind: Kind::Substring,
            text: text.into(),
            line: line.into(),
            point: text.len(),
            start,
            end: start,
            start_found: false,
            at: start,
            last_found: None,
            going_on: true,
        }
    }

    /// `s` on the match at `place` in `history`, the last it found.
    fn on_match(s: HistorySearch, place: c_int, history: &[&str]) -> HistorySearch {
        HistorySearch {
            at: place,
            last_found: Some(history[place as usize].into()),
            ..s
        }
    }

    fn entries<'a>(history: &'a [&'a str]) -> impl Fn(c_int) -> Option<&'a [u8]> + 'a {
        |place| {
            usize::try_from(place)
                .ok()
                .and_then(|i| history.get(i))
                .map(|e| e.as_bytes())
        }
    }

    fn holds(entry: &[u8], text: &[u8], ignore_case: bool) -> bool {
        SearchText::new(text, ignore_case).matches(entry, Kind::Substring)
    }

    fn starts(entry: &[u8], text: &[u8], ignore_case: bool) -> bool {
        SearchText::new(text, ignore_case).matches(entry, Kind::Prefix)
    }

    #[test]
    fn holds_matches_bytes_or_ignores_case() {
        assert!(holds(b"git status", b"stat", false));
        assert!(!holds(b"git STATUS", b"stat", false));
        assert!(holds(b"git STATUS", b"stat", true));
        assert!(holds("echo ÄRGER".as_bytes(), "ärg".as_bytes(), true));
        assert!(holds(b"cat \xff STAT", b"stat", true));
        assert!(holds(b"cat \xff stat", b"\xff s", false));
        assert!(!holds(b"sta", b"stat", false));
    }

    #[test]
    fn starts_matches_only_at_the_start() {
        assert!(starts(b"git status", b"git s", false));
        assert!(!starts(b"echo git status", b"git", false));
        assert!(!starts(b"GIT status", b"git", false));
        assert!(starts(b"GIT status", b"git", true));
        assert!(starts("ÄRGER los".as_bytes(), "ärg".as_bytes(), true));
        assert!(starts(b"GIT \xff", b"git", true));
        assert!(starts(b"\xff GIT", b"\xff g", true));
        assert!(!starts(b"gi", b"git", true));
        assert!(!starts(b"gi", b"git", false));
    }

    #[test]
    fn a_prefix_search_passes_over_entries_that_hold_the_text_later() {
        let history = ["git log", "echo git", "git status", "git"];
        let s = HistorySearch {
            kind: Kind::Prefix,
            ..search("git", "git", &history)
        };
        let at = |count| s.next_match(true, count, false, entries(&history));
        // The typed line itself and `echo git` are passed over.
        assert_eq!(at(1), Some(2));
        assert_eq!(at(2), Some(0));
    }

    /// A prefix search from an entry within history finds older entries
    /// with Up and newer ones with Down, as readline's does.
    #[test]
    fn a_prefix_search_from_within_history_looks_both_ways() {
        let history = ["git a", "ls", "git b", "pwd", "git c"];
        let s = HistorySearch {
            kind: Kind::Prefix,
            start: 2,
            end: 5,
            at: 2,
            ..search("git", "git b", &history)
        };
        assert_eq!(s.next_match(true, 1, false, entries(&history)), Some(0));
        assert_eq!(s.next_match(false, 1, false, entries(&history)), Some(4));
    }

    /// A prefix search from an entry within history does not pass over
    /// that entry when it comes to it again, and Down past its newest match
    /// finds none rather than giving back the start.
    #[test]
    fn a_prefix_search_from_within_history_takes_its_start_in_order() {
        let history = ["git a", "ls", "git b", "pwd", "git c"];
        let s = HistorySearch {
            kind: Kind::Prefix,
            start: 2,
            end: 5,
            ..search("git", "git b", &history)
        };
        let s = on_match(s, 4, &history);
        assert!(!s.gives_back_start());
        assert_eq!(s.next_match(true, 1, false, entries(&history)), Some(2));
        assert_eq!(s.next_match(false, 1, false, entries(&history)), None);
    }

    /// A counted Down from an entry within history that finds fewer
    /// matches than the count goes to the newest it finds.
    #[test]
    fn a_counted_prefix_down_goes_to_the_newest_match_found() {
        let history = ["git x", "ls", "git y", "pwd"];
        let s = HistorySearch {
            kind: Kind::Prefix,
            start: 0,
            end: 4,
            at: 0,
            ..search("git", "git x", &history)
        };
        assert_eq!(s.next_match(false, 3, false, entries(&history)), Some(2));
    }

    /// A prefix search that starts on an entry inside history finds an
    /// older entry that repeats it: it passes over only matches that repeat
    /// the last match it found, and has found none yet.
    #[test]
    fn a_prefix_search_from_within_history_finds_a_repeat_of_its_start() {
        let history = ["git a", "git b", "ls", "git b", "pwd"];
        let s = HistorySearch {
            kind: Kind::Prefix,
            start: 3,
            end: 5,
            at: 3,
            ..search("git", "git b", &history)
        };
        assert_eq!(s.next_match(true, 1, false, entries(&history)), Some(1));
    }

    /// A prefix search that comes back to the entry it started on still
    /// passes over matches that repeat the last one it found, as readline's
    /// does: from that `git b`, Up past the older `git b` it found before.
    #[test]
    fn a_prefix_search_back_on_its_start_passes_over_a_repeat_of_its_match() {
        let history = ["git a", "git b", "ls", "git b", "pwd"];
        let s = HistorySearch {
            kind: Kind::Prefix,
            start: 3,
            end: 5,
            ..search("git", "git b", &history)
        };
        let s = on_match(s, 1, &history);
        // Down finds only a repeat of the last match, and goes to it.
        assert_eq!(s.next_match(false, 1, false, entries(&history)), Some(3));
        let s = on_match(s, 3, &history);
        assert_eq!(s.next_match(true, 1, false, entries(&history)), Some(0));
    }

    /// A prefix search from within history goes to a match that repeats
    /// the last one when it finds no other, as readline's does, so it keeps
    /// going through history in order.
    #[test]
    fn a_prefix_search_from_within_history_goes_to_a_repeat_it_passed_over() {
        let history = ["git b", "ls", "git b", "pwd"];
        let s = HistorySearch {
            kind: Kind::Prefix,
            start: 2,
            end: 4,
            ..search("git", "git b", &history)
        };
        let s = on_match(s, 0, &history);
        assert_eq!(s.next_match(false, 1, false, entries(&history)), Some(2));
        let s = on_match(s, 2, &history);
        assert_eq!(s.next_match(false, 1, false, entries(&history)), None);
    }

    /// The text ends after as many characters as it has, so the end is
    /// never inside a character whose lowercase form is shorter.
    #[test]
    fn the_prefix_ends_after_whole_characters() {
        let s = search("k", "k", &[]);
        assert_eq!(s.prefix_end("\u{212A}ubectl".as_bytes()), 3);
        assert_eq!(s.prefix_end(b"kubectl"), 1);
        assert_eq!(s.prefix_end(b"k\xff"), 1);
    }

    /// Case is folded one letter at a time, so a sigma at the end of the
    /// text still matches one inside a word.
    #[test]
    fn ignoring_case_folds_each_letter_on_its_own() {
        assert!(holds("ΟΔΟΣΑΣ".as_bytes(), "ΟΔΟΣ".as_bytes(), true));
        assert!(holds("οδοσας".as_bytes(), "ΟΔΟΣ".as_bytes(), true));
        assert!(starts("ΟΔΟΣΑΣ".as_bytes(), "ΟΔΟΣ".as_bytes(), true));
        assert!(starts("οδοσας".as_bytes(), "ΟΔΟΣ".as_bytes(), true));
    }

    /// Case is folded one letter to one letter, as `towlower` does: `İ`
    /// becomes `i`, not `i` with a combining dot above.
    #[test]
    fn ignoring_case_folds_a_letter_to_one_letter() {
        assert!(holds("istanbul".as_bytes(), "İstan".as_bytes(), true));
        assert!(holds("İSTANBUL".as_bytes(), "istan".as_bytes(), true));
        assert!(!holds("istanbul".as_bytes(), "İstan".as_bytes(), false));
        assert!(starts("istanbul".as_bytes(), "İstan".as_bytes(), true));
        assert!(starts("İSTANBUL".as_bytes(), "istan".as_bytes(), true));
        assert!(!starts("İs".as_bytes(), "istan".as_bytes(), true));
    }

    #[test]
    fn up_counts_matches_older_than_the_current_one() {
        let history = ["git stash", "git status", "git st"];
        let s = search("git st", "git st", &history);
        let at = |count| s.next_match(true, count, false, entries(&history));
        // The typed line itself is passed over.
        assert_eq!(at(1), Some(1));
        assert_eq!(at(2), Some(0));
        // Past the oldest match, the oldest one found.
        assert_eq!(at(5), Some(0));
    }

    #[test]
    fn a_match_that_repeats_the_last_is_passed_over() {
        let history = ["echo stat", "git status", "echo stat", "echo stat"];
        let mut s = search("stat", "stat", &history);
        let up = |s: &HistorySearch, count| s.next_match(true, count, false, entries(&history));
        assert_eq!(up(&s, 2), Some(1));
        s = on_match(s, 3, &history);
        assert_eq!(up(&s, 1), Some(1));
        // A match that is not next to the last is found again.
        s = on_match(s, 1, &history);
        assert_eq!(up(&s, 1), Some(0));
    }

    #[test]
    fn up_finds_nothing_past_the_oldest_match() {
        let history = ["git status", "ls"];
        let s = on_match(search("stat", "stat", &history), 0, &history);
        assert_eq!(s.next_match(true, 1, false, entries(&history)), None);
    }

    #[test]
    fn down_goes_back_through_the_matches_then_past_the_newest() {
        let history = ["git status", "ls", "echo stat", "pwd"];
        let mut s = on_match(search("stat", "stat", &history), 0, &history);
        let down = |s: &HistorySearch, count| s.next_match(false, count, false, entries(&history));
        assert_eq!(down(&s, 1), Some(2));
        // Down that runs past the newest match finds none.
        assert_eq!(down(&s, 2), None);
        s = on_match(s, 2, &history);
        assert_eq!(down(&s, 1), None);
    }

    #[test]
    fn a_search_from_a_found_entry_looks_only_older() {
        let history = ["echo statX older", "git status", "echo stat"];
        let s = HistorySearch {
            start: 2,
            start_found: true,
            at: 2,
            ..search("echo statX", "echo statX", &history)
        };
        assert_eq!(s.next_match(true, 1, false, entries(&history)), Some(0));
        assert_eq!(s.found_at(), Some(2));
        assert_eq!(s.next_match(false, 1, false, entries(&history)), None);
    }

    #[test]
    fn a_start_entry_found_again_counts_as_found() {
        let history = ["git a", "git b", "ls", "git b", "pwd"];
        let s = HistorySearch {
            kind: Kind::Prefix,
            start: 3,
            end: 5,
            at: 3,
            ..search("git", "git b", &history)
        };
        assert_eq!(s.found_at(), None);
        assert_eq!(on_match(s, 3, &history).found_at(), Some(3));
    }
}
