//! One request to a copy of the shell: the fork, the pipe its answer comes
//! back on, and ending the copy.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

use super::Word;
use super::answer::{self, Answer, Decoded};
use crate::mode_server::process;

/// A copy of the shell working on bash's matches for `word`. Dropping it
/// kills the copy and what it started, unless its answer has come.
pub struct Running {
    pub word: Word,
    /// When the copy's time is up.
    pub deadline: Instant,
    /// The copy's process group: the process id of the short-lived process
    /// that made the group and started the copy in it. The id is not given
    /// to another process while the group has a process in it.
    group: libc::pid_t,
    pipe: OwnedFd,
    buf: Vec<u8>,
    /// Set once the answer came or the pipe closed: the copy's group may then
    /// be gone, and its id soon belong to another group.
    ended: bool,
}

/// What reading a copy's pipe gave.
pub enum Read {
    More,
    Came(Answer),
    Failed,
}

impl Running {
    /// Forks a copy of the shell to ask bash's completion for the word at
    /// the cursor, which is `word`; it may take `limit`. The caller holds
    /// no `RefCell` borrow the copy's code needs: the copy calls bash's
    /// completion, which calls inkline's `complete`.
    ///
    /// The copy is not the shell's child: a middle process makes a session
    /// of its own, forks the copy in it and exits at once, and the shell
    /// waits for the middle process here. So the copy's end sends the shell
    /// no `SIGCHLD`, which would look like a job of the shell ending.
    pub fn start(word: Word, limit: Duration) -> std::io::Result<Running> {
        let mut fds = [0; 2];
        // SAFETY: `pipe2` fills `fds` with two new descriptors on success.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: both descriptors are new, and nothing else owns them.
        let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let read = process::to_high_fd(read)?;
        // SAFETY: flags of a descriptor this function owns.
        unsafe {
            let flags = libc::fcntl(read.as_raw_fd(), libc::F_GETFL);
            if flags < 0
                || libc::fcntl(read.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        let deadline = Instant::now() + limit;
        // SAFETY: `getpid` only reads this process's id.
        let shell = unsafe { libc::getpid() };
        // Bash's SIGCHLD handler reaps any child of bash that exits, at any
        // moment, and the middle process exits at once: SIGCHLD waits until
        // the middle process is reaped here.
        let blocked = process::BlockChild::new();
        let mask = blocked.old;
        // SAFETY: the middle process only makes its session, forks and ends
        // with `_exit`. The copy only runs `copy::run`, which never returns,
        // so nothing of the shell's is unwound or dropped twice.
        let middle = match unsafe { libc::fork() } {
            -1 => return Err(std::io::Error::last_os_error()),
            0 => unsafe {
                if libc::setsid() < 0 {
                    libc::_exit(1);
                }
                match libc::fork() {
                    -1 => libc::_exit(1),
                    0 => {
                        // The copy runs as bash does, with SIGCHLD let in.
                        libc::pthread_sigmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
                        drop(read);
                        super::copy::run(write, shell, deadline)
                    }
                    _ => libc::_exit(0),
                }
            },
            middle => middle,
        };
        drop(write);
        let started = wait_middle(middle);
        drop(blocked);
        if !started {
            return Err(std::io::Error::other("the copy of the shell did not start"));
        }
        Ok(Running {
            word,
            deadline,
            group: middle,
            pipe: read,
            buf: Vec::new(),
            ended: false,
        })
    }

    pub fn fd(&self) -> RawFd {
        self.pipe.as_raw_fd()
    }

    /// Reads what the copy has written, without waiting. A pipe that closes
    /// before the whole answer, an answer that cannot be read, and more than
    /// `answer::MOST_BYTES` are a failure.
    pub fn read(&mut self) -> Read {
        let mut chunk = [0u8; 8192];
        let closed = loop {
            // SAFETY: reads into `chunk`, which outlives the call.
            let n = unsafe { libc::read(self.fd(), chunk.as_mut_ptr().cast(), chunk.len()) };
            match n {
                0 => break true,
                n if n > 0 => {
                    self.buf.extend_from_slice(&chunk[..n as usize]);
                    if self.buf.len() > answer::MOST_BYTES {
                        return Read::Failed;
                    }
                }
                _ => match std::io::Error::last_os_error().kind() {
                    std::io::ErrorKind::Interrupted => {}
                    std::io::ErrorKind::WouldBlock => break false,
                    _ => return Read::Failed,
                },
            }
        };
        match answer::decode(&self.buf) {
            Decoded::Done(answer) => {
                self.ended = true;
                Read::Came(answer)
            }
            Decoded::More if !closed => Read::More,
            Decoded::More | Decoded::Bad => {
                self.ended |= closed;
                Read::Failed
            }
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.ended {
            // SAFETY: signals the copy's process group, which was made
            // before the copy started in it.
            unsafe {
                libc::kill(-self.group, libc::SIGKILL);
            }
        }
    }
}

/// Waits for the middle process `pid`, which ends at once. Whether it
/// started the copy. When it was reaped by another wait first, its status is
/// unknown: it counts as started, and the copy's pipe tells the rest.
fn wait_middle(pid: libc::pid_t) -> bool {
    let mut status = 0;
    loop {
        // SAFETY: `waitpid` fills `status`, which is read only on success.
        if unsafe { libc::waitpid(pid, &mut status, 0) } == pid {
            return libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return true;
        }
    }
}
