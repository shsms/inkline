//! Command modes and the mode servers that supply them: the modes Lisp
//! defined, their servers' processes, and the protocol inkline speaks with
//! them (docs/mode-protocol.md).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use crate::args::CommandArgs;
use crate::colors::ColorSet;
use process::Process;
use protocol::{Depths, Read, Reply};

pub mod process;
pub mod protocol;

/// The longest a redraw waits for mode servers, in all: for the first line of
/// the ones starting, then for their replies.
pub const WAIT: Duration = Duration::from_millis(15);

/// The longest a new line waits for a mode server's depths, in all: for the
/// reply to a request already in flight, then for the depths.
pub const INDENT_WAIT: Duration = Duration::from_millis(100);

/// The most a mode server may have written that inkline has not used, while a
/// reply or its first line is still incomplete.
const MOST_UNREAD: usize = 1 << 20;

/// A request to a mode server, and what a kept reply is looked up by: the
/// directory (bash's `PWD`), then each argument's `raw` flag and bytes,
/// the command name first.
pub type Request = (Vec<u8>, Vec<(bool, String)>);

enum State {
    NotStarted,
    /// Started; its first line has not all come yet.
    Starting(Process),
    /// Its first line was good.
    Running(Running),
    /// Turned off until its mode is defined again or `inkline reload`, for
    /// this reason.
    Off(String),
}

struct Server {
    /// The name of the mode it supplies.
    mode: String,
    /// The program and its arguments.
    program: Vec<String>,
    /// The mode's own colours, over `inkline-colors`.
    colors: Option<ColorSet>,
    state: State,
    /// The message saying it was turned off, until `take_notices` takes it.
    notice: Option<String>,
}

/// A mode server that named itself, and the requests it was sent.
struct Running {
    process: Process,
    /// Whether its first line named `indent`.
    indent: bool,
    /// The ID of the last request sent, 0 before the first. Colour and
    /// indent requests share it.
    last_id: u64,
    /// The request sent and not answered yet, with its ID. There is at
    /// most one at a time, of either kind.
    in_flight: Option<(u64, Asked)>,
    /// The replies kept, each with the request it answers; at most one per
    /// request.
    kept: Vec<(Request, Reply)>,
    /// How far `protocol::reply` or `protocol::indent_reply` has looked
    /// into the process's buffer for the reply in flight.
    reply_seen: usize,
    /// The replies to colour requests with this ID or a lower one are
    /// dropped when they come (see `forget_replies`).
    forgotten: u64,
    /// The depths of the last indent reply read: `ask_indent` takes them
    /// once the reply to its own request comes.
    indent_answer: Option<Depths>,
}

/// What a request in flight asked.
enum Asked {
    /// Colours for these arguments.
    Colors(Request),
    /// Depths for a new line.
    Indent,
}

thread_local! {
    /// The defined modes' servers, in the order the modes were first
    /// defined. Borrowed only inside this module's functions, which run no
    /// Lisp.
    static SERVERS: RefCell<Vec<Server>> = const { RefCell::new(Vec::new()) };
}

/// Defines the mode `mode` with `program` as its server and `colors` as its
/// own colours, or with `None` removes the mode. The same program again
/// keeps a server that is not off and only changes the colours. Otherwise
/// the server starts afresh: its old process, if any, sees its input end,
/// and a message about it not yet taken is dropped.
pub fn define(mode: &str, program: Option<Vec<String>>, colors: Option<ColorSet>) {
    SERVERS.with_borrow_mut(|servers| {
        let at = servers.iter().position(|s| s.mode == mode);
        match (at, program) {
            (Some(i), Some(program))
                if servers[i].program == program && !matches!(servers[i].state, State::Off(_)) =>
            {
                servers[i].colors = colors;
            }
            (Some(i), Some(program)) => servers[i] = Server::new(mode, program, colors),
            (None, Some(program)) => servers.push(Server::new(mode, program, colors)),
            (Some(i), None) => drop(servers.remove(i)),
            (None, None) => {}
        }
    });
}

/// The mode `mode`'s own colours, if it is defined and was given some.
pub fn colors(mode: &str) -> Option<ColorSet> {
    SERVERS.with_borrow(|servers| servers.iter().find(|s| s.mode == mode)?.colors.clone())
}

/// Whether the mode `mode` is defined.
pub fn is_defined(mode: &str) -> bool {
    SERVERS.with_borrow(|servers| servers.iter().any(|s| s.mode == mode))
}

/// Whether any mode is defined.
pub fn any_defined() -> bool {
    SERVERS.with_borrow(|servers| !servers.is_empty())
}

/// The mode that the command whose name word is `word` uses: that of the
/// first pair of `table` (`inkline-command-mode-alist`'s pairs, as
/// `settings::command_modes` gives them) that names `word` (see `names`)
/// and whose mode is defined.
pub fn mode_for(word: &str, table: &[(String, String)]) -> Option<String> {
    SERVERS.with_borrow(|servers| {
        table
            .iter()
            .find(|(command, mode)| names(command, word) && servers.iter().any(|s| s.mode == *mode))
            .map(|(_, mode)| mode.clone())
    })
}

/// Whether the alist's `command` names the name word `word`: `word` itself,
/// or the part of `word` after its last `/`. An empty `command` names
/// nothing.
fn names(command: &str, word: &str) -> bool {
    !command.is_empty()
        && (command == word
            || word
                .rsplit_once('/')
                .is_some_and(|(_, last)| last == command))
}

/// Drops the replies kept so far, and the reply to any request in flight
/// when it comes: a new line starts, and the files a reply speaks of may
/// have changed since it was written.
pub fn forget_replies() {
    SERVERS.with_borrow_mut(|servers| {
        for s in servers.iter_mut() {
            if let State::Running(running) = &mut s.state {
                running.kept.clear();
                running.forgotten = running.last_id;
            }
        }
    });
}

/// Forgets every mode server, and the messages about them not yet taken; their
/// processes see their input end.
pub fn stop_all() {
    SERVERS.with_borrow_mut(Vec::clear);
}

/// One line per defined mode, in the order the modes were first defined: `mode
/// NAME (COMMANDS): STATE`. COMMANDS are the commands of the pairs of `table`
/// (as for `mode_for`) that use the mode, in order, each once; STATE is
/// `running` (also while the first line is still coming), `not started` or `off
/// (REASON)`. Then, in `table`'s order, one line for each pair that never takes
/// effect, each once: `command COMMAND: no mode named MODE` when its mode is
/// not defined, `command COMMAND: uses USED, not MODE` when an earlier pair
/// gives the command another mode, and `command "": matches nothing` for an
/// empty COMMAND. A pair that repeats the mode an earlier pair gives its
/// command gets no line. The work grows with the length of `table`, not with
/// its square, so a very long alist does not hold up `inkline status`.
pub fn status_lines(table: &[(String, String)]) -> Vec<String> {
    SERVERS.with_borrow(|servers| {
        let defined: HashSet<&str> = servers.iter().map(|s| s.mode.as_str()).collect();
        // Each COMMAND's first pair whose mode is defined: its place in
        // `table`, and its mode.
        let mut first: HashMap<&str, (usize, &str)> = HashMap::new();
        for (i, (command, mode)) in table.iter().enumerate() {
            if defined.contains(mode.as_str()) {
                first.entry(command).or_insert((i, mode));
            }
        }
        // The mode the command whose name word is `word` uses, as
        // `mode_for` finds it.
        let used = |word: &str| -> Option<&str> {
            let last = word.rsplit_once('/').map(|(_, last)| last);
            [Some(word), last]
                .into_iter()
                .flatten()
                .filter(|command| !command.is_empty())
                .filter_map(|command| first.get(command))
                .min_by_key(|(i, _)| *i)
                .map(|(_, mode)| *mode)
        };
        let mut commands: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut listed = HashSet::new();
        let mut empty_told = false;
        let mut seen = HashSet::new();
        let mut unused = Vec::new();
        for (command, mode) in table {
            if !seen.insert((command, mode)) {
                continue;
            }
            let uses = used(command);
            if let Some(uses) = uses
                && listed.insert(command)
            {
                commands.entry(uses).or_default().push(command);
            }
            if command.is_empty() {
                if !empty_told {
                    unused.push(r#"command "": matches nothing"#.to_owned());
                    empty_told = true;
                }
            } else if !defined.contains(mode.as_str()) {
                unused.push(format!("command {command}: no mode named {mode}"));
            } else if let Some(uses) = uses
                && uses != mode
            {
                unused.push(format!("command {command}: uses {uses}, not {mode}"));
            }
        }
        let mut lines: Vec<String> = servers
            .iter()
            .map(|s| {
                let commands = commands.get(s.mode.as_str()).map(|c| c.join(", "));
                let state = match &s.state {
                    State::NotStarted => "not started".to_owned(),
                    State::Starting(_) | State::Running(_) => "running".to_owned(),
                    State::Off(reason) => format!("off ({reason})"),
                };
                format!(
                    "mode {} ({}): {state}",
                    s.mode,
                    commands.unwrap_or_default()
                )
            })
            .collect();
        lines.extend(unused);
        lines
    })
}

/// Gets the servers of the modes in `modes` going, without waiting: starts
/// those not started yet, looking up programs in `path` (bash's `PATH`) and
/// giving them the environment `environment` returns (see
/// `process::start`), and reads what has come of the first line of those
/// starting. A server this call turns off gets a message for
/// `take_notices`.
pub fn prepare(modes: &[String], path: &str, environment: fn() -> Option<Vec<Vec<u8>>>) {
    SERVERS.with_borrow_mut(|servers| {
        for s in servers.iter_mut().filter(|s| modes.contains(&s.mode)) {
            if let Err(reason) = s.prepare(path, environment) {
                s.turn_off(reason);
            }
        }
    });
}

/// The reply to each of `asks` (a mode and a request), or `None` when it has
/// not come in time.
///
/// A kept reply to the same request answers at once. For the rest, each mode
/// server is sent one request at a time, in the order of `asks`, and replies
/// are read until every ask is answered or `wait` has passed, even while a
/// server keeps writing; a server still starting first has its first line
/// read in that time. A reply that comes for a request no longer asked is
/// kept as that request's reply (the line may come back to it), but is never
/// an answer by itself. Kept replies whose requests this call does not ask
/// for are dropped first. A server that breaks the protocol, writes more
/// than `MOST_UNREAD` bytes without finishing a reply, or exits is turned
/// off, with a message for `take_notices`. A signal ends the wait early;
/// `interrupted` is for `Process::send`.
pub fn replies(
    asks: &[(String, Request)],
    wait: Duration,
    interrupted: fn() -> bool,
) -> Vec<Option<Reply>> {
    let deadline = Instant::now() + wait;
    SERVERS.with_borrow_mut(|servers| {
        let requests: Vec<Vec<&Request>> = servers
            .iter()
            .map(|s| requests_for(asks, &s.mode))
            .collect();
        for (s, requests) in servers.iter_mut().zip(&requests) {
            if let State::Running(running) = &mut s.state {
                running
                    .kept
                    .retain(|(request, _)| requests.contains(&request));
            }
        }
        loop {
            let mut waiting = Vec::new();
            for (s, requests) in servers.iter_mut().zip(&requests) {
                if requests.is_empty() {
                    continue;
                }
                match s.advance(requests, deadline, interrupted) {
                    Ok(fd) => waiting.extend(fd),
                    Err(reason) => s.turn_off(reason),
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
            .map(|(mode, request)| {
                let s = servers.iter().find(|s| s.mode == *mode)?;
                match &s.state {
                    State::Running(running) => running.kept(request).cloned(),
                    State::NotStarted | State::Starting(_) | State::Off(_) => None,
                }
            })
            .collect()
    })
}

/// The sockets of the mode servers that owe inkline something: those still
/// starting, and those with a request in flight. Waiting for a key also
/// waits on them, so that `read_waiting` can take what they send.
pub fn waiting_fds() -> Vec<RawFd> {
    SERVERS.with_borrow(|servers| {
        servers
            .iter()
            .filter_map(|s| match &s.state {
                State::Starting(process) => Some(process.fd()),
                State::Running(running) if running.in_flight.is_some() => {
                    Some(running.process.fd())
                }
                State::NotStarted | State::Running(_) | State::Off(_) => None,
            })
            .collect()
    })
}

/// Reads what the mode servers in `waiting_fds` have sent, without waiting:
/// their first lines, and replies, which are kept for the next redraw to
/// find. A server that breaks the protocol, writes too much or exits is
/// turned off, with a message for `take_notices`. Whether the line must be
/// drawn again: a reply came (the redraw uses it, or sends the request for
/// the line as it is now), a server started running (the redraw sends it
/// its request), or a server was turned off.
pub fn read_waiting() -> bool {
    SERVERS.with_borrow_mut(|servers| {
        let mut changed = false;
        for s in servers.iter_mut() {
            let read = match &mut s.state {
                State::Starting(_) => s
                    .read_first_line()
                    .map(|()| matches!(s.state, State::Running(_))),
                State::Running(running) if running.in_flight.is_some() => running.read_reply(),
                State::NotStarted | State::Running(_) | State::Off(_) => continue,
            };
            match read {
                Ok(read) => changed |= read,
                Err(reason) => {
                    s.turn_off(reason);
                    changed = true;
                }
            }
        }
        changed
    })
}

/// The requests in `asks` for the server of the mode `mode`, in order.
fn requests_for<'a>(asks: &'a [(String, Request)], mode: &str) -> Vec<&'a Request> {
    asks.iter()
        .filter(|(m, _)| m == mode)
        .map(|(_, request)| request)
        .collect()
}

/// The request for `command`'s arguments, with `cwd` as the directory.
pub fn request(cwd: Vec<u8>, command: &CommandArgs) -> Request {
    let args = command
        .args
        .iter()
        .map(|a| (a.raw, a.text.clone()))
        .collect();
    (cwd, args)
}

/// Asks the server of the mode `mode` how deep the new line and the
/// cursor's line are, with the cursor at `at` (an argument's index and a
/// byte offset in its text) in `request`'s arguments. Only a running mode
/// server that named `indent` is asked. The reply to a request already in
/// flight comes first (one request at a time), all within `wait`; a signal
/// for which `interrupted` holds (such as C-c) ends the wait. `None` when no
/// depths came: the server was not asked, gave none in time, or sent only
/// `:end`. A server that fails is turned off, with a message for
/// `take_notices`.
pub fn indent(
    mode: &str,
    request: &Request,
    at: (usize, usize),
    wait: Duration,
    interrupted: fn() -> bool,
) -> Option<Depths> {
    let deadline = Instant::now() + wait;
    SERVERS.with_borrow_mut(|servers| {
        let s = servers.iter_mut().find(|s| s.mode == mode)?;
        let asked = s.read_first_line().and_then(|()| match &mut s.state {
            State::Running(running) if running.indent => {
                running.ask_indent(request, at, deadline, interrupted)
            }
            State::NotStarted | State::Starting(_) | State::Running(_) | State::Off(_) => Ok(None),
        });
        asked.unwrap_or_else(|reason| {
            s.turn_off(reason);
            None
        })
    })
}

/// Whether a mode server was turned off and the message saying so is not
/// taken yet.
pub fn has_notices() -> bool {
    SERVERS.with_borrow(|servers| servers.iter().any(|s| s.notice.is_some()))
}

/// Takes the messages saying mode servers were turned off, in the order
/// their modes were defined.
pub fn take_notices() -> Vec<String> {
    SERVERS.with_borrow_mut(|servers| servers.iter_mut().filter_map(|s| s.notice.take()).collect())
}

impl Server {
    fn new(mode: &str, program: Vec<String>, colors: Option<ColorSet>) -> Server {
        Server {
            mode: mode.to_owned(),
            program,
            colors,
            state: State::NotStarted,
            notice: None,
        }
    }

    /// Starts the mode server if it is not started, as `prepare` says, and
    /// reads what has come of its first line if it is starting. The reason
    /// it must be turned off, if it must.
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

    /// Reads what has come of the first line, if the mode server is
    /// starting, and has it running once the line is all there and good.
    /// The reason it must be turned off, if it must.
    fn read_first_line(&mut self) -> Result<(), String> {
        let State::Starting(process) = &mut self.state else {
            return Ok(());
        };
        if let Some(features) = read_version(process)? {
            let State::Starting(process) = std::mem::replace(&mut self.state, State::NotStarted)
            else {
                unreachable!("the server is starting");
            };
            self.state = State::Running(Running {
                process,
                indent: features.iter().any(|f| f == "indent"),
                last_id: 0,
                in_flight: None,
                kept: Vec::new(),
                reply_seen: 0,
                forgotten: 0,
                indent_answer: None,
            });
        }
        Ok(())
    }

    /// Does what can be done without waiting towards a reply for each of
    /// `requests`, stopping at `deadline` if the mode server keeps writing:
    /// reads the first line or replies that have come, and sends the next
    /// request when none is in flight (see `Process::send` for
    /// `interrupted`). The socket to wait on when some request is still
    /// unanswered and the server owes inkline something, or the reason it
    /// must be turned off.
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

    /// Turns the server off for `reason`, with a message saying so.
    fn turn_off(&mut self, reason: String) {
        self.notice = Some(format!("inkline: mode {}: off ({reason})", self.mode));
        self.state = State::Off(reason);
    }
}

impl Running {
    /// See `Server::advance`.
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
    /// came, or the reason the mode server must be turned off.
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
    /// it: keeps a colour reply, or drops it if it is forgotten; notes an
    /// indent reply in `indent_answer`. Whether one came, or the reason the
    /// mode server must be turned off.
    fn take_reply(&mut self) -> Result<bool, String> {
        let Some((id, asked)) = &self.in_flight else {
            return Ok(false);
        };
        let id = *id;
        let read = match asked {
            Asked::Colors(request) => {
                let lens: Vec<usize> = request.1.iter().map(|(_, text)| text.len()).collect();
                protocol::reply(self.process.buffer(), id, &lens, &mut self.reply_seen).map(Some)
            }
            Asked::Indent => {
                protocol::indent_reply(self.process.buffer(), id, &mut self.reply_seen).map(
                    |depths| {
                        self.indent_answer = depths;
                        None
                    },
                )
            }
        };
        match read {
            Read::Done(reply, used) => {
                self.process.buffer().drain(..used);
                self.reply_seen = 0;
                if let (Some((_, Asked::Colors(request))), Some(reply)) =
                    (self.in_flight.take(), reply)
                    && id > self.forgotten
                {
                    self.keep(request, reply);
                }
                Ok(true)
            }
            Read::Bad(reason) => Err(reason),
            Read::Incomplete => check_unread(&mut self.process).map(|()| false),
        }
    }

    /// See `indent`: the depths, `None` when none came by `deadline`, or
    /// the reason the mode server must be turned off.
    fn ask_indent(
        &mut self,
        request: &Request,
        at: (usize, usize),
        deadline: Instant,
        interrupted: fn() -> bool,
    ) -> Result<Option<Depths>, String> {
        // One request at a time: the reply to one in flight comes first.
        self.take_reply()?;
        while self.in_flight.is_some() {
            if !self.wait_more(deadline, interrupted)? {
                return Ok(None);
            }
            self.take_reply()?;
        }
        // Once the wait is over, or after a C-c, the answer could not be
        // used: nothing is sent.
        if interrupted() || Instant::now() >= deadline {
            return Ok(None);
        }
        let id = self.last_id + 1;
        let bytes = protocol::indent_request(id, &request.0, &request.1, at);
        self.process.send(&bytes, interrupted)?;
        self.last_id = id;
        self.in_flight = Some((id, Asked::Indent));
        loop {
            if self.take_reply()? {
                return Ok(self.indent_answer.take());
            }
            if !self.wait_more(deadline, interrupted)? {
                return Ok(None);
            }
        }
    }

    /// Waits until the mode server writes more, `deadline` passes, or a
    /// signal comes for which `interrupted` holds. Whether more came, or the
    /// reason the server must be turned off.
    fn wait_more(&mut self, deadline: Instant, interrupted: fn() -> bool) -> Result<bool, String> {
        loop {
            if interrupted() || Instant::now() >= deadline {
                return Ok(false);
            }
            let until = deadline.min(Instant::now() + process::SIGNAL_CHECK);
            if self.process.fill(Some(until))? {
                return Ok(true);
            }
        }
    }

    /// Sends `request` (see `Process::send` for `interrupted`).
    fn send(&mut self, request: &Request, interrupted: fn() -> bool) -> Result<(), String> {
        let id = self.last_id + 1;
        self.process
            .send(&protocol::request(id, &request.0, &request.1), interrupted)?;
        self.last_id = id;
        self.in_flight = Some((id, Asked::Colors(request.clone())));
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

/// Reads what has come of the mode server's first line, without waiting. The
/// extra requests the line names once it has all come and is good, `None`
/// while it has not all come, or the reason to turn the server off.
fn read_version(process: &mut Process) -> Result<Option<Vec<String>>, String> {
    loop {
        match protocol::version(process.buffer()) {
            Read::Done(features, used) => {
                process.buffer().drain(..used);
                return Ok(Some(features));
            }
            Read::Bad(reason) => return Err(reason),
            Read::Incomplete => check_unread(process)?,
        }
        if !process.fill(None)? {
            return Ok(None);
        }
    }
}

/// An error when the mode server has written more than `MOST_UNREAD` bytes
/// that do not yet make a whole reply or first line.
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
            format!("{}/tests/data/fake-mode-server", env!("CARGO_MANIFEST_DIR")),
            mode.to_owned(),
        ])
    }

    fn path() -> String {
        std::env::var("PATH").unwrap_or_default()
    }

    /// Prepares the mode `mode` until its server is no longer starting, or
    /// two seconds have passed; the messages it gave.
    fn prepare_until_started(mode: &str) -> Vec<String> {
        let modes = [mode.to_owned()];
        prepare(&modes, &path(), || None);
        let deadline = Instant::now() + Duration::from_secs(2);
        let starting = || {
            SERVERS.with_borrow(|servers| {
                servers
                    .iter()
                    .any(|s| s.mode == mode && matches!(s.state, State::Starting(_)))
            })
        };
        while starting() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            prepare(&modes, &path(), || None);
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

    /// Asks the mode server for `csvm` about `scripts`, starting it first.
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
        define("csvm", fake("words"), None);
        let got = ask(&["a 1"], Duration::from_secs(2));
        assert_eq!(got.len(), 1);
        assert_eq!(kinds(&got[0]), [Command, Number]);
        assert_eq!(ask(&["a 1"], Duration::ZERO), got, "kept, not sent again");
        assert!(take_notices().is_empty());
    }

    #[test]
    fn several_asks_of_one_server_are_all_answered() {
        use crate::lexer::Kind::{Command, Number, Variable};
        define("csvm", fake("words"), None);
        let got = ask(&["a", "b c", "1"], Duration::from_secs(2));
        assert_eq!(kinds(&got[0]), [Command]);
        assert_eq!(kinds(&got[1]), [Command, Variable]);
        assert_eq!(kinds(&got[2]), [Number]);
    }

    #[test]
    fn a_late_reply_is_not_a_failure_and_is_kept_for_its_request() {
        define("csvm", fake("late"), None);
        assert!(prepare_until_started("csvm").is_empty());
        assert_eq!(ask(&["a"], Duration::from_millis(20)), [None]);
        assert!(take_notices().is_empty());
        assert_eq!(status_lines(&[]), ["mode csvm (): running"]);
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
        define("csvm", fake("late"), None);
        assert!(prepare_until_started("csvm").is_empty());
        assert!(waiting_fds().is_empty(), "nothing asked yet");
        assert_eq!(ask(&["a"], Duration::from_millis(20)), [None]);
        assert!(!read_waiting(), "nothing has come yet");
        let answered = || {
            SERVERS.with_borrow(
                |servers| matches!(&servers[0].state, State::Running(r) if !r.kept.is_empty()),
            )
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
        define("csvm", fake("late"), None);
        assert!(prepare_until_started("csvm").is_empty());
        assert_eq!(ask(&["a"], Duration::ZERO), [None]);
        forget_replies();
        // Its request cannot go out while the forgotten one is in flight.
        assert_eq!(ask(&["a"], Duration::ZERO), [None]);
        let idle = || waiting_fds().is_empty();
        assert!(read_until(idle), "the dropped reply asks for a redraw");
        let kept = || {
            SERVERS.with_borrow(
                |servers| matches!(&servers[0].state, State::Running(r) if !r.kept.is_empty()),
            )
        };
        assert!(!kept(), "the forgotten reply is not kept");
        assert!(ask(&["a"], Duration::from_secs(2))[0].is_some());
    }

    #[test]
    fn a_server_still_starting_is_waited_on() {
        // It prints its first line after 0.3 s.
        define("csvm", fake("slow"), None);
        prepare(&["csvm".to_owned()], &path(), || None);
        assert_eq!(waiting_fds().len(), 1, "waited on while starting");
        assert!(!read_waiting(), "its first line has not come");
        let running =
            || SERVERS.with_borrow(|servers| matches!(servers[0].state, State::Running(_)));
        assert!(read_until(running), "moving to running asks for a redraw");
        assert!(running());
        assert!(waiting_fds().is_empty(), "nothing in flight");
        assert!(take_notices().is_empty());
    }

    #[test]
    fn a_server_that_exits_between_redraws_is_off() {
        define("csvm", fake("exit"), None);
        assert!(prepare_until_started("csvm").is_empty());
        // It exits once it has read the request this sends.
        assert_eq!(ask(&["a"], Duration::ZERO), [None]);
        let off = || SERVERS.with_borrow(|servers| matches!(servers[0].state, State::Off(_)));
        assert!(read_until(off));
        assert_eq!(take_notices(), ["inkline: mode csvm: off (exited)"]);
        assert!(waiting_fds().is_empty());
    }

    #[test]
    fn a_server_that_exits_is_off() {
        define("csvm", fake("exit"), None);
        assert_eq!(ask(&["a"], Duration::from_secs(2)), [None]);
        assert_eq!(take_notices(), ["inkline: mode csvm: off (exited)"]);
        assert_eq!(status_lines(&[]), ["mode csvm (): off (exited)"]);
        assert_eq!(ask(&["a"], Duration::from_secs(2)), [None]);
        assert!(take_notices().is_empty());
    }

    #[test]
    fn a_reply_that_breaks_the_protocol_is_off() {
        define("csvm", fake("garbage"), None);
        assert_eq!(ask(&["a"], Duration::from_secs(2)), [None]);
        assert_eq!(
            take_notices(),
            ["inkline: mode csvm: off (bad reply: \"nonsense\")"]
        );
    }

    /// Reads never block, and the wait ends at its deadline. The fake writes
    /// more slowly than inkline reads; a mode server that writes faster is
    /// turned off at `MOST_UNREAD` (see
    /// `a_server_that_writes_too_much_is_off`).
    #[test]
    fn a_server_that_keeps_writing_does_not_hold_up_a_redraw() {
        define("csvm", fake("spew"), None);
        assert!(prepare_until_started("csvm").is_empty());
        let began = Instant::now();
        assert_eq!(ask(&["a"], Duration::from_millis(15)), [None]);
        let took = began.elapsed();
        assert!(took < Duration::from_millis(100), "took {took:?}");
    }

    #[test]
    fn a_server_that_writes_too_much_is_off() {
        define("csvm", fake("spew"), None);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !has_notices() && Instant::now() < deadline {
            ask(&["a"], Duration::from_millis(15));
        }
        assert_eq!(
            take_notices(),
            ["inkline: mode csvm: off (bad reply: too much output)"]
        );
    }

    #[test]
    fn defining_replacing_and_removing() {
        define("a", Some(vec!["x".to_owned()]), None);
        define("b", Some(vec!["y".to_owned()]), None);
        define("a", Some(vec!["z".to_owned()]), None);
        assert!(is_defined("a") && is_defined("b") && !is_defined("c"));
        assert_eq!(
            status_lines(&[]),
            ["mode a (): not started", "mode b (): not started"]
        );
        define("a", None, None);
        define("c", None, None);
        assert_eq!(status_lines(&[]), ["mode b (): not started"]);
        stop_all();
        assert!(!any_defined());
    }

    #[test]
    fn a_server_starts_and_names_itself() {
        define("csvm", fake("words"), None);
        assert!(prepare_until_started("csvm").is_empty());
        assert_eq!(status_lines(&[]), ["mode csvm (): running"]);
        let running = SERVERS.with_borrow(|servers| matches!(servers[0].state, State::Running(_)));
        assert!(running);
    }

    #[test]
    fn only_the_names_asked_for_start() {
        define("csvm", fake("words"), None);
        define("other", fake("words"), None);
        prepare(&["csvm".to_owned()], &path(), || None);
        assert_eq!(
            status_lines(&[]),
            ["mode csvm (): running", "mode other (): not started"]
        );
    }

    #[test]
    fn a_missing_program_is_off_once() {
        define("csvm", Some(vec!["no-such-program-xyz".to_owned()]), None);
        prepare(&["csvm".to_owned()], &path(), || None);
        assert!(has_notices());
        assert_eq!(take_notices(), ["inkline: mode csvm: off (not found)"]);
        assert!(!has_notices());
        prepare(&["csvm".to_owned()], &path(), || None);
        assert!(take_notices().is_empty());
        assert_eq!(status_lines(&[]), ["mode csvm (): off (not found)"]);
    }

    #[test]
    fn a_wrong_first_line_is_off() {
        define("csvm", fake("version"), None);
        assert_eq!(
            prepare_until_started("csvm"),
            ["inkline: mode csvm: off (not a mode server)"]
        );
        assert_eq!(status_lines(&[]), ["mode csvm (): off (not a mode server)"]);
    }

    #[test]
    fn defining_again_turns_an_off_server_back_on() {
        define("csvm", Some(vec!["no-such-program-xyz".to_owned()]), None);
        prepare(&["csvm".to_owned()], &path(), || None);
        define("csvm", fake("words"), None);
        assert_eq!(status_lines(&[]), ["mode csvm (): not started"]);
        assert!(!has_notices(), "the old message is dropped");
    }

    #[test]
    fn removing_or_stopping_drops_the_messages_not_taken() {
        let missing = || Some(vec!["no-such-program-xyz".to_owned()]);
        define("a", missing(), None);
        define("b", missing(), None);
        prepare(&["a".to_owned(), "b".to_owned()], &path(), || None);
        define("a", None, None);
        assert_eq!(take_notices(), ["inkline: mode b: off (not found)"]);
        define("a", missing(), None);
        prepare(&["a".to_owned()], &path(), || None);
        stop_all();
        assert!(!has_notices());
    }

    /// Asks the mode server for `csvm` for the depths at `(1, at)` in `script`.
    fn depths(
        script: &str,
        at: usize,
        wait: Duration,
        interrupted: fn() -> bool,
    ) -> Option<Depths> {
        indent("csvm", &request(script), (1, at), wait, interrupted)
    }

    /// A mode server that names `indent`, then runs the shell code `then`
    /// once it has read a line.
    fn indenting_sh(then: &str) -> Option<Vec<String>> {
        Some(vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            format!("echo 'inkline-mode 1 indent'; read x; {then}"),
        ])
    }

    #[test]
    fn depths_come_from_a_server_that_names_indent() {
        define("csvm", fake("indent"), None);
        assert!(prepare_until_started("csvm").is_empty());
        let wait = Duration::from_secs(2);
        assert_eq!(
            depths("fn f {\n  a", 10, wait, || false),
            Some(Depths { new: 1, current: 1 })
        );
        assert_eq!(
            depths("fn f {\n  }", 10, wait, || false),
            Some(Depths { new: 0, current: 0 })
        );
        assert_eq!(
            depths("a {\n  b }", 8, wait, || false),
            Some(Depths { new: 0, current: 1 }),
            "the `}}` after the cursor goes to the new line"
        );
        assert!(waiting_fds().is_empty(), "nothing in flight");
        assert!(take_notices().is_empty());
    }

    #[test]
    fn a_server_without_indent_is_not_asked() {
        define("csvm", fake("words"), None);
        assert!(prepare_until_started("csvm").is_empty());
        let began = Instant::now();
        assert_eq!(depths("a {", 3, Duration::from_secs(2), || false), None);
        assert!(began.elapsed() < Duration::from_millis(50));
        assert!(waiting_fds().is_empty(), "no request was sent");
    }

    #[test]
    fn a_server_that_is_not_running_is_not_asked() {
        define("csvm", fake("indent"), None);
        assert_eq!(depths("a {", 3, Duration::from_secs(2), || false), None);
        assert_eq!(status_lines(&[]), ["mode csvm (): not started"]);
    }

    #[test]
    fn a_reply_with_only_end_gives_no_depths() {
        define("csvm", fake("indent"), None);
        assert!(prepare_until_started("csvm").is_empty());
        assert_eq!(
            depths("nodepth {", 9, Duration::from_secs(2), || false),
            None
        );
        assert!(take_notices().is_empty());
        assert_eq!(status_lines(&[]), ["mode csvm (): running"]);
    }

    /// A colour request in flight is answered first, and its reply kept.
    #[test]
    fn a_colour_reply_in_flight_comes_first() {
        define("csvm", fake("indent"), None);
        assert!(prepare_until_started("csvm").is_empty());
        ask(&["a 1"], Duration::ZERO);
        assert_eq!(
            depths("a {", 3, Duration::from_secs(2), || false),
            Some(Depths { new: 1, current: 0 })
        );
        assert!(
            ask(&["a 1"], Duration::ZERO)[0].is_some(),
            "the colour reply was kept"
        );
    }

    /// Depths that come too late are read and dropped, not taken for
    /// colours, and do not turn the mode server off.
    #[test]
    fn a_late_indent_reply_is_dropped() {
        define("csvm", fake("slow-indent"), None);
        assert!(prepare_until_started("csvm").is_empty());
        let began = Instant::now();
        assert_eq!(depths("a {", 3, INDENT_WAIT, || false), None);
        assert!(began.elapsed() < Duration::from_millis(300));
        let got = ask(&["b"], Duration::from_secs(3));
        assert!(got[0].is_some(), "colours after the late depths");
        assert!(take_notices().is_empty());
        assert_eq!(status_lines(&[]), ["mode csvm (): running"]);
    }

    #[test]
    fn an_interrupt_ends_the_indent_wait() {
        define("csvm", fake("slow-indent"), None);
        assert!(prepare_until_started("csvm").is_empty());
        let began = Instant::now();
        assert_eq!(depths("a {", 3, Duration::from_secs(2), || true), None);
        assert!(began.elapsed() < Duration::from_millis(100));
        assert!(take_notices().is_empty());
        assert!(!asked_anything("csvm"), "nothing is sent after a C-c");
    }

    /// Set by `c_c_during_the_indent_wait`, for its `interrupted`.
    static C_C: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// A C-c that comes while inkline waits for the depths ends the wait.
    #[test]
    fn c_c_during_the_indent_wait() {
        use std::sync::atomic::Ordering::Relaxed;
        define("csvm", fake("slow-indent"), None);
        assert!(prepare_until_started("csvm").is_empty());
        let c_c = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            C_C.store(true, Relaxed);
        });
        let began = Instant::now();
        let got = depths("a {", 3, Duration::from_secs(2), || C_C.load(Relaxed));
        let took = began.elapsed();
        c_c.join().unwrap();
        assert_eq!(got, None);
        assert!(asked_anything("csvm"), "the C-c came during the wait");
        assert!(took < Duration::from_millis(300), "took {took:?}");
        assert!(take_notices().is_empty());
        assert_eq!(status_lines(&[]), ["mode csvm (): running"]);
    }

    /// Whether a request to the server of the mode `mode` is in flight.
    fn asked_anything(mode: &str) -> bool {
        SERVERS.with_borrow(|servers| {
            servers.iter().any(|s| {
                s.mode == mode && matches!(&s.state, State::Running(r) if r.in_flight.is_some())
            })
        })
    }

    /// Once the wait is over, the indent request is not sent: its answer
    /// could not be used.
    #[test]
    fn nothing_is_sent_after_the_indent_wait() {
        define("csvm", fake("indent"), None);
        assert!(prepare_until_started("csvm").is_empty());
        assert_eq!(depths("a {", 3, Duration::ZERO, || false), None);
        assert!(!asked_anything("csvm"));
        assert!(take_notices().is_empty());
    }

    #[test]
    fn a_broken_depth_line_turns_the_server_off() {
        define(
            "csvm",
            indenting_sh("printf ':depth x 1\\n:end 1\\n'; cat >/dev/null"),
            None,
        );
        assert!(prepare_until_started("csvm").is_empty());
        assert_eq!(depths("a {", 3, Duration::from_secs(2), || false), None);
        assert_eq!(
            take_notices(),
            ["inkline: mode csvm: off (bad reply: \":depth x 1\")"]
        );
    }

    #[test]
    fn a_server_that_exits_while_asked_for_depths_is_off() {
        define("csvm", indenting_sh("exit 0"), None);
        assert!(prepare_until_started("csvm").is_empty());
        assert_eq!(depths("a {", 3, Duration::from_secs(2), || false), None);
        assert_eq!(take_notices(), ["inkline: mode csvm: off (exited)"]);
    }

    /// A mode server that writes lines without end, and never its depths,
    /// does not hold up the new line past the wait.
    #[test]
    fn a_server_that_keeps_writing_does_not_hold_up_the_indent_wait() {
        define("csvm", indenting_sh("while :; do echo :x; done"), None);
        assert!(prepare_until_started("csvm").is_empty());
        let began = Instant::now();
        assert_eq!(depths("a {", 3, INDENT_WAIT, || false), None);
        let took = began.elapsed();
        assert!(took < Duration::from_millis(200), "took {took:?}");
    }

    fn set(command: &str) -> ColorSet {
        ColorSet::from_entries(&[("command".to_owned(), command.to_owned())]).unwrap()
    }

    #[test]
    fn the_same_program_again_keeps_the_server_and_changes_its_colours() {
        define("csvm", fake("words"), Some(set("35")));
        assert!(prepare_until_started("csvm").is_empty());
        define("csvm", fake("words"), Some(set("36")));
        assert_eq!(status_lines(&[]), ["mode csvm (): running"]);
        assert_eq!(colors("csvm"), Some(set("36")));
        define("csvm", fake("words"), None);
        assert_eq!(status_lines(&[]), ["mode csvm (): running"]);
        assert_eq!(colors("csvm"), None);
        define("csvm", fake("late"), None);
        assert_eq!(status_lines(&[]), ["mode csvm (): not started"]);
    }

    #[test]
    fn the_same_program_again_turns_an_off_server_back_on() {
        let missing = || Some(vec!["no-such-program-xyz".to_owned()]);
        define("csvm", missing(), None);
        prepare(&["csvm".to_owned()], &path(), || None);
        assert_eq!(status_lines(&[]), ["mode csvm (): off (not found)"]);
        define("csvm", missing(), None);
        assert_eq!(status_lines(&[]), ["mode csvm (): not started"]);
        assert!(!has_notices(), "the old message is dropped");
    }

    fn table(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(c, m)| ((*c).to_owned(), (*m).to_owned()))
            .collect()
    }

    fn some_program() -> Option<Vec<String>> {
        Some(vec!["x".to_owned()])
    }

    #[test]
    fn a_command_finds_its_mode_by_its_word_or_last_path_part() {
        define("csvm-mode", some_program(), None);
        define("path-mode", some_program(), None);
        let t = table(&[
            ("./target/debug/csvm", "path-mode"),
            ("csvm", "csvm-mode"),
            ("c", "csvm-mode"),
        ]);
        let mode = |word: &str| mode_for(word, &t);
        assert_eq!(mode("csvm").as_deref(), Some("csvm-mode"));
        assert_eq!(mode("c").as_deref(), Some("csvm-mode"));
        assert_eq!(mode("/usr/local/bin/csvm").as_deref(), Some("csvm-mode"));
        assert_eq!(
            mode("./target/debug/csvm").as_deref(),
            Some("path-mode"),
            "the first pair that names the word wins"
        );
        assert_eq!(
            mode("target/debug/csvm").as_deref(),
            Some("csvm-mode"),
            "a pair with a path names only that exact word"
        );
        for word in ["csvmx", "xcsvm", "csvm/", "dir/", "", "C"] {
            assert_eq!(mode(word), None, "{word:?}");
        }
    }

    #[test]
    fn a_pair_with_an_undefined_mode_is_skipped() {
        define("b-mode", some_program(), None);
        define("a-mode", some_program(), None);
        let t = table(&[
            ("csvm", "no-such-mode"),
            ("csvm", "b-mode"),
            ("csvm", "a-mode"),
        ]);
        assert_eq!(mode_for("csvm", &t).as_deref(), Some("b-mode"));
        define("b-mode", None, None);
        assert_eq!(mode_for("csvm", &t).as_deref(), Some("a-mode"));
        assert_eq!(mode_for("csvm", &[]), None);
    }

    #[test]
    fn an_empty_command_names_nothing() {
        define("a-mode", some_program(), None);
        let t = table(&[("", "a-mode")]);
        assert_eq!(mode_for("", &t), None);
        assert_eq!(mode_for("dir/", &t), None);
    }

    #[test]
    fn status_names_the_commands_of_each_mode() {
        define("csvm-mode", some_program(), None);
        define(
            "other-mode",
            Some(vec!["no-such-program-xyz".to_owned()]),
            None,
        );
        prepare(&["other-mode".to_owned()], &path(), || None);
        take_notices();
        let t = table(&[
            ("csvm", "csvm-mode"),
            ("c", "cvsm-mode"),
            ("c", "csvm-mode"),
            ("csvm", "csvm-mode"),
            ("c", "cvsm-mode"),
        ]);
        assert_eq!(
            status_lines(&t),
            [
                "mode csvm-mode (csvm, c): not started",
                "mode other-mode (): off (not found)",
                "command c: no mode named cvsm-mode",
            ]
        );
    }

    #[test]
    fn status_lists_each_command_under_the_mode_it_uses() {
        define("a", some_program(), None);
        define("b", some_program(), None);
        let t = table(&[
            ("c", "a"),
            ("c", "b"),
            ("csvm", "a"),
            ("/usr/bin/csvm", "b"),
            ("x", "b"),
        ]);
        assert_eq!(mode_for("/usr/bin/csvm", &t).as_deref(), Some("a"));
        assert_eq!(
            status_lines(&t),
            [
                "mode a (c, csvm, /usr/bin/csvm): not started",
                "mode b (x): not started",
                "command c: uses a, not b",
                "command /usr/bin/csvm: uses a, not b",
            ]
        );
        // A pair for the whole path before the one for its last part, and
        // a path that has no pair for its last part.
        let t = table(&[
            ("/usr/bin/csvm", "b"),
            ("csvm", "a"),
            ("./target/debug/csvm", "b"),
            ("/opt/x", "a"),
        ]);
        assert_eq!(
            status_lines(&t),
            [
                "mode a (csvm, ./target/debug/csvm, /opt/x): not started",
                "mode b (/usr/bin/csvm): not started",
                "command ./target/debug/csvm: uses a, not b",
            ]
        );
    }

    #[test]
    fn status_says_an_empty_command_matches_nothing() {
        define("a", some_program(), None);
        let t = table(&[("", "a"), ("", "no-such-mode"), ("c", "a")]);
        assert_eq!(
            status_lines(&t),
            ["mode a (c): not started", r#"command "": matches nothing"#]
        );
    }

    /// Many different pairs whose mode is not defined, and many whose
    /// command the earlier pair for `csvm` already gives another mode: each
    /// is found without a search through the ones before.
    #[test]
    fn status_of_a_long_alist_comes_quickly() {
        define("csvm-mode", some_program(), None);
        define("other-mode", some_program(), None);
        let t: Vec<(String, String)> = (0..100_000)
            .map(|i| (format!("c{i}"), "no-such-mode".to_owned()))
            .chain([("csvm".to_owned(), "csvm-mode".to_owned())])
            .chain((0..100_000).map(|i| (format!("d{i}/csvm"), "other-mode".to_owned())))
            .collect();
        let began = Instant::now();
        let lines = status_lines(&t);
        let took = began.elapsed();
        assert_eq!(lines.len(), 200_002);
        assert!(
            lines[0].starts_with("mode csvm-mode (csvm, d0/csvm, d1/csvm, "),
            "{}",
            &lines[0][..80]
        );
        assert_eq!(lines[1], "mode other-mode (): not started");
        assert_eq!(lines[2], "command c0: no mode named no-such-mode");
        assert_eq!(
            lines[200_001],
            "command d99999/csvm: uses csvm-mode, not other-mode"
        );
        assert!(took < Duration::from_secs(2), "took {took:?}");
    }

    /// A request for `NAME SCRIPT`.
    fn request_named(name: &str, script: &str) -> Request {
        (
            b"/".to_vec(),
            vec![(false, name.to_owned()), (false, script.to_owned())],
        )
    }

    #[test]
    fn commands_that_use_one_mode_share_its_server() {
        use crate::lexer::Kind::{Command, Number};
        define("csvm-mode", fake("words"), None);
        prepare(&["csvm-mode".to_owned()], &path(), || None);
        let asks = [
            ("csvm-mode".to_owned(), request_named("csvm", "a 1")),
            ("csvm-mode".to_owned(), request_named("c", "a 1")),
        ];
        let got = replies(&asks, Duration::from_secs(2), || false);
        assert_eq!(kinds(&got[0]), [Command, Number]);
        assert_eq!(kinds(&got[1]), [Command, Number]);
        let sent = SERVERS.with_borrow(|s| {
            let State::Running(r) = &s[0].state else {
                panic!("the server is not running");
            };
            r.last_id
        });
        assert_eq!(sent, 2, "one process answered both commands");
        assert!(take_notices().is_empty());
    }

    #[test]
    fn a_server_turned_off_is_off_for_every_command_of_its_mode() {
        define("csvm-mode", fake("exit"), None);
        let asks = [
            ("csvm-mode".to_owned(), request_named("csvm", "a")),
            ("csvm-mode".to_owned(), request_named("c", "b")),
        ];
        prepare(&["csvm-mode".to_owned()], &path(), || None);
        assert_eq!(
            replies(&asks, Duration::from_secs(2), || false),
            [None, None]
        );
        assert_eq!(take_notices(), ["inkline: mode csvm-mode: off (exited)"]);
        assert_eq!(
            replies(&asks, Duration::from_secs(2), || false),
            [None, None]
        );
        assert!(take_notices().is_empty());
    }
}
