//! This shell's bash completion: the answer saved for the last word asked
//! about, the copy of the shell working on the next one, the word given up
//! on, and how many requests timed out or failed.

use std::cell::RefCell;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use super::request::{Read, Running};
use super::{Ask, Inputs, Saved, Word, decide, fits, items};
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
        match running.read() {
            Read::More => return false,
            Read::Came(answer) => {
                self.saved = Some(Saved {
                    word: running.word.clone(),
                    answer,
                });
            }
            Read::Failed => {
                self.failed += 1;
                self.given_up = Some(running.word.clone());
            }
        }
        self.running = None;
        true
    }

    /// Ends the running copy once its time is up: an answer or a failure
    /// that has come is taken, and a copy still working is killed, counted
    /// and its word given up on.
    fn expire(&mut self) -> Expired {
        if !self.running.as_ref().is_some_and(Running::expired) {
            Expired::No
        } else if self.read_running() {
            Expired::Came
        } else {
            if let Some(running) = self.running.take() {
                self.timed_out += 1;
                self.given_up = Some(running.word.clone());
            }
            Expired::Killed
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

/// What `prepare` decided for a word that gets bash's items, for `found`.
pub struct Ticket {
    word: Word,
    /// Whether a copy is to be asked once typing pauses.
    pub at_pause: bool,
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
/// `paused` says typing has paused on this line and cursor. While more keys
/// are still to come (`more_keys`), as in a macro or a paste, a copy is
/// asked once typing pauses instead. The ticket for `found`; None when
/// the word gets no bash items. A copy that cannot start counts as failed.
pub fn prepare(word: Word, settings: &Settings, paused: bool, more_keys: bool) -> Option<Ticket> {
    let ask = SESSION.with_borrow_mut(|me| {
        me.read_running();
        let ask = decide(&Inputs {
            word: &word,
            min_chars: settings.min_chars,
            saved: me.saved.as_ref(),
            running: me.running.as_ref().map(|r| &r.word),
            given_up: me.given_up.as_ref(),
            paused,
            how: settings.how,
        });
        if ask != Ask::InFlight {
            me.running = None;
        }
        match ask {
            Ask::Now if more_keys => Ask::AtPause,
            ask => ask,
        }
    });
    if ask == Ask::Nothing {
        return None;
    }
    // The fork happens with `SESSION` not borrowed, so the copy can use it.
    if ask == Ask::Now {
        let started = Running::start(word.clone(), settings.timeout);
        SESSION.with_borrow_mut(|me| match started {
            Ok(running) => me.running = Some(running),
            Err(_) => {
                me.failed += 1;
                me.given_up = Some(word.clone());
            }
        });
    }
    Some(Ticket {
        word,
        at_pause: ask == Ask::AtPause,
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
            me.expire();
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
        if given_up {
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
    SESSION.with_borrow(|me| Some(process::ms_until(me.running.as_ref()?.deadline)))
}

/// What `expire` did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Expired {
    /// No copy's time was up.
    No,
    /// The running copy's time was up, and its answer or its failure had
    /// come: it was taken, and the line must be drawn again.
    Came,
    /// The running copy's time was up, and it was killed.
    Killed,
}

/// Ends the running copy if its time is up: an answer or a failure that
/// has come is taken, and a copy still working is killed.
pub fn expire() -> Expired {
    SESSION.with_borrow_mut(Session::expire)
}

/// `inkline status`'s line, with `on` from `inkline-bash-completion`.
pub fn status_line(on: bool) -> String {
    SESSION.with_borrow(|me| super::status_line(on, me.timed_out, me.failed))
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
