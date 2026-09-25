//! `inkline-highlight-arguments`: registering the helper program that colours
//! a command's arguments.

use tulisp::{Error, TulispContext, TulispObject};

use super::buffer::refuse_when_read_only;

const NAME: &str = "inkline-highlight-arguments";

/// Defines `(inkline-highlight-arguments NAME PROGRAM)`: `PROGRAM`, a list of
/// non-empty strings, becomes the helper for the command `NAME`; `nil`
/// removes it. Refused in `inkline-suggestion-functions`, which may only
/// read.
pub fn register(ctx: &mut TulispContext) {
    ctx.defun(
        NAME,
        |name: TulispObject, program: TulispObject| -> Result<TulispObject, Error> {
            refuse_when_read_only(NAME)?;
            if !name.stringp() {
                return Err(wrong_type("stringp", &name));
            }
            let program = if program.null() {
                None
            } else {
                Some(program_words(&program)?)
            };
            crate::helper::register(&name.as_string()?, program);
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
    use crate::helper;
    use crate::lisp::buffer::{TextBuffer, install, set_writable};

    fn eval(expr: &str) -> Result<Option<String>, String> {
        crate::lisp::eval(expr)
    }

    #[test]
    fn registers_replaces_and_removes() {
        assert_eq!(
            eval(r#"(inkline-highlight-arguments "csvm" '("csvm" "--highlight"))"#),
            Ok(None)
        );
        assert!(helper::is_registered("csvm"));
        assert_eq!(
            eval(r#"(inkline-highlight-arguments "csvm" (list "other"))"#),
            Ok(None)
        );
        assert_eq!(helper::status_lines(), ["highlight csvm: not started"]);
        assert_eq!(
            eval(r#"(inkline-highlight-arguments "csvm" nil)"#),
            Ok(None)
        );
        assert!(!helper::is_registered("csvm"));
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
        assert!(helper::status_lines().is_empty());
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
        assert!(!helper::is_registered("csvm"));
    }
}
