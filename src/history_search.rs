//! The substring search of inkline's Up and Down keys: which history
//! entry the next match is. Reading history and moving to the match are the
//! keys' own (`hooks::multiline`).

use std::ffi::c_int;

use memchr::memmem::Finder;

/// A substring search that Up started.
pub struct SubstringSearch {
    /// The text to find: the text before the cursor when the search
    /// started.
    pub text: Vec<u8>,
    /// The line the search started from.
    pub line: Vec<u8>,
    /// The cursor on that line.
    pub point: usize,
    /// The history place the search started from: past the newest entry,
    /// or the place of the entry an earlier search found. Matches are older.
    pub start: c_int,
    /// Whether an earlier search found the line at `start`.
    pub start_found: bool,
    /// The history place of the current match, or `start` while the line
    /// is the one the search started from.
    pub at: c_int,
    /// Whether the run of Up and Down keys that ran this search goes on
    /// with it: a further substring key of the run continues it.
    pub going_on: bool,
}

impl SubstringSearch {
    /// The history place of the line this search found, if the line is
    /// one it found.
    pub fn found_at(&self) -> Option<c_int> {
        (self.at != self.start || self.start_found).then_some(self.at)
    }

    /// The place of the match `count` matches older (`up`) or newer than
    /// the current one, reading entries with `entry`. An entry that holds
    /// the text is passed over when it is the line the search started
    /// from, or the line or match just before it (the current one at
    /// first), as readline passes over a match that repeats the last one.
    /// Up that finds fewer matches goes to the oldest it finds; None when it
    /// finds none, or when Down runs past the newest match.
    pub fn next_match<'h>(
        &self,
        up: bool,
        count: c_int,
        ignore_case: bool,
        entry: impl Fn(c_int) -> Option<&'h [u8]>,
    ) -> Option<c_int> {
        let wanted = SearchText::new(&self.text, ignore_case);
        let mut found = None;
        let mut last: &[u8] = if self.at == self.start {
            &self.line
        } else {
            entry(self.at).unwrap_or_default()
        };
        let mut place = self.at;
        let mut left = count;
        while left > 0 {
            place += if up { -1 } else { 1 };
            if place < 0 || place >= self.start {
                break;
            }
            let Some(text) = entry(place) else { continue };
            if text == self.line || text == last || !wanted.held_by(text) {
                continue;
            }
            found = Some(place);
            last = text;
            left -= 1;
        }
        if up || left == 0 { found } else { None }
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

    /// Whether `entry` holds the text: byte for byte, or ignoring case as
    /// readline's `search-ignore-case` does. With case ignored, Unicode
    /// letters are folded one to one, like `towlower` in a UTF-8 locale,
    /// when both are UTF-8, and only ASCII letters are otherwise.
    fn held_by(&self, entry: &[u8]) -> bool {
        match self {
            SearchText::Bytes(finder) => finder.find(entry).is_some(),
            SearchText::Lowercase { finder, text } => match std::str::from_utf8(entry) {
                Ok(entry) => finder.find(lowercase(entry).as_bytes()).is_some(),
                Err(_) => holds_ignoring_ascii_case(entry, text),
            },
            SearchText::Ascii(text) => holds_ignoring_ascii_case(entry, text),
        }
    }
}

/// Whether `entry` holds `text`, ignoring the case of ASCII letters.
fn holds_ignoring_ascii_case(entry: &[u8], text: &[u8]) -> bool {
    text.is_empty()
        || entry
            .windows(text.len())
            .any(|w| w.eq_ignore_ascii_case(text))
}

/// `text` in lowercase, one letter to one letter as `towlower` folds case:
/// each letter becomes the first of its lowercase form (`İ` becomes `i`),
/// with no rule that looks at the letters around, such as Greek's final
/// sigma.
fn lowercase(text: &str) -> String {
    text.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A search for `text` started from `line` past the newest of
    /// `history`'s entries.
    fn search(text: &str, line: &str, history: &[&str]) -> SubstringSearch {
        let start = history.len() as c_int;
        SubstringSearch {
            text: text.into(),
            line: line.into(),
            point: text.len(),
            start,
            start_found: false,
            at: start,
            going_on: true,
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
        SearchText::new(text, ignore_case).held_by(entry)
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

    /// Case is folded one letter at a time, so a sigma at the end of the
    /// text still matches one inside a word.
    #[test]
    fn ignoring_case_folds_each_letter_on_its_own() {
        assert!(holds("ΟΔΟΣΑΣ".as_bytes(), "ΟΔΟΣ".as_bytes(), true));
        assert!(holds("οδοσας".as_bytes(), "ΟΔΟΣ".as_bytes(), true));
    }

    /// Case is folded one letter to one letter, as `towlower` does: `İ`
    /// becomes `i`, not `i` with a combining dot above.
    #[test]
    fn ignoring_case_folds_a_letter_to_one_letter() {
        assert!(holds("istanbul".as_bytes(), "İstan".as_bytes(), true));
        assert!(holds("İSTANBUL".as_bytes(), "istan".as_bytes(), true));
        assert!(!holds("istanbul".as_bytes(), "İstan".as_bytes(), false));
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
        let up = |s: &SubstringSearch, count| s.next_match(true, count, false, entries(&history));
        assert_eq!(up(&s, 2), Some(1));
        s.at = 3;
        assert_eq!(up(&s, 1), Some(1));
        // A match that is not next to the last is found again.
        s.at = 1;
        assert_eq!(up(&s, 1), Some(0));
    }

    #[test]
    fn up_finds_nothing_past_the_oldest_match() {
        let history = ["git status", "ls"];
        let mut s = search("stat", "stat", &history);
        s.at = 0;
        assert_eq!(s.next_match(true, 1, false, entries(&history)), None);
    }

    #[test]
    fn down_goes_back_through_the_matches_then_past_the_newest() {
        let history = ["git status", "ls", "echo stat", "pwd"];
        let mut s = search("stat", "stat", &history);
        s.at = 0;
        let down =
            |s: &SubstringSearch, count| s.next_match(false, count, false, entries(&history));
        assert_eq!(down(&s, 1), Some(2));
        // Down that runs past the newest match finds none.
        assert_eq!(down(&s, 2), None);
        s.at = 2;
        assert_eq!(down(&s, 1), None);
    }

    #[test]
    fn a_search_from_a_found_entry_looks_only_older() {
        let history = ["echo statX older", "git status", "echo stat"];
        let s = SubstringSearch {
            start: 2,
            start_found: true,
            at: 2,
            ..search("echo statX", "echo statX", &history)
        };
        assert_eq!(s.next_match(true, 1, false, entries(&history)), Some(0));
        assert_eq!(s.found_at(), Some(2));
        assert_eq!(s.next_match(false, 1, false, entries(&history)), None);
    }
}
