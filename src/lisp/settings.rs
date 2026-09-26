//! inkline's settings: Lisp variables read each time they are used. A bad
//! value falls back to the default and is reported once.

use std::cell::RefCell;

use tulisp::{TulispContext, TulispObject};

use crate::colors::Colors;

const DEFINITIONS: &str = "
(defvar inkline-indent 4)
(defvar inkline-suggestion-lines 5)
(defvar inkline-history-cursor 'start)
(defvar inkline-colors nil)
";

struct Symbols {
    indent: TulispObject,
    suggestion_lines: TulispObject,
    history_cursor: TulispObject,
    colors: TulispObject,
}

#[derive(Default)]
struct Cache {
    /// A copy of the `inkline-colors` value `colors` was made from.
    colors_from: Option<TulispObject>,
    colors: Colors,
    /// Bad values already reported, by variable name.
    reported: Vec<(&'static str, TulispObject)>,
    /// Bad values found and not reported yet.
    pending: Vec<String>,
}

thread_local! {
    static SYMBOLS: RefCell<Option<Symbols>> = const { RefCell::new(None) };
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// Defines the variables in a new interpreter and keeps their symbols, so
/// they can be read while the interpreter is busy.
pub fn register(ctx: &mut TulispContext) {
    ctx.eval_string(DEFINITIONS)
        .expect("the settings are defined");
    let symbols = Symbols {
        indent: ctx.intern("inkline-indent"),
        suggestion_lines: ctx.intern("inkline-suggestion-lines"),
        history_cursor: ctx.intern("inkline-history-cursor"),
        colors: ctx.intern("inkline-colors"),
    };
    SYMBOLS.with_borrow_mut(|s| *s = Some(symbols));
    CACHE.with_borrow_mut(|c| *c = Cache::default());
}

pub fn parse_indent(v: &TulispObject) -> Result<usize, String> {
    v.as_int()
        .ok()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n <= 16)
        .ok_or_else(|| "expected a number from 0 to 16".to_owned())
}

pub fn parse_lines(v: &TulispObject) -> Result<usize, String> {
    v.as_int()
        .ok()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n >= 1)
        .ok_or_else(|| "expected a number of at least 1".to_owned())
}

/// Whether the value asks for `end`.
pub fn parse_cursor(v: &TulispObject) -> Result<bool, String> {
    match (v.symbolp(), v.to_string().as_str()) {
        (true, "start") => Ok(false),
        (true, "end") => Ok(true),
        _ => Err("expected start or end".to_owned()),
    }
}

const BAD_COLORS: &str = "expected a list of (NAME . \"VALUE\") pairs or a string";

pub fn parse_colors(v: &TulispObject) -> Result<Colors, String> {
    if v.null() {
        return Ok(Colors::default());
    }
    if v.stringp() {
        return Ok(Colors::parse(&v.as_string().map_err(|e| e.desc())?));
    }
    if !v.consp() {
        return Err(BAD_COLORS.to_owned());
    }
    let mut entries = Vec::new();
    let mut pairs = v.base_iter();
    for pair in pairs.by_ref() {
        let (Ok(name), Ok(codes)) = (pair.car(), pair.cdr()) else {
            return Err(BAD_COLORS.to_owned());
        };
        let name = if name.stringp() {
            name.as_string().map_err(|e| e.desc())?
        } else if name.symbolp() {
            name.to_string()
        } else {
            return Err(BAD_COLORS.to_owned());
        };
        let codes = codes
            .as_string()
            .map_err(|_| format!("{name}: expected a string"))?;
        entries.push((name, codes));
    }
    pairs.take_error().map_err(|_| BAD_COLORS.to_owned())?;
    Colors::from_entries(&entries)
}

/// The value of the variable `pick` names, if it is set.
fn current(pick: fn(&Symbols) -> &TulispObject) -> Option<TulispObject> {
    SYMBOLS.with_borrow(|s| s.as_ref().and_then(|s| pick(s).get().ok()))
}

/// Reads the variable `pick` names and parses it; on a bad value, notes it
/// for `problems` once and gives `default`.
fn read<T>(
    name: &'static str,
    pick: fn(&Symbols) -> &TulispObject,
    parse: fn(&TulispObject) -> Result<T, String>,
    default: T,
) -> T {
    let Some(value) = current(pick) else {
        return default;
    };
    match parse(&value) {
        Ok(v) => v,
        Err(why) => {
            note(name, &value, why);
            default
        }
    }
}

/// `equal`, except that it gives up and says yes after 4096 conses or 32
/// levels of nesting, or on two atoms that are not symbols, strings or
/// numbers, so a circular value cannot make it recurse forever.
fn looks_equal(a: &TulispObject, b: &TulispObject) -> bool {
    fn walk(a: &TulispObject, b: &TulispObject, depth: usize, budget: &mut usize) -> bool {
        let (mut a, mut b) = (a.clone(), b.clone());
        loop {
            if a.eq(&b) {
                return true;
            }
            if !a.consp() || !b.consp() {
                let plain = |o: &TulispObject| o.symbolp() || o.stringp() || o.numberp();
                return !a.consp() && !b.consp() && (!(plain(&a) || plain(&b)) || a.equal(&b));
            }
            if *budget == 0 || depth == 0 {
                return true;
            }
            *budget -= 1;
            let (Ok(a_car), Ok(b_car), Ok(a_cdr), Ok(b_cdr)) = (a.car(), b.car(), a.cdr(), b.cdr())
            else {
                return false;
            };
            if !walk(&a_car, &b_car, depth - 1, budget) {
                return false;
            }
            (a, b) = (a_cdr, b_cdr);
        }
    }
    walk(a, b, 32, &mut 4096)
}

fn note(name: &'static str, value: &TulispObject, why: String) {
    CACHE.with_borrow_mut(|c| {
        if !c
            .reported
            .iter()
            .any(|(n, v)| *n == name && looks_equal(v, value))
        {
            c.reported
                .push((name, value.deep_copy().unwrap_or_else(|_| value.clone())));
            c.pending.push(format!("{name}: {why}"));
        }
    });
}

pub fn indent() -> usize {
    read("inkline-indent", |s| &s.indent, parse_indent, 4)
}

pub fn suggestion_lines() -> usize {
    read(
        "inkline-suggestion-lines",
        |s| &s.suggestion_lines,
        parse_lines,
        5,
    )
}

/// Whether `inkline-history-cursor` is `end`.
pub fn history_cursor_end() -> bool {
    read(
        "inkline-history-cursor",
        |s| &s.history_cursor,
        parse_cursor,
        false,
    )
}

/// The colours. A value that parses is kept, and parsed again only once the
/// value is no longer `equal` to it; a kept value is finite, so `equal` on it
/// ends.
pub fn colors() -> Colors {
    let Some(value) = current(|s| &s.colors) else {
        return Colors::default();
    };
    let cached = CACHE.with_borrow(|c| {
        c.colors_from
            .as_ref()
            .is_some_and(|from| from.equal(&value))
            .then(|| c.colors.clone())
    });
    if let Some(colors) = cached {
        return colors;
    }
    let colors = match parse_colors(&value) {
        Ok(colors) => colors,
        Err(why) => {
            note("inkline-colors", &value, why);
            return Colors::default();
        }
    };
    CACHE.with_borrow_mut(|c| {
        c.colors_from = Some(value.deep_copy().unwrap_or(value));
        c.colors = colors.clone();
    });
    colors
}

/// Reads every setting and returns the bad values not reported before, as
/// `NAME: why` lines.
pub fn problems() -> Vec<String> {
    indent();
    suggestion_lines();
    history_cursor_end();
    colors();
    CACHE.with_borrow_mut(|c| std::mem::take(&mut c.pending))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colors::Colors;
    use crate::lexer::Kind;

    fn value(ctx: &mut TulispContext, text: &str) -> TulispObject {
        ctx.eval_string(text).unwrap()
    }

    #[test]
    fn indent_values() {
        let mut ctx = TulispContext::new();
        assert_eq!(parse_indent(&value(&mut ctx, "2")), Ok(2));
        assert_eq!(parse_indent(&value(&mut ctx, "0")), Ok(0));
        let bad = Err("expected a number from 0 to 16".to_owned());
        assert_eq!(parse_indent(&value(&mut ctx, "17")), bad);
        assert_eq!(parse_indent(&value(&mut ctx, r#""four""#)), bad);
    }

    #[test]
    fn suggestion_line_values() {
        let mut ctx = TulispContext::new();
        assert_eq!(parse_lines(&value(&mut ctx, "3")), Ok(3));
        assert_eq!(
            parse_lines(&value(&mut ctx, "0")),
            Err("expected a number of at least 1".to_owned())
        );
    }

    #[test]
    fn history_cursor_values() {
        let mut ctx = TulispContext::new();
        assert_eq!(parse_cursor(&value(&mut ctx, "'end")), Ok(true));
        assert_eq!(parse_cursor(&value(&mut ctx, "'start")), Ok(false));
        assert_eq!(
            parse_cursor(&value(&mut ctx, "'middle")),
            Err("expected start or end".to_owned())
        );
    }

    #[test]
    fn colour_values() {
        let mut ctx = TulispContext::new();
        assert_eq!(parse_colors(&value(&mut ctx, "nil")), Ok(Colors::default()));
        let c = parse_colors(&value(&mut ctx, r#"'((command . "35") ("string" . "1"))"#)).unwrap();
        assert_eq!((c.sgr(Kind::Command), c.sgr(Kind::String)), ("35", "1"));
        let c = parse_colors(&value(&mut ctx, r#""command=38:5:208:error=""#)).unwrap();
        assert_eq!((c.sgr(Kind::Command), c.error()), ("38:5:208", ""));
        assert_eq!(
            parse_colors(&value(&mut ctx, "5")),
            Err("expected a list of (NAME . \"VALUE\") pairs or a string".to_owned())
        );
        assert_eq!(
            parse_colors(&value(&mut ctx, r#"'((command . 32))"#)),
            Err("command: expected a string".to_owned())
        );
        let bad = Err("expected a list of (NAME . \"VALUE\") pairs or a string".to_owned());
        for text in [
            r#"'((command . "35") . 5)"#,
            r#"(let ((l (list (cons 'command "35")))) (setcdr l l) l)"#,
            r#"(let ((l (list 1))) (setcar l l) (list l))"#,
        ] {
            assert_eq!(parse_colors(&value(&mut ctx, text)), bad, "{text}");
        }
        let c = parse_colors(&value(&mut ctx, r#"'((command . "bold magenta"))"#)).unwrap();
        assert_eq!(c.sgr(Kind::Command), "1;35");
        assert_eq!(
            parse_colors(&value(&mut ctx, r#"'((command . "magneta"))"#)),
            Err("command: unknown colour word \"magneta\"".to_owned())
        );
        let c = parse_colors(&value(&mut ctx, r#""command=bold magenta:string=magneta""#)).unwrap();
        assert_eq!(
            (c.sgr(Kind::Command), c.sgr(Kind::String)),
            ("1;35", "33"),
            "the string form skips what it cannot read"
        );
    }

    #[test]
    fn a_circular_value_set_twice_is_reported_once() {
        crate::lisp::start();
        let circular =
            r#"(setq inkline-colors (let ((l (list (cons 'command "35")))) (setcdr l l) l))"#;
        for reports in [1, 0] {
            crate::lisp::eval(circular).unwrap();
            assert_eq!(colors(), Colors::default());
            assert_eq!(colors(), Colors::default());
            assert_eq!(problems().len(), reports);
        }
    }

    #[test]
    fn reading_follows_the_variables_and_reports_each_bad_value_once() {
        crate::lisp::start();
        assert_eq!(indent(), 4);
        assert_eq!(
            crate::lisp::eval("(setq inkline-indent 2)"),
            Ok(Some("2".into()))
        );
        assert_eq!(indent(), 2);
        crate::lisp::eval(r#"(setq inkline-indent "x")"#).unwrap();
        assert_eq!(indent(), 4);
        assert_eq!(
            problems(),
            vec!["inkline-indent: expected a number from 0 to 16".to_owned()]
        );
        assert!(problems().is_empty());
        crate::lisp::eval(r#"(setq inkline-colors '((command . "35")))"#).unwrap();
        assert_eq!(colors().sgr(Kind::Command), "35");
        crate::lisp::eval(r#"(setcdr (car inkline-colors) "36")"#).unwrap();
        assert_eq!(colors().sgr(Kind::Command), "36");
    }
}
