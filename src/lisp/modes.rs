//! `inkline-define-mode`: defining a command mode, with the mode server
//! that supplies it and its own colours. Which commands use a mode is
//! `inkline-command-mode-alist` (see `settings::command_modes`).

use tulisp::{Error, TulispContext, TulispObject};

use super::buffer::refuse_when_read_only;
use super::settings::parse_color_set;
use super::values::{describe, items};

const NAME: &str = "inkline-define-mode";

/// Defines `(inkline-define-mode NAME PROGRAM &optional COLORS)`: the mode
/// `NAME`, a symbol other than `nil` and `t`, gets `PROGRAM`, a list of
/// non-empty strings, as its mode server; `nil` removes the mode. `COLORS`,
/// in the forms `inkline-colors` takes, are the mode's own colours; left
/// out or `nil`, it has none. A bad argument is an error, and nothing
/// changes. Refused in `inkline-suggestion-functions`, which may only read.
pub fn register(ctx: &mut TulispContext) {
    ctx.defun(
        NAME,
        |name: TulispObject,
         program: TulispObject,
         colors: Option<TulispObject>|
         -> Result<TulispObject, Error> {
            refuse_when_read_only(NAME)?;
            if !name.symbolp() {
                return Err(wrong_type("symbolp", &name));
            }
            if name.null() || name.eq(&TulispObject::t()) {
                return Err(wrong_type("a symbol other than nil and t", &name));
            }
            let program = if program.null() {
                None
            } else {
                Some(program_words(&program)?)
            };
            let colors = colors
                .filter(|c| !c.null())
                .map(|c| parse_color_set(&c))
                .transpose()
                .map_err(|why| Error::invalid_argument(format!("{NAME}: {why}")))?;
            crate::mode_server::define(&name.to_string(), program, colors);
            Ok(TulispObject::nil())
        },
    );
}

/// The strings of `program`, a list of non-empty strings.
fn program_words(program: &TulispObject) -> Result<Vec<String>, Error> {
    if !program.consp() {
        return Err(wrong_type("listp", program));
    }
    let mut elements = items(program);
    let words = elements
        .by_ref()
        .map(|item| {
            if !item.stringp() {
                return Err(wrong_type("stringp", &item));
            }
            let word = item.as_string()?;
            if word.is_empty() {
                return Err(wrong_type("a non-empty string", &item));
            }
            Ok(word)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !elements.proper() {
        return Err(wrong_type("a list of strings", program));
    }
    Ok(words)
}

fn wrong_type(expected: &str, value: &TulispObject) -> Error {
    Error::type_mismatch(format!(
        "Wrong type argument: {expected}, {}",
        describe(value)
    ))
}

#[cfg(test)]
mod tests {
    use crate::lisp::buffer::{TextBuffer, install, set_writable};
    use crate::lisp::settings::command_modes;
    use crate::mode_server;

    fn eval(expr: &str) -> Result<Option<String>, String> {
        crate::lisp::eval(expr)
    }

    /// The mode the command `word` uses, as the alist is now.
    fn mode(word: &str) -> Option<String> {
        mode_server::mode_for(word, &command_modes())
    }

    #[test]
    fn defines_replaces_and_removes() {
        assert_eq!(
            eval(r#"(inkline-define-mode 'csvm-mode '("csvm" "--inkline-mode"))"#),
            Ok(None)
        );
        assert!(mode_server::is_defined("csvm-mode"));
        assert_eq!(
            eval(r#"(inkline-define-mode 'csvm-mode (list "other"))"#),
            Ok(None)
        );
        assert_eq!(
            mode_server::status_lines(&[]),
            ["mode csvm-mode (): not started"]
        );
        assert_eq!(eval("(inkline-define-mode 'csvm-mode nil)"), Ok(None));
        assert!(!mode_server::is_defined("csvm-mode"));
    }

    #[test]
    fn wrong_types() {
        for (expr, message) in [
            (
                r#"(inkline-define-mode "csvm" nil)"#,
                r#"Wrong type argument: symbolp, "csvm""#,
            ),
            (
                "(inkline-define-mode 1 nil)",
                "Wrong type argument: symbolp, 1",
            ),
            (
                r#"(inkline-define-mode nil '("x"))"#,
                "Wrong type argument: a symbol other than nil and t, nil",
            ),
            (
                r#"(inkline-define-mode t '("x"))"#,
                "Wrong type argument: a symbol other than nil and t, t",
            ),
            (
                r#"(inkline-define-mode 'm "csvm")"#,
                r#"Wrong type argument: listp, "csvm""#,
            ),
            (
                r#"(inkline-define-mode 'm '("csvm" 2))"#,
                "Wrong type argument: stringp, 2",
            ),
            (
                r#"(inkline-define-mode 'm '("csvm" ""))"#,
                r#"Wrong type argument: a non-empty string, """#,
            ),
            (
                r#"(inkline-define-mode 'm '("csvm" . "x"))"#,
                r#"Wrong type argument: a list of strings, ("csvm" . "x")"#,
            ),
        ] {
            let err = eval(expr).unwrap_err();
            assert!(err.starts_with(message), "{expr}: {err}");
        }
        assert!(mode_server::status_lines(&[]).is_empty());
    }

    #[test]
    fn values_that_hold_themselves_are_described_short() {
        use crate::lisp::values::{HOLDS_ITSELF, QUOTES_ITSELF};
        // Set in its own eval, before the calls below. An error's text
        // shows the form it came from, and this form changes a quoted list
        // inside itself, so an error in it would print that list.
        eval(&format!("(progn (setq quotes-itself {QUOTES_ITSELF}) nil)")).unwrap();
        for (expr, message) in [
            (
                format!("(inkline-define-mode {HOLDS_ITSELF} nil)"),
                "Wrong type argument: symbolp, ((((((((...))))))))",
            ),
            (
                format!(r#"(inkline-define-mode 'c (list "x" {HOLDS_ITSELF}))"#),
                "Wrong type argument: stringp, ((((((((...))))))))",
            ),
            (
                r#"(inkline-define-mode 'c (cons "x" quotes-itself))"#.to_owned(),
                r#"Wrong type argument: a list of strings, ("x" . ...)"#,
            ),
            (
                format!(r#"(inkline-define-mode 'c '("x") (list (cons 'command {HOLDS_ITSELF})))"#),
                "inkline-define-mode: command: expected a string",
            ),
        ] {
            let err = eval(&expr).unwrap_err();
            assert!(err.starts_with(message), "{expr}: {err}");
        }
        assert!(mode_server::status_lines(&[]).is_empty());
    }

    #[test]
    fn refused_in_suggestion_functions() {
        let _installed = install(Box::new(TextBuffer {
            text: String::new(),
            point: 0,
            mark: 0,
            kills: Vec::new(),
        }));
        set_writable(false);
        let err = eval(r#"(inkline-define-mode 'csvm-mode '("x"))"#).unwrap_err();
        assert!(
            err.starts_with("inkline-define-mode is not allowed in inkline-suggestion-functions"),
            "{err}"
        );
        assert!(!mode_server::is_defined("csvm-mode"));
    }

    fn command_colour(mode: &str) -> String {
        use crate::colors::Colors;
        let set = mode_server::colors(mode).unwrap();
        Colors::default()
            .layered(&set)
            .sgr(crate::lexer::Kind::Command)
            .to_owned()
    }

    #[test]
    fn a_mode_gets_colours_of_its_own() {
        use crate::colors::Colors;
        eval(r#"(inkline-define-mode 'csvm-mode '("x") '((command . "bold magenta")))"#).unwrap();
        assert_eq!(command_colour("csvm-mode"), "1;35");
        eval(r#"(inkline-define-mode 'csvm-mode '("x") "script=on grey3")"#).unwrap();
        let set = mode_server::colors("csvm-mode").unwrap();
        assert_eq!(Colors::default().layered(&set).script(), "48;5;235");
        eval(r#"(inkline-define-mode 'csvm-mode '("x") nil)"#).unwrap();
        assert_eq!(mode_server::colors("csvm-mode"), None);
        eval(r#"(inkline-define-mode 'csvm-mode '("x") '((command . "1")))"#).unwrap();
        eval(r#"(inkline-define-mode 'csvm-mode '("x"))"#).unwrap();
        assert_eq!(
            mode_server::colors("csvm-mode"),
            None,
            "left out: no colours"
        );
    }

    /// A bad colour set is an error that names what is wrong, and leaves
    /// the mode as it was.
    #[test]
    fn bad_colours_define_nothing() {
        eval(r#"(inkline-define-mode 'csvm-mode '("x") '((command . "bold")))"#).unwrap();
        for (colors, message) in [
            (
                r#"'((command . "bold magneta"))"#,
                r#"inkline-define-mode: command: unknown colour word "magneta""#,
            ),
            (
                r#""command=bold magneta""#,
                r#"inkline-define-mode: command: unknown colour word "magneta""#,
            ),
            (
                r#"'((suggestion . "1"))"#,
                "inkline-define-mode: unknown colour name suggestion",
            ),
            (
                "5",
                r#"inkline-define-mode: expected a list of (NAME . "VALUE") pairs or a string"#,
            ),
        ] {
            let err = eval(&format!(
                r#"(inkline-define-mode 'csvm-mode '("y") {colors})"#
            ))
            .unwrap_err();
            assert!(err.contains(message), "{colors}: {err}");
        }
        assert_eq!(
            mode_server::status_lines(&[]),
            ["mode csvm-mode (): not started"]
        );
        assert_eq!(
            command_colour("csvm-mode"),
            "1",
            "still the first definition"
        );
    }

    #[test]
    fn the_alist_is_read_when_used() {
        eval(r#"(inkline-define-mode 'csvm-mode '("x"))"#).unwrap();
        assert_eq!(mode("csvm"), None);
        eval(r#"(setq inkline-command-mode-alist (list (cons "csvm" 'csvm-mode)))"#).unwrap();
        assert_eq!(mode("csvm").as_deref(), Some("csvm-mode"));
        assert_eq!(mode("./target/debug/csvm").as_deref(), Some("csvm-mode"));
        eval(r#"(push (cons "c" 'csvm-mode) inkline-command-mode-alist)"#).unwrap();
        assert_eq!(mode("c").as_deref(), Some("csvm-mode"));
        eval("(setq inkline-command-mode-alist nil)").unwrap();
        assert_eq!(mode("csvm"), None);
    }

    #[test]
    fn the_alist_may_name_a_mode_defined_later() {
        eval(r#"(setq inkline-command-mode-alist '(("csvm" . csvm-mode)))"#).unwrap();
        assert_eq!(mode("csvm"), None);
        assert_eq!(
            mode_server::status_lines(&command_modes()),
            ["command csvm: no mode named csvm-mode"]
        );
        eval(r#"(inkline-define-mode 'csvm-mode '("x"))"#).unwrap();
        assert_eq!(mode("csvm").as_deref(), Some("csvm-mode"));
        assert_eq!(
            mode_server::status_lines(&command_modes()),
            ["mode csvm-mode (csvm): not started"]
        );
    }
}
