//! Reading Lisp values without ever printing them whole. tulisp prints a
//! value into the text of some errors (`car` of a value that is not a list,
//! a string or a number read from anything else), and printing a list that
//! holds itself never ends: it overflows the stack and takes bash down.

use tulisp::TulispObject;

/// A string's content. `None` for anything else.
pub fn read_str(v: &TulispObject) -> Option<String> {
    if v.stringp() {
        v.as_string().ok()
    } else {
        None
    }
}

/// An integer's value. `None` for anything else.
pub fn read_int(v: &TulispObject) -> Option<i64> {
    if v.integerp() { v.as_int().ok() } else { None }
}

/// The elements of the list `list`, in order. See `Items`.
pub fn items(list: &TulispObject) -> Items {
    Items {
        rest: list.clone(),
        mark: list.clone(),
        steps: 0,
        lap: 1,
        circular: false,
    }
}

/// The elements of a list, walked in a loop. It takes a cell apart only
/// when the cell is a cons, and it stops once a circular list comes back to
/// a cell it has passed (Brent's method: after 1, 2, 4, ... steps `mark`
/// jumps to where the walk is, and the list is circular once the walk meets
/// `mark` again).
pub struct Items {
    rest: TulispObject,
    mark: TulispObject,
    steps: usize,
    lap: usize,
    circular: bool,
}

/// How a list ended.
pub enum End<'a> {
    /// With `nil`.
    Proper,
    /// With this value in place of `nil`.
    Dotted(&'a TulispObject),
    /// Back at a cell the walk had passed.
    Circular,
}

impl Items {
    /// How the list ended, once the walk is over.
    pub fn end(&self) -> End<'_> {
        if self.circular {
            End::Circular
        } else if self.rest.null() {
            End::Proper
        } else {
            End::Dotted(&self.rest)
        }
    }

    /// Whether the list ended with `nil`, once the walk is over.
    pub fn proper(&self) -> bool {
        matches!(self.end(), End::Proper)
    }
}

impl Iterator for Items {
    type Item = TulispObject;

    fn next(&mut self) -> Option<TulispObject> {
        if self.circular || !self.rest.consp() {
            return None;
        }
        let (Ok(item), Ok(next)) = (self.rest.car(), self.rest.cdr()) else {
            return None;
        };
        self.rest = next;
        self.steps += 1;
        if self.rest.consp() && self.rest.eq(&self.mark) {
            self.circular = true;
        } else if self.steps == self.lap {
            self.mark = self.rest.clone();
            self.lap *= 2;
            self.steps = 0;
        }
        Some(item)
    }
}

/// The most atoms `describe` prints.
const MOST_ATOMS: usize = 32;

/// The deepest `describe` goes into lists inside lists.
const MOST_DEPTH: usize = 8;

/// `v` as Lisp prints it, cut short: `...` stands for what is past
/// `MOST_ATOMS` atoms or `MOST_DEPTH` levels, and for any value other than
/// a list, a symbol, a string or a number. A cut at the depth limit counts
/// as an atom, so the text stays short however many elements and levels `v`
/// has. A long string or symbol is printed whole.
pub fn describe(v: &TulispObject) -> String {
    let mut out = String::new();
    let mut atoms = MOST_ATOMS;
    write_value(&mut out, v, &mut atoms, MOST_DEPTH);
    out
}

fn write_value(out: &mut String, v: &TulispObject, atoms: &mut usize, depth: usize) {
    if *atoms == 0 || depth == 0 {
        *atoms = atoms.saturating_sub(1);
        out.push_str("...");
    } else if v.consp() {
        out.push('(');
        let mut elements = items(v);
        for (i, element) in elements.by_ref().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            if *atoms == 0 {
                out.push_str("...)");
                return;
            }
            write_value(out, &element, atoms, depth - 1);
        }
        match elements.end() {
            End::Proper => {}
            End::Dotted(tail) => {
                out.push_str(" . ");
                write_value(out, tail, atoms, depth - 1);
            }
            End::Circular => out.push_str(" ..."),
        }
        out.push(')');
    } else {
        *atoms -= 1;
        if v.symbolp() || v.stringp() || v.numberp() {
            out.push_str(&v.to_string());
        } else {
            out.push_str("...");
        }
    }
}

/// Lisp that makes a list whose one element is the list itself.
#[cfg(test)]
pub const HOLDS_ITSELF: &str = "(let ((c (list 1))) (setcar c c) c)";

/// Lisp that makes a quoted form of a list that holds itself: neither a
/// list nor a string, and printed with the list inside it.
#[cfg(test)]
pub const QUOTES_ITSELF: &str = "(let ((q (car '('(1))))) (let ((in (eval q))) (setcar in in)) q)";

#[cfg(test)]
mod tests {
    use super::*;
    use tulisp::TulispContext;

    fn value(text: &str) -> TulispObject {
        TulispContext::new().eval_string(text).unwrap()
    }

    fn walk(text: &str) -> (Vec<String>, &'static str) {
        let v = value(text);
        let mut elements = items(&v);
        let got = elements.by_ref().map(|e| describe(&e)).collect();
        let end = match elements.end() {
            End::Proper => "proper",
            End::Dotted(_) => "dotted",
            End::Circular => "circular",
        };
        (got, end)
    }

    #[test]
    fn lists_are_walked_to_their_end() {
        assert_eq!(walk("nil"), (vec![], "proper"));
        assert_eq!(
            walk("'(1 \"a\" b)"),
            (vec!["1".into(), "\"a\"".into(), "b".into()], "proper")
        );
        assert_eq!(walk("'(1 . 2)"), (vec!["1".into()], "dotted"));
        assert_eq!(walk("5"), (vec![], "dotted"));
    }

    /// A list of `length` ones, as Lisp text.
    fn ones(length: usize) -> String {
        format!("(list{})", " 1".repeat(length))
    }

    #[test]
    fn a_circular_list_ends_the_walk() {
        for length in 1..20 {
            // `(cdr (cdr ... l))`: the last cell.
            let last = format!("{}l{}", "(cdr ".repeat(length - 1), ")".repeat(length - 1));
            let text = format!("(let ((l {})) (setcdr {last} l) l)", ones(length));
            let (got, end) = walk(&text);
            assert_eq!(end, "circular", "{length}");
            assert!(got.len() >= length && got.len() <= 4 * length, "{length}");
        }
        let (_, end) = walk("(let ((l (list 1 2 3))) (setcdr (cdr (cdr l)) (cdr l)) l)");
        assert_eq!(end, "circular", "a loop that starts after the head");
    }

    #[test]
    fn a_value_that_is_not_a_list_ends_the_walk_unprinted() {
        let (got, end) = walk(&format!("(cons 1 {QUOTES_ITSELF})"));
        assert_eq!((got, end), (vec!["1".to_owned()], "dotted"));
    }

    #[test]
    fn describing_prints_short_values_whole() {
        assert_eq!(
            describe(&value("'(\"csvm\" . \"x\")")),
            "(\"csvm\" . \"x\")"
        );
        assert_eq!(describe(&value("'(a (1 2.5) nil)")), "(a (1 2.5) nil)");
        assert_eq!(describe(&value("\"\"")), "\"\"");
        assert_eq!(describe(&value("t")), "t");
    }

    #[test]
    fn describing_cuts_what_never_ends() {
        let holds_itself = value(HOLDS_ITSELF);
        assert_eq!(describe(&holds_itself), "((((((((...))))))))");
        let circular = value("(let ((l (list 1))) (setcdr l l) l)");
        assert!(
            describe(&circular).ends_with(" ...)"),
            "{}",
            describe(&circular)
        );
        let long = value(&ones(100));
        assert_eq!(describe(&long).matches('1').count(), MOST_ATOMS);
        let quoted = value(QUOTES_ITSELF);
        assert_eq!(describe(&quoted), "...");
    }

    /// A list of eight elements that are each the list itself. Cut only at
    /// `MOST_DEPTH` levels, it would print eight to the eighth lists.
    #[test]
    fn describing_a_wide_list_that_holds_itself_stays_short() {
        let wide = value(&format!(
            "(let ((c {})) (let ((r c)) (while r (setcar r c) (setq r (cdr r)))) c)",
            ones(8)
        ));
        let text = describe(&wide);
        assert!(text.len() < 1000, "{} bytes", text.len());
        let tail = value(&format!("(let ((c {})) (setcdr (cdr c) 5) c)", ones(40)));
        assert_eq!(describe(&tail), "(1 1 . 5)");
    }
}
