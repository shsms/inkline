//! `inkline-highlight-arguments`: registering the helper program that colours
//! a command's arguments.

use tulisp::{Error, TulispContext, TulispObject};

use super::buffer::refuse_when_read_only;
use super::settings::parse_color_set;

const NAME: &str = "inkline-highlight-arguments";

/// Defines `(inkline-highlight-arguments NAME PROGRAM &optional COLORS)`:
/// `PROGRAM`, a list of non-empty strings, becomes the helper for the
/// command `NAME`; `nil` removes it. `COLORS`, in the forms
/// `inkline-colors` takes, are the command's own colours; left out or
/// `nil`, it has none. A bad `COLORS` is an error, and nothing changes.
/// Refused in `inkline-suggestion-functions`, which may only read.
pub fn register(ctx: &mut TulispContext) {
    ctx.defun(
        NAME,
        |name: TulispObject,
         program: TulispObject,
         colors: Option<TulispObject>|
         -> Result<TulispObject, Error> {
            refuse_when_read_only(NAME)?;
            if !name.stringp() {
                return Err(wrong_type("stringp", &name));
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
            crate::mode_server::register(&name.as_string()?, program, colors);
            Ok(TulispObject::nil())
        },
    );
}

/// The strings of `program`, a list of non-empty strings.
fn program_words(program: &TulispObject) -> Result<Vec<String>, Error> {
    if !program.consp() {
        return Err(wrong_type("listp", program));
    }
    // Stops at the end of a dotted list, and once a circular list comes back
    // to a cell it has been through.
    let mut items = program.base_iter();
    let words = items
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
    items
        .take_error()
        .map_err(|_| wrong_type("a list of strings", program))?;
    Ok(words)
}

fn wrong_type(expected: &str, value: &TulispObject) -> Error {
    Error::type_mismatch(format!("Wrong type argument: {expected}, {value}"))
}

#[cfg(test)]
mod tests {
    use crate::lisp::buffer::{TextBuffer, install, set_writable};
    use crate::mode_server;

    fn eval(expr: &str) -> Result<Option<String>, String> {
        crate::lisp::eval(expr)
    }

    #[test]
    fn registers_replaces_and_removes() {
        assert_eq!(
            eval(r#"(inkline-highlight-arguments "csvm" '("csvm" "--highlight"))"#),
            Ok(None)
        );
        assert!(mode_server::is_registered("csvm"));
        assert_eq!(
            eval(r#"(inkline-highlight-arguments "csvm" (list "other"))"#),
            Ok(None)
        );
        assert_eq!(mode_server::status_lines(), ["highlight csvm: not started"]);
        assert_eq!(
            eval(r#"(inkline-highlight-arguments "csvm" nil)"#),
            Ok(None)
        );
        assert!(!mode_server::is_registered("csvm"));
    }

    #[test]
    fn wrong_types() {
        for (expr, message) in [
            (
                r#"(inkline-highlight-arguments 1 nil)"#,
                "Wrong type argument: stringp, 1",
            ),
            (
                r#"(inkline-highlight-arguments "c" "csvm")"#,
                r#"Wrong type argument: listp, "csvm""#,
            ),
            (
                r#"(inkline-highlight-arguments "c" '("csvm" 2))"#,
                "Wrong type argument: stringp, 2",
            ),
            (
                r#"(inkline-highlight-arguments "c" '("csvm" ""))"#,
                r#"Wrong type argument: a non-empty string, """#,
            ),
            (
                r#"(inkline-highlight-arguments "c" '("csvm" . "x"))"#,
                r#"Wrong type argument: a list of strings, ("csvm" . "x")"#,
            ),
        ] {
            let err = eval(expr).unwrap_err();
            assert!(err.starts_with(message), "{expr}: {err}");
        }
        assert!(mode_server::status_lines().is_empty());
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
        let err = eval(r#"(inkline-highlight-arguments "csvm" '("x"))"#).unwrap_err();
        assert!(
            err.starts_with(
                "inkline-highlight-arguments is not allowed in inkline-suggestion-functions"
            ),
            "{err}"
        );
        assert!(!mode_server::is_registered("csvm"));
    }

    fn command_colour(name: &str) -> String {
        use crate::colors::Colors;
        let set = mode_server::colors(name).unwrap();
        Colors::default()
            .layered(&set)
            .sgr(crate::lexer::Kind::Command)
            .to_owned()
    }

    #[test]
    fn a_command_gets_colours_of_its_own() {
        use crate::colors::Colors;
        eval(r#"(inkline-highlight-arguments "csvm" '("x") '((command . "bold magenta")))"#)
            .unwrap();
        assert_eq!(command_colour("csvm"), "1;35");
        eval(r#"(inkline-highlight-arguments "csvm" '("x") "script=on grey3")"#).unwrap();
        let set = mode_server::colors("csvm").unwrap();
        assert_eq!(Colors::default().layered(&set).script(), "48;5;235");
        eval(r#"(inkline-highlight-arguments "csvm" '("x") nil)"#).unwrap();
        assert_eq!(mode_server::colors("csvm"), None);
        eval(r#"(inkline-highlight-arguments "csvm" '("x") '((command . "1")))"#).unwrap();
        eval(r#"(inkline-highlight-arguments "csvm" '("x"))"#).unwrap();
        assert_eq!(mode_server::colors("csvm"), None, "left out: no colours");
    }

    /// A bad colour set is an error that names what is wrong, and leaves
    /// the command as it was.
    #[test]
    fn bad_colours_register_nothing() {
        eval(r#"(inkline-highlight-arguments "csvm" '("x") '((command . "bold")))"#).unwrap();
        for (colors, message) in [
            (
                r#"'((command . "bold magneta"))"#,
                r#"inkline-highlight-arguments: command: unknown colour word "magneta""#,
            ),
            (
                r#""command=bold magneta""#,
                r#"inkline-highlight-arguments: command: unknown colour word "magneta""#,
            ),
            (
                r#"'((suggestion . "1"))"#,
                "inkline-highlight-arguments: unknown colour name suggestion",
            ),
            (
                "5",
                r#"inkline-highlight-arguments: expected a list of (NAME . "VALUE") pairs or a string"#,
            ),
        ] {
            let err = eval(&format!(
                r#"(inkline-highlight-arguments "csvm" '("y") {colors})"#
            ))
            .unwrap_err();
            assert!(err.contains(message), "{colors}: {err}");
        }
        assert_eq!(mode_server::status_lines(), ["highlight csvm: not started"]);
        assert_eq!(command_colour("csvm"), "1", "still the first registration");
    }
}
