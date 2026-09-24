//! The line being edited, as an Emacs buffer: the editing functions Lisp
//! commands use. Positions count characters from 1; readline and `Buffer`
//! count bytes.

use std::cell::{Cell, RefCell};

use tulisp::{Error, Rest, TulispContext, TulispObject};
use unicode_width::UnicodeWidthChar;

/// A line being edited. Positions `point`, `mark`, `start` and `end` are
/// byte offsets into `text()`.
pub trait Buffer {
    fn text(&self) -> Result<String, String>;
    fn point(&self) -> usize;
    fn mark(&self) -> usize;
    /// Moves point to `byte`, or where `char_start_within` puts it.
    fn set_point(&mut self, byte: usize);
    /// Moves the mark to `byte`, or where `char_start_within` puts it.
    fn set_mark(&mut self, byte: usize);
    /// Inserts `text` at point and leaves point after it.
    fn insert(&mut self, text: &str) -> Result<(), String>;
    /// Removes `start..end`. Does not move point or mark.
    fn delete(&mut self, start: usize, end: usize) -> Result<(), String>;
    /// Removes `start..end`, keeping the text for a later yank. Does not
    /// move point or mark. With `backward`, text joined to the previous
    /// kill goes in front of it.
    fn kill(&mut self, start: usize, end: usize, backward: bool) -> Result<(), String>;
    /// Keeps `start..end` for a later yank, without removing it; `backward`
    /// as for `kill`.
    fn copy(&mut self, start: usize, end: usize, backward: bool) -> Result<(), String>;
    fn region_active(&self) -> bool;
}

/// A plain in-memory `Buffer`, used by the unit tests and available for
/// anything else that needs one.
pub struct TextBuffer {
    pub text: String,
    pub point: usize,
    pub mark: usize,
    pub kills: Vec<String>,
}

impl Buffer for TextBuffer {
    fn text(&self) -> Result<String, String> {
        Ok(self.text.clone())
    }

    fn point(&self) -> usize {
        self.point
    }

    fn mark(&self) -> usize {
        self.mark
    }

    fn set_point(&mut self, byte: usize) {
        self.point = char_start_within(self.text.as_bytes(), byte);
    }

    fn set_mark(&mut self, byte: usize) {
        self.mark = char_start_within(self.text.as_bytes(), byte);
    }

    fn insert(&mut self, text: &str) -> Result<(), String> {
        self.text.insert_str(self.point, text);
        self.point += text.len();
        Ok(())
    }

    fn delete(&mut self, start: usize, end: usize) -> Result<(), String> {
        self.text.replace_range(start..end, "");
        Ok(())
    }

    fn kill(&mut self, start: usize, end: usize, _backward: bool) -> Result<(), String> {
        self.kills.push(self.text[start..end].to_owned());
        self.delete(start, end)
    }

    fn copy(&mut self, start: usize, end: usize, _backward: bool) -> Result<(), String> {
        self.kills.push(self.text[start..end].to_owned());
        Ok(())
    }

    fn region_active(&self) -> bool {
        false
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Box<dyn Buffer>>> = const { RefCell::new(None) };
    static WRITABLE: Cell<bool> = const { Cell::new(true) };
    /// Byte offsets `save-excursion` keeps, moved by inserts and deletes.
    static MARKERS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// Holds the installed buffer; dropping it takes the buffer out again.
pub struct Installed(());

impl Drop for Installed {
    fn drop(&mut self) {
        CURRENT.with_borrow_mut(|c| *c = None);
        WRITABLE.set(true);
        MARKERS.with_borrow_mut(Vec::clear);
    }
}

/// Installs `buffer` as the line Lisp commands read and change. Only one
/// buffer is installed at a time; dropping the guard removes it.
pub fn install(buffer: Box<dyn Buffer>) -> Installed {
    CURRENT.with_borrow_mut(|c| *c = Some(buffer));
    WRITABLE.set(true);
    Installed(())
}

/// Whether the installed buffer accepts changes; read-only otherwise.
/// True by default once a buffer is installed.
pub fn set_writable(writable: bool) {
    WRITABLE.set(writable);
}

/// Whether a line is being edited (a buffer is installed).
pub fn editing() -> bool {
    CURRENT.with_borrow(Option::is_some)
}

/// Runs `f` on the current buffer's text, point and mark (bytes). With no
/// buffer installed, `f` sees an empty line with point and mark at 0. An
/// error when the installed buffer's text is not valid UTF-8.
fn read<R>(f: impl FnOnce(&str, usize, usize, &dyn Buffer) -> R) -> Result<R, Error> {
    CURRENT.with_borrow(|slot| match slot {
        Some(buf) => {
            let text = buf
                .text()
                .map_err(|_| Error::lisp_error("the line is not UTF-8"))?;
            Ok(f(&text, buf.point(), buf.mark(), buf.as_ref()))
        }
        None => {
            let empty = TextBuffer {
                text: String::new(),
                point: 0,
                mark: 0,
                kills: Vec::new(),
            };
            Ok(f("", 0, 0, &empty))
        }
    })
}

/// Runs `f` on the buffer to change it: an error when none is installed,
/// or when the installed buffer is read-only.
fn change<R>(f: impl FnOnce(&mut dyn Buffer) -> Result<R, String>) -> Result<R, Error> {
    CURRENT.with_borrow_mut(|slot| match slot {
        Some(buf) if WRITABLE.get() => f(buf.as_mut()).map_err(Error::lisp_error),
        Some(_) => Err(Error::lisp_error("the line cannot be changed here")),
        None => Err(Error::lisp_error("no line is being edited")),
    })
}

/// Moves the installed buffer's point to `byte`. Does nothing with no
/// buffer installed: moving point is never an error, only changing a
/// missing line's text is.
fn move_to(byte: usize) {
    CURRENT.with_borrow_mut(|slot| {
        if let Some(buf) = slot {
            buf.set_point(byte);
        }
    });
}

/// Moves the installed buffer's mark to `byte`. Does nothing with no
/// buffer installed.
fn set_mark_to(byte: usize) {
    CURRENT.with_borrow_mut(|slot| {
        if let Some(buf) = slot {
            buf.set_mark(byte);
        }
    });
}

/// `byte` moved back to a place in `text`: to its end when past it, and
/// to the start of the UTF-8 character it falls inside.
pub fn char_start_within(text: &[u8], byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte > 0 && byte < text.len() && text[byte] & 0xc0 == 0x80 {
        byte -= 1;
    }
    byte
}

/// A 1-based character position clamped to `1..=` the text's character
/// count plus one.
fn to_byte(text: &str, pos: i64) -> usize {
    usize::try_from(pos.max(1) - 1)
        .ok()
        .and_then(|index| text.char_indices().nth(index))
        .map_or(text.len(), |(byte, _)| byte)
}

/// The 1-based character position of the character `byte` falls inside
/// (its start, for a byte inside a multi-byte character); one past the
/// last character when `byte` is at or past the end.
fn to_pos(text: &str, byte: usize) -> i64 {
    if byte >= text.len() {
        return point_max(text);
    }
    let mut pos = 0i64;
    for (i, _) in text.char_indices() {
        if i > byte {
            break;
        }
        pos += 1;
    }
    pos
}

/// One past the last character of `text`, as a 1-based position.
fn point_max(text: &str) -> i64 {
    text.chars().count() as i64 + 1
}

/// Whether the 1-based position `pos` is in `text`: from 1 to one past its
/// last character.
fn in_text(text: &str, pos: i64) -> bool {
    (1..=point_max(text)).contains(&pos)
}

/// The byte range between the 1-based positions `a` and `b`, in either
/// order: an error when either falls outside `text`.
fn byte_range(text: &str, a: i64, b: i64) -> Result<(usize, usize), Error> {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    if !in_text(text, lo) || !in_text(text, hi) {
        return Err(Error::out_of_range(format!(
            "Args out of range: {lo}, {hi}"
        )));
    }
    Ok((to_byte(text, lo), to_byte(text, hi)))
}

/// A position argument: an integer, else wrong-type-argument.
pub fn position(v: &TulispObject) -> Result<i64, Error> {
    i64::try_from(v)
        .map_err(|_| Error::type_mismatch(format!("Wrong type argument: integerp, {v}")))
}

/// `-n`, saturating to `i64::MAX` instead of overflowing at `i64::MIN`.
fn negate(n: i64) -> i64 {
    n.checked_neg().unwrap_or(i64::MAX)
}

/// The byte offset where the line containing `byte` starts.
fn line_start(text: &str, byte: usize) -> usize {
    text[..byte].rfind('\n').map_or(0, |i| i + 1)
}

/// The byte offset where the line containing `byte` ends (before its
/// newline, or at the end of the text).
fn line_end(text: &str, byte: usize) -> usize {
    text[byte..].find('\n').map_or(text.len(), |i| byte + i)
}

/// The screen column of `point`, counted from the start of its line: a
/// tab goes to the next multiple of 8, and every other character counts
/// its display width.
fn column_of(text: &str, point: usize) -> i64 {
    let start = line_start(text, point);
    let mut col: i64 = 0;
    for c in text[start..point].chars() {
        if c == '\t' {
            col = col / 8 * 8 + 8;
        } else {
            col += c.width().unwrap_or(0) as i64;
        }
    }
    col
}

/// The byte `forward-line` lands on for `n` lines from `point`, and the
/// shortfall `forward-line` returns.
fn forward_line_calc(text: &str, point: usize, n: i64) -> (usize, i64) {
    if n == 0 {
        return (line_start(text, point), 0);
    }
    if n > 0 {
        let mut p = point;
        let mut remaining = n;
        while remaining > 0 {
            if let Some(rel) = text[p..].find('\n') {
                p += rel + 1;
                remaining -= 1;
            } else {
                if p < text.len() {
                    p = text.len();
                    remaining -= 1;
                }
                break;
            }
        }
        (p, remaining)
    } else {
        let mut p = line_start(text, point);
        let mut remaining = negate(n);
        while remaining > 0 && p > 0 {
            p = line_start(text, p - 1);
            remaining -= 1;
        }
        (p, -remaining)
    }
}

/// A plain `skip-chars-forward`/`-backward` character set: characters,
/// `a-z` ranges, and a leading `^` that negates the set.
struct CharSet {
    negate: bool,
    ranges: Vec<(char, char)>,
}

impl CharSet {
    fn matches(&self, c: char) -> bool {
        let in_set = self.ranges.iter().any(|&(a, b)| a <= c && c <= b);
        in_set != self.negate
    }
}

fn parse_set(spec: &str) -> CharSet {
    let mut chars: Vec<char> = spec.chars().collect();
    let negate = chars.first() == Some(&'^');
    if negate {
        chars.remove(0);
    }
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i + 1] == '-' {
            ranges.push((chars[i], chars[i + 2]));
            i += 3;
        } else {
            ranges.push((chars[i], chars[i]));
            i += 1;
        }
    }
    CharSet { negate, ranges }
}

/// The byte `skip-chars-forward` lands on from `start`, stopping at
/// `limit`, and the count of characters skipped.
fn skip_forward(text: &str, start: usize, set: &CharSet, limit: usize) -> (usize, i64) {
    let mut point = start;
    let mut moved = 0i64;
    for c in text[start..limit].chars() {
        if !set.matches(c) {
            break;
        }
        point += c.len_utf8();
        moved += 1;
    }
    (point, moved)
}

/// The byte `skip-chars-backward` lands on from `start`, stopping at
/// `limit`, and the count of characters skipped (negative).
fn skip_backward(text: &str, start: usize, set: &CharSet, limit: usize) -> (usize, i64) {
    let mut point = start;
    let mut moved = 0i64;
    for c in text[limit..start].chars().rev() {
        if !set.matches(c) {
            break;
        }
        point -= c.len_utf8();
        moved -= 1;
    }
    (point, moved)
}

/// `buffer-substring` and `buffer-substring-no-properties`: the same
/// text, since neither keeps properties.
fn buffer_substring(start: &TulispObject, end: &TulispObject) -> Result<String, Error> {
    let s = position(start)?;
    let e = position(end)?;
    read(|text, _point, _mark, _buf| {
        byte_range(text, s, e).map(|(lo, hi)| text[lo..hi].to_owned())
    })?
}

/// The 1-based position `delta` characters from byte `point`, if it is in
/// `text`.
fn moved_pos(text: &str, point: usize, delta: i64) -> Option<i64> {
    to_pos(text, point)
        .checked_add(delta)
        .filter(|&pos| in_text(text, pos))
}

/// `forward-char`/`backward-char`: moves point by `delta` characters, or
/// errors without moving when that runs off either end. `delta` may be
/// any integer a Lisp caller passes, so the addition is checked instead
/// of trusting it to fit.
fn move_point(delta: i64) -> Result<(), Error> {
    let byte = read(|text, point, _mark, _buf| -> Result<usize, Error> {
        match moved_pos(text, point, delta) {
            Some(target) => Ok(to_byte(text, target)),
            None => Err(Error::out_of_range(format!("Args out of range: {delta}"))),
        }
    })??;
    move_to(byte);
    Ok(())
}

/// The validated byte range for two position args, in either order, and
/// whether `b` comes before `a`: an error when either falls outside the
/// text.
fn range(a: &TulispObject, b: &TulispObject) -> Result<(usize, usize, bool), Error> {
    let pa = position(a)?;
    let pb = position(b)?;
    let (start, end) = read(|text, _point, _mark, _buf| byte_range(text, pa, pb))??;
    Ok((start, end, pb < pa))
}

/// `insert`'s combined text: strings as given, integers as one character
/// each. A NUL character is an error, since the line being edited cannot
/// hold one.
fn insert_text(args: Rest<TulispObject>) -> Result<String, Error> {
    let mut out = String::new();
    for arg in args {
        if arg.stringp() {
            out.push_str(&arg.as_string()?);
        } else {
            let code = i64::try_from(&arg)?;
            let c = u32::try_from(code)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| Error::type_mismatch(format!("not a character: {code}")))?;
            out.push(c);
        }
    }
    if out.contains('\0') {
        return Err(Error::invalid_argument(
            "the line cannot contain a NUL character",
        ));
    }
    Ok(out)
}

/// Where `pos` lands after an insert of `len` bytes at `at`: a position
/// after `at` moves on by `len`; one at or before `at` stays, since Emacs's
/// mark stays before inserted text.
fn adjust_after_insert(pos: usize, at: usize, len: usize) -> usize {
    if pos > at { pos + len } else { pos }
}

/// Where `pos` lands after deleting `start..end`: a position at or after
/// `end` moves back by the deleted length; one inside the deleted range
/// moves to `start`; one before `start` stays.
fn adjust_after_delete(pos: usize, start: usize, end: usize) -> usize {
    if pos >= end {
        pos - (end - start)
    } else if pos > start {
        start
    } else {
        pos
    }
}

/// Moves the mark and every `save-excursion` marker for an insert of `len`
/// bytes at `at`. Point is not touched here: `Buffer::insert` leaves it
/// after the inserted text itself.
fn after_insert(at: usize, len: usize) {
    CURRENT.with_borrow_mut(|slot| {
        if let Some(buf) = slot {
            let mark = adjust_after_insert(buf.mark(), at, len);
            buf.set_mark(mark);
        }
    });
    MARKERS.with_borrow_mut(|markers| {
        for m in markers.iter_mut() {
            *m = adjust_after_insert(*m, at, len);
        }
    });
}

/// Moves point, the mark and every `save-excursion` marker for a delete of
/// `start..end`. `Buffer::delete` and `Buffer::kill` do not move point or
/// mark themselves.
fn after_delete(start: usize, end: usize) {
    CURRENT.with_borrow_mut(|slot| {
        if let Some(buf) = slot {
            let point = adjust_after_delete(buf.point(), start, end);
            let mark = adjust_after_delete(buf.mark(), start, end);
            buf.set_point(point);
            buf.set_mark(mark);
        }
    });
    MARKERS.with_borrow_mut(|markers| {
        for m in markers.iter_mut() {
            *m = adjust_after_delete(*m, start, end);
        }
    });
}

pub fn register(ctx: &mut TulispContext) {
    ctx.defun("buffer-string", || -> Result<String, Error> {
        read(|text, _point, _mark, _buf| text.to_owned())
    });
    ctx.defun(
        "buffer-substring",
        |start: TulispObject, end: TulispObject| -> Result<String, Error> {
            buffer_substring(&start, &end)
        },
    );
    ctx.defun(
        "buffer-substring-no-properties",
        |start: TulispObject, end: TulispObject| -> Result<String, Error> {
            buffer_substring(&start, &end)
        },
    );
    ctx.defun("point", || -> Result<i64, Error> {
        read(|text, point, _mark, _buf| to_pos(text, point))
    });
    ctx.defun("point-min", || -> Result<i64, Error> {
        read(|_, _, _, _| 1)
    });
    ctx.defun("point-max", || -> Result<i64, Error> {
        read(|text, _point, _mark, _buf| point_max(text))
    });
    ctx.defun(
        "char-after",
        |pos: Option<TulispObject>| -> Result<Option<i64>, Error> {
            let target = pos.as_ref().map(position).transpose()?;
            read(|text, point, _mark, _buf| {
                let at = match target {
                    None => point,
                    Some(p) if in_text(text, p) => to_byte(text, p),
                    Some(_) => return None,
                };
                text.get(at..)?.chars().next().map(|c| c as i64)
            })
        },
    );
    ctx.defun(
        "char-before",
        |pos: Option<TulispObject>| -> Result<Option<i64>, Error> {
            let target = pos.as_ref().map(position).transpose()?;
            read(|text, point, _mark, _buf| {
                let at = match target {
                    None => point,
                    Some(p) if in_text(text, p) => to_byte(text, p),
                    Some(_) => return None,
                };
                text.get(..at)?.chars().next_back().map(|c| c as i64)
            })
        },
    );
    ctx.defun("bolp", || -> Result<bool, Error> {
        read(|text, point, _mark, _buf| point == line_start(text, point))
    });
    ctx.defun("eolp", || -> Result<bool, Error> {
        read(|text, point, _mark, _buf| point == line_end(text, point))
    });
    ctx.defun("bobp", || -> Result<bool, Error> {
        read(|_text, point, _mark, _buf| point == 0)
    });
    ctx.defun("eobp", || -> Result<bool, Error> {
        read(|text, point, _mark, _buf| point == text.len())
    });
    ctx.defun("line-beginning-position", || -> Result<i64, Error> {
        read(|text, point, _mark, _buf| to_pos(text, line_start(text, point)))
    });
    ctx.defun("line-end-position", || -> Result<i64, Error> {
        read(|text, point, _mark, _buf| to_pos(text, line_end(text, point)))
    });
    ctx.defun("current-column", || -> Result<i64, Error> {
        read(|text, point, _mark, _buf| column_of(text, point))
    });
    ctx.defun("mark", || -> Result<i64, Error> {
        read(|text, _point, mark, _buf| to_pos(text, mark))
    });
    ctx.defun("region-beginning", || -> Result<i64, Error> {
        read(|text, point, mark, _buf| to_pos(text, point.min(mark)))
    });
    ctx.defun("region-end", || -> Result<i64, Error> {
        read(|text, point, mark, _buf| to_pos(text, point.max(mark)))
    });
    ctx.defun("use-region-p", || -> Result<bool, Error> {
        read(|_text, _point, _mark, buf| buf.region_active())
    });
    ctx.defun("region-active-p", || -> Result<bool, Error> {
        read(|_text, _point, _mark, buf| buf.region_active())
    });
    ctx.defun("goto-char", |pos: TulispObject| -> Result<i64, Error> {
        let target = position(&pos)?;
        let byte = read(|text, _point, _mark, _buf| to_byte(text, target))?;
        move_to(byte);
        Ok(target)
    });
    ctx.defun("forward-char", |n: Option<i64>| -> Result<(), Error> {
        move_point(n.unwrap_or(1))
    });
    ctx.defun("backward-char", |n: Option<i64>| -> Result<(), Error> {
        move_point(negate(n.unwrap_or(1)))
    });
    ctx.defun("beginning-of-line", || -> Result<(), Error> {
        let byte = read(|text, point, _mark, _buf| line_start(text, point))?;
        move_to(byte);
        Ok(())
    });
    ctx.defun("end-of-line", || -> Result<(), Error> {
        let byte = read(|text, point, _mark, _buf| line_end(text, point))?;
        move_to(byte);
        Ok(())
    });
    ctx.defun("forward-line", |n: Option<i64>| -> Result<i64, Error> {
        let n = n.unwrap_or(1);
        let (byte, shortfall) = read(|text, point, _mark, _buf| forward_line_calc(text, point, n))?;
        move_to(byte);
        Ok(shortfall)
    });
    ctx.defun(
        "skip-chars-forward",
        |spec: String, lim: Option<TulispObject>| -> Result<i64, Error> {
            let lim_target = lim.as_ref().map(position).transpose()?;
            let (byte, moved) = read(|text, point, _mark, _buf| {
                let bound = lim_target
                    .map_or(text.len(), |p| to_byte(text, p))
                    .max(point);
                skip_forward(text, point, &parse_set(&spec), bound)
            })?;
            move_to(byte);
            Ok(moved)
        },
    );
    ctx.defun(
        "skip-chars-backward",
        |spec: String, lim: Option<TulispObject>| -> Result<i64, Error> {
            let lim_target = lim.as_ref().map(position).transpose()?;
            let (byte, moved) = read(|text, point, _mark, _buf| {
                let bound = lim_target.map_or(0, |p| to_byte(text, p)).min(point);
                skip_backward(text, point, &parse_set(&spec), bound)
            })?;
            move_to(byte);
            Ok(moved)
        },
    );
    ctx.defun("set-mark", |pos: TulispObject| -> Result<(), Error> {
        let target = position(&pos)?;
        let byte = read(|text, _point, _mark, _buf| to_byte(text, target))?;
        set_mark_to(byte);
        Ok(())
    });
    ctx.defun("insert", |args: Rest<TulispObject>| -> Result<(), Error> {
        let text = insert_text(args)?;
        // An error when the line is not UTF-8.
        read(|_text, _point, _mark, _buf| ())?;
        let at = change(|buf| {
            let at = buf.point();
            buf.insert(&text)?;
            Ok(at)
        })?;
        after_insert(at, text.len());
        Ok(())
    });
    ctx.defun(
        "delete-region",
        |a: TulispObject, b: TulispObject| -> Result<(), Error> {
            let (s, e, _) = range(&a, &b)?;
            change(|buf| buf.delete(s, e))?;
            after_delete(s, e);
            Ok(())
        },
    );
    ctx.defun(
        "delete-char",
        |n: i64, killflag: Option<TulispObject>| -> Result<(), Error> {
            let (s, e) = read(
                |text, point, _mark, _buf| -> Result<(usize, usize), Error> {
                    match moved_pos(text, point, n) {
                        Some(target) => byte_range(text, to_pos(text, point), target),
                        None => Err(Error::out_of_range(format!("Args out of range: {n}"))),
                    }
                },
            )??;
            let killing = killflag.is_some_and(|v| !v.null());
            if killing {
                change(|buf| buf.kill(s, e, n < 0))?;
            } else {
                change(|buf| buf.delete(s, e))?;
            }
            after_delete(s, e);
            Ok(())
        },
    );
    ctx.defun("erase-buffer", || -> Result<(), Error> {
        let (s, e) = read(|text, _point, _mark, _buf| (0usize, text.len()))?;
        change(|buf| buf.delete(s, e))?;
        after_delete(s, e);
        Ok(())
    });
    ctx.defun(
        "kill-region",
        |a: TulispObject, b: TulispObject| -> Result<(), Error> {
            let (s, e, backward) = range(&a, &b)?;
            change(|buf| buf.kill(s, e, backward))?;
            after_delete(s, e);
            Ok(())
        },
    );
    ctx.defun(
        "copy-region-as-kill",
        |a: TulispObject, b: TulispObject| -> Result<(), Error> {
            let (s, e, backward) = range(&a, &b)?;
            change(|buf| buf.copy(s, e, backward))?;
            Ok(())
        },
    );
    ctx.defun("inkline--save-point", || -> Result<i64, Error> {
        let byte = read(|_text, point, _mark, _buf| point)?;
        Ok(MARKERS.with_borrow_mut(|m| {
            m.push(byte);
            (m.len() - 1) as i64
        }))
    });
    ctx.defun(
        "inkline--restore-point",
        |index: i64| -> Result<(), Error> {
            let byte = MARKERS.with_borrow_mut(|m| {
                let i = index.max(0) as usize;
                let byte = m.get(i).copied();
                m.truncate(i);
                byte
            });
            if let Some(byte) = byte {
                move_to(byte);
            }
            Ok(())
        },
    );
    ctx.eval_prelude(
        "<inkline-buffer>",
        r#"
(defmacro save-excursion (&rest body)
  (list 'let (list (list 'inkline--excursion '(inkline--save-point)))
        (list 'unwind-protect (cons 'progn body)
              '(inkline--restore-point inkline--excursion))))
"#,
    )
    .expect("inkline's own Lisp compiles");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tulisp::TulispContext;

    /// Runs `program` with `text` in the buffer and point at `pos`, and
    /// returns `(RESULT POINT TEXT)` as Emacs would print it.
    fn run(text: &str, pos: i64, program: &str) -> String {
        let mut ctx = TulispContext::new();
        crate::lisp::errors::register(&mut ctx);
        crate::lisp::emacs::register(&mut ctx);
        register(&mut ctx);
        let point = to_byte(text, pos);
        let _installed = install(Box::new(TextBuffer {
            text: text.to_owned(),
            point,
            mark: 0,
            kills: Vec::new(),
        }));
        let wrapped = format!(
            "(list (condition-case e (progn {program}) (error (list 'error (car e)))) (point) (buffer-string))"
        );
        match ctx.eval_string(&wrapped) {
            Ok(v) => v.to_string(),
            Err(e) => format!("ERROR {}", e.desc()),
        }
    }

    /// Like `run`, but with no buffer installed at all.
    fn run_without_buffer(program: &str) -> String {
        let mut ctx = TulispContext::new();
        crate::lisp::errors::register(&mut ctx);
        crate::lisp::emacs::register(&mut ctx);
        register(&mut ctx);
        let wrapped = format!(
            "(list (condition-case e (progn {program}) (error (list 'error (car e)))) (point) (buffer-string))"
        );
        match ctx.eval_string(&wrapped) {
            Ok(v) => v.to_string(),
            Err(e) => format!("ERROR {}", e.desc()),
        }
    }

    #[test]
    fn forward_line() {
        assert_eq!(run("ab\ncd", 1, "(forward-line 1)"), r#"(0 4 "ab\ncd")"#);
        assert_eq!(run("ab\ncd", 4, "(forward-line 1)"), r#"(0 6 "ab\ncd")"#);
        assert_eq!(
            run("ab\ncd\n", 4, "(forward-line 1)"),
            r#"(0 7 "ab\ncd\n")"#
        );
        assert_eq!(run("ab\ncd", 4, "(forward-line -1)"), r#"(0 1 "ab\ncd")"#);
        assert_eq!(run("ab\ncd", 4, "(forward-line -5)"), r#"(-4 1 "ab\ncd")"#);
        // From inside a line, as `emacs --batch` gives.
        assert_eq!(run("ab\ncd", 5, "(forward-line -1)"), r#"(0 1 "ab\ncd")"#);
        assert_eq!(run("ab\ncd", 5, "(forward-line -5)"), r#"(-4 1 "ab\ncd")"#);
        assert_eq!(run("abc", 2, "(forward-line -1)"), r#"(-1 1 "abc")"#);
        assert_eq!(run("ab", 2, "(forward-line 3)"), r#"(2 3 "ab")"#);
    }

    #[test]
    fn line_positions_and_predicates() {
        assert_eq!(
            run(
                "ab\ncd",
                5,
                "(list (line-beginning-position) (line-end-position) (current-column) (bolp) (eolp) (bobp) (eobp))"
            ),
            r#"((4 6 1 nil nil nil nil) 5 "ab\ncd")"#
        );
        assert_eq!(
            run("ab\ncd", 2, "(end-of-line) (point)"),
            r#"(3 3 "ab\ncd")"#
        );
        assert_eq!(
            run("ab\ncd", 5, "(beginning-of-line) (point)"),
            r#"(4 4 "ab\ncd")"#
        );
    }

    #[test]
    fn columns_count_cells() {
        assert_eq!(run("a\tb", 3, "(current-column)"), r#"(8 3 "a\tb")"#);
        assert_eq!(run("日本x", 3, "(current-column)"), r#"(4 3 "日本x")"#);
    }

    #[test]
    fn characters_around_point() {
        assert_eq!(
            run(
                "héllo",
                2,
                "(list (char-after) (char-before) (char-after 99) (char-before 1))"
            ),
            r#"((233 104 nil nil) 2 "héllo")"#
        );
    }

    #[test]
    fn characters_out_of_range_are_nil() {
        // As `emacs --batch` gives for the same calls on "abc".
        assert_eq!(
            run(
                "abc",
                2,
                "(list (char-after 0) (char-after 1) (char-after 4) (char-after 99) (char-after -5))"
            ),
            r#"((nil 97 nil nil nil) 2 "abc")"#
        );
        assert_eq!(
            run(
                "abc",
                2,
                "(list (char-before 0) (char-before 1) (char-before 4) (char-before 99))"
            ),
            r#"((nil nil 99 nil) 2 "abc")"#
        );
    }

    #[test]
    fn skip_chars() {
        assert_eq!(
            run("hello world", 1, r#"(skip-chars-forward "a-z")"#),
            r#"(5 6 "hello world")"#
        );
        assert_eq!(
            run("hello world", 12, r#"(skip-chars-backward "^ ")"#),
            r#"(-5 7 "hello world")"#
        );
    }

    #[test]
    fn goto_char_clamps() {
        assert_eq!(run("abc", 2, "(goto-char 99)"), r#"(99 4 "abc")"#);
        assert_eq!(run("abc", 2, "(goto-char -3)"), r#"(-3 1 "abc")"#);
    }

    #[test]
    fn out_of_range_moves_nothing() {
        assert_eq!(
            run("abc", 2, "(forward-char 5)"),
            r#"((error args-out-of-range) 2 "abc")"#
        );
        assert_eq!(
            run("abc", 2, "(buffer-substring 1 9)"),
            r#"((error args-out-of-range) 2 "abc")"#
        );
        assert_eq!(
            run("abc", 2, r#"(goto-char "x")"#),
            r#"((error wrong-type-argument) 2 "abc")"#
        );
    }

    #[test]
    fn mark_and_region() {
        assert_eq!(
            run(
                "abcdef",
                2,
                "(set-mark 5) (list (mark) (region-beginning) (region-end) (use-region-p))"
            ),
            r#"((5 2 5 nil) 2 "abcdef")"#
        );
        assert_eq!(
            run("abc", 2, "(set-mark nil)"),
            r#"((error wrong-type-argument) 2 "abc")"#
        );
    }

    #[test]
    fn save_excursion_restores_point_on_error_too() {
        assert_eq!(
            run("abcdef", 3, "(save-excursion (goto-char 6) (error \"x\"))"),
            r#"((error error) 3 "abcdef")"#
        );
    }

    #[test]
    fn without_a_buffer_the_line_is_empty() {
        let mut ctx = TulispContext::new();
        crate::lisp::errors::register(&mut ctx);
        register(&mut ctx);
        assert_eq!(
            ctx.eval_string("(list (buffer-string) (point) (point-max))")
                .unwrap()
                .to_string(),
            r#"("" 1 1)"#
        );
    }

    #[test]
    fn moving_works_without_a_buffer() {
        // Emacs, checked with `emacs --batch --eval '(with-temp-buffer
        // (prin1 (list (goto-char 5) (point) (point-max))))'` and the
        // like: moving point in an empty buffer is never an error, only
        // reading or changing its (missing) text can be.
        assert_eq!(run_without_buffer("(goto-char 5)"), r#"(5 1 "")"#);
        assert_eq!(run_without_buffer("(forward-line 1)"), r#"(1 1 "")"#);
        assert_eq!(run_without_buffer("(set-mark 5) (mark)"), r#"(1 1 "")"#);
        assert_eq!(
            run_without_buffer(r#"(skip-chars-forward "a-z")"#),
            r#"(0 1 "")"#
        );
    }

    #[test]
    fn huge_counts_do_not_panic() {
        assert_eq!(
            run("abc", 2, "(forward-char 9223372036854775807)"),
            r#"((error args-out-of-range) 2 "abc")"#
        );
        assert_eq!(
            run("abc", 2, "(backward-char -9223372036854775808)"),
            r#"((error args-out-of-range) 2 "abc")"#
        );
        // `-i64::MIN` cannot be represented, so `negate` saturates it to
        // `i64::MAX`; this only has to not panic, not match Emacs (which
        // has bignums and would move differently).
        assert_eq!(
            run("abc", 2, "(forward-line -9223372036854775808)"),
            r#"(-9223372036854775807 1 "abc")"#
        );
    }

    #[test]
    fn deleting_moves_point_like_emacs() {
        assert_eq!(
            run("abcdef", 3, "(delete-region 2 4) (point)"),
            r#"(2 2 "adef")"#
        );
        assert_eq!(
            run("abcdef", 5, "(delete-region 4 2) (point)"),
            r#"(3 3 "adef")"#
        );
        assert_eq!(run("abc", 3, "(delete-char -2) (point)"), r#"(1 1 "c")"#);
        assert_eq!(
            run("abc", 2, "(delete-char 5)"),
            r#"((error args-out-of-range) 2 "abc")"#
        );
    }

    #[test]
    fn inserting_moves_point_and_mark_like_emacs() {
        assert_eq!(
            run("abcdef", 3, r#"(insert "XY") (point)"#),
            r#"(5 5 "abXYcdef")"#
        );
        assert_eq!(
            run("abc", 2, r#"(list (insert "x" ?y) (point))"#),
            r#"((nil 4) 4 "axybc")"#
        );
        assert_eq!(
            run(
                "abcdef",
                2,
                r#"(set-mark 5) (goto-char 3) (insert "ZZ") (mark)"#
            ),
            r#"(7 5 "abZZcdef")"#
        );
        assert_eq!(
            run(
                "abcdef",
                2,
                "(set-mark 5) (delete-region 1 3) (list (mark) (point))"
            ),
            r#"((3 1) 1 "cdef")"#
        );
        // tulisp's `string` builds a string with a NUL here; `insert`
        // raises its own error, `Error::invalid_argument`, which
        // `condition-case` reports as `wrong-type-argument` (the table in
        // tulisp's error_symbol has no separate symbol for it).
        assert_eq!(
            run("abc", 2, r#"(insert (string 0))"#),
            r#"((error wrong-type-argument) 2 "abc")"#
        );
    }

    #[test]
    fn save_excursion_follows_the_text() {
        assert_eq!(
            run(
                "abcdef",
                3,
                r#"(save-excursion (goto-char 1) (insert "XX"))"#
            ),
            r#"(nil 5 "XXabcdef")"#
        );
    }

    #[test]
    fn kills_and_erase() {
        assert_eq!(
            run("abcdef", 1, "(kill-region 2 4) (buffer-string)"),
            r#"("adef" 1 "adef")"#
        );
        assert_eq!(run("abc", 2, "(erase-buffer) (point)"), r#"(1 1 "")"#);
    }

    #[test]
    fn changing_needs_a_writable_line() {
        let mut ctx = TulispContext::new();
        crate::lisp::errors::register(&mut ctx);
        register(&mut ctx);
        let e = ctx.eval_string(r#"(insert "x")"#).unwrap_err();
        assert_eq!(e.desc(), "no line is being edited");
        let _installed = install(Box::new(TextBuffer {
            text: "a".into(),
            point: 1,
            mark: 0,
            kills: vec![],
        }));
        set_writable(false);
        let e = ctx.eval_string(r#"(insert "x")"#).unwrap_err();
        assert_eq!(e.desc(), "the line cannot be changed here");
    }

    /// A line that is not UTF-8, as readline's can be. Changing it is a
    /// test failure.
    struct NotUtf8;

    impl Buffer for NotUtf8 {
        fn text(&self) -> Result<String, String> {
            Err("not UTF-8".into())
        }
        fn point(&self) -> usize {
            0
        }
        fn mark(&self) -> usize {
            0
        }
        fn set_point(&mut self, _byte: usize) {}
        fn set_mark(&mut self, _byte: usize) {}
        fn insert(&mut self, _text: &str) -> Result<(), String> {
            Err("changed".into())
        }
        fn delete(&mut self, _start: usize, _end: usize) -> Result<(), String> {
            Err("changed".into())
        }
        fn kill(&mut self, _start: usize, _end: usize, _backward: bool) -> Result<(), String> {
            Err("changed".into())
        }
        fn copy(&mut self, _start: usize, _end: usize, _backward: bool) -> Result<(), String> {
            Err("changed".into())
        }
        fn region_active(&self) -> bool {
            false
        }
    }

    #[test]
    fn a_line_that_is_not_utf8_is_an_error() {
        let mut ctx = TulispContext::new();
        crate::lisp::errors::register(&mut ctx);
        register(&mut ctx);
        let _installed = install(Box::new(NotUtf8));
        for program in [
            r#"(insert "x")"#,
            "(delete-region 1 1)",
            "(erase-buffer)",
            "(kill-region 1 1)",
            "(copy-region-as-kill 1 1)",
            "(delete-char 0)",
            "(buffer-string)",
            "(point)",
        ] {
            let e = ctx.eval_string(program).unwrap_err();
            assert_eq!(e.desc(), "the line is not UTF-8", "{program}");
        }
    }

    #[test]
    fn positions_fit_the_text() {
        assert_eq!(char_start_within("héllo".as_bytes(), 99), 6);
        assert_eq!(char_start_within("héllo".as_bytes(), 2), 1);
        assert_eq!(char_start_within("héllo".as_bytes(), 3), 3);
        assert_eq!(char_start_within(b"", 4), 0);
    }

    #[test]
    fn positions_count_characters() {
        assert_eq!(to_byte("héllo", 3), 3);
        assert_eq!(to_pos("héllo", 3), 3);
        assert_eq!(to_pos("héllo", 2), 2); // inside `é`: its start
        assert_eq!(to_byte("héllo", 99), 6);
        assert_eq!(to_byte("héllo", -4), 0);
    }
}
