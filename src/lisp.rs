//! The Lisp interpreter inkline embeds (tulisp): one per bash process, holding
//! the user's settings, key bindings and functions.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};

use tulisp::TulispContext;

pub mod buffer;
#[cfg(not(test))]
pub mod commands;
pub mod emacs;
pub mod errors;
pub mod highlight;
pub mod hooks;
pub mod init;
pub mod keydesc;
#[cfg(not(test))]
pub mod keys;
pub mod layout;
pub mod lockout;
pub mod settings;

thread_local! {
    static SLOT: RefCell<Option<TulispContext>> = const { RefCell::new(None) };
    /// Set once `start` has begun, even if it did not finish.
    static STARTED: Cell<bool> = const { Cell::new(false) };
    /// Set when a panic went through `inkline eval`, `load` or a Lisp
    /// command, until the next `reload`.
    static BROKEN: Cell<bool> = const { Cell::new(false) };
}

/// Set while Lisp runs.
pub static RUNNING: AtomicBool = AtomicBool::new(false);

/// Lisp is already running further up the stack.
#[derive(Debug, PartialEq, Eq)]
pub struct Busy;

/// Makes a fresh interpreter with inkline's functions, replacing any old one.
pub fn start() {
    STARTED.set(true);
    let ctx = new_context();
    SLOT.with_borrow_mut(|slot| *slot = Some(ctx));
}

/// Starts Lisp for the shell on the first `enable -f`: a fresh interpreter
/// and, where line editing is on, the default layout, the terminal watcher
/// and `init.el`. Once an interpreter exists it does nothing, even when a
/// panic ended the last start part way, so a later `enable -f` only switches
/// inkline on.
#[cfg(not(test))]
pub fn start_for_shell() {
    if STARTED.get() {
        return;
    }
    start();
    if crate::ffi::line_editing_shell() {
        layout::prepare_readline();
        layout::bind_defaults();
        init::warn_old_variables();
        lockout::watch_terminal();
        init::read_at_start();
    }
}

fn new_context() -> TulispContext {
    let mut ctx = TulispContext::new();
    ctx.set_max_eval_depth(lockout::max_eval_depth(lockout::stack_limit()));
    errors::register(&mut ctx);
    emacs::register(&mut ctx);
    hooks::register(&mut ctx);
    buffer::register(&mut ctx);
    #[cfg(not(test))]
    commands::register(&mut ctx);
    settings::register(&mut ctx);
    highlight::register(&mut ctx);
    #[cfg(not(test))]
    keys::register(&mut ctx);
    #[cfg(not(test))]
    ctx.defun("getenv", |name: String| crate::ffi::shell_variable(&name));
    // Lets the end-to-end tests check what a panic in Lisp does.
    #[cfg(debug_assertions)]
    ctx.defun("inkline--panic", || -> tulisp::TulispObject {
        panic!("inkline--panic")
    });
    ctx
}

/// Runs `f` with the interpreter, starting one if there is none. The
/// interpreter is out of its slot while `f` runs, so nothing reaches it
/// twice, and goes back afterwards, also when `f` panics.
pub fn with_lisp<R>(f: impl FnOnce(&mut TulispContext) -> R) -> Result<R, Busy> {
    if !STARTED.get() {
        start();
    }
    let Some(ctx) = SLOT.with_borrow_mut(Option::take) else {
        return Err(Busy);
    };
    struct PutBack(Option<TulispContext>);
    impl Drop for PutBack {
        fn drop(&mut self) {
            RUNNING.store(false, Ordering::Relaxed);
            if let Some(ctx) = self.0.take() {
                let _ = SLOT.try_with(|slot| {
                    let mut slot = slot.borrow_mut();
                    if slot.is_none() {
                        *slot = Some(ctx);
                    }
                });
            }
        }
    }
    let mut held = PutBack(Some(ctx));
    RUNNING.store(true, Ordering::Relaxed);
    let ctx = held.0.as_mut().expect("just put there");
    Ok(f(ctx))
}

/// `with_lisp` for `inkline eval`, `load`, Lisp commands and hooks: a panic in
/// `f` marks Lisp broken, so the next `inkline on`, `eval` or `load`
/// starts a fresh interpreter. A panic while reading `init.el` does not,
/// as reading it again would only panic again.
pub fn with_lisp_marking_panics<R>(f: impl FnOnce(&mut TulispContext) -> R) -> Result<R, Busy> {
    struct MarkIfPanicking;
    impl Drop for MarkIfPanicking {
        fn drop(&mut self) {
            if std::thread::panicking() {
                let _ = BROKEN.try_with(|b| b.set(true));
            }
        }
    }
    let _mark = MarkIfPanicking;
    with_lisp(f)
}

/// `inkline eval EXPR`: the result as `prin1` prints it, or None for `nil`.
pub fn eval(expr: &str) -> Result<Option<String>, String> {
    let result = with_lisp_marking_panics(|ctx| {
        ctx.eval_string(expr)
            .map_err(|e| errors::describe(&e, ctx, None))
    })
    .map_err(|Busy| "busy running Lisp".to_owned())??;
    Ok((!result.null()).then(|| result.to_string()))
}

/// `inkline load FILE`.
pub fn load(path: &str) -> Result<(), String> {
    with_lisp_marking_panics(|ctx| {
        ctx.eval_file(path)
            .map(drop)
            .map_err(|e| errors::describe(&e, ctx, Some(path)))
    })
    .map_err(|Busy| "busy running Lisp".to_owned())?
}

/// `inkline reload`: puts back the keys inkline still owns, forgets every
/// highlight helper, starts a fresh interpreter, sets the layout's readline
/// variables and binds the layout again, and reads `init.el` again.
/// `Ok(false)` when `init.el` was skipped or failed; its problem is already
/// printed.
#[cfg(not(test))]
pub fn reload() -> Result<bool, String> {
    if SLOT.with_borrow(Option::is_none) && STARTED.get() {
        return Err("busy running Lisp".into());
    }
    // Cleared first: a panic while reading `init.el` below must not make
    // every later command reload again.
    BROKEN.set(false);
    keys::restore_all();
    crate::helper::stop_all();
    start();
    let mut read_cleanly = true;
    if crate::ffi::line_editing_shell() {
        layout::prepare_readline();
        layout::bind_defaults();
        read_cleanly = init::read_again();
    }
    Ok(read_cleanly)
}

/// Runs `reload` if a panic went through `inkline eval`, `load` or a Lisp
/// command since the last `reload`.
#[cfg(not(test))]
pub fn start_again_if_broken() {
    if BROKEN.get() {
        let _ = reload();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_prints_values_and_hides_nil() {
        assert_eq!(eval("(+ 1 2)"), Ok(Some("3".to_owned())));
        assert_eq!(eval(r#""a""#), Ok(Some(r#""a""#.to_owned())));
        assert_eq!(eval("nil"), Ok(None));
    }

    #[test]
    fn eval_errors_are_one_line() {
        assert_eq!(
            eval("(car 1 2)"),
            Err("Too many arguments (in (car 1 2))".to_owned())
        );
    }

    #[test]
    fn nested_use_is_busy_and_the_interpreter_comes_back() {
        let inner = with_lisp(|_| with_lisp(|_| ())).unwrap();
        assert_eq!(inner, Err(Busy));
        assert_eq!(eval("(+ 1 1)"), Ok(Some("2".to_owned())));
    }

    #[test]
    fn the_interpreter_comes_back_after_a_panic() {
        let _ = std::panic::catch_unwind(|| with_lisp(|_| panic!("boom")));
        assert!(!BROKEN.get());
        assert_eq!(eval("(+ 2 2)"), Ok(Some("4".to_owned())));
        let _ = std::panic::catch_unwind(|| eval("(inkline--panic)"));
        assert!(BROKEN.get());
    }

    #[test]
    fn load_reports_file_and_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.el");
        std::fs::write(&path, "(setq a 1)\n(car 1 2)\n").unwrap();
        let path = path.to_str().unwrap();
        assert_eq!(
            load(path),
            Err(format!("{path}:2: Too many arguments (in (car 1 2))"))
        );
    }
}
