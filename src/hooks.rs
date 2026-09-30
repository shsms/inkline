//! Connects inkline to bash and readline: the `inkline` builtin, the hooks
//! readline calls, and the readline commands inkline adds.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int};
use std::io::Write;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{Once, OnceLock};
use std::time::Instant;

use crate::args::{self, Arg, CommandArgs};
use crate::bash_complete::{self, session};
use crate::commands::{self, PathCache};
use crate::ffi;
use crate::highlight;
use crate::lexer::{Kind, Lexer};
use crate::menu::{self, Item, Menu, Source};
use crate::mode_server::{self, protocol::Reply, protocol::ReplyItem};
use crate::pairs::{self, Action};
use crate::render::{self, MenuView, Repaint};
use crate::suggest;
use crate::syntax::{self, Checker, Status};

mod moving;
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
    /// How many rows below the cursor the menu on screen starts, if one is
    /// showing.
    menu_rows: Option<usize>,
    /// The completion menu for the line and cursor it was made for.
    menu: Option<Menu>,
    /// The line text `C-g` hid the menu on: no menu and no grey text show
    /// until a draw in plain editing finds another text.
    hidden_on: Option<String>,
    /// The line text and cursor of the last draw in plain editing.
    drawn_at: Option<(String, usize)>,
    /// Whether the cursor moved since the line's text last changed: unless
    /// `inkline-menu-on-move` is set, no menu shows until a draw in plain
    /// editing finds another text.
    cursor_moved: bool,
    /// The line text a history search put in the line, while it stays: such
    /// a line counts as brought back from history. Readline before 8.3 puts
    /// its history place back after a search, so `recalled` cannot tell it
    /// from that place alone.
    searched: Option<String>,
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
    /// The line and cursor typing paused on while a mode server's
    /// completion items waited for a pause: they are asked for there.
    items_paused_on: Option<(String, usize)>,
    /// Set when a mode server's completion items wait for a pause before
    /// they are asked for.
    items_want_pause: bool,
    /// The column a run of Up and Down keeps to.
    goal_column: Option<usize>,
    /// Some when the last vertical command that deferred to history ran
    /// readline's prefix search, so a further one continues it instead of
    /// starting fresh: whether a key of that search changed the line. Only
    /// then does moving between the lines keep that search going.
    search_continues: Option<bool>,
    /// The last substring search Up started. Its `going_on` says whether
    /// the run of Up and Down goes on with it, as `search_continues` does
    /// for the prefix search.
    substring: Option<crate::history_search::SubstringSearch>,
    /// Where an Up or Down key's own search, by prefix or by substring,
    /// last changed the line to an entry it found; while readline's history
    /// place has not moved from there, the line counts as found, and Up
    /// with the substring search starts a new search on it.
    found_at: Option<multiline::Found>,
    /// The command the last `menu-next` or `menu-previous` ran as its key's
    /// own command (see `menu_fallback`); None when it did anything else. It
    /// stays across other keys, but counts only while readline's last command
    /// is `menu-next` or `menu-previous`.
    menu_key_ran: Option<ffi::CommandFn>,
    /// Whether the last `menu-take` ran readline's `complete`, so a Tab
    /// right after it lists the choices as readline's second Tab does.
    completing: bool,
    /// Moving through the menu, from the first move until another key,
    /// `C-g` or Enter ends it.
    moving: Option<moving::Moving>,
}

impl State {
    /// Forgets what the last draw left after the line: the suggestion's
    /// column, and how many rows below the cursor the message starts, or the
    /// menu when there is no message. Both row counts are cleared.
    fn take_drawn(&mut self) -> (Option<usize>, Option<usize>) {
        let menu_rows = self.menu_rows.take();
        (self.shown_at.take(), self.message_rows.take().or(menu_rows))
    }

    /// Ends a run of Up and Down: the next one starts from the cursor's own
    /// column and a new history search.
    fn end_vertical_run(&mut self) {
        self.goal_column = None;
        self.search_continues = None;
        if let Some(search) = &mut self.substring {
            search.going_on = false;
        }
    }

    /// Asks again for the pause `getc` took out to wait for, new errors or
    /// notices (`errors`) and completion items (`items`), when the wait
    /// ended for something else.
    fn keep_pause_wanted(&mut self, errors: bool, items: bool) {
        self.wants_pause |= errors;
        self.items_want_pause |= items;
    }
}

static REGISTER: Once = Once::new();

/// Start and end of a synchronized update (DEC private mode 2026): a terminal
/// that supports it shows nothing written in between until the end, so a key's
/// erase, readline's echo and inkline's repaint appear as one frame instead of
/// flickering. Other terminals ignore both.
const BEGIN_UPDATE: &[u8] = b"\x1b[?2026h";
const END_UPDATE: &[u8] = b"\x1b[?2026l";

/// How long typing must pause before a new error is shown.
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
    /// Set when readline caught a `C-c` while Lisp ran and had not handled it:
    /// while Lisp read a key, while it computed, or under a readline command
    /// that Lisp ran. `after_lisp` hands it on to readline and bash once Lisp
    /// has stopped.
    static INTERRUPTED_IN_LISP: Cell<bool> = const { Cell::new(false) };
    /// Set when readline handled a `C-c` itself under a readline command that
    /// Lisp ran, and passed it on to bash (bash's `interrupt_state`) without a
    /// jump. `after_lisp` has bash act on it once Lisp has stopped.
    static INTERRUPT_PASSED_ON: Cell<bool> = const { Cell::new(false) };
    /// Set when another signal interrupted inkline's wait for a key while
    /// Lisp was reading one: the signal, as `ffi::Wait::Signal` gives it.
    /// Readline handles it once the key is read; `after_lisp` runs bash's
    /// part (traps, `read -e -t` timing out in bash 5.0) once Lisp has
    /// stopped.
    static SIGNAL_IN_LISP: Cell<Option<c_int>> = const { Cell::new(None) };
    /// Set when shell code that a readline command run from Lisp ran jumped
    /// to bash's top level: the value it jumped with. `after_lisp` makes the
    /// jump once Lisp has stopped.
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
    /// The errors of accept functions that let the line run, for
    /// `deprep_terminal` to print above the command's output. Kept only while
    /// inkline is on, as `deprep_terminal` does not run while it is off.
    static ACCEPT_ERRORS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static STATE: RefCell<State> = RefCell::new(State {
        enabled: false,
        lexer: Lexer::new(),
        paths: PathCache::default(),
        suggestion: None,
        shown_at: None,
        message_rows: None,
        menu_rows: None,
        menu: None,
        hidden_on: None,
        drawn_at: None,
        cursor_moved: false,
        searched: None,
        displaced: false,
        unloaded: false,
        checker: Checker::new(),
        checked: None,
        underlined: None,
        paused_on: None,
        wants_pause: false,
        items_paused_on: None,
        items_want_pause: false,
        goal_column: None,
        search_continues: None,
        substring: None,
        found_at: None,
        menu_key_ran: None,
        completing: false,
        moving: None,
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
                ffi::add_command(
                    c"previous-line-or-substring-search",
                    multiline::previous_line_or_substring_search,
                );
                ffi::add_command(
                    c"next-line-or-substring-search",
                    multiline::next_line_or_substring_search,
                );
                ffi::add_command(c"line-start", multiline::line_start);
                ffi::add_command(c"line-end", multiline::line_end);
                ffi::add_command(c"kill-to-line-end", multiline::kill_to_line_end);
                ffi::add_command(c"kill-to-line-start", multiline::kill_to_line_start);
                ffi::add_command(c"comment-lines", multiline::comment_lines);
                ffi::add_command(c"accept-as-is", accept_as_is);
                ffi::add_command(c"menu-next", menu_next);
                ffi::add_command(c"menu-previous", menu_previous);
                ffi::add_command(c"menu-take", menu_take);
                ffi::add_command(c"menu-take-previous", menu_take_previous);
                ffi::add_command(c"menu-hide", menu_hide);
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
            mode_server::stop_all();
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
            let mut text = format!(
                "inkline: {}\n{}\n{}\n",
                if on { "on" } else { "off" },
                crate::lisp::init::status_line(),
                crate::bash_complete::session::status_line(crate::lisp::settings::bash_completion()),
            );
            for line in mode_server::status_lines(&crate::lisp::settings::command_modes()) {
                text.push_str(&line);
                text.push('\n');
            }
            match std::io::stdout().write_all(text.as_bytes()) {
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
        // A hook that turned inkline off may have saved the line after
        // `disable` forgot it; the hook waits for the next line instead.
        crate::lisp::hooks::forget_line();
        s.enabled = true;
    });
}

/// Puts readline's functions back. Also the recovery after a panic, so it must
/// not depend on `STATE` being borrowable.
fn disable() {
    let _ = catch_unwind(erase_below);
    let _ = MESSAGE.try_with(|m| m.replace(None));
    let _ = ACCEPT_ERRORS.try_with(|e| e.try_borrow_mut().map(|mut e| e.clear()));
    crate::lisp::hooks::forget_line();
    unwrap_completion();
    session::forget();
    end_update();
    ffi::flush_out();
    let enabled = STATE.try_with(|s| {
        s.try_borrow_mut().map(|mut s| {
            s.suggestion = None;
            s.menu = None;
            s.moving = None;
            s.hidden_on = None;
            std::mem::replace(&mut s.enabled, false)
        })
    });
    // Restore unless the state says inkline was already off. While Lisp
    // runs, inkline's key reader stays until Lisp has stopped (see
    // `after_lisp`).
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

/// Runs the line as it is, even when unfinished, after the accept hook.
extern "C" fn accept_as_is(count: c_int, key: c_int) -> c_int {
    accept_line(count, key)
}

/// Runs the accept hook (`inkline-accept-functions`), then readline's
/// `accept-line`, unless a function refused the line: it then stays for
/// editing, with the refusal under it. A line the functions changed is drawn
/// again first, so the screen shows the line that runs. While a `C-c` waits for
/// bash, the line runs without the hook. After a panic, the line runs. What
/// came in while Lisp ran is handed on last (`after_lisp`), outside `guard`:
/// bash may jump from there to a new prompt.
pub(super) fn accept_line(count: c_int, key: c_int) -> c_int {
    use crate::lisp::hooks::{Accept, run_accept};
    let runs = guard(
        || {
            // A `C-c` still waiting for bash makes it throw the line away, so
            // the hook does not run for it.
            if !hooks_allowed() || ffi::interrupted() {
                return true;
            }
            before_lisp();
            match run_accept(key) {
                Accept::Refuse => false,
                Accept::Run { errors, changed } => {
                    if changed {
                        repaint_now();
                    }
                    // An accept function may have turned inkline off.
                    if is_on() {
                        ACCEPT_ERRORS.set(errors);
                    }
                    true
                }
            }
        },
        || true,
    );
    let result = if runs {
        ffi::accept_line(count, key)
    } else {
        0
    };
    after_lisp();
    result
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
/// inkline is off (`before_lisp`). When inkline's drawing function is not
/// in place (inkline is off), the command's message is printed once it
/// returns. What came in while the command ran is handed on last
/// (`after_lisp`).
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
    before_lisp();
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
    after_lisp();
    result
}

/// Gets ready for Lisp run from a key or a hook: puts inkline's key reader in
/// place, also when inkline is off, so a `C-c` while Lisp reads a key waits for
/// Lisp to stop.
pub fn before_lisp() {
    use_own_key_reader();
}

/// Hands on to bash, once Lisp has stopped, what came in while it ran:
///
/// - a `C-c`, also one readline already passed on to bash;
/// - bash's part of another signal that came while Lisp read a key, as after
///   inkline's own wait for a key;
/// - last, a jump to bash's top level that shell code run from Lisp made.
///
/// Does nothing while Lisp still runs, as when a readline command that Lisp
/// called ran this key. It may `longjmp` to bash's top level, so callers call
/// it last, outside `guard`, with nothing in their frame to drop.
pub fn after_lisp() {
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
        // Bash already has this one; only its signal hook has yet to run.
        let passed_on = INTERRUPT_PASSED_ON.replace(false);
        match SIGNAL_IN_LISP.take() {
            Some(signal) => {
                guard(|| before_signal(signal), || ());
                ffi::handle_interrupted_wait();
            }
            None if interrupted || passed_on => ffi::handle_interrupted_wait(),
            None => {}
        }
        if let Some(value) = jump {
            // The jump skips `deprep_terminal`: the line ends here.
            crate::lisp::hooks::forget_line();
            ffi::jump_to_shell_top_level(value);
        }
    }
}

/// Whether the Lisp hooks may run now: at the main prompt while bash reads a
/// command, while inkline is on, and while no Lisp runs. State that cannot be
/// read counts as off.
pub fn hooks_allowed() -> bool {
    ffi::reading_command()
        && !crate::lisp::RUNNING.load(Ordering::Relaxed)
        && !panicked()
        && STATE
            .try_with(|s| s.try_borrow().is_ok_and(|s| s.enabled && !s.unloaded))
            .unwrap_or(false)
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
/// Lisp made, for `after_lisp` to make once Lisp has stopped.
pub fn note_shell_jump(value: c_int) {
    SHELL_JUMP.set(Some(value));
}

/// Notes a `C-c` that readline handled itself under a readline command run from
/// Lisp, and passed on to bash without a jump, for `after_lisp` to have bash
/// act on once Lisp has stopped.
pub fn note_interrupt_passed_on() {
    INTERRUPT_PASSED_ON.set(true);
}

/// Takes a `C-c` that readline caught while Lisp ran and has not handled, as
/// `getc` takes one while Lisp reads a key: one that came while Lisp computed
/// (taken before Lisp runs a readline command, and after each hook function),
/// or one under a readline command run from Lisp that was not handled when the
/// command returned (as in a command substitution that `shell-expand-line`
/// runs).
pub fn take_interrupt_in_lisp() {
    take_interrupt(true);
}

/// Whether a `C-c` or a jump to bash's top level is waiting for Lisp to
/// stop: the running Lisp command or hook function then quits, and no more
/// readline commands or questions run from it.
pub fn lisp_must_stop() -> bool {
    INTERRUPTED_IN_LISP.get() || INTERRUPT_PASSED_ON.get() || SHELL_JUMP.get().is_some()
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

/// Runs what this key had before inkline first bound it (see
/// `run_fallback`).
fn run_saved_binding(slot: Option<usize>, count: c_int, key: c_int) -> c_int {
    use crate::lisp::keys::{self, Fallback};
    run_fallback(
        guard(|| keys::saved_binding(slot), || Fallback::Nothing),
        count,
        key,
    )
}

/// Runs `saved`, what a key had before inkline first bound it: the saved
/// command or macro text, or the bell when nothing was saved. The saved
/// command runs last, with nothing in the caller's frame to drop, as
/// readline may jump from it back to its top level.
fn run_fallback(saved: crate::lisp::keys::Fallback, count: c_int, key: c_int) -> c_int {
    use crate::lisp::keys::Fallback;
    match saved {
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
    let text = before_control(text);
    MESSAGE.set(Some(text.to_owned()).filter(|t| !t.is_empty()));
}

/// `text` up to its first control character other than a tab.
fn before_control(text: &str) -> &str {
    let end = text
        .find(|c: char| c.is_control() && c != '\t')
        .unwrap_or(text.len());
    &text[..end]
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
/// - once the key arrives, the suggestion, the message and the menu are
///   erased before readline runs the key's command, so Enter, `C-o`, a
///   completion listing or `C-c` never leave them behind;
/// - after a signal interrupts the wait and bash returns to it (a window
///   resize, a background job ending), inkline repaints the line straight away;
///   for a resize, readline has redrawn it first. If bash may have printed
///   while handling the signal, readline draws the rest of the line instead;
/// - a mode server's reply that comes after the redraw stopped waiting
///   for it is painted as soon as it comes, while readline waits at the
///   main prompt for the key of the next command and no Lisp runs.
///
/// Readline's own drawing function is in place while waiting, so a resize is
/// redrawn the way readline expects; inkline's is installed once the key
/// arrives.
///
/// While Lisp runs (a Lisp command reading a key), bash must not jump from a
/// signal to a new prompt: that would skip the Lisp and Rust frames. A
/// `C-c` then comes back as `C-g`, and is handed on once Lisp has stopped
/// (see `after_lisp`); readline handles other signals after the key, and
/// bash's part of them waits for Lisp too. A line of its
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
    // When the pause asked for began: a repaint for a server's reply does
    // not start it again.
    let mut pause_began = None;
    let key = loop {
        if take_interrupt(in_lisp) {
            break Some(CTRL_G);
        }
        // Whether readline reads the key of the next command in plain
        // editing. While it waits for the answer to a question, a search's
        // keys or a count, the line is not drawn as it was, and a repaint
        // would draw over what readline shows: a pause asked for then stays
        // asked for, and mode servers are not waited on.
        let plain_key = guard(
            || ffi::reading_command_key() && ffi::normal_editing(),
            || false,
        );
        // What the pause is for: a new error or notice, or a mode server's
        // completion items.
        let (for_errors, for_items) = guard(
            || {
                if plain_key {
                    STATE.with_borrow_mut(|s| {
                        (
                            std::mem::take(&mut s.wants_pause),
                            std::mem::take(&mut s.items_want_pause),
                        )
                    })
                } else {
                    (false, false)
                }
            },
            || (false, false),
        );
        let pause = (for_errors || for_items).then(|| {
            let began: &Instant = pause_began.get_or_insert_with(Instant::now);
            let waited = c_int::try_from(began.elapsed().as_millis()).unwrap_or(c_int::MAX);
            (PAUSE_MS - waited).max(0)
        });
        // The mode servers that owe inkline a reply or their first
        // line: waited on only at the main prompt, for a `plain_key`, while
        // inkline is on and no Lisp runs (a line that shell code run from
        // Lisp reads counts as Lisp running).
        let for_servers = guard(
            || !running && plain_key && ffi::reading_command() && STATE.with_borrow(|s| s.enabled),
            || false,
        );
        let servers = guard(
            || {
                if for_servers {
                    let mut fds = mode_server::waiting_fds();
                    fds.extend(session::waiting_fd());
                    fds
                } else {
                    Vec::new()
                }
            },
            Vec::new,
        );
        // A signal that came before the wait did not interrupt it: it is
        // handled first, or it would wait for a key. When a key is typed
        // ahead, or keys typed after a `C-c` have already been read, it waits
        // for the key as readline's own reader does.
        let typed_ahead = ffi::key_waiting();
        KEYS_AFTER_C_C.set(ffi::interrupted() && (typed_ahead || KEYS_AFTER_C_C.get()));
        let signal = (!in_lisp && !typed_ahead && !KEYS_AFTER_C_C.get())
            .then(ffi::signal_before_wait)
            .flatten();
        // The wait also ends when a copy of the shell's time is up, so it is
        // killed while no key comes.
        let limit = guard(session::until_deadline, || None);
        let wait = [pause, limit].into_iter().flatten().min();
        match signal.unwrap_or_else(|| ffi::wait_for_input(stream, wait, &servers)) {
            ffi::Wait::Ready | ffi::Wait::Error => break None,
            ffi::Wait::Paused => match guard(session::expire, || session::Expired::No) {
                // Typing paused with a new error or notice on the line, or
                // with a mode server's completion items still to ask for:
                // show the error or notice, and ask for the items.
                session::Expired::No => {
                    pause_began = None;
                    guard(
                        || {
                            STATE.with_borrow_mut(|s| {
                                if for_errors {
                                    s.paused_on = ffi::line();
                                }
                                if for_items {
                                    s.items_paused_on =
                                        ffi::line().map(|line| (line, ffi::point()));
                                }
                            });
                            redraw();
                        },
                        draw_below_notice,
                    );
                }
                // A copy's time ran out, not the pause: a pause asked for is
                // still waited for. Where the servers are waited on, the
                // answer that came is drawn, and so is the line of a copy
                // killed while bash's items show: its word gets none now.
                expired @ (session::Expired::Came | session::Expired::Killed) => guard(
                    || {
                        STATE.with_borrow_mut(|s| s.keep_pause_wanted(for_errors, for_items));
                        if for_servers
                            && (expired == session::Expired::Came
                                || STATE.with_borrow(shows_bash_items))
                        {
                            redraw();
                        }
                    },
                    draw_below_notice,
                ),
            },
            // A mode server sent something, or a copy of the shell answered:
            // paint the reply or the items, or say the server was turned off
            // once typing pauses. The redraw asks the servers again: a reply
            // kept for the line's arguments now answers, one for arguments no
            // longer on the line is dropped, and a request for the line as it
            // is now is sent (for completion items, once typing pauses
            // there). Without a redraw, a pause asked for is still waited for.
            ffi::Wait::Other => guard(
                || {
                    let mode = mode_server::read_waiting();
                    let bash = session::read_waiting();
                    if mode || bash {
                        redraw();
                    } else {
                        STATE.with_borrow_mut(|s| s.keep_pause_wanted(for_errors, for_items));
                    }
                },
                draw_below_notice,
            ),
            // Readline's reader stops on these and leaves the signal to be
            // handled after the read.
            ffi::Wait::Signal(libc::SIGHUP | libc::SIGTERM) => {
                guard(
                    || {
                        erase_below();
                        ffi::flush_out();
                    },
                    || (),
                );
                return ffi::read_error();
            }
            // A `C-c` is taken at the top of the loop.
            ffi::Wait::Signal(signal) if in_lisp => SIGNAL_IN_LISP.set(Some(signal)),
            ffi::Wait::Signal(signal) => {
                pause_began = None;
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
                    draw_below_notice,
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
            if STATE.with_borrow(|s| {
                s.shown_at.is_some() || s.message_rows.is_some() || s.menu_rows.is_some()
            }) {
                begin_update();
            }
            erase_below();
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
    erase_below();
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
/// function, so inkline paints it here. At the main prompt, the line-start hook
/// (`inkline-line-start-functions`) runs first; a line it changed is drawn by
/// readline again before inkline paints it. What came in while Lisp ran is
/// handed on last (`after_lisp`), outside `guard`: bash may jump from there to
/// a new prompt.
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
                s.items_paused_on = None;
                s.items_want_pause = false;
                s.goal_column = None;
                s.search_continues = None;
                s.substring = None;
                s.found_at = None;
                s.menu = None;
                s.moving = None;
                s.hidden_on = None;
                s.drawn_at = None;
                s.cursor_moved = false;
                s.searched = None;
            });
            // A new line at the main prompt drops the mode servers' replies;
            // a line read while Lisp runs is part of the line Lisp runs in.
            if ffi::reading_command() && !crate::lisp::RUNNING.load(Ordering::Relaxed) {
                mode_server::forget_replies();
                session::forget();
            }
            wrap_completion();
            crate::lisp::hooks::line_started();
            let changed = hooks_allowed() && {
                before_lisp();
                crate::lisp::hooks::run_line_start()
            };
            // As in `redisplay`: nothing is drawn before a jump to a new
            // prompt.
            if !lisp_must_stop() {
                if changed {
                    ffi::call_redisplay(originals().redisplay);
                }
                draw();
            }
            ffi::flush_out();
        },
        draw_below_notice,
    );
    after_lisp();
    result
}

/// Readline calls this when it returns a line. Enter can reach readline without
/// `getc` (typed ahead in one burst); readline's final update has then left the
/// cursor at the start of the row below the line, so the suggestion is erased
/// there: one row up, or on the cursor's own row when the line exactly filled
/// its last row and the suggestion started at column 0 of the next. Clearing to
/// the end of the screen also clears the suggestion's other rows, the message
/// and the menu, which are on the cursor's row or below it. The errors of
/// accept functions that let the line run then go there, each on a row of its
/// own, above the command's output. Readline's own drawing function goes back
/// in place, so a later terminal setup (such as after `TERM` changes) sees it.
/// The after-change hook forgets the line, unless the line that ends is one
/// read while Lisp runs.
extern "C" fn deprep_terminal() {
    guard(
        || {
            if ffi::line_done() {
                session::forget();
            }
            if !crate::lisp::RUNNING.load(Ordering::Relaxed) {
                crate::lisp::hooks::forget_line();
            }
            clear_message();
            let (shown_at, below) = STATE.with_borrow_mut(|s| {
                // A pause asked for belongs to the line that ends.
                s.wants_pause = false;
                s.items_want_pause = false;
                s.take_drawn()
            });
            if (shown_at.is_some() || below.is_some()) && ffi::line_done() {
                let erase = match shown_at {
                    Some(col) if col > 0 => format!("\x1b[A\x1b[{}G\x1b[J\x1b[B\r", col + 1),
                    _ => "\r\x1b[J".to_string(),
                };
                ffi::write_queued(erase.as_bytes());
            }
            let errors = ACCEPT_ERRORS.take();
            if ffi::line_done() {
                for error in &errors {
                    ffi::write_queued(before_control(error).as_bytes());
                    ffi::write_queued(b"\r\n");
                }
            }
            ffi::set_redisplay_function(originals().redisplay);
        },
        || (),
    );
    end_update();
    ffi::flush_out();
    ffi::call_deprep(originals().deprep);
}

/// Erases what the last draw left after the line: the suggestion, the
/// message and the menu, with the cursor where that draw left it. A
/// suggestion starts at the cursor, at the end of the line, with the message
/// and the menu below it: clearing from the cursor to the end of the screen
/// erases them all. Otherwise what is under the line is erased from the
/// start of its first row down, and the cursor comes back: the line may go
/// on after the cursor, and readline would not draw it again.
fn erase_below() {
    let (shown, below) = STATE.with_borrow_mut(State::take_drawn);
    if shown.is_some() {
        ffi::write_queued(b"\x1b[J");
    } else if let Some(rows) = below {
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

/// Readline's drawing function while inkline is on. Readline calls it once
/// after each key's command has returned, unless the key ran the line; the
/// after-change hook (`inkline-after-change-functions`) runs then, before the
/// line is drawn. Every other call comes while a command runs
/// (`RL_STATE_DISPATCHING`, also left set by readline's abort) or while
/// readline waits for a key (`RL_STATE_READCMD`), or outside plain editing.
/// What came in while Lisp ran is handed on last (`after_lisp`), outside
/// `guard`: bash may jump from there to a new prompt.
extern "C" fn redisplay() {
    let lisp_started = Cell::new(false);
    let lisp_ran = guard(
        || {
            let after_key = hooks_allowed()
                && !ffi::dispatching()
                && !ffi::reading_command_key()
                && ffi::normal_editing();
            if after_key {
                before_lisp();
                lisp_started.set(true);
            }
            let lisp_ran = after_key && crate::lisp::hooks::after_key();
            begin_update();
            erase_below();
            lisp_ran
        },
        // The panic's notice ended on a new row: readline's draw below starts
        // the line again under it.
        || {
            ffi::on_new_line();
            lisp_started.get()
        },
    );
    // Bash may have printed while Lisp ran (a shell error), and it jumps to a
    // new prompt next: the line is not drawn again, and no suggestion function
    // runs first.
    if !(lisp_ran && lisp_must_stop()) {
        ffi::call_redisplay(originals().redisplay);
        if !panicked() {
            guard(draw, draw_below_notice);
        }
    }
    end_update();
    ffi::flush_out();
    if lisp_ran {
        after_lisp();
    }
}

/// Whether the menu or the grey text on screen shows one of bash's items.
fn shows_bash_items(s: &State) -> bool {
    let bash = |item: &Item| item.source == Source::Bash;
    s.menu.as_ref().is_some_and(|m| {
        (s.menu_rows.is_some() && m.items.iter().any(bash))
            || (s.shown_at.is_some() && m.grey_item().is_some_and(bash))
    })
}

/// Draws the line again at once, as one update, while readline waits for a
/// key.
fn redraw() {
    begin_update();
    erase_below();
    draw();
    end_update();
    ffi::flush_out();
}

/// After a panic, whose notice ended on a new row: readline draws the line
/// again under it.
fn draw_below_notice() {
    ffi::on_new_line();
    ffi::call_redisplay(originals().redisplay);
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
/// when the cursor is at the end and the completion menu under it. Outside
/// plain editing (a count prefix, a search) the stored suggestion and menu are
/// kept, so `M-3 C-f` can still take from the suggestion and `M-2 C-n` can
/// move through the menu; `accept` checks the suggestion against the line
/// before using it.
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
    let line = ffi::line();
    let point = ffi::point();
    // While moving through the menu, its rows are drawn as they were at the
    // first move, and nothing is gathered.
    let moving_menu = if editing {
        moving::menu_to_draw()
    } else {
        None
    };
    let moving = moving_menu.is_some();
    // Whether the line changed since the last draw.
    let changed =
        STATE.with_borrow(|s| s.drawn_at.as_ref().map(|(was, _)| was.as_str()) != line.as_deref());
    // Whether the key ran a history search that changed the line to a
    // history entry's text. A search that found nothing leaves the typed
    // line, which may be an entry's text too.
    let searched = editing
        && ran_history_search()
        && changed
        && line
            .as_deref()
            .is_some_and(|l| ffi::history_find_map(|entry| (entry == l).then_some(())).is_some());
    STATE.with_borrow_mut(|s| {
        if editing {
            // The menu counts as shown only once this draw puts it on
            // screen. A draw outside plain editing, such as while a count is
            // typed, leaves it as it was, so the menu keys act on it after
            // the count.
            if let Some(m) = &mut s.menu {
                m.shown = false;
            }
            // A line a search found stays found while its text stays.
            if searched {
                s.searched.clone_from(&line);
            } else if s.searched != line {
                s.searched = None;
            }
            s.suggestion = None;
            // A kept menu belongs to one text of the line and one cursor
            // place; `C-g` hides the menu until the text changes.
            let text = line.as_deref();
            if s.menu
                .as_ref()
                .is_some_and(|m| text.is_none_or(|l| !m.is_for(l, point)))
            {
                s.menu = None;
            }
            if s.hidden_on.as_deref() != text {
                s.hidden_on = None;
            }
            let same_text = matches!((&s.drawn_at, text), (Some((was, _)), Some(l)) if was == l);
            let moved = s.drawn_at.as_ref().is_some_and(|(_, at)| *at != point);
            s.cursor_moved = same_text && (s.cursor_moved || moved);
            s.drawn_at = line.clone().map(|l| (l, point));
        }
    });
    let Some(line) = line else {
        return false;
    };
    if (line.is_empty() && MESSAGE.with_borrow(Option::is_none))
        || STATE.with_borrow(|s| s.displaced)
        || left_to_readline()
    {
        return false;
    }
    let Some(prompt_width) = render::prompt_width(&ffi::display_prompt()) else {
        return false;
    };
    let (rows, cols) = ffi::screen_size();
    let colors = crate::lisp::settings::colors();
    let suggestion_lines = crate::lisp::settings::suggestion_lines();
    let path = ffi::shell_variable("PATH").unwrap_or_default();
    let show_menu = crate::lisp::settings::show_menu()
        && (crate::lisp::settings::menu_on_move() || !STATE.with_borrow(|s| s.cursor_moved));
    let show_suggestion = crate::lisp::settings::show_suggestion();
    // With no menu to show and no grey text possible (it shows only with the
    // cursor at the end), nothing is gathered.
    let gather = !moving
        && editing
        && !line.is_empty()
        && (show_menu || (show_suggestion && point == line.len()))
        && !is_hidden(&line)
        && !recalled(&line);
    let began = Instant::now();
    // bash's copy of the shell is started first, so it works while the mode
    // servers are waited on; both share `mode_server::WAIT`.
    let bash = gather.then(|| ask_bash(&line, point)).flatten();
    // Mode server items are asked for only when a menu is gathered.
    let (found, mode_items) = ask_mode_servers(&line, &path, gather.then_some(point), began);
    let bash_items = bash
        .map(|ticket| session::found(&ticket, began + mode_server::WAIT, stop_waiting_for_bash))
        .unwrap_or_default();
    let menu =
        moving_menu.or_else(|| gather.then(|| menu_for(&line, point, mode_items, bash_items)));
    let suggestion = menu
        .as_ref()
        .filter(|_| show_suggestion && !moving)
        .and_then(|m| m.grey_item())
        .and_then(|item| menu::grey(&line, point, item))
        .map(str::to_owned);
    let menu_lines = crate::lisp::settings::menu_lines();
    let (found, sets) = highlight::with_sets(found, &colors, mode_server::colors);
    show_mode_server_notices(&line);
    // Read after the suggestion hook and the mode server notices, which may
    // have set it.
    let message = MESSAGE.with_borrow(Clone::clone);
    STATE.with_borrow_mut(|s| {
        let paths = &mut s.paths;
        let spans = s.lexer.spans(&line, |word| {
            !commands::is_plain(word) || commands::exists(word, &path, paths, ffi::known_to_bash)
        });
        let mut painted = highlight::paint(line.len(), &spans, &found);
        let (error, error_message) = error_to_underline(s, &line, point, painted.error.take());
        // A message set with `show_message` (Lisp's, or why a mode server
        // was turned off) comes first; the error's message is only for this
        // draw.
        // Without a free row under the line it is not shown, so an error
        // with no place on the line then shows nothing.
        let message = message.or(error_message);
        let repaint = Repaint {
            prompt_width,
            line: &line,
            point,
            spans: &painted.spans,
            colors: &colors,
            suggestion: suggestion.as_deref(),
            suggestion_lines,
            error: error.clone(),
            script: &painted.script,
            sets: &sets,
            span_sets: &painted.span_sets,
            script_sets: &painted.script_sets,
            message: message.as_deref(),
            menu: menu.as_ref().filter(|_| show_menu).map(|m| MenuView {
                items: &m.items,
                highlighted: m.picked,
                max_rows: menu_lines,
            }),
            rows,
            cols,
        };
        let Some(out) = render::build(&repaint) else {
            return false;
        };
        ffi::write_queued(&out.bytes);
        s.shown_at = out.suggestion_col;
        s.message_rows = out.message_rows;
        s.menu_rows = out.menu_rows;
        if editing {
            let shown = out.menu_rows.is_some();
            if let Some(m) = s.moving.as_mut().filter(|_| moving) {
                m.menu.shown = shown;
            } else {
                s.menu = menu.map(|mut m| {
                    m.shown = shown;
                    m
                });
            }
        }
        s.underlined = error;
        if let (Some(_), Some(rest)) = (out.suggestion_col, suggestion) {
            s.suggestion = Some((line.clone(), rest));
        }
        true
    })
}

/// The menu for `line` with the cursor at `point`: the one kept in `STATE` when
/// it was made for the same line and cursor, else a new one gathered from
/// the sources: history and the suggestion hook when the cursor is at the
/// end of the line, then the mode server's items (`mode`), then bash's own
/// completion (`bash`), then the completion hook. It lists only the items
/// `settings::menu_listed` allows; its grey text comes from all of them.
fn menu_for(line: &str, point: usize, mode: ModeItems, bash: session::Found) -> Menu {
    let lisp_runs = crate::lisp::RUNNING.load(Ordering::Relaxed);
    let waiting = mode.waiting;
    let bash_waiting = bash.waiting;
    let mut kept = STATE.with_borrow(|s| s.menu.clone().filter(|m| m.is_for(line, point)));
    // A menu made while Lisp ran has no items from the Lisp hooks: once Lisp
    // has stopped, a new one is gathered. One made while the mode server had
    // not answered is gathered again once it has. One made while a copy of
    // the shell was working on bash's items is gathered again once it has
    // answered.
    if let Some(menu) = kept.take_if(|m| {
        (lisp_runs || !m.lisp_ran)
            && (!m.mode_waiting || waiting)
            && (!m.bash_waiting || bash_waiting)
    }) {
        return menu;
    }
    let how = crate::lisp::settings::completion_matching();
    let at_end = point == line.len();
    let mut history = menu::HistoryGather::new(line, how);
    if at_end {
        ffi::history_find_map(|entry| history.offer(entry).then_some(()));
    }
    let whole = at_end
        .then(|| crate::lisp::hooks::suggestion(line))
        .flatten()
        .map(|text| Item {
            text,
            start: 0,
            end: line.len(),
            source: Source::Lisp,
            note: None,
        });
    let words = crate::lisp::hooks::completions(line, point);
    let mode = mode
        .came
        .map(|(items, arg)| menu::mode::place(line, point, &arg, &items))
        .unwrap_or_default();
    let bash = bash.items;
    let history = history.into_items();
    let listed = crate::lisp::settings::menu_listed();
    let lists = |item: &Item| listed.lists(line, point, item);
    let every_item_listed = history
        .iter()
        .chain(&whole)
        .chain(&mode)
        .chain(&bash)
        .chain(&words)
        .all(lists);
    let menu = if every_item_listed {
        Menu::new(
            line,
            point,
            menu::assemble(line, point, how, history, whole, mode, bash, words),
        )
    } else {
        // The listed items are assembled on their own, so that an item the
        // menu does not list cannot hide one it lists as a duplicate.
        let keep =
            |items: &[Item]| -> Vec<Item> { items.iter().filter(|i| lists(i)).cloned().collect() };
        let items = menu::assemble(
            line,
            point,
            how,
            keep(&history),
            whole.as_ref().filter(|i| lists(i)).cloned(),
            keep(&mode),
            keep(&bash),
            keep(&words),
        );
        let top = menu::assemble(line, point, how, history, whole, mode, bash, words)
            .into_iter()
            .next();
        Menu {
            top,
            ..Menu::new(line, point, items)
        }
    };
    Menu {
        lisp_ran: lisp_runs,
        mode_waiting: waiting,
        bash_waiting,
        ..menu
    }
}

/// Whether `C-g` hid the menu on this text of the line.
fn is_hidden(line: &str) -> bool {
    STATE.with_borrow(|s| s.hidden_on.as_deref() == Some(line))
}

/// Whether `line` is a history entry brought back as it was: readline's
/// history position is on an entry and the line's text is that entry's, or
/// the line is the text a history search left (`searched`). Such a line has
/// no menu and no grey text, so `C-p` and `C-n` keep walking history;
/// changing its text ends this.
fn recalled(line: &str) -> bool {
    ffi::history_entry_here_is(line) || STATE.with_borrow(|s| s.searched.as_deref() == Some(line))
}

/// Whether the key just handled ran a history search that puts a history
/// entry in the line: readline's prefix, substring and non-incremental
/// searches; an Up or Down command that went on to readline's prefix
/// search; or a `menu-next` or `menu-previous` that ran one of these. The
/// substring search of the Up and Down commands is not one: it walks to the
/// entry it finds, so `recalled` sees that entry at readline's history
/// place.
fn ran_history_search() -> bool {
    let last = ffi::last_command();
    if multiline::is_vertical(last) {
        return STATE.with_borrow(|s| {
            s.search_continues.is_some()
                || (is_menu_key(last) && s.menu_key_ran.is_some_and(is_history_search))
        });
    }
    last.is_some_and(is_history_search)
}

/// Whether `f` is `menu-next` or `menu-previous`.
fn is_menu_key(f: Option<ffi::CommandFn>) -> bool {
    f.is_some_and(|f| {
        std::ptr::fn_addr_eq(f, menu_next as ffi::CommandFn)
            || std::ptr::fn_addr_eq(f, menu_previous as ffi::CommandFn)
    })
}

/// Whether `f` is one of readline's prefix, substring or non-incremental
/// history searches.
fn is_history_search(f: ffi::CommandFn) -> bool {
    static SEARCHES: OnceLock<Vec<ffi::CommandFn>> = OnceLock::new();
    SEARCHES
        .get_or_init(|| {
            [
                "history-search-backward",
                "history-search-forward",
                "history-substring-search-backward",
                "history-substring-search-forward",
                "non-incremental-reverse-search-history",
                "non-incremental-forward-search-history",
                "non-incremental-reverse-search-history-again",
                "non-incremental-forward-search-history-again",
            ]
            .into_iter()
            .filter_map(ffi::named_command)
            .collect()
        })
        .iter()
        .any(|&g| std::ptr::fn_addr_eq(f, g))
}

/// The menu the last draw in plain editing showed, when it is for the line
/// and cursor as they are.
fn shown_menu() -> Option<Menu> {
    let (Some(line), point) = (ffi::line(), ffi::point()) else {
        return None;
    };
    STATE.with_borrow(|s| {
        s.menu
            .as_ref()
            .filter(|m| m.shown && m.is_for(&line, point))
            .cloned()
    })
}

/// Whether the last draw in plain editing showed the menu, and it is for
/// the line and cursor as they are now.
fn showing_menu() -> bool {
    let (Some(line), point) = (ffi::line(), ffi::point()) else {
        return false;
    };
    STATE.with_borrow(|s| {
        s.menu
            .as_ref()
            .is_some_and(|m| m.shown && m.is_for(&line, point))
    })
}

/// Asks the mode servers of the commands on `line` that use a mode how to
/// colour their arguments, at the main prompt while no Lisp runs: starts
/// the servers not started yet, looking up their programs in `path`
/// (bash's `PATH`), and waits until `mode_server::WAIT` after `began` at
/// most for their first lines and replies. The commands answered in time,
/// each with its mode and its reply, in line order. Why a server was turned
/// off waits for `show_mode_server_notices`.
///
/// With `point`, the cursor's place when a menu is gathered, the server of
/// the innermost of those commands with an argument (after its name) that
/// holds the cursor is also asked for completion items there, within the
/// same wait. The request is sent only once typing has paused on this line
/// and cursor, so a burst of keys never waits behind it; until then a kept
/// reply still answers, and otherwise the items are waiting and a pause is
/// asked for.
fn ask_mode_servers(
    line: &str,
    path: &str,
    point: Option<usize>,
    began: Instant,
) -> (Vec<(String, CommandArgs, Reply)>, ModeItems) {
    let nothing = || (Vec::new(), ModeItems::default());
    if !ffi::reading_command()
        || crate::lisp::RUNNING.load(Ordering::Relaxed)
        || !mode_server::any_defined()
    {
        return nothing();
    }
    let table = crate::lisp::settings::command_modes();
    if table.is_empty() {
        return nothing();
    }
    // `STATE` is borrowed only for the parse: never while waiting on a
    // server.
    let Some(tree) = STATE.with_borrow_mut(|s| s.lexer.tree(line)) else {
        return nothing();
    };
    let found: Vec<(String, CommandArgs)> = args::commands(&tree, line, |word| {
        mode_server::mode_for(word, &table).is_some()
    })
    .into_iter()
    .filter_map(|c| Some((mode_server::mode_for(&c.name, &table)?, c)))
    .collect();
    if found.is_empty() {
        return nothing();
    }
    let modes: Vec<String> = found.iter().map(|(mode, _)| mode.clone()).collect();
    mode_server::prepare(&modes, path, || Some(ffi::exported_environment()));
    let cwd = ffi::shell_variable("PWD").unwrap_or_default().into_bytes();
    let asks: Vec<(String, mode_server::Request)> = found
        .iter()
        .map(|(mode, c)| (mode.clone(), mode_server::request(cwd.clone(), c)))
        .collect();
    // The command and the argument the cursor is in, and the cursor. A
    // command inside another's argument comes after it in `found`.
    let at_cursor = point.and_then(|point| {
        found
            .iter()
            .enumerate()
            .rev()
            .find_map(|(c, (_, command))| {
                let arg = command.args.iter().skip(1).position(|a| a.holds(point))? + 1;
                Some((c, arg, point))
            })
    });
    let paused = at_cursor.is_some_and(|(_, _, point)| items_paused_at(line, point));
    let items_ask = at_cursor.map(|(c, arg, point)| {
        let (mode, request) = asks[c].clone();
        mode_server::ItemsAsk {
            mode,
            request,
            at: (arg, found[c].1.args[arg].offset_at(point)),
            send: paused,
        }
    });
    let wait = mode_server::WAIT.saturating_sub(began.elapsed());
    let (replies, items) =
        mode_server::replies(&asks, items_ask.as_ref(), wait, ffi::signal_to_act_on);
    let mut mode_items = ModeItems::default();
    if let Some((c, arg, _)) = at_cursor {
        match items {
            mode_server::Items::Came(items) => {
                mode_items.came = Some((items, found[c].1.args[arg].clone()));
            }
            mode_server::Items::Waiting => {
                mode_items.waiting = true;
                if !paused {
                    STATE.with_borrow_mut(|s| s.items_want_pause = true);
                }
            }
            mode_server::Items::NotAsked => {}
        }
    }
    let found = found
        .into_iter()
        .zip(replies)
        .filter_map(|((mode, command), reply)| Some((mode, command, reply?)))
        .collect();
    (found, mode_items)
}

/// Readies bash's own completion for the word at the cursor (see
/// `session::prepare`), at the main prompt while no Lisp runs. With
/// `inkline-bash-completion` off, a copy still running is stopped and
/// nothing is asked. A pause wanted to ask again is asked of `getc`. The
/// ticket for `session::found`; None when the word gets no bash items.
fn ask_bash(line: &str, point: usize) -> Option<session::Ticket> {
    use crate::lisp::settings;
    if !ffi::reading_command() || crate::lisp::RUNNING.load(Ordering::Relaxed) {
        return None;
    }
    if !settings::bash_completion() {
        session::forget();
        return None;
    }
    let word = bash_complete::Word::new(line, ffi::completion_word_start(), point)?;
    let wanted = session::Settings {
        min_chars: settings::command_min_chars(),
        timeout: settings::bash_completion_timeout(),
        how: settings::completion_matching(),
    };
    let ticket = session::prepare(
        word,
        &wanted,
        items_paused_at(line, point),
        ffi::more_keys_coming(),
    )?;
    if ticket.at_pause {
        STATE.with_borrow_mut(|s| s.items_want_pause = true);
    }
    Some(ticket)
}

/// Whether the draw stops waiting for a copy of the shell: a signal must be
/// acted on, or more keys are still to come, as in a macro or a paste.
fn stop_waiting_for_bash() -> bool {
    ffi::signal_to_act_on() || ffi::more_keys_coming()
}

/// Whether typing paused on `line` with the cursor at `point` while
/// completion items waited for the pause.
fn items_paused_at(line: &str, point: usize) -> bool {
    STATE.with_borrow(|s| {
        s.items_paused_on
            .as_ref()
            .is_some_and(|(on, at)| on == line && *at == point)
    })
}

/// What the mode server of the command the cursor is in gave for the
/// completion menu.
#[derive(Default)]
struct ModeItems {
    /// The items that came, not yet placed on the line, and the argument
    /// they are for.
    came: Option<(Rc<[ReplyItem]>, Arg)>,
    /// Whether items may still come for this line and cursor.
    waiting: bool,
}

/// Shows why mode servers were turned off once typing pauses on
/// `line`, as a new syntax error waits for the pause before it is
/// underlined; until then, asks `getc` for the pause. The message then
/// stays until the next key. Only at the main prompt: a line a script reads
/// or bash's `>` prompt has nothing to do with mode servers.
fn show_mode_server_notices(line: &str) {
    if !ffi::reading_command() || !mode_server::has_notices() {
        return;
    }
    let paused = STATE.with_borrow_mut(|s| {
        let paused = s.paused_on.as_deref() == Some(line);
        if !paused {
            s.wants_pause = true;
        }
        paused
    });
    if paused {
        show_message(&mode_server::take_notices().join("; "));
    }
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

/// The error to show on `line`: the bytes to underline and the message to
/// put under the line. Bash's syntax error comes first, with no message;
/// without one, `from_server`, the error a mode server sent (its bytes of
/// the line, `None` when it has no place there, and its message). A new
/// error waits for a pause in typing and asks `getc` for one; an error
/// already underlined stays, and a server's message with it, as long as the
/// underline does. An error with no place shows its message while the line
/// stays the one typing paused on. The word the cursor is at the end of is
/// being typed, so an error there is not shown.
fn error_to_underline(
    s: &mut State,
    line: &str,
    point: usize,
    from_server: Option<(Option<Range<usize>>, String)>,
) -> (Option<Range<usize>>, Option<String>) {
    if !ffi::reading_command() {
        return (None, None);
    }
    let (place, message) = match status_of(s, line) {
        Status::Wrong(range) => (Some(syntax::word_around(line, range)), None),
        Status::Fine | Status::Unfinished => match from_server {
            Some((place, message)) => (place, Some(message)),
            None => return (None, None),
        },
    };
    if place
        .as_ref()
        .is_some_and(|p| p.is_empty() || p.end == point)
    {
        return (None, None);
    }
    let kept = place.is_some() && s.underlined == place;
    if kept || s.paused_on.as_deref() == Some(line) {
        (place, message)
    } else {
        s.wants_pause = true;
        (None, None)
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

/// `C-n` and `<down>`: move to the next row of the menu, writing it into the
/// line; with no menu, see `menu_fallback`.
pub(super) extern "C" fn menu_next(count: c_int, key: c_int) -> c_int {
    move_pick(count, key, true)
}

/// `C-p` and `<up>`: move to the row above in the menu, writing it into the
/// line; with no menu, see `menu_fallback`.
pub(super) extern "C" fn menu_previous(count: c_int, key: c_int) -> c_int {
    move_pick(count, key, false)
}

/// Moves through the menu `count` rows (see `moving::step`), or runs
/// `menu_fallback` when there is nothing to move through.
fn move_pick(count: c_int, key: c_int, down: bool) -> c_int {
    let (moved, ran_before) = guard(
        || {
            let ran_before = STATE.with_borrow_mut(|s| s.menu_key_ran.take());
            (moving::step(down, i64::from(count)), ran_before)
        },
        || (false, None),
    );
    if moved {
        0
    } else {
        menu_fallback(count, key, down, ran_before)
    }
}

/// What `menu-next` (`down`) or `menu-previous` runs with no menu. While the
/// multi-line group is bound, the Up or Down command `menu_key_fallback`
/// picks from what the key had before inkline bound it. Otherwise, or when
/// it picks none, what the key had: a command, which runs last (readline may
/// jump from it back to its top level) and ends a run of Up and Down, or
/// macro text; a key that had nothing rings the bell. When the last key was
/// also `menu-next` or `menu-previous` and ran its key's own command
/// (`ran_before`), readline sees that command as the last one, as when its
/// own key ran it, so a search goes on from where it stopped; and after one
/// of readline's history searches, the line it finds counts as found by a
/// search (see `ran_history_search`).
fn menu_fallback(
    count: c_int,
    key: c_int,
    down: bool,
    ran_before: Option<ffi::CommandFn>,
) -> c_int {
    use crate::lisp::keys::{self, Fallback};
    use crate::lisp::layout::{self, Group};
    let own: ffi::CommandFn = if down { menu_next } else { menu_previous };
    let (saved, lines) = guard(
        || {
            (
                keys::saved_binding_of(own),
                keys::group_bound(Group::MultiLine),
            )
        },
        || (Fallback::Nothing, true),
    );
    if lines {
        let had = match &saved {
            Fallback::Command(f) => ffi::Binding::Command(*f),
            // Macro text gives no Up or Down command, whatever it is.
            Fallback::Macro(_) => ffi::Binding::Macro(Vec::new()),
            Fallback::Nothing => ffi::Binding::Unbound,
        };
        if let Some(f) = layout::menu_key_fallback(down, &had).and_then(ffi::named_command) {
            return ffi::run_command(f, count, key);
        }
    }
    match saved {
        Fallback::Command(f) => {
            let before = ran_before.filter(|_| is_menu_key(ffi::last_command()));
            guard(
                || {
                    STATE.with_borrow_mut(|s| {
                        s.end_vertical_run();
                        s.menu_key_ran = Some(f);
                    });
                },
                || (),
            );
            if let Some(g) = before {
                ffi::set_last_command(g);
            }
            ffi::run_command(f, count, key)
        }
        saved @ (Fallback::Macro(_) | Fallback::Nothing) => run_fallback(saved, count, key),
    }
}

/// Tab: moves through the menu, or writes its only row (see `moving::tab`).
/// With no menu, readline's `complete`, and right after a Tab that ran it,
/// the listing of readline's second Tab. `complete` may jump back to
/// readline's or bash's top level, so it runs last with nothing here to
/// drop.
extern "C" fn menu_take(count: c_int, key: c_int) -> c_int {
    let (done, again) = guard(
        || {
            let right_after = ffi::last_command()
                .is_some_and(|f| std::ptr::fn_addr_eq(f, menu_take as ffi::CommandFn));
            let again = right_after && STATE.with_borrow(|s| s.completing);
            let done = moving::tab(true);
            STATE.with_borrow_mut(|s| s.completing = !done);
            (done, again)
        },
        || (false, false),
    );
    if done {
        return 0;
    }
    if again {
        ffi::continue_completion();
    }
    ffi::complete(count, key)
}

/// Shift-Tab: moves up through the menu, or writes its only row (see
/// `moving::tab`). With no menu, runs what the key had before inkline bound
/// it, or rings the bell when it had nothing.
extern "C" fn menu_take_previous(count: c_int, key: c_int) -> c_int {
    use crate::lisp::keys::{self, Fallback};
    let (moved, saved) = guard(
        || {
            if moving::tab(false) {
                STATE.with_borrow_mut(|s| s.completing = false);
                return (true, Fallback::Nothing);
            }
            (false, keys::saved_binding_of(menu_take_previous))
        },
        || (false, Fallback::Nothing),
    );
    if moved {
        return 0;
    }
    run_fallback(saved, count, key)
}

/// `C-g`: while moving, puts back the typed line (see `moving::cancel`);
/// otherwise hides the menu and the grey text until the line's text changes;
/// with no menu, readline's `abort`, which jumps back to readline's top
/// level, so it runs last with nothing here to drop.
extern "C" fn menu_hide(count: c_int, key: c_int) -> c_int {
    let hidden = guard(
        || {
            if moving::cancel() {
                return true;
            }
            if !showing_menu() {
                return false;
            }
            let line = ffi::line();
            STATE.with_borrow_mut(|s| {
                s.hidden_on = line;
                s.menu = None;
            });
            true
        },
        || false,
    );
    if hidden { 0 } else { ffi::abort(count, key) }
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
