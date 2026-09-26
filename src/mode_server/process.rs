//! A highlight helper's process, started apart from bash, and inkline's end of
//! the one socket that is the helper's stdin and stdout.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The longest `send` waits for the helper to take a request.
const SEND_WAIT: Duration = Duration::from_secs(1);

/// How often a wait looks for a signal to act on: one that came just before
/// a wait began does not end it.
pub const SIGNAL_CHECK: Duration = Duration::from_millis(20);

/// The most one read takes from the helper, so a helper that writes without
/// end cannot keep a read going.
const MOST_PER_READ: usize = 64 << 10;

/// inkline's end of a socket goes on the highest free descriptor below
/// this, or below the open-file limit when that is lower, as bash places its
/// own long-lived descriptors. Users redirect low descriptors (`exec
/// 3>file`), and bash takes any close-on-exec descriptor from 10 up for one
/// of its own: it puts it back after a redirection such as `exec 10>file`,
/// which then fails.
const HIGH_FDS_END: RawFd = 256;

/// The lowest descriptor inkline's end of a socket may have.
const LOWEST_FD: RawFd = 10;

/// Why a helper whose descriptor no longer holds its socket is turned off.
const LOST: &str = "connection lost";

/// A started helper. Dropping it closes inkline's end of the socket: the
/// helper sees the end of its input and exits. A subshell bash forked while
/// the helper ran (such as `while :; do sleep 100; done &`) holds a copy of
/// that end, as close-on-exec only closes it for programs bash runs, so the
/// helper then sees the end of its input only once that subshell ends too.
pub struct Process {
    /// inkline's end of the socket, until it is given up (see `socket`):
    /// on a high descriptor (see `HIGH_FDS_END`), close-on-exec and
    /// non-blocking, so commands bash runs do not inherit it and a stuck
    /// helper never blocks inkline.
    socket: Option<OwnedFd>,
    /// The socket's device and inode, which tell whether its descriptor
    /// still holds it.
    id: FileId,
    /// What the helper wrote and the caller has not taken yet.
    buf: Vec<u8>,
}

/// A file's device and inode.
type FileId = (libc::dev_t, libc::ino_t);

/// Starts `program` (a program, then its arguments) with a socket as its
/// stdin and stdout and `/dev/null` as its stderr. A program named without a
/// `/` is looked up in `path`, bash's `PATH`. The helper's environment is
/// `environment`, `NAME=VALUE` entries (bash's exported variables and
/// functions), or with `None` this process's own. It starts in `/`, so that
/// it keeps no directory busy (requests give it the shell's). The helper is
/// not bash's child: a middle process forks it and exits at once. It has a
/// session of its own, so C-c at the prompt does not reach it. Errors are
/// `not found` and `cannot run: ERROR`.
pub fn start(
    program: &[String],
    path: &str,
    environment: Option<Vec<Vec<u8>>>,
) -> Result<Process, String> {
    let cannot_run = |e: std::io::Error| format!("cannot run: {e}");
    let Some((name, args)) = program.split_first() else {
        return Err("cannot run: no program".to_owned());
    };
    let file = if name.contains('/') {
        PathBuf::from(name)
    } else {
        crate::commands::find_program(name, path).ok_or_else(|| "not found".to_owned())?
    };
    // The helper starts in `/`, so a relative path is made whole from the
    // shell's directory first.
    let file = std::path::absolute(file).map_err(cannot_run)?;
    let (ours, theirs) = UnixStream::pair().map_err(cannot_run)?;
    ours.set_nonblocking(true).map_err(cannot_run)?;
    let ours = to_high_fd(OwnedFd::from(ours)).map_err(cannot_run)?;
    let id =
        file_id(ours.as_raw_fd()).ok_or_else(|| cannot_run(std::io::Error::last_os_error()))?;
    let out = theirs.try_clone().map_err(cannot_run)?;
    // Bash's SIGCHLD handler reaps any child of bash that exits, at any
    // moment. The middle process exits at once, but when the program cannot
    // start, std waits for the middle process itself and panics if it is
    // already gone. So SIGCHLD waits until the middle process is reaped here.
    let blocked = BlockChild::new();
    let mask = blocked.old;
    let fd_limit = open_file_limit();
    let mut command = Command::new(file);
    command
        .arg0(name)
        .args(args)
        .current_dir("/")
        .stdin(Stdio::from(OwnedFd::from(theirs)))
        .stdout(Stdio::from(OwnedFd::from(out)))
        .stderr(Stdio::null());
    if let Some(entries) = environment {
        command.env_clear();
        for entry in &entries {
            if let Some(eq) = entry.iter().position(|&b| b == b'=')
                && eq > 0
            {
                command.env(
                    OsStr::from_bytes(&entry[..eq]),
                    OsStr::from_bytes(&entry[eq + 1..]),
                );
            }
        }
    }
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(move || {
            // A session of its own first: until then a key such as C-z
            // reaches this process too, as it is in bash's process group.
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The helper starts as bash starts its own commands: with the
            // signal mask bash had and the signals interactive bash ignores
            // back to their defaults.
            libc::pthread_sigmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
            for signal in [
                libc::SIGINT,
                libc::SIGQUIT,
                libc::SIGTERM,
                libc::SIGTSTP,
                libc::SIGTTIN,
                libc::SIGTTOU,
            ] {
                libc::signal(signal, libc::SIG_DFL);
            }
            match libc::fork() {
                -1 => Err(std::io::Error::last_os_error()),
                0 => {
                    close_on_exec_from_3(fd_limit);
                    Ok(())
                }
                _ => libc::_exit(0),
            }
        });
    }
    let mut middle = command.spawn().map_err(|e| match e.kind() {
        ErrorKind::NotFound => "not found".to_owned(),
        _ => cannot_run(e),
    })?;
    let _ = middle.wait();
    drop(blocked);
    Ok(Process {
        socket: Some(ours),
        id,
        buf: Vec::new(),
    })
}

/// A close-on-exec copy of `fd`, in its place, on the highest free
/// descriptor below `HIGH_FDS_END` and the open-file limit; when none from
/// `LOWEST_FD` up is free there, on the lowest free one from `LOWEST_FD` up.
fn to_high_fd(fd: OwnedFd) -> std::io::Result<OwnedFd> {
    let end = open_file_limit().min(HIGH_FDS_END);
    let copy_from = |lowest: RawFd| {
        // `F_DUPFD_CLOEXEC` takes the lowest free descriptor from `lowest`
        // up, so it never replaces one that is open.
        // SAFETY: it makes a new descriptor, owned by nothing yet.
        let high = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, lowest) };
        // SAFETY: `high`, when not -1, is open and nothing else owns it.
        (high >= 0).then(|| unsafe { OwnedFd::from_raw_fd(high) })
    };
    for candidate in (LOWEST_FD..end).rev() {
        // Another thread can take `candidate` first; a copy at `end` or
        // above is then closed again, and the search goes on.
        // SAFETY: `F_GETFD` only reads the descriptor's flags.
        if unsafe { libc::fcntl(candidate, libc::F_GETFD) } == -1
            && let Some(high) = copy_from(candidate)
            && high.as_raw_fd() < end
        {
            return Ok(high);
        }
    }
    copy_from(LOWEST_FD).ok_or_else(std::io::Error::last_os_error)
}

/// The device and inode of the file open on `fd`, or `None` when `fd` is
/// not open.
fn file_id(fd: RawFd) -> Option<FileId> {
    // SAFETY: `fstat` fills `stat`, which is read only when it succeeds.
    unsafe {
        let mut stat: libc::stat = std::mem::zeroed();
        (libc::fstat(fd, &mut stat) == 0).then_some((stat.st_dev, stat.st_ino))
    }
}

/// A number above every open descriptor: the soft limit on open files, at
/// most `1 << 20`, so that marking descriptors one at a time up to it stays
/// quick when there is no limit.
fn open_file_limit() -> libc::c_int {
    const MOST: libc::c_int = 1 << 20;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` fills `limit`, which is read only when it succeeds.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return MOST;
    }
    libc::c_int::try_from(limit.rlim_cur).map_or(MOST, |cur| cur.min(MOST))
}

/// Marks every descriptor from 3 up close-on-exec, so that the program
/// about to run holds none of bash's descriptors (a pipe bash closes later
/// must not stay open in the helper). `limit` is above every open one.
/// Only async-signal-safe calls: it runs between fork and exec. std has
/// already put the helper's stdin, stdout and stderr on 0 to 2, and its
/// pipe that reports a failed exec is close-on-exec already.
fn close_on_exec_from_3(limit: libc::c_int) {
    // SAFETY: `close_range` and `fcntl` change only descriptor flags.
    unsafe {
        let all = libc::syscall(
            libc::SYS_close_range,
            3 as libc::c_uint,
            libc::c_uint::MAX,
            libc::CLOSE_RANGE_CLOEXEC,
        );
        if all == 0 {
            return;
        }
        // Linux before 5.11 has no `CLOSE_RANGE_CLOEXEC`.
        for fd in 3..limit {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

/// SIGCHLD blocked in this thread until it is dropped.
struct BlockChild {
    /// The signal mask before.
    old: libc::sigset_t,
}

impl BlockChild {
    fn new() -> BlockChild {
        // SAFETY: the sets are initialised by `sigemptyset` and
        // `pthread_sigmask` before they are read.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            let mut old: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGCHLD);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old);
            BlockChild { old }
        }
    }
}

impl Drop for BlockChild {
    fn drop(&mut self) {
        // SAFETY: `old` is the mask `pthread_sigmask` returned.
        unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &self.old, std::ptr::null_mut());
        }
    }
}

impl Process {
    /// Writes all of `bytes`, waiting up to a second for the helper to make
    /// room, or until a signal arrives for which `interrupted` holds (one
    /// that must be acted on at once, such as C-c). A helper that has gone is
    /// `exited`, and never raises SIGPIPE; one that takes too long is `bad
    /// reply: request not read`; one whose descriptor no longer holds its
    /// socket is `connection lost`.
    pub fn send(&mut self, bytes: &[u8], interrupted: fn() -> bool) -> Result<(), String> {
        let fd = self.socket()?;
        let deadline = Instant::now() + SEND_WAIT;
        let mut rest = bytes;
        while !rest.is_empty() {
            // `MSG_NOSIGNAL`: writing to a helper that has gone returns
            // `EPIPE` instead of raising SIGPIPE, which would kill bash. std's
            // own `write` on a `UnixStream` sets it only since Rust 1.90, and
            // its `write_vectored` never does, so it is set here.
            // `MSG_DONTWAIT`: the write never blocks, whatever the flags of
            // the descriptor.
            // SAFETY: `rest` is a live slice of `rest.len()` bytes.
            let sent = unsafe {
                libc::send(
                    fd,
                    rest.as_ptr().cast(),
                    rest.len(),
                    libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT,
                )
            };
            if let Ok(sent) = usize::try_from(sent) {
                rest = &rest[sent..];
                continue;
            }
            match std::io::Error::last_os_error().kind() {
                ErrorKind::Interrupted | ErrorKind::WouldBlock => loop {
                    if Instant::now() >= deadline || interrupted() {
                        return Err("bad reply: request not read".to_owned());
                    }
                    let until = deadline.min(Instant::now() + SIGNAL_CHECK);
                    if ready(fd, libc::POLLOUT, until) {
                        break;
                    }
                },
                _ => return Err("exited".to_owned()),
            }
        }
        Ok(())
    }

    /// Reads what the helper has written into the buffer. With `until`, when
    /// nothing is there yet, waits until then for something to come, and
    /// returns as soon as it does. Whether it added anything to the buffer;
    /// `Err("exited")` once the helper's output has ended and all of it is
    /// in the buffer; `Err("connection lost")` once its descriptor no longer
    /// holds its socket.
    pub fn fill(&mut self, until: Option<Instant>) -> Result<bool, String> {
        let mut waited = false;
        loop {
            let got = self.read_now()?;
            if got || waited {
                return Ok(got);
            }
            match until {
                Some(deadline) if ready(self.fd(), libc::POLLIN, deadline) => waited = true,
                _ => return Ok(false),
            }
        }
    }

    /// Reads what the helper has written so far, up to `MOST_PER_READ`
    /// bytes, without waiting. Whether it added anything to the buffer;
    /// `Err("exited")` at the end of the helper's output when it added
    /// nothing; `Err("connection lost")` as `socket` says.
    fn read_now(&mut self) -> Result<bool, String> {
        let fd = self.socket()?;
        let mut chunk = [0u8; 4096];
        let mut got = false;
        let most = self.buf.len() + MOST_PER_READ;
        while self.buf.len() < most {
            // `MSG_DONTWAIT`: the read never blocks, whatever the flags of
            // the descriptor.
            // SAFETY: `chunk` is a live buffer of `chunk.len()` bytes.
            let read = unsafe {
                libc::recv(
                    fd,
                    chunk.as_mut_ptr().cast(),
                    chunk.len(),
                    libc::MSG_DONTWAIT,
                )
            };
            match usize::try_from(read) {
                Ok(0) if got => return Ok(true),
                Ok(0) => return Err("exited".to_owned()),
                Ok(n) => {
                    self.buf.extend_from_slice(&chunk[..n]);
                    got = true;
                }
                // A signal counts as nothing more to read now, so that a
                // stream of signals cannot keep the read going.
                Err(_) => match std::io::Error::last_os_error().kind() {
                    ErrorKind::Interrupted | ErrorKind::WouldBlock => return Ok(got),
                    _ if got => return Ok(true),
                    _ => return Err("exited".to_owned()),
                },
            }
        }
        Ok(got)
    }

    /// What the helper wrote and the caller has not taken yet. The caller
    /// removes what it has read.
    pub fn buffer(&mut self) -> &mut Vec<u8> {
        &mut self.buf
    }

    /// inkline's end of the socket, for waiting on it; -1, which `poll`
    /// passes over, once it is given up.
    pub fn fd(&self) -> RawFd {
        self.socket.as_ref().map_or(-1, AsRawFd::as_raw_fd)
    }

    /// inkline's end of the socket, while its descriptor still holds it.
    /// A command run at the prompt can close that descriptor (`exec N>&-`)
    /// and open another file there; the descriptor then belongs to bash, so
    /// it is given up without being closed, and this is
    /// `Err("connection lost")` from then on.
    fn socket(&mut self) -> Result<RawFd, String> {
        let fd = self.fd();
        if fd >= 0 && file_id(fd) == Some(self.id) {
            return Ok(fd);
        }
        self.give_up();
        Err(LOST.to_owned())
    }

    /// Forgets the socket's descriptor without closing it.
    fn give_up(&mut self) {
        if let Some(fd) = self.socket.take() {
            let _ = fd.into_raw_fd();
        }
    }
}

impl Drop for Process {
    /// Closes the socket, unless its descriptor no longer holds it.
    fn drop(&mut self) {
        if file_id(self.fd()) != Some(self.id) {
            self.give_up();
        }
    }
}

/// Waits until `fd` is ready for `events`, has hung up or failed,
/// `deadline` has passed, or a signal arrives. Whether it became ready.
fn ready(fd: RawFd, events: libc::c_short, deadline: Instant) -> bool {
    ready_any(&[fd], events, deadline)
}

/// Waits until one of the helpers' sockets `fds` has something to read, has
/// hung up or failed, `deadline` has passed, or a signal arrives. Whether
/// one did.
pub fn readable(fds: &[RawFd], deadline: Instant) -> bool {
    ready_any(fds, libc::POLLIN, deadline)
}

/// Waits until one of `fds` is ready for `events`, has hung up or failed,
/// `deadline` has passed, or a signal arrives: a C-c is then acted on at
/// once. Whether one became ready.
fn ready_any(fds: &[RawFd], events: libc::c_short, deadline: Instant) -> bool {
    let mut polls: Vec<libc::pollfd> = fds
        .iter()
        .map(|&fd| libc::pollfd {
            fd,
            events,
            revents: 0,
        })
        .collect();
    let count = libc::nfds_t::try_from(polls.len()).unwrap_or(libc::nfds_t::MAX);
    let left = deadline.saturating_duration_since(Instant::now());
    // Rounded up, so that a caller waiting in a loop does not spin when
    // less than a millisecond is left.
    let ms = left.as_micros().div_ceil(1000);
    let ms = libc::c_int::try_from(ms).unwrap_or(libc::c_int::MAX);
    // SAFETY: `polls` holds `count` valid `pollfd`s.
    unsafe { libc::poll(polls.as_mut_ptr(), count, ms) > 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(mode: &str) -> Vec<String> {
        vec![
            format!("{}/tests/data/fake-mode-server", env!("CARGO_MANIFEST_DIR")),
            mode.to_owned(),
        ]
    }

    fn path() -> String {
        std::env::var("PATH").unwrap_or_default()
    }

    /// Reads from `p` until its buffer holds a whole line, or two seconds
    /// have passed.
    fn line(p: &mut Process) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !p.buffer().contains(&b'\n') && Instant::now() < deadline {
            p.fill(Some(deadline))?;
        }
        let nl = p.buffer().iter().position(|&b| b == b'\n').expect("a line");
        let line: Vec<u8> = p.buffer().drain(..=nl).collect();
        Ok(String::from_utf8(line).unwrap())
    }

    fn flags(fd: RawFd, get: libc::c_int) -> libc::c_int {
        // SAFETY: `fcntl` with a get command reads no memory of ours.
        unsafe { libc::fcntl(fd, get) }
    }

    #[test]
    fn a_missing_program_is_not_found() {
        let err = start(
            &["no-such-helper-xyz".to_owned()],
            "/nonexistent:/bin",
            None,
        )
        .err();
        assert_eq!(err.as_deref(), Some("not found"));
    }

    #[test]
    fn a_plain_name_is_found_in_the_path_given() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&fake("words")[0], dir.path().join("fake-hl")).unwrap();
        let dirs = format!("/nonexistent:{}:{}", dir.path().display(), path());
        let mut p = start(&["fake-hl".to_owned(), "words".to_owned()], &dirs, None).unwrap();
        assert_eq!(line(&mut p).unwrap(), "inkline-mode 1\n");
        assert_eq!(
            start(&["fake-hl".to_owned()], &path(), None)
                .err()
                .as_deref(),
            Some("not found")
        );
    }

    #[test]
    fn a_helper_starts_in_the_root_directory() {
        let program = ["/bin/sh", "-c", "pwd -P"].map(str::to_owned);
        let mut p = start(&program, &path(), None).unwrap();
        assert_eq!(line(&mut p).unwrap(), "/\n");
    }

    #[test]
    fn a_helper_gets_the_environment_given_and_no_other() {
        let program = ["/bin/sh", "-c", "echo \"$CSVM_X ${HOME-unset}\""].map(str::to_owned);
        let environment = Some(vec![b"CSVM_X=a=b".to_vec()]);
        let mut p = start(&program, &path(), environment).unwrap();
        assert_eq!(line(&mut p).unwrap(), "a=b unset\n");
    }

    /// A program named with a `/` but not from `/` is found from the
    /// shell's directory, which `cargo test` sets to the crate's.
    #[test]
    fn a_relative_program_is_found_from_the_shells_directory() {
        let program = ["tests/data/fake-mode-server", "words"].map(str::to_owned);
        let mut p = start(&program, &path(), None).unwrap();
        assert_eq!(line(&mut p).unwrap(), "inkline-mode 1\n");
    }

    #[test]
    fn a_program_that_cannot_run_says_why() {
        let err = start(&["/dev/null".to_owned()], &path(), None)
            .err()
            .unwrap();
        assert!(err.starts_with("cannot run: "), "{err}");
    }

    #[test]
    fn the_socket_is_close_on_exec_and_non_blocking() {
        let p = start(&fake("words"), &path(), None).unwrap();
        assert_ne!(flags(p.fd(), libc::F_GETFD) & libc::FD_CLOEXEC, 0);
        assert_ne!(flags(p.fd(), libc::F_GETFL) & libc::O_NONBLOCK, 0);
    }

    /// Other tests running at the same time hold a few of the highest
    /// descriptors too.
    #[test]
    fn the_socket_is_on_a_high_descriptor() {
        let p = start(&fake("words"), &path(), None).unwrap();
        let end = open_file_limit().min(HIGH_FDS_END);
        assert!((end - 32..end).contains(&p.fd()), "{} of {end}", p.fd());
    }

    /// Another file on the socket's descriptor (as after `exec N>&-
    /// N>file`) belongs to whoever put it there: the helper stops using
    /// it, and dropping the helper leaves it open.
    #[test]
    fn a_socket_replaced_under_inkline_is_given_up_not_closed() {
        let mut p = start(&fake("words"), &path(), None).unwrap();
        assert_eq!(line(&mut p).unwrap(), "inkline-mode 1\n");
        let fd = p.fd();
        let null = std::fs::File::open("/dev/null").unwrap();
        // SAFETY: `fd` is the helper's socket, which this test owns.
        assert_eq!(unsafe { libc::dup2(null.as_raw_fd(), fd) }, fd);
        assert_eq!(
            p.send(b":request 1\n", || false),
            Err("connection lost".to_owned())
        );
        assert_eq!(p.fill(None), Err("connection lost".to_owned()));
        drop(p);
        assert_ne!(flags(fd, libc::F_GETFD), -1, "the descriptor was closed");
        // SAFETY: `fd` is the copy of `/dev/null` this test made.
        unsafe { libc::close(fd) };
    }

    /// A read or write never blocks, even when the socket's descriptor has
    /// lost its non-blocking flag.
    #[test]
    fn a_blocking_descriptor_does_not_block() {
        let mut p = start(&["sleep".to_owned(), "5".to_owned()], "/usr/bin:/bin", None).unwrap();
        // SAFETY: `fcntl` with `F_SETFL` changes only the descriptor's flags.
        unsafe { libc::fcntl(p.fd(), libc::F_SETFL, 0) };
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let read = p.fill(None);
            let sent = p.send(&vec![b'x'; 16 << 20], || false);
            let _ = done.send((read, sent));
        });
        let got = finished.recv_timeout(Duration::from_secs(5));
        assert_eq!(
            got,
            Ok((Ok(false), Err("bad reply: request not read".to_owned())))
        );
    }

    #[test]
    fn a_helper_answers_a_request() {
        let mut p = start(&fake("words"), &path(), None).unwrap();
        assert_eq!(line(&mut p).unwrap(), "inkline-mode 1\n");
        p.send(
            b":request 1\n:cwd 0\n\n:arg final 4\ncsvm\n:arg final 1\na\n:done\n",
            || false,
        )
        .unwrap();
        assert_eq!(line(&mut p).unwrap(), ":span 1 0 1 command\n");
        assert_eq!(line(&mut p).unwrap(), ":end 1\n");
    }

    #[test]
    fn fill_without_a_deadline_does_not_wait() {
        let mut p = start(&fake("words"), &path(), None).unwrap();
        assert_eq!(line(&mut p).unwrap(), "inkline-mode 1\n");
        let before = Instant::now();
        p.fill(None).unwrap();
        assert!(before.elapsed() < Duration::from_millis(100));
        assert!(p.buffer().is_empty());
        let until = Instant::now() + Duration::from_millis(30);
        p.fill(Some(until)).unwrap();
        assert!(Instant::now() >= until);
    }

    #[test]
    fn a_helper_that_exits_is_exited_without_sigpipe() {
        let mut p = start(&fake("exit"), &path(), None).unwrap();
        assert_eq!(line(&mut p).unwrap(), "inkline-mode 1\n");
        p.send(b":request 1\n:cwd 0\n\n:arg final 4\ncsvm\n:done\n", || {
            false
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut end = Ok(false);
        while end.is_ok() && Instant::now() < deadline {
            end = p.fill(Some(deadline));
        }
        assert_eq!(end, Err("exited".to_owned()));
        // The test harness ignores SIGPIPE, which would drop the signal;
        // blocked, a raised one stays pending where this test can see it.
        // SAFETY: the sets are initialised before they are read, and the
        // mask goes back before the test ends.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            let mut old: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGPIPE);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old);
            let sent = p.send(b":request 2\n", || false);
            let mut pending: libc::sigset_t = std::mem::zeroed();
            libc::sigpending(&mut pending);
            let raised = libc::sigismember(&pending, libc::SIGPIPE) == 1;
            if raised {
                let zero = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                libc::sigtimedwait(&set, std::ptr::null_mut(), &zero);
            }
            libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
            assert_eq!(sent, Err("exited".to_owned()));
            assert!(!raised, "the send raised SIGPIPE");
        }
    }

    #[test]
    fn a_helper_that_reads_nothing_is_not_reading() {
        let mut p = start(&["sleep".to_owned(), "5".to_owned()], "/usr/bin:/bin", None).unwrap();
        let big = vec![b'x'; 16 << 20];
        let before = Instant::now();
        assert_eq!(
            p.send(&big, || false),
            Err("bad reply: request not read".to_owned())
        );
        assert!(before.elapsed() >= SEND_WAIT);
    }

    /// Sends a request that a helper which reads nothing cannot take, while
    /// SIGUSR2 arrives every 50 ms. What `send` returned, and how long it
    /// took.
    fn send_while_signalled(interrupted: fn() -> bool) -> (Result<(), String>, Duration) {
        extern "C" fn nothing(_: libc::c_int) {}
        // SAFETY: the handler does nothing; a signal ends a `poll` under way.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = nothing as extern "C" fn(libc::c_int) as usize;
            libc::sigaction(libc::SIGUSR2, &action, std::ptr::null_mut());
        }
        let mut p = start(&["sleep".to_owned(), "5".to_owned()], "/usr/bin:/bin", None).unwrap();
        let (id, thread_id) = std::sync::mpsc::channel();
        let sending = std::thread::spawn(move || {
            // SAFETY: `pthread_self` only names this thread.
            let _ = id.send(unsafe { libc::pthread_self() });
            let before = Instant::now();
            let sent = p.send(&vec![b'x'; 16 << 20], interrupted);
            (sent, before.elapsed())
        });
        let thread = thread_id.recv().unwrap();
        while !sending.is_finished() {
            // SAFETY: the thread is not joined yet, so `thread` still names it.
            unsafe { libc::pthread_kill(thread, libc::SIGUSR2) };
            std::thread::sleep(Duration::from_millis(50));
        }
        sending.join().unwrap()
    }

    /// A signal to act on at once, such as the one C-c sends, ends the wait
    /// for the helper to take a request.
    #[test]
    fn a_signal_to_act_on_ends_the_wait_to_send() {
        let (sent, took) = send_while_signalled(|| true);
        assert_eq!(sent, Err("bad reply: request not read".to_owned()));
        assert!(took < SEND_WAIT / 2, "took {took:?}");
    }

    /// A signal to act on that came before the wait began ends it too.
    #[test]
    fn a_signal_to_act_on_before_the_wait_ends_it() {
        let mut p = start(&["sleep".to_owned(), "5".to_owned()], "/usr/bin:/bin", None).unwrap();
        let before = Instant::now();
        let sent = p.send(&vec![b'x'; 16 << 20], || true);
        assert_eq!(sent, Err("bad reply: request not read".to_owned()));
        assert!(before.elapsed() < SEND_WAIT / 2);
    }

    /// A signal to act on that is caught while a `poll` goes on, without
    /// waking it, still ends the wait soon.
    #[test]
    fn a_signal_to_act_on_is_noticed_while_waiting() {
        static CAUGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let mut p = start(&["sleep".to_owned(), "5".to_owned()], "/usr/bin:/bin", None).unwrap();
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(200));
            CAUGHT.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let before = Instant::now();
        let sent = p.send(&vec![b'x'; 16 << 20], || {
            CAUGHT.load(std::sync::atomic::Ordering::Relaxed)
        });
        assert_eq!(sent, Err("bad reply: request not read".to_owned()));
        assert!(before.elapsed() < SEND_WAIT / 2);
    }

    /// Other signals, such as SIGCHLD when a job ends, do not.
    #[test]
    fn other_signals_do_not_end_the_wait_to_send() {
        let (sent, took) = send_while_signalled(|| false);
        assert_eq!(sent, Err("bad reply: request not read".to_owned()));
        assert!(took >= SEND_WAIT, "took {took:?}");
    }
}
