//! What a copy of the shell does for one request: it sets itself apart from
//! the terminal and the shell's jobs, starts a watcher that ends it at its
//! time limit, asks bash's completion for the word at the cursor as Tab
//! would, replays Tab for each match to see what it puts on the line,
//! writes the answer and exits.

use std::io::Write;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::{Duration, Instant};

use super::answer::{self, Answer, MOST};
use crate::ffi;
use crate::mode_server::process;

/// How long after its time is up the watcher kills a copy. The shell kills
/// it at that time itself when it is not busy, and counts it as timed out.
const WATCH_GRACE: Duration = Duration::from_secs(1);

/// Runs in the copy, a fork of a fork of the shell `shell`, in a process
/// group of its own, with `out` the pipe's writing end, and never returns.
/// It ends with `_exit`: 0 once the answer is written, 1 when it cannot be
/// written or no watcher can start. A rule's error that jumps out of it to
/// bash's top level ends it through bash's own exit instead (see
/// `ffi::become_copy`).
pub fn run(out: OwnedFd, shell: libc::pid_t, deadline: Instant) -> ! {
    let written = std::panic::catch_unwind(|| {
        ffi::become_copy();
        if !start_watcher(&out, shell, deadline + WATCH_GRACE) {
            return false;
        }
        let answer = gather();
        std::fs::File::from(out)
            .write_all(&answer::encode(&answer))
            .is_ok()
    });
    ffi::exit_copy(if matches!(written, Ok(true)) { 0 } else { 1 })
}

/// Forks the watcher: a process that kills the copy's whole process group
/// once `deadline` passes or the shell `shell` ends, and ends itself with
/// the copy. The copy is not the shell's child, so without it a copy could
/// run on after the shell. Whether it started.
fn start_watcher(out: &OwnedFd, shell: libc::pid_t, deadline: Instant) -> bool {
    // SAFETY: `getpid` only reads this process's id.
    let copy = unsafe { libc::getpid() };
    // SAFETY: the watcher only runs `watch`, which ends with `_exit`.
    match unsafe { libc::fork() } {
        -1 => false,
        0 => watch(out, copy, shell, deadline),
        _ => true,
    }
}

/// The watcher's work (see `start_watcher`); `copy` is its parent.
fn watch(out: &OwnedFd, copy: libc::pid_t, shell: libc::pid_t, deadline: Instant) -> ! {
    // SAFETY: plain calls on this process's own descriptors and group.
    unsafe {
        // The copy's pipe closes when the copy ends, not when this does.
        libc::close(out.as_raw_fd());
        #[cfg(target_os = "linux")]
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
        // The copy may have ended before the line above.
        if libc::getppid() != copy {
            libc::_exit(0);
        }
        let shell_fd = shell_ended_fd(shell);
        if shell_fd.is_some() || crate::lisp::lockout::process_alive(shell) {
            // With no pidfd there is nothing to wait on, so this only sleeps.
            let fds: Vec<libc::c_int> = shell_fd.into_iter().collect();
            while Instant::now() < deadline && !process::readable(&fds, deadline) {}
        }
        libc::kill(0, libc::SIGKILL);
        libc::_exit(0)
    }
}

/// A descriptor that becomes readable when the process `pid` ends: a
/// pidfd, on Linux from 5.3. None where there is none, or when `pid` has
/// already ended.
fn shell_ended_fd(pid: libc::pid_t) -> Option<libc::c_int> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `pidfd_open` makes a new descriptor, or fails.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        libc::c_int::try_from(fd).ok().filter(|&fd| fd >= 0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
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
