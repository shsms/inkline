//! The user's `init.el`: where it is, whether it may be read, and reading it
//! when inkline loads.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// `$XDG_CONFIG_HOME/inkline/init.el` when that is an absolute path, else
/// `$HOME/.config/inkline/init.el`.
pub fn location(xdg_config: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    match xdg_config.filter(|d| d.starts_with('/')) {
        Some(dir) => Some(Path::new(dir).join("inkline/init.el")),
        None => home
            .filter(|h| h.starts_with('/'))
            .map(|h| Path::new(h).join(".config/inkline/init.el")),
    }
}

/// What the checks need to know about a file or directory.
pub struct Meta {
    pub is_file: bool,
    pub uid: u32,
    pub mode: u32,
}

/// Whether `init.el` (`file`, in `dir`) may be read by a process running as
/// `euid`: it and its directory belong to that user or to root (only root
/// when running as root), and neither is writable by group or others.
pub fn check(file: &Meta, dir: &Meta, euid: u32) -> Result<(), String> {
    if !file.is_file {
        return Err("not a regular file".into());
    }
    check_owner(file, euid)?;
    check_owner(dir, euid).map_err(|why| format!("its directory is {why}"))
}

/// Whether `m` belongs to `euid` or to root (only root when `euid` is root),
/// and is not writable by group or others.
pub fn check_owner(m: &Meta, euid: u32) -> Result<(), String> {
    let owner_ok = if euid == 0 {
        m.uid == 0
    } else {
        m.uid == euid || m.uid == 0
    };
    if !owner_ok {
        return Err("owned by another user".into());
    }
    if m.mode & 0o022 != 0 {
        return Err("writable by group or others".into());
    }
    Ok(())
}

impl From<&std::fs::Metadata> for Meta {
    fn from(m: &std::fs::Metadata) -> Meta {
        use std::os::unix::fs::MetadataExt;
        Meta {
            is_file: m.is_file(),
            uid: m.uid(),
            mode: m.mode(),
        }
    }
}

/// What happened to `init.el`, for `inkline status`.
#[derive(Clone, Debug)]
pub enum Outcome {
    NotRead(&'static str),
    NotFound(PathBuf),
    Skipped(PathBuf, String),
    /// The error, without the file's path.
    Failed(PathBuf, String),
    Loaded(PathBuf),
}

/// `error`, a line from `errors::describe` for `file`, with the path taken
/// off: `line 3: text` for `<file>:3: text`.
pub fn without_path(file: &str, error: &str) -> String {
    let place = error
        .strip_prefix(file)
        .and_then(|rest| rest.strip_prefix(':'))
        .and_then(|rest| rest.split_once(": "));
    match place {
        Some((line, text)) if line.parse::<u32>().is_ok() => format!("line {line}: {text}"),
        _ => error.to_owned(),
    }
}

thread_local! {
    static OUTCOME: RefCell<Outcome> = const { RefCell::new(Outcome::NotRead("not an interactive shell with line editing on")) };
}

pub fn status_line() -> String {
    OUTCOME.with_borrow(|o| match o {
        Outcome::NotRead(why) => format!("init.el: not read ({why})"),
        Outcome::NotFound(p) => format!("init.el: {} (not found)", p.display()),
        Outcome::Skipped(p, why) => format!("init.el: {} (skipped: {why})", p.display()),
        Outcome::Failed(p, e) => format!("init.el: {} (error: {e})", p.display()),
        Outcome::Loaded(p) => format!("init.el: {} (loaded)", p.display()),
    })
}

#[cfg(not(test))]
fn set_outcome(o: Outcome) {
    OUTCOME.with_borrow_mut(|s| *s = o);
}

#[cfg(not(test))]
fn meta(path: &Path) -> std::io::Result<Meta> {
    std::fs::metadata(path).map(|m| Meta::from(&m))
}

/// Where `init.el` is, from bash's `HOME` and `XDG_CONFIG_HOME`.
#[cfg(not(test))]
pub fn path() -> Option<PathBuf> {
    let home = crate::ffi::shell_variable("HOME");
    let xdg = crate::ffi::shell_variable("XDG_CONFIG_HOME");
    location(xdg.as_deref(), home.as_deref())
}

/// Prints one line for each old `INKLINE_*` variable still set: inkline no
/// longer reads them.
#[cfg(not(test))]
pub fn warn_old_variables() {
    use std::io::Write;
    let init_el = path().map_or_else(|| "init.el".to_owned(), |p| p.display().to_string());
    for name in [
        "INKLINE_COLORS",
        "INKLINE_INDENT",
        "INKLINE_HISTORY_CURSOR",
        "INKLINE_SUGGESTION_LINES",
    ] {
        if crate::ffi::shell_variable(name).is_some() {
            let setting = name.to_lowercase().replace('_', "-");
            let _ = writeln!(
                std::io::stderr(),
                "inkline: {name} is no longer read; set {setting} in {init_el}"
            );
        }
    }
}

/// Reads `init.el` from where bash's `HOME` and `XDG_CONFIG_HOME` say, if it
/// passes the checks, and prints one line for each problem.
#[cfg(not(test))]
pub fn read_at_start() {
    let Some(path) = path() else {
        set_outcome(Outcome::NotRead("no HOME"));
        return;
    };
    read(&path);
}

/// The directory for the markers, from bash's `HOME` and `XDG_STATE_HOME`,
/// made if it is missing; None when there is none or it cannot be made. When
/// it fails the checks `init.el`'s directory gets, markers are off and the
/// error is the line that says so.
#[cfg(not(test))]
fn marker_dir() -> Result<Option<PathBuf>, String> {
    use std::os::unix::fs::DirBuilderExt;
    let home = crate::ffi::shell_variable("HOME");
    let xdg = crate::ffi::shell_variable("XDG_STATE_HOME");
    let Some(dir) = super::lockout::state_dir(xdg.as_deref(), home.as_deref()) else {
        return Ok(None);
    };
    let made = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir);
    let Ok(m) = made.and_then(|()| meta(&dir)) else {
        return Ok(None);
    };
    match check_owner(&m, unsafe { libc::geteuid() }) {
        Ok(()) => Ok(Some(dir)),
        Err(why) => Err(format!("{}: markers off: {why}", dir.display())),
    }
}

/// The stuck markers in `state`.
#[cfg(not(test))]
fn stuck_markers(state: &Path) -> Vec<super::lockout::Marker> {
    super::lockout::stuck_markers(
        state,
        std::time::SystemTime::now(),
        super::lockout::process_alive,
    )
}

#[cfg(not(test))]
fn read(path: &Path) {
    use std::io::Write;
    let say = |line: String| {
        let _ = writeln!(std::io::stderr(), "inkline: {line}");
    };
    let skip = |why: String| {
        say(format!("{}: not read: {why}", path.display()));
        set_outcome(Outcome::Skipped(path.to_owned(), why));
    };
    // The checks and the reading go to the file a link points at.
    let real = match std::fs::canonicalize(path) {
        Ok(real) => real,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            set_outcome(Outcome::NotFound(path.to_owned()));
            return;
        }
        Err(e) => return skip(e.to_string()),
    };
    if let Err(why) = check_link_dirs(path) {
        return skip(why);
    }
    let dir_path = real.parent().unwrap_or(Path::new("/"));
    let text = match read_checked(&real, dir_path) {
        Ok(text) => text,
        Err(why) if real != path => return skip(format!("{}: {why}", real.display())),
        Err(why) => return skip(why),
    };
    let name = path.to_string_lossy().into_owned();
    let state = marker_dir().unwrap_or_else(|line| {
        say(line);
        None
    });
    let changed = std::fs::metadata(&real).and_then(|m| m.modified()).ok();
    if let Some(state) = &state {
        let stuck = stuck_markers(state);
        if changed.is_some_and(|c| super::lockout::skip_init(c, &stuck)) {
            say("init.el did not finish in an earlier shell; not read. Fix it, then run inkline reload".into());
            set_outcome(Outcome::Skipped(
                path.to_owned(),
                "did not finish in an earlier shell".into(),
            ));
            return;
        }
        for m in stuck {
            let _ = std::fs::remove_file(m.path);
        }
    }
    let marker = state
        .as_deref()
        .and_then(|s| super::lockout::make_marker(s, "loading"));
    // Stays if a panic ends the reading; a normal finish replaces it.
    set_outcome(Outcome::Failed(path.to_owned(), "internal error".into()));
    let result = super::with_lisp(|ctx| {
        let _ = ctx.set_load_path(Some(dir_path));
        ctx.eval_prelude(&name, &text)
            .map(drop)
            .map_err(|e| super::errors::describe(&e, ctx, Some(&name)))
    });
    drop(marker);
    match result {
        Ok(Ok(())) => set_outcome(Outcome::Loaded(path.to_owned())),
        Ok(Err(e)) => {
            say(e.clone());
            set_outcome(Outcome::Failed(path.to_owned(), without_path(&name, &e)));
        }
        Err(super::Busy) => set_outcome(Outcome::Skipped(path.to_owned(), "busy".into())),
    }
    for problem in super::settings::problems() {
        say(problem);
    }
}

/// Follows `path` one link at a time; the directory that holds each link
/// must pass `check_owner`, as whoever can write there can point the link at
/// any file.
#[cfg(not(test))]
fn check_link_dirs(path: &Path) -> Result<(), String> {
    let euid = unsafe { libc::geteuid() };
    let mut link = path.to_owned();
    for _ in 0..=40 {
        let dir = link.parent().unwrap_or(Path::new("/")).to_owned();
        let checked = meta(&dir)
            .map_err(|e| e.to_string())
            .and_then(|m| check_owner(&m, euid));
        if let Err(why) = checked {
            return Err(if link == path {
                format!("its directory is {why}")
            } else {
                format!("{}: its directory is {why}", link.display())
            });
        }
        match std::fs::read_link(&link) {
            Ok(target) => link = dir.join(target),
            Err(_) => return Ok(()),
        }
    }
    Err("too many links".into())
}

/// The text of the file at `real`, a path with no links in it, if it and its
/// directory `dir` pass `check`. The checks look at the open file, so the
/// text is that of the file that passed them.
#[cfg(not(test))]
fn read_checked(real: &Path, dir: &Path) -> Result<String, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    // Non-blocking, so opening a FIFO does not wait for a writer.
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(real)
        .map_err(|e| e.to_string())?;
    let file_meta = file.metadata().map_err(|e| e.to_string())?;
    let dir_meta = meta(dir).map_err(|e| e.to_string())?;
    check(&Meta::from(&file_meta), &dir_meta, unsafe {
        libc::geteuid()
    })?;
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(|e| e.to_string())?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_init_el_is() {
        assert_eq!(
            location(Some("/x"), Some("/h")),
            Some("/x/inkline/init.el".into())
        );
        assert_eq!(
            location(Some(""), Some("/h")),
            Some("/h/.config/inkline/init.el".into())
        );
        assert_eq!(
            location(Some("rel"), Some("/h")),
            Some("/h/.config/inkline/init.el".into())
        );
        assert_eq!(location(None, Some("")), None);
        assert_eq!(location(None, None), None);
    }

    fn meta(is_file: bool, uid: u32, mode: u32) -> Meta {
        Meta { is_file, uid, mode }
    }

    #[test]
    fn errors_lose_the_path() {
        assert_eq!(
            without_path("/h/init.el", "/h/init.el:3: Unclosed list"),
            "line 3: Unclosed list"
        );
        assert_eq!(
            without_path("/h/init.el", "no catch for x"),
            "no catch for x"
        );
        assert_eq!(
            without_path("/h/init.el", "/h/init.el: x: y"),
            "/h/init.el: x: y"
        );
    }

    #[test]
    fn checks() {
        let dir = meta(false, 1000, 0o755);
        assert_eq!(check(&meta(true, 1000, 0o644), &dir, 1000), Ok(()));
        assert_eq!(check(&meta(true, 0, 0o644), &dir, 1000), Ok(()));
        assert_eq!(
            check(&meta(false, 1000, 0o644), &dir, 1000),
            Err("not a regular file".into())
        );
        assert_eq!(
            check(&meta(true, 1001, 0o644), &dir, 1000),
            Err("owned by another user".into())
        );
        assert_eq!(
            check(&meta(true, 1000, 0o664), &dir, 1000),
            Err("writable by group or others".into())
        );
        assert_eq!(
            check(&meta(true, 1000, 0o644), &meta(false, 1000, 0o775), 1000),
            Err("its directory is writable by group or others".into())
        );
        assert_eq!(
            check(&meta(true, 1000, 0o644), &meta(false, 1001, 0o755), 1000),
            Err("its directory is owned by another user".into())
        );
        // As root, only root's files.
        assert_eq!(
            check(&meta(true, 1000, 0o644), &meta(false, 0, 0o755), 0),
            Err("owned by another user".into())
        );
    }
}
