//! What a copy of the shell does for one request: it sets itself apart from
//! the terminal and the shell's jobs, asks bash's completion for the word at
//! the cursor as Tab would, replays Tab for each match to see what it puts
//! on the line, writes the answer and exits.

use std::io::Write;
use std::os::fd::OwnedFd;

use super::answer::{self, Answer, MOST};
use crate::ffi;

/// Runs in the child of the fork, with `out` the pipe's writing end, and
/// never returns: it ends with `_exit`, 0 once the answer is written, or
/// through bash's own exit when a rule's error jumps out of it to bash's
/// top level (see `ffi::become_copy`).
pub fn run(out: OwnedFd) -> ! {
    let written = std::panic::catch_unwind(|| {
        ffi::become_copy();
        let answer = gather();
        std::fs::File::from(out)
            .write_all(&answer::encode(&answer))
            .is_ok()
    });
    ffi::exit_copy(if matches!(written, Ok(true)) { 0 } else { 1 })
}

/// bash's matches for the word at the cursor, each placed on the line by a
/// replayed Tab; at most `MOST`. A match whose line is not UTF-8 is left
/// out.
fn gather() -> Answer {
    let (Some(line), point) = (ffi::line(), ffi::point()) else {
        return Answer::default();
    };
    let Some(found) = ffi::bash_matches() else {
        return Answer::default();
    };
    let matches = found
        .matches
        .iter()
        .take(MOST)
        .filter_map(|one| {
            let after = ffi::replay_tab(&line, point, one, &found.settings);
            answer::placed(&line, point, found.start, &after)
        })
        .collect();
    Answer {
        matches,
        cut: found.matches.len() > MOST,
    }
}
