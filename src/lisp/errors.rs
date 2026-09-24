//! Lisp errors: `error` and `user-error` as in Emacs, and the one line inkline
//! shows for an error.

use tulisp::{Error, ErrorKind, Rest, TulispContext, TulispObject};

/// Starts the text of an error raised by `user-error`, so it can be told
/// apart from other errors. Never shown.
const USER_ERROR: &str = "\u{0}user-error\u{0}";

/// The `catch` tag `quit` throws to. A command that ends with it shows
/// nothing.
pub const QUIT: &str = "inkline--quit";

/// The most characters of a form shown after an error.
const FORM_WIDTH: usize = 60;

pub fn register(ctx: &mut TulispContext) {
    ctx.defun(
        "error",
        |ctx: &mut TulispContext, args: Rest<TulispObject>| -> Result<TulispObject, Error> {
            Err(Error::lisp_error(format_args(ctx, args)?))
        },
    );
    ctx.defun(
        "user-error",
        |ctx: &mut TulispContext, args: Rest<TulispObject>| -> Result<TulispObject, Error> {
            Err(Error::lisp_error(format!(
                "{USER_ERROR}{}",
                format_args(ctx, args)?
            )))
        },
    );
}

/// `(format ARGS…)`.
pub fn format_args(
    ctx: &mut TulispContext,
    args: impl IntoIterator<Item = TulispObject>,
) -> Result<String, Error> {
    let format = ctx.intern("format");
    ctx.apply(&format, args.into_iter().collect::<Vec<_>>())?
        .as_string()
}

/// The message of an error raised by `user-error`.
pub fn user_error_text(err: &Error) -> Option<String> {
    if !matches!(err.kind(), ErrorKind::LispError) {
        return None;
    }
    err.desc().strip_prefix(USER_ERROR).map(str::to_owned)
}

/// The error as one line: `<file>:<line>: <text>` for an error in `file`,
/// else `<text>`, with the innermost form added when the text does not name
/// it.
pub fn describe(err: &Error, ctx: &TulispContext, file: Option<&str>) -> String {
    if let ErrorKind::Throw(thrown) = err.kind() {
        let tag = thrown
            .car()
            .map_or_else(|_| thrown.to_string(), |t| t.to_string());
        if tag == QUIT {
            return "Quit".to_owned();
        }
        return format!("no catch for {tag}");
    }
    if let Some(text) = user_error_text(err) {
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
    line_from(&err.desc(), &err.format(ctx), file, names_no_form)
}

/// One trace line of `Error::format`: `PATH:L.C-L.C:  at FORM`.
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
        register(&mut ctx);
        let e = ctx
            .eval_string(r#"(error "bad %s: %d" "x" 3)"#)
            .unwrap_err();
        assert_eq!(e.desc(), "bad x: 3");
        assert_eq!(user_error_text(&e), None);
        let e = ctx
            .eval_string(r#"(user-error "no %s" "way")"#)
            .unwrap_err();
        assert_eq!(user_error_text(&e).as_deref(), Some("no way"));
        assert_eq!(describe(&e, &ctx, None), "no way");
    }

    #[test]
    fn condition_case_catches_user_error_as_an_error() {
        let mut ctx = TulispContext::new();
        register(&mut ctx);
        let caught = ctx
            .eval_string(r#"(condition-case nil (user-error "x") (error 'caught))"#)
            .unwrap();
        assert_eq!(caught.to_string(), "caught");
    }

    #[test]
    fn an_uncaught_throw_names_its_tag() {
        let mut ctx = TulispContext::new();
        let e = ctx.eval_string("(throw 'done 1)").unwrap_err();
        assert_eq!(describe(&e, &ctx, None), "no catch for done");
    }

    #[test]
    fn quit_reads_quit() {
        let mut ctx = TulispContext::new();
        let e = ctx
            .eval_string(&format!("(throw '{QUIT} nil)"))
            .unwrap_err();
        assert_eq!(describe(&e, &ctx, None), "Quit");
    }
}
