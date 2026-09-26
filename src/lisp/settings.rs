//! inkline's settings: Lisp variables read each time they are used. A bad
//! value falls back to the default and is reported once.

use std::cell::RefCell;

use tulisp::{TulispContext, TulispObject};

use super::values::{items, read_int, read_str};
use crate::colors::{ColorSet, Colors};

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
    /// What the `inkline-colors` value held when `colors` was made from it.
    colors_from: Option<ColorsSource>,
    colors: Colors,
    /// Bad values already reported, by variable name.
    reported: Vec<(&'static str, TulispObject)>,
    /// Bad values found and not reported yet.
    pending: Vec<String>,
}

/// What an `inkline-colors` value holds: `nil`, a string, or a list of
/// `(NAME . VALUE)` pairs.
enum ColorsSource {
    Nil,
    Str(String),
    Pairs(Vec<(String, String)>),
}

impl ColorsSource {
    fn read(v: &TulispObject) -> Result<ColorsSource, String> {
        if v.null() {
            Ok(ColorsSource::Nil)
        } else if let Some(s) = read_str(v) {
            Ok(ColorsSource::Str(s))
        } else if v.consp() {
            Ok(ColorsSource::Pairs(color_pairs(v)?))
        } else {
            Err(BAD_COLORS.to_owned())
        }
    }

    fn colors(&self) -> Result<Colors, String> {
        match self {
            ColorsSource::Nil => Ok(Colors::default()),
            ColorsSource::Str(s) => Ok(Colors::parse(s)),
            ColorsSource::Pairs(pairs) => Colors::from_entries(pairs),
        }
    }

    /// Whether the live value `v` still holds the same.
    fn matches(&self, v: &TulispObject) -> bool {
        match self {
            ColorsSource::Nil => v.null(),
            ColorsSource::Str(s) => read_str(v).as_deref() == Some(s.as_str()),
            ColorsSource::Pairs(pairs) => same_pairs(pairs, v),
        }
    }
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
    read_int(v)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n <= 16)
        .ok_or_else(|| "expected a number from 0 to 16".to_owned())
}

pub fn parse_lines(v: &TulispObject) -> Result<usize, String> {
    read_int(v)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n >= 1)
        .ok_or_else(|| "expected a number of at least 1".to_owned())
}

/// Whether the value asks for `end`.
pub fn parse_cursor(v: &TulispObject) -> Result<bool, String> {
    let bad = || Err("expected start or end".to_owned());
    if !v.symbolp() {
        return bad();
    }
    match v.to_string().as_str() {
        "start" => Ok(false),
        "end" => Ok(true),
        _ => bad(),
    }
}

const BAD_COLORS: &str = "expected a list of (NAME . \"VALUE\") pairs or a string";

pub fn parse_colors(v: &TulispObject) -> Result<Colors, String> {
    ColorsSource::read(v)?.colors()
}

/// A command's own colours (`inkline-highlight-arguments`' third
/// argument), in the forms `inkline-colors` takes. Unlike `inkline-colors`,
/// a string with an entry that cannot be read is an error.
pub fn parse_color_set(v: &TulispObject) -> Result<ColorSet, String> {
    if v.stringp() {
        return ColorSet::parse(&v.as_string().map_err(|e| e.desc())?);
    }
    if !v.consp() {
        return Err(BAD_COLORS.to_owned());
    }
    ColorSet::from_entries(&color_pairs(v)?)
}

/// The `(NAME . VALUE)` pairs of the list `v`, a name being a symbol or a
/// string and a value a string. A `nil` entry reads as `(nil . nil)`.
fn color_pairs(v: &TulispObject) -> Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    let mut pairs = items(v);
    for pair in pairs.by_ref() {
        if !pair.listp() {
            return Err(BAD_COLORS.to_owned());
        }
        let (Ok(name), Ok(codes)) = (pair.car(), pair.cdr()) else {
            return Err(BAD_COLORS.to_owned());
        };
        let Some(name) = read_name(&name) else {
            return Err(BAD_COLORS.to_owned());
        };
        let Some(codes) = read_str(&codes) else {
            return Err(format!("{name}: expected a string"));
        };
        entries.push((name, codes));
    }
    if !pairs.proper() {
        return Err(BAD_COLORS.to_owned());
    }
    Ok(entries)
}

/// A symbol's name or a string's content. `None` for anything else.
fn read_name(o: &TulispObject) -> Option<String> {
    if o.symbolp() {
        Some(o.to_string())
    } else {
        read_str(o)
    }
}

/// Whether the list `value` holds exactly the `(NAME . VALUE)` pairs in
/// `kept`, in order. The walk stops after `kept.len()` pairs and one more,
/// so a very long or circular `value` neither overflows the stack nor takes
/// long.
fn same_pairs(kept: &[(String, String)], value: &TulispObject) -> bool {
    let mut pairs = items(value);
    let same = kept.iter().all(|(name, codes)| {
        pairs.next().is_some_and(|pair| {
            pair.consp()
                && matches!(
                    (pair.car(), pair.cdr()),
                    (Ok(n), Ok(c))
                        if read_name(&n).as_deref() == Some(name.as_str())
                            && read_str(&c).as_deref() == Some(codes.as_str())
                )
        })
    });
    same && pairs.next().is_none() && pairs.proper()
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

/// The colours. What a value that parses held is kept, and the value is
/// parsed again only once it no longer holds the same. The comparison
/// walks a list in a loop, so a very long list does not overflow the stack,
/// and reads the live strings each time, so a string changed in place is
/// seen.
pub fn colors() -> Colors {
    let Some(value) = current(|s| &s.colors) else {
        return Colors::default();
    };
    let cached = CACHE.with_borrow(|c| {
        c.colors_from
            .as_ref()
            .is_some_and(|from| from.matches(&value))
            .then(|| c.colors.clone())
    });
    if let Some(colors) = cached {
        return colors;
    }
    let read = ColorsSource::read(&value).and_then(|from| Ok((from.colors()?, from)));
    let (colors, from) = match read {
        Ok(read) => read,
        Err(why) => {
            note("inkline-colors", &value, why);
            return Colors::default();
        }
    };
    CACHE.with_borrow_mut(|c| {
        c.colors_from = Some(from);
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

    /// Sets `variable` to `value` without printing the result.
    fn set(variable: &str, value: &str) {
        crate::lisp::eval(&format!("(progn (setq {variable} {value}) nil)")).unwrap();
    }

    #[test]
    fn values_that_hold_themselves_are_reported_not_printed() {
        use crate::lisp::values::{HOLDS_ITSELF, QUOTES_ITSELF};
        crate::lisp::start();
        set("inkline-indent", HOLDS_ITSELF);
        assert_eq!(indent(), 4);
        set("inkline-suggestion-lines", QUOTES_ITSELF);
        assert_eq!(suggestion_lines(), 5);
        set("inkline-history-cursor", HOLDS_ITSELF);
        assert!(!history_cursor_end());
        for colors in [
            format!("(list {QUOTES_ITSELF})"),
            format!("(cons (cons 'command \"35\") {QUOTES_ITSELF})"),
            format!("(list (cons 'command {HOLDS_ITSELF}))"),
        ] {
            set("inkline-colors", &colors);
            assert_eq!(self::colors(), Colors::default(), "{colors}");
        }
        assert_eq!(problems().len(), 6);
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

    #[test]
    fn command_colour_values() {
        let mut ctx = TulispContext::new();
        let set = parse_color_set(&value(
            &mut ctx,
            r#"'((command . "bold magenta") ("script" . "on grey23"))"#,
        ))
        .unwrap();
        let c = Colors::default().layered(&set);
        assert_eq!((c.sgr(Kind::Command), c.script()), ("1;35", "48;5;255"));
        let set = parse_color_set(&value(&mut ctx, r#""number=yellow""#)).unwrap();
        assert_eq!(Colors::default().layered(&set).sgr(Kind::Number), "33");
        assert_eq!(
            parse_color_set(&value(&mut ctx, r#"'((suggestion . "1"))"#)),
            Err("unknown colour name suggestion".to_owned())
        );
        assert_eq!(
            parse_color_set(&value(&mut ctx, r#""command=1:bogus=2""#)),
            Err("unknown colour name bogus".to_owned())
        );
        assert_eq!(
            parse_color_set(&value(&mut ctx, "5")),
            Err(BAD_COLORS.to_owned())
        );
        assert_eq!(
            parse_color_set(&value(&mut ctx, r#"'((command . 32))"#)),
            Err("command: expected a string".to_owned())
        );
        assert_eq!(
            parse_color_set(&value(&mut ctx, r#"'((command . "1") . 5)"#)),
            Err(BAD_COLORS.to_owned())
        );
    }

    /// Long enough that a comparison that recursed once per list element
    /// would overflow a test thread's 2 MB stack. The second read of such a
    /// list is the one that compares it with what was kept.
    const LONG_LIST: i64 = 200_000;

    #[test]
    fn a_long_colour_list_is_compared_without_recursion() {
        crate::lisp::start();
        crate::lisp::eval(&format!(
            r#"(setq inkline-colors
                   (let ((l nil) (i 0))
                     (while (< i {LONG_LIST})
                       (setq l (cons (cons "string" "1") l))
                       (setq i (1+ i)))
                     l))"#
        ))
        .unwrap();
        assert_eq!(colors().sgr(Kind::String), "1");
        assert_eq!(colors().sgr(Kind::String), "1", "read again");
    }

    #[test]
    fn a_colour_string_changed_in_place_is_seen() {
        crate::lisp::start();
        // `49` and `50` are the character codes of `1` and `2`; tulisp has
        // no character literals.
        crate::lisp::eval(r#"(setq inkline-colors (list (cons 'string (make-string 1 49))))"#)
            .unwrap();
        assert_eq!(colors().sgr(Kind::String), "1");
        crate::lisp::eval("(aset (cdr (car inkline-colors)) 0 50)").unwrap();
        assert_eq!(
            colors().sgr(Kind::String),
            "2",
            "a string changed in place is seen"
        );
    }

    #[test]
    fn a_colour_string_value_changed_is_seen() {
        crate::lisp::start();
        crate::lisp::eval(r#"(setq inkline-colors "command=35")"#).unwrap();
        assert_eq!(colors().sgr(Kind::Command), "35");
        crate::lisp::eval(r#"(setq inkline-colors "command=36")"#).unwrap();
        assert_eq!(colors().sgr(Kind::Command), "36");
    }

    #[test]
    fn a_kept_colour_value_changed_to_one_that_holds_itself_is_reported() {
        use crate::lisp::values::{HOLDS_ITSELF, QUOTES_ITSELF};
        crate::lisp::start();
        set("quotes-itself", QUOTES_ITSELF);
        set("inkline-colors", r#""command=35""#);
        assert_eq!(colors().sgr(Kind::Command), "35");
        set("inkline-colors", HOLDS_ITSELF);
        assert_eq!(colors(), Colors::default());
        set("inkline-colors", r#"(list (cons 'command "35"))"#);
        assert_eq!(colors().sgr(Kind::Command), "35");
        crate::lisp::eval("(progn (setcar inkline-colors quotes-itself) nil)").unwrap();
        assert_eq!(colors(), Colors::default(), "an entry that is not a list");
        set(
            "inkline-colors",
            r#"(list (cons 'command "35") (cons 'string "1"))"#,
        );
        assert_eq!(colors().sgr(Kind::Command), "35");
        crate::lisp::eval("(progn (setcdr inkline-colors quotes-itself) nil)").unwrap();
        assert_eq!(colors(), Colors::default(), "an end that is not a list");
        assert_eq!(problems().len(), 3);
    }
}
