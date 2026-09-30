//! bash's own completion as a source of menu items: the word bash would
//! complete, when a copy of the shell must be asked for its matches, and
//! the matches as items. `session` keeps this shell's answers and the copy
//! working on the next one, which `request` starts and `copy` runs.

pub mod answer;

// These run bash's completion in a fork of bash, so they are left out of
// the unit-test binary, which runs without bash.
#[cfg(not(test))]
mod copy;
#[cfg(not(test))]
mod request;
#[cfg(not(test))]
pub mod session;

use crate::menu::{self, Item, Matching, Source};
use answer::Answer;

/// The word bash would complete: `line` from `start` to the cursor at
/// `point`, both on character boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    line: String,
    start: usize,
    point: usize,
}

impl Word {
    pub fn new(line: &str, start: usize, point: usize) -> Option<Word> {
        (start <= point
            && point <= line.len()
            && line.is_char_boundary(start)
            && line.is_char_boundary(point))
        .then(|| Word {
            line: line.to_owned(),
            start,
            point,
        })
    }

    /// The word as typed, from its start to the cursor.
    pub fn typed(&self) -> &str {
        &self.line[self.start..self.point]
    }

    /// Whether `other` is at the same place of the same line: only the text
    /// typed into the word may differ.
    pub fn same_place(&self, other: &Word) -> bool {
        self.line[..self.start] == other.line[..other.start]
            && self.line[self.point..] == other.line[other.point..]
    }
}

/// Whether a word starting at byte `start` of `line` is where bash completes
/// a command name, as bash's `attempt_shell_completion` decides: going back
/// over blanks, and over one quote the word opened and blanks before it,
/// the line starts, or there is `;`, `|`, `&`, `{`, `(`, a backquote or a
/// newline, which inkline shows bash as `;`, but not the `>|`, `>&` or `<&`
/// of a redirection.
pub fn command_position(line: &str, start: usize) -> bool {
    let blanks = [' ', '\t'];
    let mut before = line[..start].trim_end_matches(blanks);
    if let Some(rest) = before.strip_suffix(['"', '\'']) {
        before = rest.trim_end_matches(blanks);
    }
    let mut back = before.chars().rev();
    match back.next() {
        None => true,
        Some('&') => !matches!(back.next(), Some('>' | '<')),
        Some('|') => back.next() != Some('>'),
        Some(c) => matches!(c, ';' | '{' | '(' | '`' | '\n'),
    }
}

/// The part of `typed` up to and including its last `/`.
fn directory(typed: &str) -> &str {
    typed.rfind('/').map_or("", |i| &typed[..=i])
}

/// Whether bash's answer for `asked` serves `word`: the same place on the
/// same line, and a typed text that goes on from the one asked for without
/// leaving its directory. A `-` typed into a word asked for empty does not
/// count: a rule may give flags only for a word that starts with `-`.
pub fn fits(asked: &Word, word: &Word) -> bool {
    let (a, w) = (asked.typed(), word.typed());
    asked.same_place(word)
        && w.starts_with(a)
        && directory(a) == directory(w)
        && !(a.is_empty() && w.starts_with('-'))
}

/// An answer, and the word it was asked for.
pub struct Saved {
    pub word: Word,
    pub answer: Answer,
}

/// What bash's completion does for the word at the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The word gets no bash items, and no copy is asked.
    Nothing,
    /// The saved answer serves the word; no copy is asked.
    Saved,
    /// A copy is asked now.
    Now,
    /// Once typing pauses on this line and cursor.
    AtPause,
    /// A copy is already working on an answer that will serve.
    InFlight,
}

pub struct Inputs<'a> {
    pub word: &'a Word,
    /// `inkline-command-min-chars`.
    pub min_chars: usize,
    pub saved: Option<&'a Saved>,
    /// The word the running copy was asked about.
    pub running: Option<&'a Word>,
    /// The word whose copy timed out or failed.
    pub given_up: Option<&'a Word>,
    /// Whether typing has paused on this line and cursor.
    pub paused: bool,
    pub how: Matching,
}

/// What to do for the word at the cursor:
///
/// - a command name shorter than `min_chars` characters, and a word whose
///   copy timed out or failed, get no items and no copy;
/// - a saved answer that fits the word serves it, until nothing in it
///   matches the typed text, or it was cut and the typed text is longer
///   than the one it was asked for: a copy is then asked once typing
///   pauses;
/// - otherwise a copy is asked at once, unless one already asked will
///   serve.
pub fn decide(i: &Inputs) -> Ask {
    let word = i.word;
    let too_short =
        command_position(&word.line, word.start) && word.typed().chars().count() < i.min_chars;
    if too_short || i.given_up.is_some_and(|w| w.same_place(word)) {
        return Ask::Nothing;
    }
    let in_flight = i.running.is_some_and(|r| fits(r, word));
    match i.saved.filter(|s| fits(&s.word, word)) {
        None if in_flight => Ask::InFlight,
        None => Ask::Now,
        Some(s)
            if s.word.typed() == word.typed()
                || (!s.answer.cut && any_match(&s.answer, word, i.how)) =>
        {
            Ask::Saved
        }
        Some(_) if in_flight => Ask::InFlight,
        Some(_) if i.paused => Ask::Now,
        Some(_) => Ask::AtPause,
    }
}

/// Whether any of `answer`'s matches that fits the line matches the text
/// typed from where it starts to the cursor.
fn any_match(answer: &Answer, word: &Word, how: Matching) -> bool {
    answer.matches.iter().any(|m| {
        end_on(m, word).is_some() && menu::matches(how, &word.line[m.start..word.point], &m.text)
    })
}

/// Where `m` ends on `word`'s line: `extra` bytes after the cursor. None
/// when its range is not on the line, or not on character boundaries.
fn end_on(m: &answer::Match, word: &Word) -> Option<usize> {
    let end = word.point.checked_add(m.extra)?;
    (m.start <= word.point
        && end <= word.line.len()
        && word.line.is_char_boundary(m.start)
        && word.line.is_char_boundary(end))
    .then_some(end)
}

/// `answer`'s matches as menu items for `word`'s line and cursor: each
/// replaces the line from its start to `extra` bytes after the cursor. A
/// match whose range is not on the line, or not on character boundaries, is
/// left out.
pub fn items(answer: &Answer, word: &Word) -> Vec<Item> {
    answer
        .matches
        .iter()
        .filter_map(|m| {
            Some(Item {
                text: m.text.clone(),
                start: m.start,
                end: end_on(m, word)?,
                source: Source::Bash,
                note: None,
            })
        })
        .collect()
}

/// `inkline status`'s line: `on` or `off`, and how many requests timed out
/// and failed, when any did.
pub fn status_line(on: bool, timed_out: u64, failed: u64) -> String {
    let state = if on { "on" } else { "off" };
    let mut counts = Vec::new();
    if timed_out > 0 {
        counts.push(format!("{timed_out} timed out"));
    }
    if failed > 0 {
        counts.push(format!("{failed} failed"));
    }
    if counts.is_empty() {
        format!("bash completion: {state}")
    } else {
        format!("bash completion: {state} ({})", counts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::answer::Match;
    use super::*;
    use crate::menu::Style;

    const PREFIX: Matching = Matching {
        style: Style::Prefix,
        ignore_case: false,
    };

    /// The word of `line` that starts at `start`, with the cursor at the end.
    fn w(line: &str, start: usize) -> Word {
        Word::new(line, start, line.len()).unwrap()
    }

    fn saved(line: &str, start: usize, texts: &[&str], cut: bool) -> Saved {
        Saved {
            word: w(line, start),
            answer: Answer {
                matches: texts
                    .iter()
                    .map(|t| Match {
                        start,
                        extra: 0,
                        text: (*t).to_owned(),
                    })
                    .collect(),
                cut,
            },
        }
    }

    /// `decide` for `word` as an argument (not a command name), with
    /// nothing running or given up, not paused.
    fn ask(word: &Word, saved: Option<&Saved>) -> Ask {
        decide(&Inputs {
            word,
            min_chars: 1,
            saved,
            running: None,
            given_up: None,
            paused: false,
            how: PREFIX,
        })
    }

    #[test]
    fn a_word_is_asked_about_at_once() {
        assert_eq!(ask(&w("gg ", 3), None), Ask::Now);
    }

    #[test]
    fn the_saved_answer_serves_as_the_word_grows() {
        let s = saved("gg ", 3, &["switch ", "show "], false);
        assert_eq!(ask(&w("gg ", 3), Some(&s)), Ask::Saved);
        assert_eq!(ask(&w("gg sw", 3), Some(&s)), Ask::Saved);
    }

    #[test]
    fn nothing_matching_asks_again_once_typing_pauses() {
        let s = saved("gg ", 3, &["switch ", "show "], false);
        let word = w("gg x", 3);
        assert_eq!(ask(&word, Some(&s)), Ask::AtPause);
        let paused = decide(&Inputs {
            word: &word,
            min_chars: 1,
            saved: Some(&s),
            running: None,
            given_up: None,
            paused: true,
            how: PREFIX,
        });
        assert_eq!(paused, Ask::Now);
    }

    #[test]
    fn another_place_on_the_line_asks_at_once() {
        let s = saved("gg ", 3, &["switch "], false);
        // The line before the word changed.
        assert_eq!(ask(&w("hh s", 3), Some(&s)), Ask::Now);
        // The line after the cursor changed.
        let after = Word::new("gg s x", 3, 4).unwrap();
        assert_eq!(ask(&after, Some(&s)), Ask::Now);
    }

    #[test]
    fn a_new_directory_or_a_shorter_word_asks_at_once() {
        let s = saved("cat ", 4, &["src/", "alpha.txt "], false);
        assert_eq!(ask(&w("cat src/", 4), Some(&s)), Ask::Now);
        let s = saved("cat src/", 4, &["src/main.rs "], false);
        assert_eq!(ask(&w("cat src/m", 4), Some(&s)), Ask::Saved);
        let s = saved("cat sr", 4, &["src/"], false);
        assert_eq!(ask(&w("cat s", 4), Some(&s)), Ask::Now);
    }

    /// A rule may give flags only for a word that starts with `-`.
    #[test]
    fn a_first_dash_asks_at_once() {
        let s = saved("fl ", 3, &["build ", "test "], false);
        assert_eq!(ask(&w("fl -", 3), Some(&s)), Ask::Now);
        let s = saved("fl -", 3, &["--verbose ", "--version "], false);
        assert_eq!(ask(&w("fl --verb", 3), Some(&s)), Ask::Saved);
    }

    #[test]
    fn a_cut_answer_asks_again_at_a_pause_for_a_longer_word() {
        let s = saved("cat f", 4, &["f0000 ", "f0001 "], true);
        assert_eq!(ask(&w("cat f", 4), Some(&s)), Ask::Saved);
        assert_eq!(ask(&w("cat f0", 4), Some(&s)), Ask::AtPause);
    }

    #[test]
    fn a_copy_already_asked_is_waited_for() {
        let word = w("gg s", 3);
        let running = w("gg ", 3);
        let stale = saved("gg ", 3, &["x "], false);
        for saved in [None, Some(&stale)] {
            let d = decide(&Inputs {
                word: &word,
                min_chars: 1,
                saved,
                running: Some(&running),
                given_up: None,
                paused: false,
                how: PREFIX,
            });
            assert_eq!(d, Ask::InFlight);
        }
    }

    #[test]
    fn a_short_command_name_or_a_word_given_up_on_lists_nothing() {
        let d = |word: &Word, min_chars, given_up: Option<&Word>| {
            decide(&Inputs {
                word,
                min_chars,
                saved: None,
                running: None,
                given_up,
                paused: false,
                how: PREFIX,
            })
        };
        let none = Ask::Nothing;
        assert_eq!(d(&w("", 0), 1, None), none);
        assert_eq!(d(&w("ls | ", 5), 1, None), none);
        assert_eq!(d(&w("ls | g", 5), 1, None), Ask::Now);
        assert_eq!(d(&w("ls | ", 5), 0, None), Ask::Now);
        // Characters, not bytes.
        assert_eq!(d(&w("é", 0), 2, None), none);
        let slow = w("slow ", 5);
        assert_eq!(d(&w("slow abc", 5), 0, Some(&slow)), none);
        // Another word on the line is asked about again.
        assert_eq!(d(&w("slow abc x", 9), 0, Some(&slow)), Ask::Now);
    }

    #[test]
    fn command_positions_are_those_bash_completes_commands_at() {
        for (line, start) in [
            ("", 0),
            ("gi", 0),
            ("  gi", 2),
            ("ls | g", 5),
            ("a; b", 3),
            ("a && b", 5),
            ("a || b", 5),
            ("(b", 1),
            ("{ b", 2),
            ("`b", 1),
            ("a\nb", 2),
            ("a; \"b", 4),
        ] {
            assert!(command_position(line, start), "{line:?}");
        }
        for (line, start) in [
            ("ls a", 3),
            ("ls >| f", 6),
            ("ls >& f", 6),
            ("ls <& f", 6),
            ("x=\"a", 3),
        ] {
            assert!(!command_position(line, start), "{line:?}");
        }
    }

    #[test]
    fn an_answer_gives_items_for_the_line_as_it_is_now() {
        let answer = Answer {
            matches: vec![
                Match {
                    start: 4,
                    extra: 1,
                    text: "my file\" ".to_owned(),
                },
                Match {
                    start: 4,
                    extra: 5,
                    text: "out of range".to_owned(),
                },
            ],
            cut: false,
        };
        let word = Word::new("ls \"my f\"", 4, 8).unwrap();
        let items = items(&answer, &word);
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].start, items[0].end), (4, 9));
        assert_eq!(items[0].source, Source::Bash);
    }

    #[test]
    fn a_match_past_any_line_is_left_out() {
        let answer = Answer {
            matches: vec![Match {
                start: 0,
                extra: usize::MAX,
                text: "x".to_owned(),
            }],
            cut: false,
        };
        let word = Word::new("ab", 0, 2).unwrap();
        assert!(items(&answer, &word).is_empty());
    }

    #[test]
    fn a_word_is_on_character_boundaries_in_order() {
        assert!(Word::new("é", 1, 2).is_none());
        assert!(Word::new("ab", 2, 1).is_none());
        assert!(Word::new("ab", 0, 3).is_none());
        assert_eq!(Word::new("ab c", 3, 4).unwrap().typed(), "c");
    }

    #[test]
    fn the_status_line_counts_what_went_wrong() {
        assert_eq!(status_line(true, 0, 0), "bash completion: on");
        assert_eq!(status_line(false, 0, 0), "bash completion: off");
        assert_eq!(status_line(true, 3, 0), "bash completion: on (3 timed out)");
        assert_eq!(status_line(true, 0, 1), "bash completion: on (1 failed)");
        assert_eq!(
            status_line(true, 3, 1),
            "bash completion: on (3 timed out, 1 failed)"
        );
    }
}
