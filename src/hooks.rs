//! Connects inkline to bash and readline: the `inkline` builtin, the hooks
//! readline calls, and the readline commands inkline adds.

use std::cell::RefCell;
use std::ffi::c_int;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::colors::Colors;
use crate::commands::{self, PathCache};
use crate::ffi;
use crate::lexer::Lexer;
use crate::render::{self, Repaint};
use crate::suggest;

struct State {
    enabled: bool,
    orig_redisplay: Option<ffi::VoidFn>,
    orig_getc: Option<ffi::GetcFn>,
    orig_deprep: Option<ffi::VoidFn>,
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
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        enabled: false,
        orig_redisplay: None,
        orig_getc: None,
        orig_deprep: None,
        lexer: Lexer::new(),
        colors: Colors::default(),
        colors_spec: None,
        paths: PathCache::default(),
        suggestion: None,
        shown_at: None,
    });
}

pub fn load() {
    enable();
}

pub fn unload() {
    disable();
}

pub fn builtin(args: &[String]) -> c_int {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] | ["status"] => {
            let on = STATE.with_borrow(|s| s.enabled);
            println!("inkline: {}", if on { "on" } else { "off" });
            ffi::EXECUTION_SUCCESS
        }
        ["on"] => {
            enable();
            ffi::EXECUTION_SUCCESS
        }
        ["off"] => {
            disable();
            ffi::EXECUTION_SUCCESS
        }
        _ => {
            eprintln!("inkline: usage: inkline [on|off|status]");
            ffi::EX_USAGE
        }
    }
}

/// Installs the key-reading and terminal-restore hooks. The drawing hook is
/// installed by `getc` once a key arrives: readline changes how it sets up the
/// terminal and how it redraws after a resize whenever a custom drawing
/// function is in place (it skips the terminal description, losing cursor
/// movement and bracketed paste, and redraws below the old line), so inkline's
/// is only in place while a key's command runs.
fn enable() {
    STATE.with_borrow_mut(|s| {
        if s.enabled {
            return;
        }
        s.orig_redisplay = ffi::redisplay_function();
        s.orig_getc = ffi::getc_function();
        s.orig_deprep = ffi::deprep_function();
        ffi::set_getc_function(Some(getc as ffi::GetcFn));
        ffi::set_deprep_function(Some(deprep_terminal as ffi::VoidFn));
        s.enabled = true;
    });
}

fn disable() {
    erase_suggestion();
    STATE.with_borrow_mut(|s| {
        if !s.enabled {
            return;
        }
        ffi::set_redisplay_function(s.orig_redisplay);
        ffi::set_getc_function(s.orig_getc);
        ffi::set_deprep_function(s.orig_deprep);
        s.suggestion = None;
        s.enabled = false;
    });
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

/// Readline's key reader. Once the next key arrives, erases the suggestion
/// before readline runs the key's command, so Enter, `C-o`, a completion
/// listing or `C-c` never leave grey text behind. Readline's own drawing
/// function is in place while it waits for the key, so a resize is redrawn
/// the way readline expects; inkline's is installed once the key arrives.
extern "C" fn getc(stream: *mut libc::FILE) -> c_int {
    let (orig_getc, orig_redisplay, shown) =
        STATE.with_borrow(|s| (s.orig_getc, s.orig_redisplay, s.shown_at.is_some()));
    ffi::set_redisplay_function(orig_redisplay);
    if shown {
        let interrupted = !ffi::wait_for_input(stream);
        guard(erase_suggestion, || ());
        if interrupted {
            // Bash may jump out of here back to a new prompt; nothing in
            // this frame needs dropping.
            ffi::handle_interrupted_wait();
        }
    }
    let key = ffi::call_getc(orig_getc, stream);
    if STATE.with_borrow(|s| s.enabled) {
        ffi::set_redisplay_function(Some(redisplay as ffi::VoidFn));
    }
    key
}

/// Readline calls this when it returns a line. Enter can reach readline
/// without `getc` (typed ahead in one burst); readline has then moved to the
/// row below, so the suggestion is erased one row up. Readline's own drawing
/// function goes back in place, so a later terminal setup (such as after
/// `TERM` changes) sees it.
extern "C" fn deprep_terminal() {
    let (orig_deprep, orig_redisplay, shown_at) =
        STATE.with_borrow_mut(|s| (s.orig_deprep, s.orig_redisplay, s.shown_at.take()));
    if let Some(col) = shown_at
        && ffi::line_done()
    {
        ffi::write_out(format!("\x1b[A\x1b[{}G\x1b[K\x1b[B\r", col + 1).as_bytes());
    }
    ffi::set_redisplay_function(orig_redisplay);
    ffi::call_deprep(orig_deprep);
}

/// Clears from the cursor to the end of its row. The cursor is where the last
/// draw left it: at the end of the line, where the suggestion starts.
fn erase_suggestion() {
    let shown = STATE.with_borrow_mut(|s| s.shown_at.take());
    if shown.is_some() {
        ffi::write_out(b"\x1b[K");
    }
}

extern "C" fn redisplay() {
    guard(erase_suggestion, || ());
    let orig = STATE.with_borrow(|s| s.orig_redisplay);
    ffi::call_redisplay(orig);
    guard(draw, || ());
}

/// Repaints the line readline just drew, in colour, with a suggestion after
/// it when the cursor is at the end.
fn draw() {
    STATE.with_borrow_mut(|s| s.suggestion = None);
    let Some(line) = ffi::line() else { return };
    if line.is_empty() || ffi::horizontal_scroll_mode() {
        return;
    }
    let point = ffi::point();
    let (rows, cols) = ffi::screen_size();
    let prompt_width = render::prompt_width(&ffi::display_prompt());
    let colors_spec = ffi::shell_variable("INKLINE_COLORS");
    let path = ffi::shell_variable("PATH").unwrap_or_default();
    let suggestion = if point == line.len() && ffi::normal_editing() {
        ffi::history_find_map(|entry| suggest::rest(&line, entry).map(str::to_owned))
    } else {
        None
    };
    STATE.with_borrow_mut(|s| {
        if s.colors_spec != colors_spec {
            s.colors = Colors::parse(colors_spec.as_deref().unwrap_or(""));
            s.colors_spec = colors_spec;
        }
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
            rows,
            cols,
        };
        let Some(out) = render::build(&repaint) else {
            return;
        };
        ffi::write_out(&out.bytes);
        s.shown_at = out.suggestion_col;
        if let (Some(_), Some(rest)) = (out.suggestion_col, suggestion) {
            s.suggestion = Some((line.clone(), rest));
        }
    });
}
