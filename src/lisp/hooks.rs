//! Hooks: lists of Lisp functions inkline runs when a line is run, changed,
//! started or needs a suggestion.

#[cfg(not(test))]
use std::ffi::c_int;

#[cfg(not(test))]
use super::commands::Failure;

#[cfg(not(test))]
use tulisp::FuncallArgs;
use tulisp::{TulispContext, TulispObject};

pub const ACCEPT: &str = "inkline-accept-functions";
pub const AFTER_CHANGE: &str = "inkline-after-change-functions";
pub const LINE_START: &str = "inkline-line-start-functions";
pub const SUGGESTION: &str = "inkline-suggestion-functions";

/// `add-hook` and `remove-hook` follow Emacs: LOCAL is ignored (inkline has no
/// buffer-local variables), and a hook may hold a single function instead of a
/// one-element list, as Emacs allows. A function stored by itself is a symbol
/// other than `nil`/`t`, a list whose `car` is `lambda`, or (once `lambda` has
/// been evaluated into a callable value) anything else that is not `nil` and
/// not an ordinary list.
const PRELUDE: &str = r#"
(defvar inkline-accept-functions nil)
(defvar inkline-after-change-functions nil)
(defvar inkline-line-start-functions nil)
(defvar inkline-suggestion-functions nil)
(defun inkline--hook-single-p (value)
  (cond ((null value) nil)
        ((symbolp value) (not (eq value t)))
        ((consp value) (eq (car value) 'lambda))
        (t t)))
(defun add-hook (hook function &optional at-end _local)
  (unless (boundp hook) (set hook nil))
  (let* ((old (symbol-value hook))
         (fns (if (inkline--hook-single-p old) (list old) old)))
    (if (member function fns)
        (set hook fns)
      (set hook (if at-end (append fns (list function)) (cons function fns))))))
(defun remove-hook (hook function &optional _local)
  (when (boundp hook)
    (let ((old (symbol-value hook)))
      (set hook (if (inkline--hook-single-p old)
                    (if (equal old function) nil old)
                  (delete function old)))))
  nil)
"#;

pub fn register(ctx: &mut TulispContext) {
    ctx.eval_prelude("<inkline-hooks>", PRELUDE)
        .expect("inkline's own Lisp compiles");
}

/// The functions a hook variable holds, in order: none when `hook` is unbound
/// or `nil`; the value itself when it is a single function (a symbol other than
/// `nil`/`t`, or a list whose `car` is `lambda`); otherwise each element of the
/// list, skipping `t`.
pub fn functions(hook: &TulispObject) -> Vec<TulispObject> {
    let Ok(value) = hook.get() else {
        return Vec::new();
    };
    if is_single_function(&value) {
        return vec![value];
    }
    value
        .base_iter()
        .filter(|function| !function.eq(&TulispObject::t()))
        .collect()
}

fn is_single_function(value: &TulispObject) -> bool {
    if value.null() {
        false
    } else if value.symbolp() {
        !value.eq(&TulispObject::t())
    } else if value.consp() {
        value
            .car()
            .is_ok_and(|car| car.symbolp() && car.to_string() == "lambda")
    } else {
        // Anything else is an already-evaluated function value (e.g. a compiled
        // lambda), which is never a proper (nil-terminated) list.
        true
    }
}

/// Sets `hook` to its functions (as `functions` gives them) without every
/// element `eq` to `function`; `nil` when none are left.
pub fn remove(hook: &TulispObject, function: &TulispObject) {
    let kept = functions(hook).into_iter().filter(|f| !f.eq(function));
    let value = kept
        .rev()
        .fold(TulispObject::nil(), |rest, f| TulispObject::cons(f, rest));
    let _ = hook.set(value);
}

/// A symbol's name, else `lambda`.
pub fn function_name(function: &TulispObject) -> String {
    if function.symbolp() {
        function.to_string()
    } else {
        "lambda".to_owned()
    }
}

/// What the accept hook says about the line about to run.
#[cfg(not(test))]
pub enum Accept {
    /// Run the line. `errors` has a line for each function that failed, and
    /// `changed` says whether the functions changed the line.
    Run { errors: Vec<String>, changed: bool },
    /// Keep the line for editing: a function refused it or quit.
    Refuse,
}

/// Runs `inkline-accept-functions` on readline's line, in order. Each function
/// is one undo step; one that fails has its own changes undone, and the next
/// one runs. A `user-error`, a `quit`, or a `C-c` or a jump to bash's top level
/// while a function ran refuses the line: every change of this run is undone,
/// point goes back, and a `user-error`'s text shows under the line. A busy
/// interpreter runs nothing.
#[cfg(not(test))]
pub fn run_accept(key: c_int) -> Accept {
    let accept = super::with_lisp_marking_panics(|ctx| {
        let mut errors = Vec::new();
        let result = run_hook(ctx, ACCEPT, key, (), |function, failure| match failure {
            Failure::Error(text) => {
                errors.push(format!("inkline: {}: {text}", function_name(function)));
                Ok(())
            }
            failure => Err(failure),
        });
        match result {
            Ok(changed) => Accept::Run { errors, changed },
            Err(Failure::Refused(text)) => {
                crate::hooks::show_message(&text);
                Accept::Refuse
            }
            Err(Failure::Quit | Failure::Error(_)) => Accept::Refuse,
        }
    });
    accept.unwrap_or(Accept::Run {
        errors: Vec::new(),
        changed: false,
    })
}

/// Runs the functions of the hook named `hook` on readline's line, in order,
/// with `args` and `KEY` set to `key`, as one undo step; each function's
/// changes are a step of their own inside it, undone when that function fails.
/// `on_failure` gets each function that fails and its failure, and returns an
/// error to end the run; a `C-c` or a jump to bash's top level while a function
/// ran ends it with `Failure::Quit`. An ended run has all its changes undone
/// and point back where it was. Otherwise, whether the line changed; false when
/// the hook has no functions.
#[cfg(not(test))]
fn run_hook(
    ctx: &mut TulispContext,
    hook: &'static str,
    key: c_int,
    args: impl FuncallArgs + Clone,
    mut on_failure: impl FnMut(&TulispObject, Failure) -> Result<(), Failure>,
) -> Result<bool, Failure> {
    use super::commands;
    let hook_functions = functions(&ctx.intern(hook));
    if hook_functions.is_empty() {
        return Ok(false);
    }
    let before = crate::ffi::line_bytes();
    let _line = commands::install_line(true);
    commands::in_hook(hook, || {
        commands::one_step(key, true, || {
            for function in &hook_functions {
                let result = commands::one_step(key, false, || {
                    ctx.funcall(function, args.clone())
                        .map(drop)
                        .map_err(|e| commands::failure_of(ctx, &e))
                });
                if let Err(failure) = result {
                    on_failure(function, failure)?;
                }
                if crate::hooks::lisp_must_stop() {
                    return Err(Failure::Quit);
                }
            }
            Ok(())
        })
    })?;
    Ok(crate::ffi::line_bytes() != before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tulisp::TulispContext;

    fn ctx() -> TulispContext {
        let mut ctx = TulispContext::new();
        crate::lisp::errors::register(&mut ctx);
        crate::lisp::emacs::register(&mut ctx);
        register(&mut ctx);
        ctx
    }

    fn eval(ctx: &mut TulispContext, s: &str) -> String {
        match ctx.eval_string(s) {
            Ok(v) => v.to_string(),
            Err(e) => format!("ERROR {}", e.desc()),
        }
    }

    #[test]
    fn add_hook_puts_functions_in_front_or_at_the_end_once() {
        let mut ctx = ctx();
        eval(&mut ctx, "(add-hook 'inkline-accept-functions 'a)");
        eval(&mut ctx, "(add-hook 'inkline-accept-functions 'b)");
        eval(&mut ctx, "(add-hook 'inkline-accept-functions 'c t)");
        eval(&mut ctx, "(add-hook 'inkline-accept-functions 'a)");
        assert_eq!(eval(&mut ctx, "inkline-accept-functions"), "(b a c)");
    }

    #[test]
    fn add_hook_makes_an_unbound_hook_and_wraps_a_single_function() {
        let mut ctx = ctx();
        assert_eq!(eval(&mut ctx, "(add-hook 'my-hook 'f)"), "(f)");
        eval(&mut ctx, "(setq other 'g)");
        assert_eq!(eval(&mut ctx, "(add-hook 'other 'h)"), "(h g)");
        eval(&mut ctx, "(setq lam (lambda () 1))");
        assert_eq!(eval(&mut ctx, "(length (add-hook 'lam 'h))"), "2");
    }

    #[test]
    fn remove_hook_takes_every_copy_out() {
        let mut ctx = ctx();
        eval(&mut ctx, "(setq inkline-line-start-functions '(a b a))");
        eval(&mut ctx, "(remove-hook 'inkline-line-start-functions 'a)");
        assert_eq!(eval(&mut ctx, "inkline-line-start-functions"), "(b)");
        eval(&mut ctx, "(setq single 'b)");
        eval(&mut ctx, "(remove-hook 'single 'b)");
        assert_eq!(eval(&mut ctx, "single"), "nil");
        assert_eq!(eval(&mut ctx, "(remove-hook 'never-bound 'b)"), "nil");
    }

    #[test]
    fn functions_reads_lists_single_functions_and_skips_t() {
        let mut ctx = ctx();
        let names = |ctx: &mut TulispContext, value: &str| {
            eval(ctx, &format!("(setq h {value})"));
            let h = ctx.intern("h");
            functions(&h).iter().map(function_name).collect::<Vec<_>>()
        };
        assert_eq!(names(&mut ctx, "nil"), Vec::<String>::new());
        assert_eq!(names(&mut ctx, "'f"), ["f"]);
        assert_eq!(names(&mut ctx, "(lambda () 1)"), ["lambda"]);
        assert_eq!(
            names(&mut ctx, "(list 'f t (lambda () 1))"),
            ["f", "lambda"]
        );
        let unbound = ctx.intern("never-bound");
        assert!(functions(&unbound).is_empty());
    }

    #[test]
    fn remove_drops_the_failing_function_only() {
        let mut ctx = ctx();
        eval(&mut ctx, "(setq h (list 'f 'g 'f))");
        let h = ctx.intern("h");
        let f = ctx.intern("f");
        remove(&h, &f);
        assert_eq!(eval(&mut ctx, "h"), "(g)");
        let g = ctx.intern("g");
        remove(&h, &g);
        assert_eq!(eval(&mut ctx, "h"), "nil");
    }

    #[test]
    fn the_four_hooks_start_empty() {
        let mut ctx = ctx();
        for hook in [ACCEPT, AFTER_CHANGE, LINE_START, SUGGESTION] {
            assert_eq!(eval(&mut ctx, hook), "nil");
        }
    }
}
