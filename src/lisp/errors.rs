//! The one line inkline shows for a Lisp error, and `quit`.

use tulisp::{Error, ErrorKind, TulispContext};

/// The most characters of a form shown after an error.
const FORM_WIDTH: usize = 60;

/// The text of `quit`'s error.
const QUIT: &str = "Quit";

/// The error `quit` raises. No `condition-case` or `catch` catches it, and only
/// `unwind-protect` cleanups run as it passes. A command that ends with it
/// shows nothing.
pub fn quit() -> Error {
    Error::interrupted(QUIT)
}

/// What tulisp's interrupt check returns to stop running Lisp with `quit`.
pub fn stop() -> tulisp::Interrupt {
    tulisp::Interrupt::Stop(QUIT.to_owned())
}

/// Whether `err` is a `quit`.
pub fn is_quit(err: &Error) -> bool {
    matches!(err.kind(), ErrorKind::Interrupted)
}

/// The message of an error raised by `user-error`.
pub fn user_error_text(err: &Error, ctx: &TulispContext) -> Option<String> {
    err.is_a(ctx, "user-error").then(|| err.desc().into_owned())
}

/// The error as one line: `<file>:<line>: <text>` for an error in `file`,
/// else `<text>`, with the innermost form added when the text does not name
/// it.
pub fn describe(err: &Error, ctx: &TulispContext, file: Option<&str>) -> String {
    if is_quit(err) {
        return err.desc().into_owned();
    }
    if let Some(text) = user_error_text(err, ctx) {
        return text;
    }
    let names_no_form = matches!(
        err.kind(),
        ErrorKind::ArityMismatch
            | ErrorKind::TypeMismatch
            | ErrorKind::OutOfRange
            | ErrorKind::InvalidArgument
            | ErrorKind::MissingArgument
            | ErrorKind::ArithError
    );
    let trace = err.clone().with_file_names(ctx).to_string();
    line_from(&err.desc(), &trace, file, names_no_form)
}

/// One trace line of a printed `Error`: `PATH:L.C-L.C:  at FORM`.
struct TraceLine<'a> {
    path: &'a str,
    line: &'a str,
    form: &'a str,
}

fn trace_lines(trace: &str) -> impl Iterator<Item = TraceLine<'_>> {
    trace.lines().skip(1).filter_map(|l| {
        let (place, form) = l.split_once(":  at ")?;
        let (path, span) = place.rsplit_once(':')?;
        let (line, _) = span.split_once('.')?;
        Some(TraceLine { path, line, form })
    })
}

fn line_from(desc: &str, trace: &str, file: Option<&str>, add_form: bool) -> String {
    let mut text = desc.to_owned();
    if add_form
        && let Some(first) = trace_lines(trace).next()
        && first.form != "nil"
    {
        let form: String = if first.form.chars().count() > FORM_WIDTH {
            first.form.chars().take(FORM_WIDTH).chain(['…']).collect()
        } else {
            first.form.to_owned()
        };
        text = format!("{text} (in {form})");
    }
    let place = file.and_then(|f| {
        trace_lines(trace)
            .find(|t| t.path == f)
            .map(|t| (f, t.line))
    });
    match place {
        Some((f, line)) => format!("{f}:{line}: {text}"),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tulisp::TulispObject;

    const TRACE: &str = "ERR ArityMismatch: Too many arguments\n\
        /home/u/.config/inkline/init.el:4.25-4.33:  at (car 1 2)\n\
        /home/u/.config/inkline/init.el:4.1-4.34:  at (defun f nil (car 1 2))\n";

    #[test]
    fn a_file_error_names_the_file_line_and_form() {
        assert_eq!(
            line_from(
                "Too many arguments",
                TRACE,
                Some("/home/u/.config/inkline/init.el"),
                true
            ),
            "/home/u/.config/inkline/init.el:4: Too many arguments (in (car 1 2))"
        );
    }

    #[test]
    fn trace_lines_in_other_files_do_not_give_the_place() {
        let trace = "ERR TypeMismatch: Expected number\n\
            /vagrant/tulisp/src/builtin/prelude.lisp:24.23-24.41:  at (funcall func item)\n\
            /tmp/x.el:2.1-2.20:  at (mapcar (quote car) 5)\n";
        assert_eq!(
            line_from("Expected number", trace, Some("/tmp/x.el"), true),
            "/tmp/x.el:2: Expected number (in (funcall func item))"
        );
    }

    #[test]
    fn eval_errors_have_no_place_and_long_forms_are_cut() {
        let trace = format!(
            "ERR ArityMismatch: Too few arguments\n<eval_string>:1.1-1.99:  at (list {})\n",
            "x ".repeat(50)
        );
        let line = line_from("Too few arguments", &trace, None, true);
        assert!(
            line.starts_with("Too few arguments (in (list x x "),
            "{line}"
        );
        assert!(line.ends_with("…)"), "{line}");
        assert!(line.chars().count() < "Too few arguments (in )".len() + 62);
    }

    #[test]
    fn a_parse_error_names_the_line_without_a_form() {
        let trace = "ERR ParsingError: Unclosed list\n/tmp/x.el:2.10-2.10:  at nil\n";
        assert_eq!(
            line_from("Unclosed list", trace, Some("/tmp/x.el"), true),
            "/tmp/x.el:2: Unclosed list"
        );
    }

    #[test]
    fn error_and_user_error_format_their_arguments() {
        let mut ctx = TulispContext::new();
        let e = ctx
            .eval_string(r#"(error "bad %s: %d" "x" 3)"#)
            .unwrap_err();
        assert_eq!(e.desc(), "bad x: 3");
        assert_eq!(user_error_text(&e, &ctx), None);
        let e = ctx
            .eval_string(r#"(user-error "no %s" "way")"#)
            .unwrap_err();
        assert_eq!(user_error_text(&e, &ctx).as_deref(), Some("no way"));
        assert_eq!(describe(&e, &ctx, None), "no way");
    }

    #[test]
    fn condition_case_catches_user_error_as_itself_or_an_error() {
        let mut ctx = TulispContext::new();
        for handler in ["user-error", "error"] {
            let caught = ctx
                .eval_string(&format!(
                    r#"(condition-case e (user-error "x") ({handler} e))"#
                ))
                .unwrap();
            assert_eq!(caught.to_string(), r#"(user-error "x")"#);
        }
    }

    #[test]
    fn an_uncaught_throw_names_its_tag() {
        let mut ctx = TulispContext::new();
        let e = ctx.eval_string("(throw 'done 1)").unwrap_err();
        assert_eq!(describe(&e, &ctx, None), "No catch for tag: done, 1");
    }

    #[test]
    fn quit_reads_quit_and_nothing_catches_it() {
        let mut ctx = TulispContext::new();
        ctx.defun("quit", || -> Result<TulispObject, Error> { Err(quit()) });
        for program in [
            "(quit)",
            "(condition-case nil (quit) (t 'caught))",
            "(catch 'x (quit))",
        ] {
            let e = ctx.eval_string(program).unwrap_err();
            assert!(is_quit(&e), "{program}");
            assert_eq!(describe(&e, &ctx, None), "Quit");
        }
    }
}
