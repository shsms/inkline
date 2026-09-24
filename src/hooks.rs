//! Connects inkline to bash and readline: the `inkline` builtin, the hooks
//! readline calls, and the readline commands inkline adds.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int};
use std::io::Write;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Once;
use std::sync::atomic::Ordering;

use crate::commands::{self, PathCache};
use crate::ffi;
use crate::lexer::{Kind, Lexer};
use crate::pairs::{self, Action};
use crate::render::{self, Repaint};
use crate::suggest;
use crate::syntax::{self, Checker, Status};

mod multiline;

/// The readline functions inkline replaced. Kept apart from `State` in a
/// `Cell`, so the fallback after a panic can always read them.
#[derive(Clone, Copy, Default)]
struct Originals {
    redisplay: Option<ffi::VoidFn>,
    getc: Option<ffi::GetcFn>,
    deprep: Option<ffi::VoidFn>,
    pre_input: Option<ffi::HookFn>,
}

struct State {
    enabled: bool,
    lexer: Lexer,
    paths: PathCache,
    /// The line a suggestion was drawn for, and the suggestion, for the accept
    /// commands.
    suggestion: Option<(String, String)>,
    /// The column where the suggestion on screen starts, if one is showing.
    shown_at: Option<usize>,
    /// How many rows below the cursor the message on screen is, if one is
    /// showing.
    message_rows: Option<usize>,
    /// Set when bash may have printed while the line was being edited: the
    /// cursor may not be where readline thinks it is, so readline draws the
    /// rest of the line on its own. Cleared when the next line starts.
    displaced: bool,
    /// Set by `enable -d`: the readline commands stay registered but only run
    /// readline's own.
    unloaded: bool,
    checker: Checker,
    /// The line last checked for syntax errors, and what the check said.
    checked: Option<(String, Status)>,
    /// The bytes of the line underlined on screen.
    underlined: Option<Range<usize>>,
    /// The line typing paused on: an error in it is underlined.
    paused_on: Option<String>,
    /// Set when a new error waits for a pause before it is underlined.
    wants_pause: bool,
    /// The column a run of Up and Down keeps to.
    goal_column: Option<usize>,
    /// Whether the last vertical command that deferred to history did a
    /// search, so a further one continues it instead of starting fresh.
    search_continues: bool,
}

static REGISTER: Once = Once::new();

/// Start and end of a synchronized update (DEC private mode 2026): a terminal
/// that supports it shows nothing written in between until the end, so a key's
/// erase, readline's echo and inkline's repaint appear as one frame instead of
/// flickering. Other terminals ignore both.
const BEGIN_UPDATE: &[u8] = b"\x1b[?2026h";
const END_UPDATE: &[u8] = b"\x1b[?2026l";

/// How long typing must pause before a new syntax error is underlined.
const PAUSE_MS: c_int = 150;

/// `C-g` as a key. While Lisp runs, a `C-c` comes back as this key.
pub const CTRL_G: c_int = 7;
/// `C-c` as a key, where the terminal does not turn it into `SIGINT`.
pub const CTRL_C: c_int = 3;

thread_local! {
    static ORIGINALS: Cell<Originals> = Cell::default();
    /// Whether a synchronized update is open.
    static UPDATING: Cell<bool> = const { Cell::new(false) };
    /// bash's completion function, which `complete` calls.
    static COMPLETION: Cell<Option<ffi::CompletionFn>> = const { Cell::new(None) };
    /// The message to show under the line until the next key. Kept apart
    /// from `State` so Lisp can set it while it runs.
    static MESSAGE: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Set when `C-c` came while Lisp was reading a key; `run_lisp_key`
    /// hands it on to readline and bash once the command has returned.
    static INTERRUPTED_IN_LISP: Cell<bool> = const { Cell::new(false) };
    /// Set when another signal interrupted inkline's wait for a key while
    /// Lisp was reading one: the signal, as `ffi::Wait::Signal` gives it.
    /// Readline handles it once the key is read; `run_lisp_key` runs bash's
    /// part (traps, `read -e -t` timing out in bash 5.0) once the command
    /// has returned.
    static SIGNAL_IN_LISP: Cell<Option<c_int>> = const { Cell::new(None) };
    /// Set when shell code that a readline command run from Lisp ran jumped
    /// to bash's top level: the value it jumped with. `run_lisp_key` makes
    /// the jump once Lisp has stopped.
    static SHELL_JUMP: Cell<Option<c_int>> = const { Cell::new(None) };
    /// Set once readline reads a line of its own (`read -e` in shell code)
    /// under a readline command that Lisp runs, until that command returns.
    static NESTED_LINE: Cell<bool> = const { Cell::new(false) };
    /// Set when a key typed after a `C-c` bash has yet to act on is waiting
    /// or has been read; cleared at the first wait after bash acts on it.
    /// Those keys and the ones after them go to the interrupted line, as
    /// with readline's own reader.
    static KEYS_AFTER_C_C: Cell<bool> = const { Cell::new(false) };
    /// Set when a panic turned inkline off, until `enable -f` or `inkline on`.
    static PANICKED: Cell<bool> = const { Cell::new(false) };
    static STATE: RefCell<State> = RefCell::new(State {
        enabled: false,
        lexer: Lexer::new(),
        paths: PathCache::default(),
        suggestion: None,
        shown_at: None,
        message_rows: None,
        displaced: false,
        unloaded: false,
        checker: Checker::new(),
        checked: None,
        underlined: None,
        paused_on: None,
        wants_pause: false,
        goal_column: None,
        search_continues: false,
    });
}

fn originals() -> Originals {
    ORIGINALS.get()
}

pub fn load() {
    guard(
        || {
            PANICKED.set(false);
            // Readline has no way to remove a command, and the library stays
            // loaded (see build.rs), so the commands are registered once per
            // process.
            REGISTER.call_once(|| {
                // One short line instead of Rust's panic report in the prompt.
                std::panic::set_hook(Box::new(|_| {
                    let _ = writeln!(std::io::stderr(), "\ninkline: internal error, turned off");
                }));
                ffi::add_command(c"accept-suggestion-char", accept_suggestion_char);
                ffi::add_command(c"accept-suggestion-word", accept_suggestion_word);
                ffi::add_command(c"accept-suggestion", accept_suggestion);
                ffi::add_command(c"insert-pair", insert_pair);
                ffi::add_command(c"insert-close", insert_close);
                ffi::add_command(c"delete-pair", delete_pair);
                ffi::add_command(c"accept-or-newline", multiline::accept_or_newline);
                ffi::add_command(c"insert-newline", multiline::insert_newline);
                ffi::add_command(
                    c"previous-line-or-history",
                    multiline::previous_line_or_history,
                );
                ffi::add_command(c"next-line-or-history", multiline::next_line_or_history);
                ffi::add_command(
                    c"previous-line-or-search",
                    multiline::previous_line_or_search,
                );
                ffi::add_command(c"next-line-or-search", multiline::next_line_or_search);
                ffi::add_command(c"line-start", multiline::line_start);
                ffi::add_command(c"line-end", multiline::line_end);
                ffi::add_command(c"kill-to-line-end", multiline::kill_to_line_end);
                ffi::add_command(c"kill-to-line-start", multiline::kill_to_line_start);
                ffi::add_command(c"comment-lines", multiline::comment_lines);
                ffi::add_command(c"accept-as-is", accept_as_is);
                ffi::add_command(c"inkline-lisp-key", crate::lisp::commands::SHARED);
            });
            crate::lisp::start_for_shell();
            STATE.with_borrow_mut(|s| s.unloaded = false);
            enable();
        },
        || (),
    );
}

pub fn unload() {
    guard(
        || {
            disable();
            STATE.with_borrow_mut(|s| s.unloaded = true);
        },
        || (),
    );
}

pub fn builtin(args: &[String]) -> c_int {
    guard(|| run_builtin(args), || ffi::EXECUTION_FAILURE)
}

fn run_builtin(args: &[String]) -> c_int {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] | ["status"] => {
            let on = STATE.with_borrow(|s| s.enabled);
            match writeln!(
                std::io::stdout(),
                "inkline: {}\n{}",
                if on { "on" } else { "off" },
                crate::lisp::init::status_line()
            ) {
                Ok(()) => ffi::EXECUTION_SUCCESS,
                Err(_) => ffi::EXECUTION_FAILURE,
            }
        }
        ["on"] => {
            PANICKED.set(false);
            crate::lisp::start_again_if_broken();
            enable();
            ffi::EXECUTION_SUCCESS
        }
        ["off"] => {
            disable();
            ffi::EXECUTION_SUCCESS
        }
        ["eval", expr] => {
            crate::lisp::start_again_if_broken();
            lisp_status(crate::lisp::eval(expr).map(|value| {
                if let Some(text) = value {
                    let _ = writeln!(std::io::stdout(), "{text}");
                }
            }))
        }
        ["load", file] => {
            crate::lisp::start_again_if_broken();
            lisp_status(crate::lisp::load(file))
        }
        ["keys"] => {
            for line in crate::lisp::keys::lines() {
                let _ = writeln!(std::io::stdout(), "{line}");
            }
            ffi::EXECUTION_SUCCESS
        }
        ["reload"] => match crate::lisp::reload() {
            Ok(true) => ffi::EXECUTION_SUCCESS,
            Ok(false) => ffi::EXECUTION_FAILURE,
            Err(e) => {
                let _ = writeln!(std::io::stderr(), "inkline: {e}");
                ffi::EXECUTION_FAILURE
            }
        },
        _ => {
            let _ = writeln!(
                std::io::stderr(),
                "inkline: usage: inkline [on|off|status|keys|load FILE|eval EXPR|reload]"
            );
            ffi::EX_USAGE
        }
    }
}

/// The exit status for a Lisp run. Prints its error, then each bad setting
/// value not yet reported, to stderr.
fn lisp_status(result: Result<(), String>) -> c_int {
    let status = match result {
        Ok(()) => ffi::EXECUTION_SUCCESS,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "inkline: {e}");
            ffi::EXECUTION_FAILURE
        }
    };
    for line in crate::lisp::settings::problems() {
        let _ = writeln!(std::io::stderr(), "inkline: {line}");
    }
    status
}

/// Installs the key-reading, pre-input and terminal-restore hooks. The drawing
/// hook is installed by `getc` once a key arrives: readline changes how it sets
/// up the terminal and how it redraws after a resize whenever a custom drawing
/// function is in place (it skips the terminal description, losing cursor
/// movement and bracketed paste, and redraws below the old line), so inkline's
/// is only in place while a key's command runs.
fn enable() {
    STATE.with_borrow_mut(|s| {
        if s.enabled {
            return;
        }
        ORIGINALS.set(Originals {
            redisplay: ffi::redisplay_function(),
            deprep: ffi::deprep_function(),
            pre_input: ffi::pre_input_hook(),
            ..originals()
        });
        use_own_key_reader();
        ffi::set_deprep_function(Some(deprep_terminal as ffi::VoidFn));
        ffi::set_pre_input_hook(Some(pre_input as ffi::HookFn));
        s.enabled = true;
    });
}

/// Puts readline's functions back. Also the recovery after a panic, so it must
/// not depend on `STATE` being borrowable.
fn disable() {
    let _ = catch_unwind(erase_suggestion_and_message);
    let _ = MESSAGE.try_with(|m| m.replace(None));
    unwrap_completion();
    end_update();
    ffi::flush_out();
    let enabled = STATE.try_with(|s| {
        s.try_borrow_mut().map(|mut s| {
            s.suggestion = None;
            std::mem::replace(&mut s.enabled, false)
        })
    });
    // Restore unless the state says inkline was already off. While Lisp
    // runs, inkline's key reader stays until the Lisp command's key has
    // returned (see `run_lisp_key`).
    if !matches!(enabled, Ok(Ok(false))) {
        let orig = originals();
        ffi::set_redisplay_function(orig.redisplay);
        if !crate::lisp::RUNNING.load(Ordering::Relaxed) {
            ffi::set_getc_function(orig.getc);
        }
        ffi::set_deprep_function(orig.deprep);
        ffi::set_pre_input_hook(orig.pre_input);
    }
}

/// Runs the line as it is.
extern "C" fn accept_as_is(count: c_int, key: c_int) -> c_int {
    guard(
        || ffi::accept_line(count, key),
        || ffi::accept_line(count, key),
    )
}

/// Whether a panic turned inkline off since the last `enable -f` or
/// `inkline on`.
fn panicked() -> bool {
    PANICKED.try_with(Cell::get).unwrap_or(true)
}

/// Runs a Lisp command's key with `run`. After `enable -d` or a panic, the
/// key instead runs what it had before inkline bound it (`slot` as for
/// `keys::saved_binding`). Holds no borrow of `STATE` while Lisp runs.
/// inkline's key reader is in place while the command runs, also when
/// inkline is off, so a `C-c` while the command reads a key waits for
/// Lisp to stop. When inkline's drawing function is not in place (inkline
/// is off), the command's message is printed once it returns. A `C-c`
/// that came while the command ran, or bash's part of another signal that
/// came while it read a key, is handled last, as it is after inkline's
/// wait for a key, and then a jump to bash's top level that shell code
/// run from Lisp made; only once Lisp has stopped running, so not when
/// this command's key was run by a readline command that Lisp called.
pub fn run_lisp_key(
    slot: Option<usize>,
    count: c_int,
    key: c_int,
    run: impl FnOnce() -> c_int,
) -> c_int {
    // Unreadable state counts as unloaded.
    let unloaded = STATE
        .try_with(|s| s.try_borrow().map(|s| s.unloaded))
        .map_or(true, |s| s.unwrap_or(true));
    if unloaded || panicked() {
        return run_saved_binding(slot, count, key);
    }
    use_own_key_reader();
    let result = guard(
        || {
            let result = run();
            if !drawing() {
                print_message_above();
            }
            result
        },
        // The panic's notice ended on a new row: readline draws the line
        // again under it.
        || {
            ffi::on_new_line();
            0
        },
    );
    if !crate::lisp::RUNNING.load(Ordering::Relaxed) {
        // Shell code the command ran may have switched inkline on or off;
        // inkline keeps its reader only while it is on.
        if !is_on() {
            ffi::set_getc_function(originals().getc);
        }
        // Bash may jump from here back to a new prompt; nothing in this
        // frame needs dropping.
        let jump = SHELL_JUMP.take();
        // A `C-c` that came while Lisp ran but read no key is still waiting
        // in readline, which would only echo it, as inkline's key reader
        // does not call bash's hook: it is handed on here too. So is any
        // other signal that came while Lisp read a key (bash 5.0 times out
        // `read -e -t` in its hook).
        let interrupted = INTERRUPTED_IN_LISP.replace(false) || ffi::hold_interrupt();
        if interrupted {
            ffi::release_interrupt();
        }
        match SIGNAL_IN_LISP.take() {
            Some(signal) => {
                guard(|| before_signal(signal), || ());
                ffi::handle_interrupted_wait();
            }
            None if interrupted => ffi::handle_interrupted_wait(),
            None => {}
        }
        if let Some(value) = jump {
            ffi::jump_to_shell_top_level(value);
        }
    }
    result
}

/// Runs `f`, which runs a readline command for Lisp with
/// `ffi::call_command`. Keys read for a line of its own under that command
/// get their signals as usual (see `getc`).
pub fn in_readline_command<R>(f: impl FnOnce() -> R) -> R {
    let outer = NESTED_LINE.replace(false);
    let result = f();
    NESTED_LINE.set(outer);
    result
}

/// Notes a jump to bash's top level with `value` that shell code run from
/// Lisp made, for `run_lisp_key` to make once Lisp has stopped.
pub fn note_shell_jump(value: c_int) {
    SHELL_JUMP.set(Some(value));
}

/// Whether a `C-c` or a jump to bash's top level is waiting for Lisp to
/// stop: the running Lisp command then quits, and no more readline
/// commands or questions run from it.
pub fn lisp_must_stop() -> bool {
    INTERRUPTED_IN_LISP.get() || SHELL_JUMP.get().is_some()
}

/// Puts inkline's key reader in place, and the one it replaces in
/// `ORIGINALS`. Does nothing when inkline's reader is already in place, so
/// its own reader never becomes the original, which it calls to read a
/// key.
fn use_own_key_reader() {
    let ours = getc as ffi::GetcFn;
    let current = ffi::getc_function();
    if current.is_some_and(|f| std::ptr::fn_addr_eq(f, ours)) {
        return;
    }
    ORIGINALS.set(Originals {
        getc: current,
        ..originals()
    });
    ffi::set_getc_function(Some(ours));
}

/// Whether inkline is on. State that cannot be read counts as on.
fn is_on() -> bool {
    STATE
        .try_with(|s| s.try_borrow().map_or(true, |s| s.enabled))
        .unwrap_or(true)
}

/// Runs what this key had before inkline first bound it: the saved
/// command or macro text, or the bell when nothing was saved. The saved
/// command runs last, with nothing in this frame to drop, as readline may
/// jump from it back to its top level.
fn run_saved_binding(slot: Option<usize>, count: c_int, key: c_int) -> c_int {
    use crate::lisp::keys::{self, Fallback};
    match guard(|| keys::saved_binding(slot), || Fallback::Nothing) {
        Fallback::Command(f) => ffi::run_command(f, count, key),
        Fallback::Macro(text) => {
            ffi::push_macro_input(text);
            0
        }
        Fallback::Nothing => {
            ffi::ding();
            0
        }
    }
}

/// Whether inkline's drawing function is in place.
fn drawing() -> bool {
    let ours = redisplay as ffi::VoidFn;
    ffi::redisplay_function().is_some_and(|f| std::ptr::fn_addr_eq(f, ours))
}

/// Draws the line and the message now, for a Lisp command that waits for a
/// key: the message goes under the line, or where inkline does not draw,
/// on a row of its own above the line.
pub fn show_message_now() {
    if !drawing() {
        print_message_above();
    }
    repaint_now();
}

/// Shows `text` under the line from the next draw until the next key; a
/// later message replaces it. Only the text before the first control
/// character other than a tab shows. Touches nothing but the message, so
/// Lisp can call it while it runs.
pub fn show_message(text: &str) {
    let end = text
        .find(|c: char| c.is_control() && c != '\t')
        .unwrap_or(text.len());
    let text = &text[..end];
    MESSAGE.set(Some(text.to_owned()).filter(|t| !t.is_empty()));
}

/// Takes away the message `show_message` set.
pub fn clear_message() {
    MESSAGE.set(None);
}

/// Prints the message waiting to be shown, if any, on a row of its own
/// below the line, and leaves readline to draw the prompt and line again
/// below it.
fn print_message_above() {
    if let Some(text) = MESSAGE.take() {
        ffi::new_line_for_message();
        ffi::write_queued(text.as_bytes());
        // Clears the rest of the row.
        ffi::write_queued(b"\x1b[K");
        ffi::new_line_for_message();
    }
}

/// Runs `f`. If it panics, turns inkline off and runs `on_panic` instead, so a
/// bug never unwinds into readline and never kills the shell.
fn guard<R>(f: impl FnOnce() -> R, on_panic: impl FnOnce() -> R) -> R {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
            let _ = PANICKED.try_with(|p| p.set(true));
            disable();
            on_panic()
        }
    }
}

/// Readline's key reader. inkline waits for the key itself, so that:
///
/// - once the key arrives, the suggestion is erased before readline runs the
///   key's command, so Enter, `C-o`, a completion listing or `C-c` never leave
///   grey text behind;
/// - after a signal interrupts the wait and bash returns to it (a window
///   resize, a background job ending), inkline repaints the line straight away;
///   for a resize, readline has redrawn it first. If bash may have printed
///   while handling the signal, readline draws the rest of the line instead.
///
/// Readline's own drawing function is in place while waiting, so a resize is
/// redrawn the way readline expects; inkline's is installed once the key
/// arrives.
///
/// While Lisp runs (a Lisp command reading a key), bash must not jump from a
/// signal to a new prompt: that would skip the Lisp and Rust frames. A
/// `C-c` then comes back as `C-g`, and is handed on once the command has
/// returned (see `run_lisp_key`); readline handles other signals after the
/// key, and bash's part of them waits for the command too. A line of its
/// own that shell code run from Lisp reads (`read -e`) gets its signals as
/// usual, from its first key until the readline command that ran the shell
/// code returns: bash's jump from there stops at `ffi::call_command` (see
/// `in_readline_command`).
///
/// A signal that came before the wait (while readline redrew the line or ran
/// a command) did not interrupt it, and is acted on before the wait, unless a
/// key is typed ahead or, for a `C-c`, keys typed after it have already been
/// read (see `KEYS_AFTER_C_C`).
extern "C" fn getc(stream: *mut libc::FILE) -> c_int {
    guard(
        || {
            ffi::set_redisplay_function(originals().redisplay);
            // Nothing may be held back while waiting for a key.
            end_update();
            ffi::flush_out();
        },
        || (),
    );
    let running = crate::lisp::RUNNING.load(Ordering::Relaxed);
    if running && ffi::reading_command_key() {
        NESTED_LINE.set(true);
    }
    let in_lisp = running && !NESTED_LINE.get();
    let key = loop {
        if take_interrupt(in_lisp) {
            break Some(CTRL_G);
        }
        let pause = guard(
            || STATE.with_borrow_mut(|s| std::mem::take(&mut s.wants_pause)),
            || false,
        )
        .then_some(PAUSE_MS);
        // A signal that came before the wait did not interrupt it: it is
        // handled first, or it would wait for a key. When a key is typed
        // ahead, or keys typed after a `C-c` have already been read, it waits
        // for the key as readline's own reader does.
        let typed_ahead = ffi::key_waiting();
        KEYS_AFTER_C_C.set(ffi::interrupted() && (typed_ahead || KEYS_AFTER_C_C.get()));
        let signal = (!in_lisp && !typed_ahead && !KEYS_AFTER_C_C.get())
            .then(ffi::signal_before_wait)
            .flatten();
        match signal.unwrap_or_else(|| ffi::wait_for_input(stream, pause)) {
            ffi::Wait::Ready | ffi::Wait::Error => break None,
            // Typing paused with a new error on the line: underline it.
            ffi::Wait::Paused => guard(
                || {
                    STATE.with_borrow_mut(|s| s.paused_on = ffi::line());
                    begin_update();
                    erase_suggestion_and_message();
                    draw();
                    end_update();
                    ffi::flush_out();
                },
                || (),
            ),
            // Readline's reader stops on these and leaves the signal to be
            // handled after the read.
            ffi::Wait::Signal(libc::SIGHUP | libc::SIGTERM) => {
                guard(
                    || {
                        erase_suggestion_and_message();
                        ffi::flush_out();
                    },
                    || (),
                );
                return ffi::read_error();
            }
            // A `C-c` is taken at the top of the loop.
            ffi::Wait::Signal(signal) if in_lisp => SIGNAL_IN_LISP.set(Some(signal)),
            ffi::Wait::Signal(signal) => {
                // Readline's redraw after a resize and inkline's repaint go out
                // as one update. Other signals are not held back: bash may jump
                // from them to a new prompt and run commands there. Bash may
                // also print while it handles a signal, which can move the
                // cursor: then readline draws the rest of the line on its own.
                guard(
                    || {
                        if signal == libc::SIGWINCH && !ffi::signal_may_print(signal) {
                            begin_update();
                        }
                        before_signal(signal);
                    },
                    || (),
                );
                // Bash may jump out of here back to a new prompt; nothing in
                // this frame needs dropping.
                ffi::handle_interrupted_wait();
                guard(
                    || {
                        draw();
                        end_update();
                        ffi::flush_out();
                    },
                    || (),
                );
            }
        }
    };
    guard(
        || {
            // The update opens here only to hide the erase; otherwise it opens
            // when readline redraws. The key's command may run a `bind -x`
            // command, whose output a terminal would hold back while the
            // update is open.
            if STATE.with_borrow(|s| s.shown_at.is_some() || s.message_rows.is_some()) {
                begin_update();
            }
            erase_suggestion_and_message();
            clear_message();
        },
        || (),
    );
    // Readline handles a signal it caught just before it reads the key and
    // again once the key is back, so a `C-c` is taken before and after the
    // read too (the key read is then dropped). One that comes after the
    // check at the top of the loop but before the wait starts is only seen
    // once a key arrives.
    let key = match key {
        Some(key) => key,
        None if take_interrupt(in_lisp) => CTRL_G,
        None => {
            let key = ffi::call_getc(originals().getc, stream);
            if take_interrupt(in_lisp) {
                CTRL_G
            } else {
                // Bash has a `C-c` it has yet to act on, and readline's
                // reader read this key after acting on it: as with readline's
                // own reader, this key and the ones after it go to the
                // interrupted line.
                if ffi::interrupted() {
                    KEYS_AFTER_C_C.set(true);
                }
                key
            }
        }
    };
    guard(
        || {
            if STATE.with_borrow(|s| s.enabled) {
                ffi::set_redisplay_function(Some(redisplay as ffi::VoidFn));
            }
        },
        || (),
    );
    key
}

/// Gets the line ready for bash to handle `signal`, a value from
/// `ffi::Wait::Signal`: bash may print while it does, which can move the
/// cursor, and readline then draws the rest of the line on its own.
fn before_signal(signal: c_int) {
    let may_print = ffi::signal_may_print(signal);
    if may_print {
        STATE.with_borrow_mut(|s| {
            s.displaced = true;
            s.suggestion = None;
        });
    }
    erase_suggestion_and_message();
    if may_print {
        // What bash prints must not wait behind an open update.
        end_update();
    }
    ffi::flush_out();
}

/// While Lisp runs, takes a `C-c` readline caught and has not handled yet,
/// and notes it in `INTERRUPTED_IN_LISP`. Returns whether there was one.
fn take_interrupt(in_lisp: bool) -> bool {
    let taken = in_lisp && ffi::hold_interrupt();
    if taken {
        INTERRUPTED_IN_LISP.set(true);
    }
    taken
}

/// Readline calls this at the start of each line, once it has drawn the prompt
/// and before the first key. A line it filled in by then (the next history
/// entry after `C-o`, `read -e -i`) was drawn with readline's own drawing
/// function, so inkline paints it here.
extern "C" fn pre_input() -> c_int {
    let result = ffi::call_hook(originals().pre_input);
    guard(
        || {
            STATE.with_borrow_mut(|s| {
                s.displaced = false;
                // Aliases and `extglob` may have changed since the last line.
                s.checked = None;
                s.underlined = None;
                s.paused_on = None;
                s.wants_pause = false;
                s.goal_column = None;
                s.search_continues = false;
            });
            wrap_completion();
            draw();
            ffi::flush_out();
        },
        || (),
    );
    result
}

/// Readline calls this when it returns a line. Enter can reach readline without
/// `getc` (typed ahead in one burst); readline's final update has then left the
/// cursor at the start of the row below the line, so the suggestion is erased
/// there: one row up, or on the cursor's own row when the line exactly filled
/// its last row and the suggestion started at column 0 of the next. Clearing to
/// the end of the screen also clears the suggestion's other rows and the
/// message, which is on the cursor's row or the row below it. Readline's own
/// drawing function goes back in place, so a later terminal setup (such as
/// after `TERM` changes) sees it.
extern "C" fn deprep_terminal() {
    guard(
        || {
            clear_message();
            let (shown_at, message_rows) =
                STATE.with_borrow_mut(|s| (s.shown_at.take(), s.message_rows.take()));
            if (shown_at.is_some() || message_rows.is_some()) && ffi::line_done() {
                let erase = match shown_at {
                    Some(col) if col > 0 => format!("\x1b[A\x1b[{}G\x1b[J\x1b[B\r", col + 1),
                    _ => "\r\x1b[J".to_string(),
                };
                ffi::write_queued(erase.as_bytes());
            }
            ffi::set_redisplay_function(originals().redisplay);
        },
        || (),
    );
    end_update();
    ffi::flush_out();
    ffi::call_deprep(originals().deprep);
}

/// Erases the suggestion and the message the last draw left on screen, with
/// the cursor where that draw left it. A suggestion starts at the cursor, at
/// the end of the line, with the message and any other suggestion rows below:
/// clearing from the cursor to the end of the screen erases them all. A
/// message alone is erased from the start of its row down, and the cursor
/// comes back: the line may go on after the cursor, and readline would not
/// draw it again.
fn erase_suggestion_and_message() {
    let (shown, message_rows) =
        STATE.with_borrow_mut(|s| (s.shown_at.take(), s.message_rows.take()));
    if shown.is_some() {
        ffi::write_queued(b"\x1b[J");
    } else if let Some(rows) = message_rows {
        ffi::write_queued(format!("\x1b7\x1b[{rows}B\r\x1b[J\x1b8").as_bytes());
    }
}

fn begin_update() {
    if !UPDATING.replace(true) {
        ffi::write_queued(BEGIN_UPDATE);
    }
}

fn end_update() {
    if UPDATING.replace(false) {
        ffi::write_queued(END_UPDATE);
    }
}

extern "C" fn redisplay() {
    guard(
        || {
            begin_update();
            erase_suggestion_and_message();
        },
        || (),
    );
    ffi::call_redisplay(originals().redisplay);
    guard(draw, || ());
    end_update();
    ffi::flush_out();
}

/// Setups where readline's own drawing is left alone: readline does not draw
/// the line, or inkline cannot tell where readline put each character.
fn left_to_readline() -> bool {
    !ffi::echoing()
        || ffi::variable_on(c"horizontal-scroll-mode")
        // The mode string and the modified-line mark are drawn before the
        // prompt but are not part of `rl_display_prompt`.
        || ffi::variable_on(c"show-mode-in-prompt")
        || ffi::variable_on(c"mark-modified-lines")
        // Without cursor-up, readline scrolls long lines sideways.
        || !ffi::terminal_can_move_up()
        // Outside UTF-8, readline counts bytes and draws them as `\303`.
        || !ffi::utf8_locale()
        // Readline 8.1+ highlights a search match or pasted text itself.
        || ffi::region_active()
}

/// Whether readline draws a newline in the line as a line break. With
/// `horizontal-scroll-mode`, or on a terminal it cannot move the cursor up
/// on, it draws `^J` instead.
fn shows_newlines() -> bool {
    !ffi::variable_on(c"horizontal-scroll-mode") && ffi::terminal_can_move_up()
}

/// Draws the line now, with the drawing function in place for this key.
fn repaint_now() {
    ffi::call_redisplay(ffi::redisplay_function());
}

/// Repaints the line readline just drew, in colour, with a suggestion after it
/// when the cursor is at the end. Outside plain editing (a count prefix, a
/// search) the stored suggestion is kept, so `M-3 C-f` can still take from it;
/// `accept` checks it against the line before using it.
fn draw() {
    if !repaint_line() {
        // Readline's plain drawing has no underline, and an error that shows
        // up later, even the same one, waits for a new pause.
        STATE.with_borrow_mut(|s| {
            s.underlined = None;
            s.paused_on = None;
        });
    }
    // A message that could not go under the line goes above it, once, and
    // the line is drawn again below it.
    if STATE.with_borrow(|s| s.message_rows.is_none()) && MESSAGE.with_borrow(Option::is_some) {
        print_message_above();
        ffi::call_redisplay(originals().redisplay);
        draw();
    }
}

/// Does `draw`'s work. Returns whether it repainted the line.
fn repaint_line() -> bool {
    let editing = ffi::normal_editing();
    if editing {
        STATE.with_borrow_mut(|s| s.suggestion = None);
    }
    let Some(line) = ffi::line() else {
        return false;
    };
    let message = MESSAGE.with_borrow(Clone::clone);
    if (line.is_empty() && message.is_none())
        || STATE.with_borrow(|s| s.displaced)
        || left_to_readline()
    {
        return false;
    }
    let Some(prompt_width) = render::prompt_width(&ffi::display_prompt()) else {
        return false;
    };
    let point = ffi::point();
    let (rows, cols) = ffi::screen_size();
    let colors = crate::lisp::settings::colors();
    let suggestion_lines = crate::lisp::settings::suggestion_lines();
    let path = ffi::shell_variable("PATH").unwrap_or_default();
    let suggestion = if editing && !line.is_empty() && point == line.len() {
        ffi::history_find_map(|entry| suggest::rest(&line, entry).map(str::to_owned))
    } else {
        None
    };
    STATE.with_borrow_mut(|s| {
        let error = error_to_underline(s, &line, point);
        let paths = &mut s.paths;
        let spans = s.lexer.spans(&line, |word| {
            !commands::is_plain(word) || commands::exists(word, &path, paths, ffi::known_to_bash)
        });
        let repaint = Repaint {
            prompt_width,
            line: &line,
            point,
            spans: &spans,
            colors: &colors,
            suggestion: suggestion.as_deref(),
            suggestion_lines,
            error: error.clone(),
            message: message.as_deref(),
            rows,
            cols,
        };
        let Some(out) = render::build(&repaint) else {
            return false;
        };
        ffi::write_queued(&out.bytes);
        s.shown_at = out.suggestion_col;
        s.message_rows = out.message_rows;
        s.underlined = error;
        if let (Some(_), Some(rest)) = (out.suggestion_col, suggestion) {
            s.suggestion = Some((line.clone(), rest));
        }
        true
    })
}

/// What bash would make of `line`, checked once for each text of the line.
fn status_of(s: &mut State, line: &str) -> Status {
    if let Some((checked, status)) = &s.checked
        && checked == line
    {
        return status.clone();
    }
    let status = s.checker.check(line, ffi::extglob());
    s.checked = Some((line.to_owned(), status.clone()));
    status
}

/// The bytes of `line` to underline as a syntax error. A new error waits for
/// a pause in typing and asks `getc` for one; an error already underlined
/// stays. The word the cursor is at the end of is being typed, so it is never
/// underlined.
fn error_to_underline(s: &mut State, line: &str, point: usize) -> Option<Range<usize>> {
    if !ffi::reading_command() {
        return None;
    }
    let Status::Wrong(range) = status_of(s, line) else {
        return None;
    };
    let word = syntax::word_around(line, range);
    if word.is_empty() || word.end == point {
        return None;
    }
    if s.underlined.as_ref() == Some(&word) || s.paused_on.as_deref() == Some(line) {
        Some(word)
    } else {
        s.wants_pause = true;
        None
    }
}

extern "C" fn accept_suggestion_char(count: c_int, key: c_int) -> c_int {
    accept(count, key, suggest::chars, ffi::forward_char)
}

extern "C" fn accept_suggestion_word(count: c_int, key: c_int) -> c_int {
    accept(count, key, suggest::words, ffi::forward_word)
}

extern "C" fn accept_suggestion(count: c_int, key: c_int) -> c_int {
    accept(count, key, suggest::all, multiline::end_of_line)
}

/// Inserts the part of the suggestion `take` picks. Without a suggestion for
/// the current line and cursor, runs readline's own command instead.
fn accept(
    count: c_int,
    key: c_int,
    take: fn(&str, usize) -> &str,
    fallback: fn(c_int, c_int) -> c_int,
) -> c_int {
    guard(
        || {
            let suggestion = STATE.with_borrow(|s| s.suggestion.clone());
            match suggestion {
                Some((line, rest))
                    if count > 0
                        && ffi::point() == line.len()
                        && ffi::line().as_deref() == Some(line.as_str()) =>
                {
                    ffi::insert_text(take(&rest, count as usize));
                    0
                }
                _ => fallback(count, key),
            }
        },
        || fallback(count, key),
    )
}

/// Runs a pairing command: `edit` gets the line and cursor and returns whether
/// it handled the key. Otherwise `fallback` runs, the readline command the key
/// normally runs. It also runs after `enable -d`, after a panic, and while echo
/// is off, where the user cannot see a closer inkline would add.
fn pairing(
    count: c_int,
    key: c_int,
    fallback: fn(c_int, c_int) -> c_int,
    edit: impl FnOnce(&str, usize) -> bool,
) -> c_int {
    guard(
        || {
            if !STATE.with_borrow(|s| s.unloaded)
                && ffi::echoing()
                && let Some(line) = ffi::line()
                && edit(&line, ffi::point())
            {
                0
            } else {
                fallback(count, key)
            }
        },
        || fallback(count, key),
    )
}

fn typed_char(key: c_int) -> Option<char> {
    u32::try_from(key).ok().and_then(char::from_u32)
}

extern "C" fn insert_pair(count: c_int, key: c_int) -> c_int {
    pairing(count, key, ffi::self_insert, |line, point| {
        let Some(typed) = typed_char(key) else {
            return false;
        };
        let context = STATE.with_borrow_mut(|s| s.lexer.context_at(line, point));
        match pairs::open(line, point, typed, ffi::explicit_count(), context) {
            Action::InsertPair(open, close) => {
                ffi::begin_undo_group();
                ffi::insert_text(&format!("{open}{close}"));
                ffi::set_point(point + open.len_utf8());
                ffi::end_undo_group();
                true
            }
            Action::Skip => {
                ffi::set_point(point + typed.len_utf8());
                true
            }
            Action::Fallback | Action::DeletePair => false,
        }
    })
}

extern "C" fn insert_close(count: c_int, key: c_int) -> c_int {
    pairing(count, key, ffi::self_insert, |line, point| {
        let Some(typed) = typed_char(key) else {
            return false;
        };
        let moves_over = pairs::close(line, point, typed, ffi::explicit_count()) == Action::Skip;
        if moves_over {
            ffi::set_point(point + typed.len_utf8());
        }
        moves_over
    })
}

extern "C" fn delete_pair(count: c_int, key: c_int) -> c_int {
    pairing(count, key, ffi::rubout, |line, point| {
        let deletes = pairs::backspace(line, point, ffi::explicit_count()) == Action::DeletePair;
        if deletes {
            // Pairs are ASCII: one byte on each side of the cursor.
            ffi::delete_text(point - 1, point + 1);
            ffi::set_point(point - 1);
        }
        deletes
    })
}

/// Puts `complete` in front of bash's completion function. Bash sets its
/// function when it first sets up readline, which can be after inkline
/// loads, so this runs at the start of each line.
fn wrap_completion() {
    let ours = complete as ffi::CompletionFn;
    if let Some(f) = ffi::completion_function()
        && !std::ptr::fn_addr_eq(f, ours)
    {
        COMPLETION.set(Some(f));
        ffi::set_completion_function(Some(ours));
    }
}

/// Puts bash's completion function back.
fn unwrap_completion() {
    let ours = complete as ffi::CompletionFn;
    if ffi::completion_function().is_some_and(|f| std::ptr::fn_addr_eq(f, ours)) {
        ffi::set_completion_function(COMPLETION.get());
    }
}

/// bash's completion, with the command's newlines shown to it as `;`: bash
/// only takes `;|&{(` and backquotes as the start of a new command, so on the
/// lines after the first it would complete the wrong word. The newlines are
/// back before readline inserts the match.
extern "C" fn complete(text: *const c_char, start: c_int, end: c_int) -> *mut *mut c_char {
    let Some(original) = COMPLETION.get() else {
        return std::ptr::null_mut();
    };
    let hidden = guard(newlines_between_commands, Vec::new);
    for &i in &hidden {
        ffi::set_line_byte(i, b';');
    }
    let matches = ffi::call_completion(original, text, start, end);
    for &i in &hidden {
        ffi::set_line_byte(i, b'\n');
    }
    matches
}

/// Where the line's newlines are, outside strings and here-document bodies.
fn newlines_between_commands() -> Vec<usize> {
    if !STATE.with_borrow(|s| s.enabled && !s.unloaded) {
        return Vec::new();
    }
    let Some(line) = ffi::line().filter(|l| l.contains('\n')) else {
        return Vec::new();
    };
    let spans = STATE.with_borrow_mut(|s| s.lexer.spans(&line, |_| true));
    line.match_indices('\n')
        .map(|(i, _)| i)
        .filter(|&i| {
            !spans
                .iter()
                .any(|s| s.kind == Kind::String && s.start <= i && i < s.end)
                && !syntax::quote_open(&line[..i])
        })
        .collect()
}
