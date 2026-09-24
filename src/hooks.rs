//! Connects inkline to bash and readline: the `inkline` builtin, the hooks
//! readline calls, and the readline commands inkline adds.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int};
use std::io::Write;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Once;

use crate::colors::Colors;
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
    colors: Colors,
    /// The `INKLINE_COLORS` value `colors` was parsed from.
    colors_spec: Option<String>,
    paths: PathCache,
    /// The line a suggestion was drawn for, and the suggestion, for the accept
    /// commands.
    suggestion: Option<(String, String)>,
    /// The column where the suggestion on screen starts, if one is showing.
    shown_at: Option<usize>,
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

thread_local! {
    static ORIGINALS: Cell<Originals> = Cell::default();
    /// Whether a synchronized update is open.
    static UPDATING: Cell<bool> = const { Cell::new(false) };
    /// bash's completion function, which `complete` calls.
    static COMPLETION: Cell<Option<ffi::CompletionFn>> = const { Cell::new(None) };
    static STATE: RefCell<State> = RefCell::new(State {
        enabled: false,
        lexer: Lexer::new(),
        colors: Colors::default(),
        colors_spec: None,
        paths: PathCache::default(),
        suggestion: None,
        shown_at: None,
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
                crate::lisp::start();
            });
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
                "inkline: {}",
                if on { "on" } else { "off" }
            ) {
                Ok(()) => ffi::EXECUTION_SUCCESS,
                Err(_) => ffi::EXECUTION_FAILURE,
            }
        }
        ["on"] => {
            enable();
            ffi::EXECUTION_SUCCESS
        }
        ["off"] => {
            disable();
            ffi::EXECUTION_SUCCESS
        }
        ["eval", expr] => match crate::lisp::eval(expr) {
            Ok(value) => {
                if let Some(text) = value {
                    let _ = writeln!(std::io::stdout(), "{text}");
                }
                ffi::EXECUTION_SUCCESS
            }
            Err(e) => {
                let _ = writeln!(std::io::stderr(), "inkline: {e}");
                ffi::EXECUTION_FAILURE
            }
        },
        ["load", file] => match crate::lisp::load(file) {
            Ok(()) => ffi::EXECUTION_SUCCESS,
            Err(e) => {
                let _ = writeln!(std::io::stderr(), "inkline: {e}");
                ffi::EXECUTION_FAILURE
            }
        },
        _ => {
            let _ = writeln!(
                std::io::stderr(),
                "inkline: usage: inkline [on|off|status|load FILE|eval EXPR]"
            );
            ffi::EX_USAGE
        }
    }
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
            getc: ffi::getc_function(),
            deprep: ffi::deprep_function(),
            pre_input: ffi::pre_input_hook(),
        });
        ffi::set_getc_function(Some(getc as ffi::GetcFn));
        ffi::set_deprep_function(Some(deprep_terminal as ffi::VoidFn));
        ffi::set_pre_input_hook(Some(pre_input as ffi::HookFn));
        s.enabled = true;
    });
}

/// Puts readline's functions back. Also the recovery after a panic, so it must
/// not depend on `STATE` being borrowable.
fn disable() {
    let _ = catch_unwind(erase_suggestion);
    unwrap_completion();
    end_update();
    ffi::flush_out();
    let enabled = STATE.try_with(|s| {
        s.try_borrow_mut().map(|mut s| {
            s.suggestion = None;
            std::mem::replace(&mut s.enabled, false)
        })
    });
    // Restore unless the state says inkline was already off.
    if !matches!(enabled, Ok(Ok(false))) {
        let orig = originals();
        ffi::set_redisplay_function(orig.redisplay);
        ffi::set_getc_function(orig.getc);
        ffi::set_deprep_function(orig.deprep);
        ffi::set_pre_input_hook(orig.pre_input);
    }
}

/// Runs `f`. If it panics, turns inkline off and runs `on_panic` instead, so a
/// bug never unwinds into readline and never kills the shell.
fn guard<R>(f: impl FnOnce() -> R, on_panic: impl FnOnce() -> R) -> R {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
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
    loop {
        let pause = guard(
            || STATE.with_borrow_mut(|s| std::mem::take(&mut s.wants_pause)),
            || false,
        )
        .then_some(PAUSE_MS);
        match ffi::wait_for_input(stream, pause) {
            ffi::Wait::Ready | ffi::Wait::Error => break,
            // Typing paused with a new error on the line: underline it.
            ffi::Wait::Paused => guard(
                || {
                    STATE.with_borrow_mut(|s| s.paused_on = ffi::line());
                    begin_update();
                    erase_suggestion();
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
                        erase_suggestion();
                        ffi::flush_out();
                    },
                    || (),
                );
                return ffi::read_error();
            }
            ffi::Wait::Signal(signal) => {
                // Readline's redraw after a resize and inkline's repaint go out
                // as one update. Other signals are not held back: bash may jump
                // from them to a new prompt and run commands there. Bash may
                // also print while it handles a signal, which can move the
                // cursor: then readline draws the rest of the line on its own.
                let may_print = ffi::signal_may_print(signal);
                guard(
                    || {
                        if may_print {
                            STATE.with_borrow_mut(|s| {
                                s.displaced = true;
                                s.suggestion = None;
                            });
                        } else if signal == libc::SIGWINCH {
                            begin_update();
                        }
                        erase_suggestion();
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
    }
    guard(
        || {
            // The update opens here only to hide the erase; otherwise it opens
            // when readline redraws. The key's command may run a `bind -x`
            // command, whose output a terminal would hold back while the
            // update is open.
            if STATE.with_borrow(|s| s.shown_at.is_some()) {
                begin_update();
            }
            erase_suggestion();
        },
        || (),
    );
    let key = ffi::call_getc(originals().getc, stream);
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
/// the end of the screen also clears the suggestion's other rows. Readline's
/// own drawing function goes back in place, so a later terminal setup (such as
/// after `TERM` changes) sees it.
extern "C" fn deprep_terminal() {
    guard(
        || {
            let shown_at = STATE.with_borrow_mut(|s| s.shown_at.take());
            if let Some(col) = shown_at
                && ffi::line_done()
            {
                let erase = if col == 0 {
                    "\r\x1b[J".to_string()
                } else {
                    format!("\x1b[A\x1b[{}G\x1b[J\x1b[B\r", col + 1)
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

/// Clears from the cursor to the end of the screen: the suggestion starts at
/// the cursor and may take rows below it. The cursor is where the last draw
/// left it, at the end of the line.
fn erase_suggestion() {
    let shown = STATE.with_borrow_mut(|s| s.shown_at.take());
    if shown.is_some() {
        ffi::write_queued(b"\x1b[J");
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
            erase_suggestion();
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
    if line.is_empty() || STATE.with_borrow(|s| s.displaced) || left_to_readline() {
        return false;
    }
    let Some(prompt_width) = render::prompt_width(&ffi::display_prompt()) else {
        return false;
    };
    let point = ffi::point();
    let (rows, cols) = ffi::screen_size();
    let colors_spec = ffi::shell_variable("INKLINE_COLORS");
    let suggestion_lines =
        suggest::line_limit(ffi::shell_variable("INKLINE_SUGGESTION_LINES").as_deref());
    let path = ffi::shell_variable("PATH").unwrap_or_default();
    let suggestion = if editing && point == line.len() {
        ffi::history_find_map(|entry| suggest::rest(&line, entry).map(str::to_owned))
    } else {
        None
    };
    STATE.with_borrow_mut(|s| {
        if s.colors_spec != colors_spec {
            s.colors = Colors::parse(colors_spec.as_deref().unwrap_or(""));
            s.colors_spec = colors_spec;
        }
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
            colors: &s.colors,
            suggestion: suggestion.as_deref(),
            suggestion_lines,
            error: error.clone(),
            rows,
            cols,
        };
        let Some(out) = render::build(&repaint) else {
            return false;
        };
        ffi::write_queued(&out.bytes);
        s.shown_at = out.suggestion_col;
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
