//! Shared helpers for the end-to-end tests.
#![allow(
    dead_code,
    reason = "each test binary uses a different subset of the helpers"
)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
pub use vt100::Color;

/// The crate's directory, found when the tests run, not when they are
/// built: cargo can reuse a test binary built in another checkout that shares
/// the target directory, and a path fixed at build time would name that
/// checkout. `cargo test` sets `CARGO_MANIFEST_DIR` for the test process and
/// runs it in the crate's directory.
pub fn crate_dir() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default()
}

/// The fake mode server the mode tests run, `tests/data/fake-mode-server`.
pub fn fake_mode_server() -> PathBuf {
    let server = crate_dir().join("tests/data/fake-mode-server");
    assert!(
        server.is_file(),
        "fake-mode-server missing at {}",
        server.display()
    );
    server
}

/// The bash to test: `$INKLINE_TEST_BASH` (made absolute, since the shells
/// start in other directories); else the bash 5.3 that `make test` builds
/// into `target/bash-5.3`, when it is there; else `bash` from `PATH`. It
/// must be bash 5.3 or later, which is checked before the first shell
/// starts.
pub fn bash_path() -> PathBuf {
    static PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let bash = match std::env::var_os("INKLINE_TEST_BASH") {
            Some(path) => std::path::absolute(path).unwrap(),
            None => {
                let built = crate_dir().join("target/bash-5.3/bin/bash");
                if built.is_file() {
                    built
                } else {
                    PathBuf::from("bash")
                }
            }
        };
        let (major, minor) = version_of(&bash);
        assert!(
            (major, minor) >= (5, 3),
            "the bash to test, {bash:?}, is {major}.{minor}, and inkline needs 5.3 or \
             later: run `make test`, or set INKLINE_TEST_BASH to a bash 5.3 or later"
        );
        bash
    })
    .clone()
}

/// The library `cargo test` built for this run, next to the test binary in
/// `target/<profile>/deps`.
pub fn so_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().join("libinkline.so")
}

/// An empty home for non-interactive shells, so the developer's own
/// `init.el` is never read.
fn empty_home() -> PathBuf {
    let dir = std::env::temp_dir().join("inkline-test-empty-home");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A non-interactive bash, for tests that need no terminal.
pub fn bash_command() -> Command {
    bash_command_at(&bash_path())
}

/// A non-interactive run of the bash at `bash`, set up as `bash_command`
/// sets up the bash under test.
pub fn bash_command_at(bash: &Path) -> Command {
    let mut cmd = Command::new(bash);
    let home = empty_home();
    cmd.env("INPUTRC", "/dev/null")
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("XDG_STATE_HOME", &home);
    cmd
}

/// The major and minor version of the bash at `bash`, such as (5, 3).
pub fn version_of(bash: &Path) -> (u32, u32) {
    let out = bash_command_at(bash)
        .args(["-c", "echo ${BASH_VERSINFO[0]} ${BASH_VERSINFO[1]}"])
        .output()
        .unwrap_or_else(|err| panic!("cannot run {bash:?}: {err}"));
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.split_whitespace().map(str::parse);
    match (parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor))) => (major, minor),
        _ => panic!("cannot tell the version of {bash:?}: it printed {text:?}"),
    }
}

pub struct Options {
    /// The bash to start; the bash under test (`bash_path`) when None.
    pub bash: Option<PathBuf>,
    pub rows: u16,
    pub cols: u16,
    /// Load inkline.
    pub inkline: bool,
    /// History entries, oldest first.
    pub history: Vec<&'static str>,
    /// Extra lines for the end of the rc file.
    pub rc: String,
    /// Lines written to the rc file before the `enable -f` line, for tests
    /// that set variables inkline reads while it loads.
    pub before_inkline: String,
    /// Contents for the `INPUTRC` file; `/dev/null` when None.
    pub inputrc: Option<String>,
    /// Directory bash starts in; its temporary home when None.
    pub cwd: Option<PathBuf>,
    /// Use this directory as `HOME` instead of a new temporary one; the
    /// caller keeps it alive.
    pub home: Option<PathBuf>,
    /// Contents for `~/.config/inkline/init.el`; none when None.
    pub init_el: Option<String>,
    /// Leave bash's own completion on. Off by default, so the menus of
    /// tests about other sources hold only their items. The harness adds the
    /// line that turns it off to `init_el`, and runs it once in the rc file:
    /// after `inkline reload`, a test with no `init_el`, or one that writes
    /// `init.el` itself, has it on again.
    pub bash_completion: bool,
    /// How the cursor row starts once the first prompt is up.
    pub prompt: &'static str,
    pub term: &'static str,
    pub lang: &'static str,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            bash: None,
            rows: 24,
            cols: 80,
            inkline: true,
            history: Vec::new(),
            rc: String::new(),
            before_inkline: String::new(),
            inputrc: None,
            cwd: None,
            home: None,
            init_el: None,
            bash_completion: false,
            prompt: "$",
            term: "xterm-256color",
            lang: "C.UTF-8",
        }
    }
}

/// Start and end of a synchronized update (DEC private mode 2026).
pub const BEGIN_UPDATE: &[u8] = b"\x1b[?2026h";
pub const END_UPDATE: &[u8] = b"\x1b[?2026l";

/// How long a synchronized update may stay open before the screen is shown
/// anyway, as terminals do so that one never closed cannot hide the screen
/// for good.
pub const UPDATE_TIMEOUT: Duration = Duration::from_secs(1);

/// A terminal emulator that supports synchronized updates: what bash writes
/// goes to `live` as it arrives, and while an update is open the screen the
/// tests see is `shown`, the copy of `live`'s taken when the update began.
/// inkline's frames reach the terminal in more than one write, and a test
/// that read the screen between them would see half a frame.
pub struct Term {
    live: vt100::Parser,
    /// The screen as it was when the update now open began.
    shown: vt100::Screen,
    /// When the update now open began.
    open_since: Option<Instant>,
    /// The last bytes read, kept to find a marker that a read splits.
    tail: Vec<u8>,
}

impl Term {
    pub fn new(rows: u16, cols: u16) -> Term {
        let live = vt100::Parser::new(rows, cols, 0);
        let shown = live.screen().clone();
        Term {
            live,
            shown,
            open_since: None,
            tail: Vec::new(),
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        let mut scan = std::mem::take(&mut self.tail);
        // `scan[..fed]` has gone to `live`; the tail went with the last read.
        let mut fed = scan.len();
        scan.extend_from_slice(bytes);
        for at in 0..scan.len() {
            let rest = &scan[at..];
            let begin = rest.starts_with(BEGIN_UPDATE);
            if !begin && !rest.starts_with(END_UPDATE) {
                continue;
            }
            // The tail is shorter than a marker, so each one found ends in
            // `bytes` and was not found before.
            let end = at + BEGIN_UPDATE.len();
            self.live.process(&scan[fed..end]);
            fed = end;
            if begin {
                if self.open_since.is_none() {
                    self.shown = self.live.screen().clone();
                    self.open_since = Some(Instant::now());
                }
            } else {
                self.open_since = None;
            }
        }
        self.live.process(&scan[fed..]);
        let keep = scan.len().saturating_sub(BEGIN_UPDATE.len() - 1);
        self.tail = scan.split_off(keep);
    }

    /// The screen as the terminal shows it: the one from when the update
    /// began while an update is open, unless it has been open for
    /// `UPDATE_TIMEOUT`, and the live one otherwise.
    pub fn screen(&self) -> vt100::Screen {
        match self.open_since {
            Some(since) if since.elapsed() < UPDATE_TIMEOUT => self.shown.clone(),
            _ => self.live.screen().clone(),
        }
    }

    pub fn set_size(&mut self, rows: u16, cols: u16) {
        self.live.screen_mut().set_size(rows, cols);
        self.shown.set_size(rows, cols);
    }
}

/// An interactive bash in a pseudo-terminal, with its screen kept up to date by
/// a terminal emulator.
pub struct Shell {
    term: Arc<Mutex<Term>>,
    /// Everything bash wrote since the last `take_output`.
    output: Arc<Mutex<Vec<u8>>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    _home: Option<tempfile::TempDir>,
}

impl Shell {
    /// Starts bash and waits for its first prompt.
    pub fn start(opts: Options) -> Shell {
        let prompt = opts.prompt;
        let sh = Shell::spawn(opts);
        sh.wait_for("the first prompt", |s| cursor_row(s).starts_with(prompt));
        sh
    }

    /// Starts bash without waiting for its first prompt.
    pub fn spawn(opts: Options) -> Shell {
        let (home_path, keep_home) = match &opts.home {
            Some(dir) => (dir.clone(), None),
            None => {
                let tmp = tempfile::tempdir().unwrap();
                (tmp.path().to_owned(), Some(tmp))
            }
        };
        if let Some(text) = &opts.init_el {
            use std::os::unix::fs::PermissionsExt;
            let text = if opts.bash_completion {
                text.clone()
            } else {
                format!("{text}\n(setq inkline-bash-completion nil)\n")
            };
            let dir = home_path.join(".config/inkline");
            std::fs::create_dir_all(&dir).unwrap();
            for d in [home_path.join(".config"), dir.clone()] {
                std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let file = dir.join("init.el");
            std::fs::write(&file, text).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let mut rc = String::new();
        rc += &opts.before_inkline;
        if opts.inkline {
            rc += &format!("enable -f {} inkline\n", so_path().display());
            if !opts.bash_completion {
                rc += "inkline eval '(setq inkline-bash-completion nil)'\n";
            }
        }
        rc += "PS1='$ '\nPS2='> '\nHISTFILE=\n";
        rc += "bind 'set bell-style none'\nbind 'set enable-bracketed-paste on'\n";
        for entry in &opts.history {
            rc += &format!("history -s {}\n", quote(entry));
        }
        rc += &opts.rc;
        let rcfile = home_path.join("rc");
        std::fs::write(&rcfile, rc).unwrap();
        let inputrc = match &opts.inputrc {
            Some(text) => {
                let path = home_path.join("inputrc");
                std::fs::write(&path, text).unwrap();
                path
            }
            None => PathBuf::from("/dev/null"),
        };

        let size = PtySize {
            rows: opts.rows,
            cols: opts.cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pty = native_pty_system().openpty(size).unwrap();
        let mut cmd = CommandBuilder::new(opts.bash.clone().unwrap_or_else(bash_path));
        cmd.args(["--noprofile", "--rcfile"]);
        cmd.arg(&rcfile);
        cmd.arg("-i");
        cmd.env_clear();
        cmd.env("PATH", std::env::var_os("PATH").unwrap());
        cmd.env("TERM", opts.term);
        cmd.env("HOME", &home_path);
        cmd.env("INPUTRC", &inputrc);
        cmd.env("LANG", opts.lang);
        cmd.cwd(opts.cwd.clone().unwrap_or_else(|| home_path.clone()));
        let child = pty.slave.spawn_command(cmd).unwrap();
        drop(pty.slave);

        let term = Arc::new(Mutex::new(Term::new(opts.rows, opts.cols)));
        let mut reader = pty.master.try_clone_reader().unwrap();
        let output = Arc::new(Mutex::new(Vec::new()));
        let screen = term.clone();
        let raw = output.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                raw.lock().unwrap().extend_from_slice(&buf[..n]);
                screen.lock().unwrap().process(&buf[..n]);
            }
        });
        let writer = pty.master.take_writer().unwrap();
        Shell {
            term,
            output,
            writer,
            master: pty.master,
            child,
            _home: keep_home,
        }
    }

    /// Types `keys` as one write.
    pub fn send(&mut self, keys: &str) {
        self.writer.write_all(keys.as_bytes()).unwrap();
        self.writer.flush().unwrap();
    }

    /// The bytes bash wrote since the last call.
    pub fn take_output(&self) -> Vec<u8> {
        std::mem::take(&mut *self.output.lock().unwrap())
    }

    /// Polls the raw output since the last `take_output` until it contains
    /// `needle`, for up to 5 seconds.
    pub fn wait_for_output(&self, what: &str, needle: &[u8]) {
        poll(|| find_bytes(&self.output.lock().unwrap(), needle).map(|_| ())).unwrap_or_else(|| {
            panic!(
                "timed out waiting for {what}; screen:\n{}",
                dump(&self.screen())
            )
        })
    }

    /// The screen, as a terminal that supports synchronized updates shows it.
    pub fn screen(&self) -> vt100::Screen {
        self.term.lock().unwrap().screen()
    }

    /// Polls the screen until `done` holds, for up to 5 seconds.
    pub fn wait_for(&self, what: &str, done: impl Fn(&vt100::Screen) -> bool) -> vt100::Screen {
        poll(|| Some(self.screen()).filter(|s| done(s))).unwrap_or_else(|| {
            panic!(
                "timed out waiting for {what}; screen:\n{}",
                dump(&self.screen())
            )
        })
    }

    /// Waits until the screen, colours included, has not changed for 200ms.
    pub fn settle(&self) -> vt100::Screen {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut last = self.screen().contents_formatted();
        let mut since = Instant::now();
        while Instant::now() < deadline {
            sleep(Duration::from_millis(30));
            let now = self.screen().contents_formatted();
            if now != last {
                last = now;
                since = Instant::now();
            } else if since.elapsed() >= Duration::from_millis(200) {
                break;
            }
        }
        self.screen()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.term.lock().unwrap().set_size(rows, cols);
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
    }

    /// Whether the terminal hands bash each key as it is typed (canonical
    /// mode off), as readline sets it while it reads a line.
    pub fn keys_one_by_one(&self) -> bool {
        let fd = self.master.as_raw_fd().expect("the pty's master");
        // SAFETY: `termios` is plain data that `tcgetattr` fills.
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::tcgetattr(fd, &mut termios) }, 0);
        termios.c_lflag & libc::ICANON == 0
    }

    pub fn signal(&self, signal: libc::c_int) {
        let pid = self.child.process_id().expect("bash is running");
        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, signal) }, 0);
    }

    /// Whether bash exits within a second.
    pub fn exits(&mut self) -> bool {
        poll_for(Duration::from_secs(1), || {
            self.child.try_wait().ok().flatten()
        })
        .is_some()
    }

    /// Waits up to `time` for bash to exit, returning its exit status.
    pub fn wait_exit(&mut self, time: Duration) -> Option<portable_pty::ExitStatus> {
        poll_for(time, || self.child.try_wait().ok().flatten())
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.child.kill();
        // Reaps bash, so a test that checks whether its process is still
        // running never sees it as a zombie kept alive by an unclaimed exit
        // status.
        let _ = self.wait_exit(Duration::from_secs(2));
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The text of `row`, without trailing blanks.
pub fn row_text(screen: &vt100::Screen, row: u16) -> String {
    let (_, cols) = screen.size();
    screen
        .contents_between(row, 0, row, cols)
        .trim_end()
        .to_string()
}

/// The text of the row the cursor is on.
pub fn cursor_row(screen: &vt100::Screen) -> String {
    row_text(screen, screen.cursor_position().0)
}

pub fn has_row(screen: &vt100::Screen, text: &str) -> bool {
    (0..screen.size().0).any(|row| row_text(screen, row) == text)
}

/// Row and column of the last occurrence of `needle` on the screen, searching
/// from the bottom row up.
pub fn find(screen: &vt100::Screen, needle: &str) -> Option<(u16, u16)> {
    let (rows, cols) = screen.size();
    for row in (0..rows).rev() {
        let mut text = String::new();
        let mut starts = Vec::new();
        for col in 0..cols {
            let cell = screen.cell(row, col)?;
            if cell.is_wide_continuation() {
                continue;
            }
            starts.push((text.len(), col));
            text.push_str(if cell.has_contents() {
                cell.contents()
            } else {
                " "
            });
        }
        if let Some(at) = text.rfind(needle) {
            return starts
                .iter()
                .find(|(byte, _)| *byte == at)
                .map(|&(_, col)| (row, col));
        }
    }
    None
}

/// The cell where the last occurrence of `needle` starts.
pub fn cell(screen: &vt100::Screen, needle: &str) -> Option<vt100::Cell> {
    let (row, col) = find(screen, needle)?;
    screen.cell(row, col).cloned()
}

/// The foreground colour `needle` starts with; panics if it isn't on screen.
pub fn fg(screen: &vt100::Screen, needle: &str) -> Color {
    match cell(screen, needle) {
        Some(cell) => cell.fgcolor(),
        None => panic!("{needle:?} not on screen:\n{}", dump(screen)),
    }
}

/// Whether `needle` is on screen and starts with colour `color`.
pub fn fg_is(screen: &vt100::Screen, needle: &str, color: Color) -> bool {
    cell(screen, needle).is_some_and(|c| c.fgcolor() == color)
}

pub fn dump(screen: &vt100::Screen) -> String {
    let rows: Vec<String> = (0..screen.size().0)
        .map(|row| format!("{row:2}|{}", row_text(screen, row)))
        .collect();
    format!(
        "{}\ncursor at {:?}",
        rows.join("\n"),
        screen.cursor_position()
    )
}

/// Settles both shells, then waits until they show the same text with the
/// cursor in the same place.
pub fn wait_same(a: &Shell, b: &Shell, what: &str) {
    a.settle();
    b.settle();
    let same = || {
        let (sa, sb) = (a.screen(), b.screen());
        (sa.contents() == sb.contents() && sa.cursor_position() == sb.cursor_position())
            .then_some(())
    };
    if poll(same).is_none() {
        panic!(
            "screens differ after {what}\n--- with inkline ---\n{}\n--- plain bash ---\n{}",
            dump(&a.screen()),
            dump(&b.screen())
        );
    }
}

/// Calls `poll_for` with a 5-second limit.
pub fn poll<T>(attempt: impl FnMut() -> Option<T>) -> Option<T> {
    poll_for(Duration::from_secs(5), attempt)
}

/// Calls `attempt` every 20ms until it returns Some, for up to `time`.
fn poll_for<T>(time: Duration, mut attempt: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + time;
    loop {
        if let Some(value) = attempt() {
            return Some(value);
        }
        if Instant::now() > deadline {
            return None;
        }
        sleep(Duration::from_millis(20));
    }
}

/// Where `needle` first occurs in `haystack`.
pub fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// How often `needle` occurs in `haystack`.
pub fn count_bytes(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

/// Keys, as a terminal sends them.
pub const ENTER: &str = "\r";
pub const CTRL_J: &str = "\n";
/// Alt+Enter, or Esc then Enter.
pub const ALT_ENTER: &str = "\x1b\r";
pub const UP: &str = "\x1b[A";
pub const DOWN: &str = "\x1b[B";
/// `C-v C-j`: a newline inserted as text, as plain bash takes it.
pub const LITERAL_NEWLINE: &str = "\x16\n";
/// `C-v Tab`: a tab inserted as text.
pub const LITERAL_TAB: &str = "\x16\t";

/// Whether any cell on the screen is underlined.
pub fn any_underlined(screen: &vt100::Screen) -> bool {
    let (rows, cols) = screen.size();
    (0..rows).any(|row| (0..cols).any(|col| screen.cell(row, col).is_some_and(|c| c.underline())))
}

/// Whether `needle`, which must be ASCII, is on screen with every cell
/// underlined.
pub fn underlined(screen: &vt100::Screen, needle: &str) -> bool {
    let Some((row, col)) = find(screen, needle) else {
        return false;
    };
    (0..needle.len() as u16).all(|i| screen.cell(row, col + i).is_some_and(|c| c.underline()))
}
