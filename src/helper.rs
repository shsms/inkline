//! Helper programs that colour a command's arguments: the ones Lisp
//! registered, their processes, and the protocol inkline speaks with them
//! (docs/highlight-protocol.md).

use std::cell::RefCell;
use std::time::{Duration, Instant};

use process::Process;
use protocol::Read;

pub mod process;
pub mod protocol;

/// How long a redraw that starts helpers waits for them to name themselves.
const START_WAIT: Duration = Duration::from_millis(15);

enum State {
    NotStarted,
    /// Started; its first line has not all come yet.
    Starting(Process),
    /// Its first line was good.
    #[allow(dead_code)]
    Running(Process),
    /// Turned off until it is registered again or `inkline reload`, for this
    /// reason.
    Off(String),
}

struct Helper {
    /// The command name it colours.
    name: String,
    /// The program and its arguments.
    program: Vec<String>,
    state: State,
    /// The message saying it was turned off, until `take_notices` takes it.
    notice: Option<String>,
}

thread_local! {
    /// The registered helpers, in the order their names were first
    /// registered. Borrowed only inside this module's functions, which run no
    /// Lisp.
    static HELPERS: RefCell<Vec<Helper>> = const { RefCell::new(Vec::new()) };
}

/// Registers `program` as the helper for the command `name`, or with `None`
/// removes the helper. A helper registered again starts afresh; its old
/// process, if any, sees its input end, and a message about it not yet
/// taken is dropped.
pub fn register(name: &str, program: Option<Vec<String>>) {
    HELPERS.with_borrow_mut(|helpers| {
        let at = helpers.iter().position(|h| h.name == name);
        match (at, program) {
            (Some(i), Some(program)) => helpers[i] = Helper::new(name, program),
            (None, Some(program)) => helpers.push(Helper::new(name, program)),
            (Some(i), None) => drop(helpers.remove(i)),
            (None, None) => {}
        }
    });
}

/// Whether a helper is registered for the command `name`.
pub fn is_registered(name: &str) -> bool {
    HELPERS.with_borrow(|helpers| helpers.iter().any(|h| h.name == name))
}

/// Whether any helper is registered.
pub fn any_registered() -> bool {
    HELPERS.with_borrow(|helpers| !helpers.is_empty())
}

/// Forgets every helper, and the messages about them not yet taken; their
/// processes see their input end.
pub fn stop_all() {
    HELPERS.with_borrow_mut(Vec::clear);
}

/// One line per helper, in registration order: `highlight NAME: running`
/// (also while its first line is still coming), `not started`, or
/// `off (REASON)`.
pub fn status_lines() -> Vec<String> {
    HELPERS.with_borrow(|helpers| {
        helpers
            .iter()
            .map(|h| {
                let state = match &h.state {
                    State::NotStarted => "not started".to_owned(),
                    State::Starting(_) | State::Running(_) => "running".to_owned(),
                    State::Off(reason) => format!("off ({reason})"),
                };
                format!("highlight {}: {state}", h.name)
            })
            .collect()
    })
}

/// Gets the helpers for the command names in `names` going: starts those
/// not started yet, looking up programs in `path` (bash's `PATH`) and
/// giving them the environment `environment` returns (see
/// `process::start`), and reads the first line of those still starting,
/// waiting up to 15 ms in all for the ones this call started. A helper this
/// call turns off gets a message for `take_notices`.
pub fn prepare(names: &[String], path: &str, environment: fn() -> Option<Vec<Vec<u8>>>) {
    let deadline = Instant::now() + START_WAIT;
    HELPERS.with_borrow_mut(|helpers| {
        for h in helpers.iter_mut().filter(|h| names.contains(&h.name)) {
            if let Err(reason) = h.prepare(deadline, path, environment) {
                h.notice = Some(format!("inkline: highlight {}: off ({reason})", h.name));
                h.state = State::Off(reason);
            }
        }
    });
}

/// Whether a helper was turned off and the message saying so is not taken
/// yet.
pub fn has_notices() -> bool {
    HELPERS.with_borrow(|helpers| helpers.iter().any(|h| h.notice.is_some()))
}

/// Takes the messages saying helpers were turned off, in registration
/// order.
pub fn take_notices() -> Vec<String> {
    HELPERS.with_borrow_mut(|helpers| helpers.iter_mut().filter_map(|h| h.notice.take()).collect())
}

impl Helper {
    fn new(name: &str, program: Vec<String>) -> Helper {
        Helper {
            name: name.to_owned(),
            program,
            state: State::NotStarted,
            notice: None,
        }
    }

    /// Starts the helper if it is not started, as `prepare` says, waiting
    /// until `deadline` for its first line, and reads what has come of the
    /// first line if it is starting. The reason it must be turned off, if it
    /// must.
    fn prepare(
        &mut self,
        deadline: Instant,
        path: &str,
        environment: fn() -> Option<Vec<Vec<u8>>>,
    ) -> Result<(), String> {
        let until = match &self.state {
            State::NotStarted => {
                let started = process::start(&self.program, path, environment())?;
                self.state = State::Starting(started);
                Some(deadline)
            }
            State::Starting(_) => None,
            State::Running(_) | State::Off(_) => return Ok(()),
        };
        let State::Starting(process) = &mut self.state else {
            unreachable!("the helper is starting");
        };
        if read_version(process, until)? {
            let State::Starting(process) = std::mem::replace(&mut self.state, State::NotStarted)
            else {
                unreachable!("the helper is starting");
            };
            self.state = State::Running(process);
        }
        Ok(())
    }
}

/// Reads the helper's first line, waiting for it until `until` or, without
/// it, taking only what has come. Whether the line has all come and is
/// good, or the reason to turn the helper off. The extra requests the line
/// may name are not used: this version of inkline sends none of them.
fn read_version(process: &mut Process, until: Option<Instant>) -> Result<bool, String> {
    loop {
        match protocol::version(process.buffer()) {
            Read::Done(_, used) => {
                process.buffer().drain(..used);
                return Ok(true);
            }
            Read::Bad(reason) => return Err(reason),
            Read::Incomplete => {}
        }
        if !process.fill(until)? {
            return Ok(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(mode: &str) -> Option<Vec<String>> {
        Some(vec![
            format!("{}/tests/data/fake-highlight", env!("CARGO_MANIFEST_DIR")),
            mode.to_owned(),
        ])
    }

    fn path() -> String {
        std::env::var("PATH").unwrap_or_default()
    }

    /// Prepares `name` until it is no longer starting, or two seconds have
    /// passed; the messages it gave.
    fn prepare_until_started(name: &str) -> Vec<String> {
        let names = [name.to_owned()];
        prepare(&names, &path(), || None);
        let deadline = Instant::now() + Duration::from_secs(2);
        let starting = || {
            HELPERS.with_borrow(|hs| {
                hs.iter()
                    .any(|h| h.name == name && matches!(h.state, State::Starting(_)))
            })
        };
        while starting() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            prepare(&names, &path(), || None);
        }
        take_notices()
    }

    #[test]
    fn registering_replacing_and_removing() {
        register("a", Some(vec!["x".to_owned()]));
        register("b", Some(vec!["y".to_owned()]));
        register("a", Some(vec!["z".to_owned()]));
        assert!(is_registered("a") && is_registered("b") && !is_registered("c"));
        assert_eq!(
            status_lines(),
            ["highlight a: not started", "highlight b: not started"]
        );
        register("a", None);
        register("c", None);
        assert_eq!(status_lines(), ["highlight b: not started"]);
        stop_all();
        assert!(!any_registered());
    }

    #[test]
    fn a_helper_starts_and_names_itself() {
        register("csvm", fake("words"));
        assert!(prepare_until_started("csvm").is_empty());
        assert_eq!(status_lines(), ["highlight csvm: running"]);
        let running = HELPERS.with_borrow(|hs| matches!(hs[0].state, State::Running(_)));
        assert!(running);
    }

    #[test]
    fn only_the_names_asked_for_start() {
        register("csvm", fake("words"));
        register("other", fake("words"));
        prepare(&["csvm".to_owned()], &path(), || None);
        assert_eq!(
            status_lines(),
            ["highlight csvm: running", "highlight other: not started"]
        );
    }

    #[test]
    fn a_missing_program_is_off_once() {
        register("csvm", Some(vec!["no-such-helper-xyz".to_owned()]));
        prepare(&["csvm".to_owned()], &path(), || None);
        assert!(has_notices());
        assert_eq!(take_notices(), ["inkline: highlight csvm: off (not found)"]);
        assert!(!has_notices());
        prepare(&["csvm".to_owned()], &path(), || None);
        assert!(take_notices().is_empty());
        assert_eq!(status_lines(), ["highlight csvm: off (not found)"]);
    }

    #[test]
    fn a_wrong_first_line_is_off() {
        register("csvm", fake("version"));
        assert_eq!(
            prepare_until_started("csvm"),
            ["inkline: highlight csvm: off (not a highlight helper)"]
        );
        assert_eq!(
            status_lines(),
            ["highlight csvm: off (not a highlight helper)"]
        );
    }

    #[test]
    fn registering_again_turns_an_off_helper_back_on() {
        register("csvm", Some(vec!["no-such-helper-xyz".to_owned()]));
        prepare(&["csvm".to_owned()], &path(), || None);
        register("csvm", fake("words"));
        assert_eq!(status_lines(), ["highlight csvm: not started"]);
        assert!(!has_notices(), "the old message is dropped");
    }

    #[test]
    fn removing_or_stopping_drops_the_messages_not_taken() {
        let missing = || Some(vec!["no-such-helper-xyz".to_owned()]);
        register("a", missing());
        register("b", missing());
        prepare(&["a".to_owned(), "b".to_owned()], &path(), || None);
        register("a", None);
        assert_eq!(take_notices(), ["inkline: highlight b: off (not found)"]);
        register("a", missing());
        prepare(&["a".to_owned()], &path(), || None);
        stop_all();
        assert!(!has_notices());
    }
}
