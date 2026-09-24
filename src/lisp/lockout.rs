//! Keeps a Lisp mistake from locking the user out of new shells: markers
//! left while `init.el` runs, a watcher that ends bash when its terminal
//! closes while Lisp runs, and a call-depth limit that fits the stack.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

/// How old a marker of a still running shell must be to count as stuck.
const STUCK_AFTER: Duration = Duration::from_secs(10);

/// `$XDG_STATE_HOME/inkline`, else `~/.local/state/inkline`.
pub fn state_dir(xdg_state: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    match xdg_state.filter(|d| d.starts_with('/')) {
        Some(dir) => Some(Path::new(dir).join("inkline")),
        None => home
            .filter(|h| h.starts_with('/'))
            .map(|h| Path::new(h).join(".local/state/inkline")),
    }
}

/// A marker a shell made before running Lisp at start-up, named
/// `<kind>.<pid>`.
pub struct Marker {
    pub path: PathBuf,
    pub made: SystemTime,
}

/// The markers in `dir` whose shell has ended, or that are older than ten
/// seconds.
pub fn stuck_markers(dir: &Path, now: SystemTime, alive: impl Fn(i32) -> bool) -> Vec<Marker> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let (_, pid) = name.split_once('.')?;
            let pid: i32 = pid.parse().ok()?;
            let made = entry.metadata().ok()?.modified().ok()?;
            let old = now.duration_since(made).is_ok_and(|age| age > STUCK_AFTER);
            (!alive(pid) || old).then(|| Marker {
                path: entry.path(),
                made,
            })
        })
        .collect()
}

/// Whether `init.el`, last changed at `changed`, should be skipped: a stuck
/// marker was made after that change.
pub fn skip_init(changed: SystemTime, stuck: &[Marker]) -> bool {
    stuck.iter().any(|m| m.made >= changed)
}

/// A marker this shell made. Dropping it removes the file, also during a
/// panic.
pub struct OwnMarker(PathBuf);

impl Drop for OwnMarker {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Makes `dir/<kind>.<pid>` in the directory `dir`. It never writes through a
/// link: an old file of that name is removed first, and `create_new` refuses
/// anything put there since.
pub fn make_marker(dir: &Path, kind: &str) -> Option<OwnMarker> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = dir.join(format!("{kind}.{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .ok()?;
    Some(OwnMarker(path))
}

/// Whether process `pid` is still running.
pub fn process_alive(pid: i32) -> bool {
    (unsafe { libc::kill(pid, 0) == 0 })
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The call-depth limit for a stack of `stack` bytes (None: no limit): 1000
/// for 8 MiB or more, less for smaller stacks, never below 50.
pub fn max_eval_depth(stack: Option<u64>) -> u32 {
    const FULL: u64 = 8 << 20;
    match stack {
        None => 1000,
        Some(bytes) => (1000 * bytes.min(FULL) / FULL).max(50) as u32,
    }
}

/// The stack size limit, or None when there is none.
pub fn stack_limit() -> Option<u64> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit) } != 0
        || limit.rlim_cur == libc::RLIM_INFINITY
    {
        return None;
    }
    Some(limit.rlim_cur)
}

/// Starts a thread that ends bash when its terminal (standard input) hangs
/// up while Lisp runs: bash only notes signals while a builtin runs, so
/// without it a loop in Lisp would outlive its window. The thread blocks
/// every signal, so bash's handlers always run on bash's own thread.
///
/// The thread polls a duplicate of file descriptor 0, made once here, not
/// file descriptor 0 itself: a redirection on the `inkline eval`/`load`
/// command (`<<<text`, `0<&-`) changes what fd 0 points at while Lisp runs,
/// and polling the number would then watch that redirection instead of the
/// terminal. The duplicate keeps pointing at the terminal; a real hang-up
/// still reaches it, since every file descriptor open on one side of a
/// pseudo-terminal sees the same hang-up when the other side closes.
///
/// The duplicate is made at fd 100 or above (10 or above if that fails),
/// above the numbers scripts usually use. A script can still close it or open
/// another file on its number, so a hang-up only ends bash while the number
/// still holds the terminal; once it holds something else, the thread stops.
pub fn watch_terminal() {
    if unsafe { libc::isatty(0) } != 1 {
        return;
    }
    let mut fd = unsafe { libc::fcntl(0, libc::F_DUPFD_CLOEXEC, 100) };
    if fd < 0 {
        fd = unsafe { libc::fcntl(0, libc::F_DUPFD_CLOEXEC, 10) };
    }
    if fd < 0 {
        return;
    }
    let Some(terminal) = file_id(fd) else {
        unsafe { libc::close(fd) };
        return;
    };
    unsafe {
        let mut all: libc::sigset_t = std::mem::zeroed();
        let mut old: libc::sigset_t = std::mem::zeroed();
        libc::sigfillset(&mut all);
        libc::pthread_sigmask(libc::SIG_SETMASK, &all, &mut old);
        let _ = std::thread::Builder::new()
            .name("inkline-watch".into())
            .spawn(move || watch(fd, terminal));
        libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
    }
}

/// The device and inode of the file open on `fd`.
fn file_id(fd: libc::c_int) -> Option<(libc::dev_t, libc::ino_t)> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    (unsafe { libc::fstat(fd, &mut st) } == 0).then_some((st.st_dev, st.st_ino))
}

/// Polls `fd` until it no longer holds `terminal`.
fn watch(fd: libc::c_int, terminal: (libc::dev_t, libc::ino_t)) {
    loop {
        let mut poll = libc::pollfd {
            fd,
            events: 0,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, 1000) } < 0 {
            continue;
        }
        if poll.revents & libc::POLLNVAL != 0 || file_id(fd) != Some(terminal) {
            return;
        }
        if poll.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            if crate::lisp::RUNNING.load(Ordering::Relaxed) {
                unsafe { libc::_exit(129) };
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn where_markers_go() {
        assert_eq!(state_dir(Some("/s"), Some("/h")), Some("/s/inkline".into()));
        assert_eq!(
            state_dir(None, Some("/h")),
            Some("/h/.local/state/inkline".into())
        );
        assert_eq!(state_dir(None, None), None);
    }

    #[test]
    fn stuck_markers_are_ended_or_old() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["loading.100", "loading.200", "hooks.300", "other"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let now = SystemTime::now();
        let alive = |pid: i32| pid == 200 || pid == 300;
        let mut stuck: Vec<String> = stuck_markers(dir.path(), now, alive)
            .into_iter()
            .map(|m| m.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        stuck.sort();
        assert_eq!(stuck, ["loading.100"]);
        let later = now + Duration::from_secs(11);
        assert_eq!(stuck_markers(dir.path(), later, alive).len(), 3);
    }

    #[test]
    fn a_marker_does_not_write_through_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, "keep").unwrap();
        let path = dir.path().join(format!("loading.{}", std::process::id()));
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let marker = make_marker(dir.path(), "loading");
        assert!(marker.is_some());
        assert!(!path.is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    }

    #[test]
    fn a_marker_goes_away_also_after_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("loading.{}", std::process::id()));
        let marker = make_marker(dir.path(), "loading");
        assert!(path.exists());
        let _ = std::panic::catch_unwind(move || {
            let _marker = marker;
            panic!("boom");
        });
        assert!(!path.exists());
    }

    #[test]
    fn init_is_skipped_until_it_changes() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let marker = |secs| Marker {
            path: "m".into(),
            made: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
        };
        assert!(skip_init(t0, &[marker(1005)]));
        assert!(!skip_init(t0, &[marker(995)]));
        assert!(!skip_init(t0, &[]));
    }

    #[test]
    fn depth_follows_the_stack() {
        assert_eq!(max_eval_depth(None), 1000);
        assert_eq!(max_eval_depth(Some(8 << 20)), 1000);
        assert_eq!(max_eval_depth(Some(64 << 20)), 1000);
        assert_eq!(max_eval_depth(Some(2 << 20)), 250);
        assert_eq!(max_eval_depth(Some(64 << 10)), 50);
    }
}
