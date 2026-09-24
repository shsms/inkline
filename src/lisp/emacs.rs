//! Emacs functions that tulisp lacks.

use tulisp::{Error, Rest, TulispContext, TulispObject};

/// The ones easiest to write in Lisp. tulisp calls the innermost binding of
/// an operator's name, so no parameter may share a name with a function the
/// body calls: `add-to-list`'s third parameter is `at-end` (the body calls
/// `append`), and `delq`'s list is `lst`. `(length ...)` before a walk down a
/// list signals an error on a circular list, where the walk would never end.
const PRELUDE: &str = r#"
(defmacro push (newelt place) (list 'setq place (list 'cons newelt place)))
(defmacro pop (place) (list 'prog1 (list 'car place) (list 'setq place (list 'cdr place))))
(defmacro defconst (symbol value &optional _doc)
  (list 'progn (list 'defvar symbol) (list 'setq symbol value) (list 'quote symbol)))
(defmacro ignore-errors (&rest body)
  (list 'condition-case nil (cons 'progn body) '(error nil)))
(defun add-to-list (list-var element &optional at-end)
  (let ((old (symbol-value list-var)))
    (if (member element old)
        old
      (set list-var (if at-end (append old (list element)) (cons element old))))))
(defun assq (key alist)
  (length alist)
  (let ((found nil))
    (while (and alist (not found))
      (when (and (consp (car alist)) (eq (car (car alist)) key))
        (setq found (car alist)))
      (setq alist (cdr alist)))
    found))
(defun inkline--keep (pred seq)
  (length seq)
  (let ((out nil))
    (dolist (x seq) (when (funcall pred x) (setq out (cons x out))))
    (reverse out)))
(defun delete (elt seq) (inkline--keep (lambda (x) (not (equal x elt))) seq))
(defun remove (elt seq) (inkline--keep (lambda (x) (not (equal x elt))) seq))
(defun delq (elt lst) (inkline--keep (lambda (x) (not (eq x elt))) lst))
(defun mapc (function sequence) (length sequence) (dolist (x sequence) (funcall function x)) sequence)
(defun nreverse (seq) (reverse seq))
(defun car-safe (x) (if (consp x) (car x) nil))
(defun cdr-safe (x) (if (consp x) (cdr x) nil))
(defun ignore (&rest _args) nil)
(defun identity (x) x)
(defun zerop (n) (= n 0))
(defun defalias (symbol definition &optional _doc) (set symbol definition) symbol)
"#;

pub fn register(ctx: &mut TulispContext) {
    ctx.defun(
        "substring",
        |string: String, from: Option<i64>, to: Option<i64>| -> Result<String, Error> {
            let chars: Vec<char> = string.chars().collect();
            let len = chars.len() as i64;
            let from = from.unwrap_or(0);
            let to = to.unwrap_or(len);
            let (from, to) = (
                if from < 0 { len + from } else { from },
                if to < 0 { len + to } else { to },
            );
            if from < 0 || to > len || from > to {
                return Err(Error::out_of_range(format!(
                    "Args out of range: {string:?}, {from}, {to}"
                )));
            }
            Ok(chars[from as usize..to as usize].iter().collect())
        },
    );
    ctx.defun(
        "string-prefix-p",
        |prefix: String, string: String, ignore_case: Option<TulispObject>| -> bool {
            if ignore_case.is_some_and(|c| !c.null()) {
                string.to_lowercase().starts_with(&prefix.to_lowercase())
            } else {
                string.starts_with(&prefix)
            }
        },
    );
    ctx.defun(
        "string-suffix-p",
        |suffix: String, string: String, ignore_case: Option<TulispObject>| -> bool {
            if ignore_case.is_some_and(|c| !c.null()) {
                string.to_lowercase().ends_with(&suffix.to_lowercase())
            } else {
                string.ends_with(&suffix)
            }
        },
    );
    ctx.defun(
        "string-search",
        |needle: String, haystack: String, start: Option<i64>| -> Result<Option<i64>, Error> {
            let start = start.unwrap_or(0);
            let Some((from, _)) = haystack
                .char_indices()
                .chain([(haystack.len(), ' ')])
                .nth(start.max(0) as usize)
                .filter(|_| start >= 0)
            else {
                return Err(Error::out_of_range(format!("Args out of range: {start}")));
            };
            Ok(haystack[from..]
                .find(&needle)
                .map(|at| start + haystack[from..from + at].chars().count() as i64))
        },
    );
    ctx.defun(
        "split-string",
        |string: String,
         separators: Option<String>,
         omit_nulls: Option<TulispObject>|
         -> Vec<String> {
            match separators {
                None => string.split_whitespace().map(str::to_owned).collect(),
                Some(sep) => {
                    let omit = omit_nulls.is_some_and(|o| !o.null());
                    string
                        .split(sep.as_str())
                        .filter(|p| !omit || !p.is_empty())
                        .map(str::to_owned)
                        .collect()
                }
            }
        },
    );
    ctx.defun("string-trim", |string: String| -> String {
        string.trim_matches([' ', '\t', '\n', '\r']).to_owned()
    });
    ctx.defun(
        "string-replace",
        |from: String, to: String, string: String| -> Result<String, Error> {
            if from.is_empty() {
                return Err(Error::invalid_argument(
                    "string-replace: empty FROM".to_owned(),
                ));
            }
            Ok(string.replace(&from, &to))
        },
    );
    ctx.defun("string-empty-p", |string: String| -> bool {
        string.is_empty()
    });
    ctx.defun(
        "string-to-number",
        |string: String, base: Option<i64>| -> Result<TulispObject, Error> {
            let base = base.unwrap_or(10);
            if !(2..=16).contains(&base) {
                return Err(Error::out_of_range(format!("Args out of range: {base}")));
            }
            Ok(string_to_number(&string, base))
        },
    );
    ctx.defun("number-to-string", |n: TulispObject| -> String {
        n.to_string()
    });
    ctx.defun("upcase", |x: TulispObject| -> Result<TulispObject, Error> {
        change_case(
            &x,
            |c| c.to_uppercase().next().unwrap_or(c),
            str::to_uppercase,
        )
    });
    ctx.defun(
        "downcase",
        |x: TulispObject| -> Result<TulispObject, Error> {
            change_case(
                &x,
                |c| c.to_lowercase().next().unwrap_or(c),
                str::to_lowercase,
            )
        },
    );
    ctx.defun("char-to-string", |c: i64| -> Result<String, Error> {
        Ok(char_of(c)?.to_string())
    });
    ctx.defun("string", |chars: Rest<i64>| -> Result<String, Error> {
        chars.into_iter().map(char_of).collect()
    });
    ctx.defun("string-to-char", |s: String| -> i64 {
        s.chars().next().map_or(0, |c| c as i64)
    });
    ctx.defun(
        "fboundp",
        |ctx: &mut TulispContext, symbol: TulispObject| -> bool { symbol.functionp(ctx) },
    );
    ctx.defun(
        "symbol-name",
        |symbol: TulispObject| -> Result<String, Error> {
            if symbol.symbolp() {
                Ok(symbol.to_string())
            } else {
                Err(Error::type_mismatch(format!("Expected a symbol: {symbol}")))
            }
        },
    );
    ctx.eval_prelude("<inkline>", PRELUDE)
        .expect("inkline's own Lisp compiles");
}

fn char_of(c: i64) -> Result<char, Error> {
    u32::try_from(c)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| Error::type_mismatch(format!("Not a character: {c}")))
}

fn change_case(
    x: &TulispObject,
    one: fn(char) -> char,
    all: fn(&str) -> String,
) -> Result<TulispObject, Error> {
    if x.stringp() {
        Ok(all(&x.as_string()?).into())
    } else {
        Ok((one(char_of(x.as_int()?)?) as i64).into())
    }
}

/// Emacs's `string-to-number`: the number at the start of `text`, after
/// blanks; 0 when there is none. Only base 10 reads fractions and exponents.
/// `base` is from 2 to 16.
fn string_to_number(text: &str, base: i64) -> TulispObject {
    let text = text.trim_start_matches([' ', '\t', '\n']);
    if base != 10 {
        let digits: String = text
            .chars()
            .take_while(|c| c.is_digit(base as u32))
            .collect();
        return i64::from_str_radix(&digits, base as u32)
            .unwrap_or(0)
            .into();
    }
    let bytes = text.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        end = 1;
    }
    let digits_start = end;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    let mut float = false;
    if bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_digit) {
        float = true;
        end += 1;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
    }
    if end > digits_start && matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exp = end + 1;
        if matches!(bytes.get(exp), Some(b'+' | b'-')) {
            exp += 1;
        }
        if bytes.get(exp).is_some_and(u8::is_ascii_digit) {
            float = true;
            end = exp;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
        }
    }
    let number = &text[..end];
    if float {
        number.parse::<f64>().unwrap_or(0.0).into()
    } else {
        number.parse::<i64>().unwrap_or(0).into()
    }
}

#[cfg(test)]
mod tests {
    use tulisp::TulispContext;

    fn eval(program: &str) -> String {
        let mut ctx = TulispContext::new();
        crate::lisp::errors::register(&mut ctx);
        super::register(&mut ctx);
        match ctx.eval_string(program) {
            Ok(v) => v.to_string(),
            Err(e) => format!("ERROR {}", e.desc()),
        }
    }

    #[test]
    fn strings() {
        assert_eq!(eval(r#"(substring "héllo" 1 3)"#), r#""él""#);
        assert_eq!(eval(r#"(substring "hello" -3)"#), r#""llo""#);
        assert!(eval(r#"(substring "abc" 2 9)"#).starts_with("ERROR"));
        assert_eq!(eval(r#"(string-prefix-p "gi" "git")"#), "t");
        assert_eq!(eval(r#"(string-prefix-p "GI" "git" t)"#), "t");
        assert_eq!(eval(r#"(string-suffix-p "it" "git")"#), "t");
        assert_eq!(eval(r#"(string-search "l" "héllo")"#), "2");
        assert_eq!(eval(r#"(string-search "l" "héllo" 3)"#), "3");
        assert_eq!(eval(r#"(string-search "z" "abc")"#), "nil");
        assert_eq!(eval(r#"(split-string "  a b\tc ")"#), r#"("a" "b" "c")"#);
        assert_eq!(eval(r#"(split-string "a,,b" ",")"#), r#"("a" "" "b")"#);
        assert_eq!(eval(r#"(split-string "a,,b" "," t)"#), r#"("a" "b")"#);
        assert_eq!(eval(r#"(string-trim "  x y \n")"#), r#""x y""#);
        assert_eq!(eval(r#"(string-replace "o" "0" "foo")"#), r#""f00""#);
        assert_eq!(eval(r#"(string-empty-p "")"#), "t");
        assert_eq!(eval(r#"(string-to-number " 42abc")"#), "42");
        assert_eq!(eval(r#"(string-to-number "1.5")"#), "1.5");
        assert_eq!(eval(r#"(string-to-number "ff" 16)"#), "255");
        assert_eq!(eval(r#"(string-to-number "x")"#), "0");
        for base in [1, -1, 17, 99] {
            assert_eq!(
                eval(&format!(r#"(string-to-number "1" {base})"#)),
                format!("ERROR Args out of range: {base}")
            );
        }
        assert_eq!(eval(r#"(string-to-number "11" 2)"#), "3");
        assert_eq!(eval("(number-to-string 7)"), r#""7""#);
        assert_eq!(eval(r#"(upcase "abé")"#), r#""ABÉ""#);
        assert_eq!(eval("(upcase 97)"), "65");
        assert_eq!(eval(r#"(downcase "AB")"#), r#""ab""#);
    }

    #[test]
    fn characters() {
        assert_eq!(eval("(char-to-string 233)"), r#""é""#);
        assert_eq!(eval("(string 97 98)"), r#""ab""#);
        assert_eq!(eval(r#"(string-to-char "é")"#), "233");
        assert_eq!(eval(r#"(string-to-char "")"#), "0");
    }

    #[test]
    fn lists_and_symbols() {
        assert_eq!(eval("(let ((l '(2))) (push 1 l) l)"), "(1 2)");
        assert_eq!(eval("(let ((l '(1 2))) (list (pop l) l))"), "(1 (2))");
        assert_eq!(
            eval("(progn (defvar xs '(a)) (add-to-list 'xs 'b) (add-to-list 'xs 'a) xs)"),
            "(b a)"
        );
        assert_eq!(
            eval("(progn (defvar ys '(a)) (add-to-list 'ys 'b t) ys)"),
            "(a b)"
        );
        assert_eq!(eval("(assq 'b '((a . 1) (b . 2)))"), "(b . 2)");
        assert_eq!(eval(r#"(delete "a" '("a" "b" "a"))"#), r#"("b")"#);
        assert_eq!(eval("(remove 1 '(1 2 1))"), "(2)");
        assert_eq!(eval("(delq 'a '(a b a))"), "(b)");
        assert_eq!(
            eval("(let ((n 0)) (mapc (lambda (x) (setq n (+ n x))) '(1 2 3)) n)"),
            "6"
        );
        assert_eq!(eval("(nreverse '(1 2 3))"), "(3 2 1)");
        for call in [
            "(assq 'a l)",
            "(delete 5 l)",
            "(remove 5 l)",
            "(delq 5 l)",
            "(mapc 'ignore l)",
        ] {
            let out = eval(&format!("(let ((l (list 1 2))) (setcdr (cdr l) l) {call})"));
            assert!(out.starts_with("ERROR Circular list"), "{call}: {out}");
        }
        assert_eq!(eval("(list (car-safe 5) (cdr-safe '(1 . 2)))"), "(nil 2)");
        assert_eq!(
            eval("(list (fboundp 'car) (fboundp 'no-such-thing))"),
            "(t nil)"
        );
        assert_eq!(eval("(symbol-name 'abc)"), r#""abc""#);
        assert_eq!(
            eval("(list (ignore 1 2) (identity 3) (zerop 0))"),
            "(nil 3 t)"
        );
        assert_eq!(
            eval("(progn (defalias 'twice (lambda (x) (* 2 x))) (twice 4))"),
            "8"
        );
        assert_eq!(eval("(progn (defconst k 5) k)"), "5");
    }

    #[test]
    fn ignore_errors() {
        assert_eq!(eval("(ignore-errors (car 1))"), "nil");
        assert_eq!(eval("(ignore-errors 1 2)"), "2");
    }
}
