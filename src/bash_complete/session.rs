//! This shell's bash completion: the answer saved for the last word asked
//! about, the copy of the shell working on the next one, the word given up
//! on, and how many requests timed out or failed.

use std::cell::RefCell;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use super::request::{Read, Running};
use super::{Ask, Inputs, Saved, Word, command_position, decide, fits, items};
use crate::menu::{Item, Matching};
use crate::mode_server::process;

#[derive(Default)]
struct Session {
    saved: Option<Saved>,
    running: Option<Running>,
    given_up: Option<Word>,
    timed_out: u64,
    failed: u64,
}

impl Session {
    /// Takes the running copy's answer if it has come: it becomes the saved
    /// answer. A copy that failed is counted, and its word given up on.
    /// Whether either happened.
    fn read_running(&mut self) -> bool {
        let Some(running) = self.running.as_mut() else {
            return false;
        };
        let read = running.read();
        if matches!(read, Read::More) {
            return false;
        }
        let word = running.word.clone();
        self.running = None;
        match read {
            Read::Came(answer) => self.saved = Some(Saved { word, answer }),
            Read::Failed | Read::More => {
                self.failed += 1;
                self.given_up = Some(word);
            }
        }
        true
    }

    /// Kills the running copy once its time is up; it is counted, and its
    /// word given up on. Whether it did.
    fn expire_running(&mut self) -> bool {
        let Some(running) = self.running.take_if(|r| Instant::now() >= r.deadline) else {
            return false;
        };
        self.timed_out += 1;
        self.given_up = Some(running.word.clone());
        true
    }

    /// Asks a copy of the shell about `word` now, in place of any copy
    /// running. A copy that cannot start counts as failed.
    fn start(&mut self, word: &Word, limit: Duration) {
        self.running = None;
        match Running::start(word.clone(), limit) {
            Ok(running) => self.running = Some(running),
            Err(_) => {
                self.failed += 1;
                self.given_up = Some(word.clone());
            }
        }
    }
}

thread_local! {
    static SESSION: RefCell<Session> = RefCell::new(Session::default());
}

/// What a request follows.
pub struct Settings {
    /// `inkline-command-min-chars`.
    pub min_chars: usize,
    /// `inkline-bash-completion-timeout`.
    pub timeout: Duration,
    pub how: Matching,
}

/// What `prepare` decided for a word, for `found`.
pub struct Ticket {
    word: Word,
    listed: bool,
    /// Whether a copy is to be asked once typing pauses.
    at_pause: bool,
}

/// What bash's completion gives the menu for a word.
#[derive(Default)]
pub struct Found {
    pub items: Vec<Item>,
    /// Whether a copy is still working on an answer for the word, or is to
    /// be asked for one once typing pauses.
    pub waiting: bool,
}

/// Decides what bash's completion does for `word` (see `decide`): a copy
/// of the shell is asked now, and one working on anything else is killed.
/// `paused` says typing has paused on this line and cursor. The ticket for
/// `found`, and whether a pause is wanted to ask again.
pub fn prepare(word: Word, settings: &Settings, paused: bool) -> (Ticket, bool) {
    SESSION.with_borrow_mut(|me| {
        me.read_running();
        let decision = decide(&Inputs {
            word: &word,
            command: command_position(word.line(), word.start()),
            min_chars: settings.min_chars,
            saved: me.saved.as_ref(),
            running: me.running.as_ref().map(|r| &r.word),
            given_up: me.given_up.as_ref(),
            paused,
            how: settings.how,
        });
        match decision.ask {
            // The copy works on nothing in this shell's `RefCell`s but the
            // line: the fork holds this borrow of `SESSION` in the copy too,
            // and the copy never uses `SESSION`.
            Ask::Now => me.start(&word, settings.timeout),
            Ask::InFlight => {}
            Ask::No | Ask::AtPause => me.running = None,
        }
        let at_pause = decision.ask == Ask::AtPause;
        (
            Ticket {
                word,
                listed: decision.listed,
                at_pause,
            },
            at_pause,
        )
    })
}

/// Waits, until `deadline` at most and while `interrupted` says no signal
/// must be acted on, for a copy working on an answer for `ticket`'s word;
/// then gives the items of the answer that serves the word, and whether a
/// copy is still working on one or is to be asked once typing pauses.
pub fn found(ticket: &Ticket, deadline: Instant, interrupted: fn() -> bool) -> Found {
    SESSION.with_borrow_mut(|me| {
        loop {
            me.read_running();
            me.expire_running();
            let Some(fd) = me
                .running
                .as_ref()
                .filter(|r| fits(&r.word, &ticket.word))
                .map(Running::fd)
            else {
                break;
            };
            if interrupted() || Instant::now() >= deadline || !process::readable(&[fd], deadline) {
                break;
            }
        }
        let given_up = me
            .given_up
            .as_ref()
            .is_some_and(|w| w.same_place(&ticket.word));
        if !ticket.listed || given_up {
            return Found::default();
        }
        Found {
            items: me
                .saved
                .as_ref()
                .filter(|s| fits(&s.word, &ticket.word))
                .map(|s| items(&s.answer, &ticket.word))
                .unwrap_or_default(),
            waiting: ticket.at_pause
                || me
                    .running
                    .as_ref()
                    .is_some_and(|r| fits(&r.word, &ticket.word)),
        }
    })
}

/// The pipe of the copy running, for the key reader to wait on.
pub fn waiting_fd() -> Option<RawFd> {
    SESSION.with_borrow(|me| me.running.as_ref().map(Running::fd))
}

/// Milliseconds until the running copy's time is up, rounded up; None with
/// no copy running.
pub fn until_deadline() -> Option<std::ffi::c_int> {
    SESSION.with_borrow(|me| {
        let left = me
            .running
            .as_ref()?
            .deadline
            .saturating_duration_since(Instant::now());
        Some(
            std::ffi::c_int::try_from(left.as_micros().div_ceil(1000))
                .unwrap_or(std::ffi::c_int::MAX),
        )
    })
}

/// Kills the running copy if its time is up. Whether it did.
pub fn expire() -> bool {
    SESSION.with_borrow_mut(Session::expire_running)
}

/// `inkline status`'s line, with `on` from `inkline-bash-completion`. In a
/// copy of the shell, where a rule may run `inkline status` while the
/// session is borrowed, the counts read zero.
pub fn status_line(on: bool) -> String {
    SESSION.with(|me| match me.try_borrow() {
        Ok(me) => super::status_line(on, me.timed_out, me.failed),
        Err(_) => super::status_line(on, 0, 0),
    })
}

/// Takes the running copy's answer if it came, or notes that it failed,
/// without waiting. Whether the line must be drawn again.
pub fn read_waiting() -> bool {
    SESSION.with_borrow_mut(Session::read_running)
}

/// Kills the running copy and forgets the saved answer and the word given
/// up on: a new line starts, or bash completion stops. The counts stay.
/// State that cannot be borrowed is left alone, so a panic's recovery can
/// call it.
pub fn forget() {
    let _ = SESSION.try_with(|s| {
        s.try_borrow_mut().map(|mut me| {
            me.running = None;
            me.saved = None;
            me.given_up = None;
        })
    });
}
