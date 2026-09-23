//! Shared helpers for the end-to-end tests.
#![allow(
    dead_code,
    reason = "each test binary uses a different subset of the helpers"
)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
pub use vt100::Color;

/// The bash to test: `$INKLINE_TEST_BASH` (made absolute, since the shells
/// start in other directories), or `bash` from `PATH`.
pub fn bash_path() -> PathBuf {
    match std::env::var_os("INKLINE_TEST_BASH") {
        Some(path) => std::path::absolute(path).unwrap(),
        None => PathBuf::from("bash"),
    }
}

/// The library `cargo test` built for this run, next to the test binary in
/// `target/<profile>/deps`.
pub fn so_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().join("libinkline.so")
}

/// A non-interactive bash, for tests that need no terminal.
pub fn bash_command() -> Command {
    let mut cmd = Command::new(bash_path());
    cmd.env("INPUTRC", "/dev/null");
    cmd
}

/// `bind` lines the tests load right after inkline.
pub const BINDINGS: &str = r#"bind '"\C-f": accept-suggestion-char'
bind '"\e[C": accept-suggestion-char'
bind '"\ef": accept-suggestion-word'
bind '"\C-e": accept-suggestion'
bind '"\e[F": accept-suggestion'
"#;

pub struct Options {
    pub rows: u16,
    pub cols: u16,
    /// Load inkline and `BINDINGS`.
    pub inkline: bool,
    /// History entries, oldest first.
    pub history: Vec<&'static str>,
    /// Extra lines for the end of the rc file.
    pub rc: String,
    /// Contents for the `INPUTRC` file; `/dev/null` when None.
    pub inputrc: Option<String>,
    /// Directory bash starts in; its temporary home when None.
    pub cwd: Option<PathBuf>,
    /// How the cursor row starts once the first prompt is up.
    pub prompt: &'static str,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            rows: 24,
            cols: 80,
            inkline: true,
            history: Vec::new(),
            rc: String::new(),
            inputrc: None,
            cwd: None,
            prompt: "$",
        }
    }
}

/// An interactive bash in a pseudo-terminal, with its screen kept up to date by
/// a terminal emulator.
pub struct Shell {
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    _home: tempfile::TempDir,
}

impl Shell {
    pub fn start(opts: Options) -> Shell {
        let home = tempfile::tempdir().unwrap();
        let mut rc = String::new();
        if opts.inkline {
            rc += &format!("enable -f {} inkline\n{BINDINGS}", so_path().display());
        }
        rc += "PS1='$ '\nPS2='> '\nHISTFILE=\n";
        rc += "bind 'set bell-style none'\nbind 'set enable-bracketed-paste on'\n";
        for entry in &opts.history {
            rc += &format!("history -s {}\n", quote(entry));
        }
        rc += &opts.rc;
        let rcfile = home.path().join("rc");
        std::fs::write(&rcfile, rc).unwrap();
        let inputrc = match &opts.inputrc {
            Some(text) => {
                let path = home.path().join("inputrc");
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
        let mut cmd = CommandBuilder::new(bash_path());
        cmd.args(["--noprofile", "--rcfile"]);
        cmd.arg(&rcfile);
        cmd.arg("-i");
        cmd.env_clear();
        cmd.env("PATH", std::env::var_os("PATH").unwrap());
        cmd.env("TERM", "xterm-256color");
        cmd.env("HOME", home.path());
        cmd.env("INPUTRC", &inputrc);
        cmd.env("LANG", "C.UTF-8");
        cmd.cwd(
            opts.cwd
                .clone()
                .unwrap_or_else(|| home.path().to_path_buf()),
        );
        let child = pty.slave.spawn_command(cmd).unwrap();
        drop(pty.slave);

        let parser = Arc::new(Mutex::new(vt100::Parser::new(opts.rows, opts.cols, 0)));
        let mut reader = pty.master.try_clone_reader().unwrap();
        let screen = parser.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                screen.lock().unwrap().process(&buf[..n]);
            }
        });
        let writer = pty.master.take_writer().unwrap();
        let sh = Shell {
            parser,
            writer,
            master: pty.master,
            child,
            _home: home,
        };
        let prompt = opts.prompt;
        sh.wait_for("the first prompt", |s| cursor_row(s).starts_with(prompt));
        sh
    }

    /// Types `keys` as one write.
    pub fn send(&mut self, keys: &str) {
        self.writer.write_all(keys.as_bytes()).unwrap();
        self.writer.flush().unwrap();
    }

    pub fn screen(&self) -> vt100::Screen {
        self.parser.lock().unwrap().screen().clone()
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
        self.parser
            .lock()
            .unwrap()
            .screen_mut()
            .set_size(rows, cols);
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.child.kill();
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

/// Calls `attempt` every 20ms until it returns Some, for up to 5 seconds.
fn poll<T>(mut attempt: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(5);
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
