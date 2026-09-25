//! Helper programs that colour a command's arguments: the ones Lisp
//! registered, their processes, and the protocol inkline speaks with them
//! (docs/highlight-protocol.md).

use std::cell::RefCell;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use process::Process;
use protocol::{Read, Reply};

pub mod process;
pub mod protocol;

/// The longest a redraw waits for helpers, in all: for the first line of
/// the ones starting, then for their replies.
pub const WAIT: Duration = Duration::from_millis(15);

/// The most a helper may have written that inkline has not used, while a
/// reply or its first line is still incomplete.
const MOST_UNREAD: usize = 1 << 20;

/// A request to a helper, and what a kept reply is looked up by: the
/// directory (bash's `PWD`), then each argument's `raw` flag and bytes,
/// the command name first.
pub type Request = (Vec<u8>, Vec<(bool, String)>);

enum State {
    NotStarted,
    /// Started; its first line has not all come yet.
    Starting(Process),
    /// Its first line was good.
    Running(Running),
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

/// A helper that named itself, and the requests it was sent.
struct Running {
    process: Process,
    /// The ID of the last request sent, 0 before the first.
    last_id: u64,
    /// The request sent and not answered yet, with its ID. There is at
    /// most one at a time.
    in_flight: Option<(u64, Request)>,
    /// The replies kept, each with the request it answers; at most one per
    /// request.
    kept: Vec<(Request, Reply)>,
    /// How far `protocol::reply` has looked into the process's buffer for
    /// the reply in flight.
    reply_seen: usize,
    /// The replies to requests with this ID or a lower one are dropped when
    /// they come (see `forget_replies`).
    forgotten: u64,
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

/// Drops the replies kept so far, and the reply to any request in flight
/// when it comes: a new line starts, and the files a reply speaks of may
/// have changed since it was written.
pub fn forget_replies() {
    HELPERS.with_borrow_mut(|helpers| {
        for h in helpers.iter_mut() {
            if let State::Running(running) = &mut h.state {
                running.kept.clear();
                running.forgotten = running.last_id;
            }
        }
    });
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

/// Gets the helpers for the command names in `names` going, without
/// waiting: starts those not started yet, looking up programs in `path`
/// (bash's `PATH`) and giving them the environment `environment` returns
/// (see `process::start`), and reads what has come of the first line of
/// those starting. A helper this call turns off gets a message for
/// `take_notices`.
pub fn prepare(names: &[String], path: &str, environment: fn() -> Option<Vec<Vec<u8>>>) {
    HELPERS.with_borrow_mut(|helpers| {
        for h in helpers.iter_mut().filter(|h| names.contains(&h.name)) {
            if let Err(reason) = h.prepare(path, environment) {
                h.turn_off(reason);
            }
        }
    });
}

/// The reply to each of `asks` (a helper's command name and a request), or
/// `None` when it has not come in time.
///
/// A kept reply to the same request answers at once. For the rest, each helper
/// is sent one request at a time, in the order of `asks`, and replies are read
/// until every ask is answered or `wait` has passed, even while a helper keeps
/// writing; a helper still starting first has its first line read in that time.
/// A reply that comes for a request no longer asked is kept as that request's
/// reply (the line may come back to it), but is never an answer by itself. Kept
/// replies whose requests this call does not ask for are dropped first. A
/// helper that breaks the protocol, writes more than `MOST_UNREAD` bytes
/// without finishing a reply, or exits is turned off, with a message for
/// `take_notices`. A signal ends the wait early; `interrupted` is for
/// `Process::send`.
pub fn replies(
    asks: &[(String, Request)],
    wait: Duration,
    interrupted: fn() -> bool,
) -> Vec<Option<Reply>> {
    let deadline = Instant::now() + wait;
    HELPERS.with_borrow_mut(|helpers| {
        let requests: Vec<Vec<&Request>> = helpers
            .iter()
            .map(|h| requests_for(asks, &h.name))
            .collect();
        for (h, requests) in helpers.iter_mut().zip(&requests) {
            if let State::Running(running) = &mut h.state {
                running
                    .kept
                    .retain(|(request, _)| requests.contains(&request));
            }
        }
        loop {
            let mut waiting = Vec::new();
            for (h, requests) in helpers.iter_mut().zip(&requests) {
                if requests.is_empty() {
                    continue;
                }
                match h.advance(requests, deadline, interrupted) {
                    Ok(fd) => waiting.extend(fd),
                    Err(reason) => h.turn_off(reason),
                }
            }
            if waiting.is_empty()
                || Instant::now() >= deadline
                || !process::readable(&waiting, deadline)
            {
                break;
            }
        }
        asks.iter()
            .map(|(name, request)| {
                let h = helpers.iter().find(|h| h.name == *name)?;
                match &h.state {
                    State::Running(running) => running.kept(request).cloned(),
                    State::NotStarted | State::Starting(_) | State::Off(_) => None,
                }
            })
            .collect()
    })
}

/// The sockets of the helpers that owe inkline something: those still
/// starting, and those with a request in flight. Waiting for a key also
/// waits on them, so that `read_waiting` can take what they send.
pub fn waiting_fds() -> Vec<RawFd> {
    HELPERS.with_borrow(|helpers| {
        helpers
            .iter()
            .filter_map(|h| match &h.state {
                State::Starting(process) => Some(process.fd()),
                State::Running(running) if running.in_flight.is_some() => {
                    Some(running.process.fd())
                }
                State::NotStarted | State::Running(_) | State::Off(_) => None,
            })
            .collect()
    })
}

/// Reads what the helpers in `waiting_fds` have sent, without waiting:
/// their first lines, and replies, which are kept for the next redraw to
/// find. A helper that breaks the protocol, writes too much or exits is
/// turned off, with a message for `take_notices`. Whether the line must be
/// drawn again: a reply came (the redraw uses it, or sends the request for
/// the line as it is now), a helper started running (the redraw sends it
/// its request), or a helper was turned off.
pub fn read_waiting() -> bool {
    HELPERS.with_borrow_mut(|helpers| {
        let mut changed = false;
        for h in helpers.iter_mut() {
            let read = match &mut h.state {
                State::Starting(_) => h
                    .read_first_line()
                    .map(|()| matches!(h.state, State::Running(_))),
                State::Running(running) if running.in_flight.is_some() => running.read_reply(),
                State::NotStarted | State::Running(_) | State::Off(_) => continue,
            };
            match read {
                Ok(read) => changed |= read,
                Err(reason) => {
                    h.turn_off(reason);
                    changed = true;
                }
            }
        }
        changed
    })
}

/// The requests in `asks` for the helper for `name`, in order.
fn requests_for<'a>(asks: &'a [(String, Request)], name: &str) -> Vec<&'a Request> {
    asks.iter()
        .filter(|(n, _)| n == name)
        .map(|(_, request)| request)
        .collect()
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

    /// Starts the helper if it is not started, as `prepare` says, and reads
    /// what has come of its first line if it is starting. The reason it
    /// must be turned off, if it must.
    fn prepare(
        &mut self,
        path: &str,
        environment: fn() -> Option<Vec<Vec<u8>>>,
    ) -> Result<(), String> {
        if let State::NotStarted = self.state {
            let started = process::start(&self.program, path, environment())?;
            self.state = State::Starting(started);
        }
        self.read_first_line()
    }

    /// Reads what has come of the first line, if the helper is starting,
    /// and has it running once the line is all there and good. The reason
    /// it must be turned off, if it must.
    fn read_first_line(&mut self) -> Result<(), String> {
        let State::Starting(process) = &mut self.state else {
            return Ok(());
        };
        if read_version(process)? {
            let State::Starting(process) = std::mem::replace(&mut self.state, State::NotStarted)
            else {
                unreachable!("the helper is starting");
            };
            self.state = State::Running(Running {
                process,
                last_id: 0,
                in_flight: None,
                kept: Vec::new(),
                reply_seen: 0,
                forgotten: 0,
            });
        }
        Ok(())
    }

    /// Does what can be done without waiting towards a reply for each of
    /// `requests`, stopping at `deadline` if the helper keeps writing: reads
    /// the first line or replies that have come, and sends the next request
    /// when none is in flight (see `Process::send` for `interrupted`). The
    /// socket to wait on when some request is still unanswered and the helper
    /// owes inkline something, or the reason it must be turned off.
    fn advance(
        &mut self,
        requests: &[&Request],
        deadline: Instant,
        interrupted: fn() -> bool,
    ) -> Result<Option<RawFd>, String> {
        self.read_first_line()?;
        match &mut self.state {
            State::Starting(process) => Ok(Some(process.fd())),
            State::Running(running) => running.advance(requests, deadline, interrupted),
            State::NotStarted | State::Off(_) => Ok(None),
        }
    }

    /// Turns the helper off for `reason`, with a message saying so.
    fn turn_off(&mut self, reason: String) {
        self.notice = Some(format!("inkline: highlight {}: off ({reason})", self.name));
        self.state = State::Off(reason);
    }
}

impl Running {
    /// See `Helper::advance`.
    ///
    /// What it read is always looked at before it returns, so a whole reply
    /// is never left in the buffer, where waiting on the socket would not
    /// see it.
    fn advance(
        &mut self,
        requests: &[&Request],
        deadline: Instant,
        interrupted: fn() -> bool,
    ) -> Result<Option<RawFd>, String> {
        let mut read_some = false;
        loop {
            self.take_reply()?;
            let Some(&unanswered) = requests.iter().find(|request| self.kept(request).is_none())
            else {
                return Ok(None);
            };
            if self.in_flight.is_none() {
                self.send(unanswered, interrupted)?;
            }
            if read_some && Instant::now() >= deadline {
                return Ok(Some(self.process.fd()));
            }
            if !self.process.fill(None)? {
                return Ok(Some(self.process.fd()));
            }
            read_some = true;
        }
    }

    /// Reads what has come towards the reply to the request in flight,
    /// without waiting, and takes the reply once it is whole. Whether one
    /// came, or the reason the helper must be turned off.
    fn read_reply(&mut self) -> Result<bool, String> {
        loop {
            if self.take_reply()? {
                return Ok(true);
            }
            if self.in_flight.is_none() {
                return Ok(false);
            }
            if !self.process.fill(None)? {
                return Ok(false);
            }
        }
    }

    /// Takes the reply to the request in flight if the buffer holds all of
    /// it: keeps it, or drops it if it is forgotten. Whether one came, or
    /// the reason the helper must be turned off.
    fn take_reply(&mut self) -> Result<bool, String> {
        let Some((id, request)) = &self.in_flight else {
            return Ok(false);
        };
        let lens: Vec<usize> = request.1.iter().map(|(_, text)| text.len()).collect();
        match protocol::reply(self.process.buffer(), *id, &lens, &mut self.reply_seen) {
            Read::Done(reply, used) => {
                let wanted = *id > self.forgotten;
                self.process.buffer().drain(..used);
                self.reply_seen = 0;
                if let Some((_, request)) = self.in_flight.take()
                    && wanted
                {
                    self.keep(request, reply);
                }
                Ok(true)
            }
            Read::Bad(reason) => Err(reason),
            Read::Incomplete => check_unread(&mut self.process).map(|()| false),
        }
    }

    /// Sends `request` (see `Process::send` for `interrupted`).
    fn send(&mut self, request: &Request, interrupted: fn() -> bool) -> Result<(), String> {
        let id = self.last_id + 1;
        self.process
            .send(&protocol::request(id, &request.0, &request.1), interrupted)?;
        self.last_id = id;
        self.in_flight = Some((id, request.clone()));
        Ok(())
    }

    /// Keeps `reply` as the reply for `request`, in place of any before it.
    fn keep(&mut self, request: Request, reply: Reply) {
        match self.kept.iter_mut().find(|(k, _)| *k == request) {
            Some(slot) => slot.1 = reply,
            None => self.kept.push((request, reply)),
        }
    }

    /// The reply kept for `request`.
    fn kept(&self, request: &Request) -> Option<&Reply> {
        self.kept
            .iter()
            .find(|(k, _)| k == request)
            .map(|(_, reply)| reply)
    }
}

/// Reads what has come of the helper's first line, without waiting.
/// Whether the line has all come and is good, or the reason to turn the
/// helper off. The extra requests the line may name are not used: this
/// version of inkline sends none of them.
fn read_version(process: &mut Process) -> Result<bool, String> {
    loop {
        match protocol::version(process.buffer()) {
            Read::Done(_, used) => {
                process.buffer().drain(..used);
                return Ok(true);
            }
            Read::Bad(reason) => return Err(reason),
            Read::Incomplete => check_unread(process)?,
        }
        if !process.fill(None)? {
            return Ok(false);
        }
    }
}

/// An error when the helper has written more than `MOST_UNREAD` bytes that
/// do not yet make a whole reply or first line.
fn check_unread(process: &mut Process) -> Result<(), String> {
    if process.buffer().len() > MOST_UNREAD {
        return Err("bad reply: too much output".to_owned());
    }
    Ok(())
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

    /// A request for `csvm SCRIPT`.
    fn request(script: &str) -> Request {
        (
            b"/".to_vec(),
            vec![(false, "csvm".to_owned()), (false, script.to_owned())],
        )
    }

    /// Asks the helper for `csvm` about `scripts`, starting it first.
    fn ask(scripts: &[&str], wait: Duration) -> Vec<Option<Reply>> {
        prepare(&["csvm".to_owned()], &path(), || None);
        let asks: Vec<_> = scripts
            .iter()
            .map(|s| ("csvm".to_owned(), request(s)))
            .collect();
        replies(&asks, wait, || false)
    }

    /// The kinds of a reply's spans.
    fn kinds(reply: &Option<Reply>) -> Vec<crate::lexer::Kind> {
        reply
            .as_ref()
            .unwrap()
            .spans
            .iter()
            .map(|s| s.kind)
            .collect()
    }

    #[test]
    fn replies_come_and_are_kept() {
        use crate::lexer::Kind::{Command, Number};
        register("csvm", fake("words"));
        let got = ask(&["a 1"], Duration::from_secs(2));
        assert_eq!(got.len(), 1);
        assert_eq!(kinds(&got[0]), [Command, Number]);
        assert_eq!(ask(&["a 1"], Duration::ZERO), got, "kept, not sent again");
        assert!(take_notices().is_empty());
    }

    #[test]
    fn several_asks_of_one_helper_are_all_answered() {
        use crate::lexer::Kind::{Command, Number, Variable};
        register("csvm", fake("words"));
        let got = ask(&["a", "b c", "1"], Duration::from_secs(2));
        assert_eq!(kinds(&got[0]), [Command]);
        assert_eq!(kinds(&got[1]), [Command, Variable]);
        assert_eq!(kinds(&got[2]), [Number]);
    }

    #[test]
    fn a_late_reply_is_not_a_failure_and_is_kept_for_its_request() {
        register("csvm", fake("late"));
        assert!(prepare_until_started("csvm").is_empty());
        assert_eq!(ask(&["a"], Duration::from_millis(20)), [None]);
        assert!(take_notices().is_empty());
        assert_eq!(status_lines(), ["highlight csvm: running"]);
        // The reply for `a` comes while waiting for `b`'s.
        let got = ask(&["b"], Duration::from_secs(3));
        assert!(got[0].is_some());
        let got = ask(&["a"], Duration::ZERO);
        assert!(got[0].is_some(), "the reply for `a` was kept");
    }

    /// Waits on `waiting_fds` and reads them until `done`, or two seconds
    /// have passed. Whether `read_waiting` said the line must be drawn
    /// again.
    fn read_until(done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut changed = false;
        while !done() && Instant::now() < deadline {
            let fds = waiting_fds();
            assert!(!fds.is_empty(), "nothing to wait on");
            process::readable(&fds, deadline);
            changed |= read_waiting();
        }
        changed
    }

    #[test]
    fn a_reply_that_comes_between_redraws_is_kept() {
        register("csvm", fake("late"));
        assert!(prepare_until_started("csvm").is_empty());
        assert!(waiting_fds().is_empty(), "nothing asked yet");
        assert_eq!(ask(&["a"], Duration::from_millis(20)), [None]);
        assert!(!read_waiting(), "nothing has come yet");
        let answered = || {
            HELPERS
                .with_borrow(|hs| matches!(&hs[0].state, State::Running(r) if !r.kept.is_empty()))
        };
        assert!(read_until(answered));
        assert!(waiting_fds().is_empty(), "nothing in flight");
        assert!(ask(&["a"], Duration::ZERO)[0].is_some());
    }

    /// A reply to a request sent before `forget_replies` is dropped when it
    /// comes, and asks for a redraw, which sends the request for the line
    /// as it is now.
    #[test]
    fn a_forgotten_reply_is_dropped_and_the_line_asked_about_again() {
        register("csvm", fake("late"));
        assert!(prepare_until_started("csvm").is_empty());
        assert_eq!(ask(&["a"], Duration::ZERO), [None]);
        forget_replies();
        // Its request cannot go out while the forgotten one is in flight.
        assert_eq!(ask(&["a"], Duration::ZERO), [None]);
        let idle = || waiting_fds().is_empty();
        assert!(read_until(idle), "the dropped reply asks for a redraw");
        let kept = || {
            HELPERS
                .with_borrow(|hs| matches!(&hs[0].state, State::Running(r) if !r.kept.is_empty()))
        };
        assert!(!kept(), "the forgotten reply is not kept");
        assert!(ask(&["a"], Duration::from_secs(2))[0].is_some());
    }

    #[test]
    fn a_helper_still_starting_is_waited_on() {
        // It prints its first line after 0.3 s.
        register("csvm", fake("slow"));
        prepare(&["csvm".to_owned()], &path(), || None);
        assert_eq!(waiting_fds().len(), 1, "waited on while starting");
        assert!(!read_waiting(), "its first line has not come");
        let running = || HELPERS.with_borrow(|hs| matches!(hs[0].state, State::Running(_)));
        assert!(read_until(running), "moving to running asks for a redraw");
        assert!(running());
        assert!(waiting_fds().is_empty(), "nothing in flight");
        assert!(take_notices().is_empty());
    }

    #[test]
    fn a_helper_that_exits_between_redraws_is_off() {
        register("csvm", fake("exit"));
        assert!(prepare_until_started("csvm").is_empty());
        // It exits once it has read the request this sends.
        assert_eq!(ask(&["a"], Duration::ZERO), [None]);
        let off = || HELPERS.with_borrow(|hs| matches!(hs[0].state, State::Off(_)));
        assert!(read_until(off));
        assert_eq!(take_notices(), ["inkline: highlight csvm: off (exited)"]);
        assert!(waiting_fds().is_empty());
    }

    #[test]
    fn a_helper_that_exits_is_off() {
        register("csvm", fake("exit"));
        assert_eq!(ask(&["a"], Duration::from_secs(2)), [None]);
        assert_eq!(take_notices(), ["inkline: highlight csvm: off (exited)"]);
        assert_eq!(status_lines(), ["highlight csvm: off (exited)"]);
        assert_eq!(ask(&["a"], Duration::from_secs(2)), [None]);
        assert!(take_notices().is_empty());
    }

    #[test]
    fn a_reply_that_breaks_the_protocol_is_off() {
        register("csvm", fake("garbage"));
        assert_eq!(ask(&["a"], Duration::from_secs(2)), [None]);
        assert_eq!(
            take_notices(),
            ["inkline: highlight csvm: off (bad reply: \"nonsense\")"]
        );
    }

    /// Reads never block, and the wait ends at its deadline. The fake writes
    /// more slowly than inkline reads; a helper that writes faster is turned
    /// off at `MOST_UNREAD` (see `a_helper_that_writes_too_much_is_off`).
    #[test]
    fn a_helper_that_keeps_writing_does_not_hold_up_a_redraw() {
        register("csvm", fake("spew"));
        assert!(prepare_until_started("csvm").is_empty());
        let began = Instant::now();
        assert_eq!(ask(&["a"], Duration::from_millis(15)), [None]);
        let took = began.elapsed();
        assert!(took < Duration::from_millis(100), "took {took:?}");
    }

    #[test]
    fn a_helper_that_writes_too_much_is_off() {
        register("csvm", fake("spew"));
        let deadline = Instant::now() + Duration::from_secs(20);
        while !has_notices() && Instant::now() < deadline {
            ask(&["a"], Duration::from_millis(15));
        }
        assert_eq!(
            take_notices(),
            ["inkline: highlight csvm: off (bad reply: too much output)"]
        );
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
