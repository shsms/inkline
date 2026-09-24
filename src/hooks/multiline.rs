//! The readline commands for a command that spans several lines.

use std::ffi::c_int;

use super::{STATE, guard, status_of};
use crate::ffi;
use crate::indent;
use crate::lines;
use crate::render;
use crate::syntax::{self, Status};

/// The line being edited, when the multi-line commands do their own work:
/// inkline is on, readline reads a command for bash and draws a newline as a
/// line break. Otherwise these commands do what readline's own do.
fn active_line() -> Option<String> {
    let on = STATE.with_borrow(|s| s.enabled && !s.unloaded);
    (on && ffi::reading_command() && super::shows_newlines())
        .then(ffi::line)
        .flatten()
}

/// The width of the prompt's last line, or 0 when inkline cannot tell.
fn prompt_width() -> usize {
    render::prompt_width(&ffi::display_prompt()).unwrap_or(0)
}

/// Enter: adds a line to an unfinished command, and accepts any other.
pub(super) extern "C" fn accept_or_newline(count: c_int, key: c_int) -> c_int {
    guard(
        || {
            // A C-c that came with more keys is still waiting for bash to act
            // on it, which bash does once the line is accepted.
            if ffi::interrupted() {
                return ffi::accept_line(count, key);
            }
            let Some(line) = active_line() else {
                return ffi::accept_line(count, key);
            };
            let point = ffi::point();
            // With pairing, the closer typed with an opener already follows
            // the cursor: `f() {}` is wrong as it stands, but bash would ask
            // for another line after `f() {`. Only an empty pair counts, so
            // `echo "hi"` and `echo $(date)` run wherever the cursor is. The
            // pair decides this whether or not the whole command is finished,
            // so inside an open block the closer goes below too.
            if empty_pair(&line, point)
                && starts_code(&line[..point])
                && STATE.with_borrow_mut(|s| s.checker.check(&line[..point], ffi::extglob()))
                    == Status::Unfinished
            {
                // Pasted text brings its own closer, which moves over this one.
                let below = !ffi::input_waiting();
                if !fits_more_rows(&line, point, if below { 2 } else { 1 }) {
                    return ffi::accept_line(count, key);
                }
                new_line(&line, point, below);
                return 0;
            }
            let status = STATE.with_borrow_mut(|s| status_of(s, &line));
            if status == Status::Unfinished {
                if !fits_more_rows(&line, point, 1) {
                    return ffi::accept_line(count, key);
                }
                new_line(&line, point, false);
                return 0;
            }
            if !ffi::input_waiting() && move_out(&line, point).is_some() {
                // Readline does not redraw before it accepts a line.
                super::repaint_now();
            }
            ffi::accept_line(count, key)
        },
        || ffi::accept_line(count, key),
    )
}

/// Whether the cursor is between an opening bracket and its closer, such as
/// pairing puts there, with only closing brackets after it on its line.
fn empty_pair(line: &str, point: usize) -> bool {
    let before = line[lines::line_start(line, point)..point].trim_end_matches([' ', '\t']);
    let rest = line[point..lines::line_end(line, point)].trim_start_matches([' ', '\t']);
    let close = match before.chars().last() {
        Some('{') => '}',
        Some('(') => ')',
        Some('[') => ']',
        _ => return false,
    };
    rest.starts_with(close) && rest.chars().all(|c| ")]} \t".contains(c))
}

/// M-Enter: always adds a line.
pub(super) extern "C" fn insert_newline(_count: c_int, _key: c_int) -> c_int {
    guard(
        || {
            match active_line() {
                Some(line) => new_line(&line, ffi::point(), false),
                None => ffi::insert_text("\n"),
            }
            0
        },
        || {
            ffi::insert_text("\n");
            0
        },
    )
}

/// Inserts a newline at `point` as one undo step with what goes with it: the
/// line left moves out when it starts with a closing word, and the new line
/// gets its indentation. Neither happens when more input is already waiting
/// (pasted text keeps its own spacing) or `INKLINE_INDENT` is 0. With
/// `close_below`, the text after the cursor goes on a line of its own below
/// the new one, as indented as the cursor's line.
fn new_line(line: &str, point: usize, close_below: bool) {
    let step = indent_step();
    ffi::begin_undo_group();
    let mut indentation = String::new();
    let mut point = point;
    if step > 0 && !ffi::input_waiting() {
        point = move_out(line, point).unwrap_or(point);
        let text = ffi::line().unwrap_or_default();
        if starts_code(&text[..point]) {
            indentation = indent::for_new_line(&text, point, step);
        }
    }
    ffi::insert_text(&format!("\n{indentation}"));
    if close_below {
        // The newline went in at `point`, so the cursor's line still ends
        // there.
        let text = ffi::line().unwrap_or_default();
        let base = indent::indentation(&text[lines::line_start(&text, point)..point]);
        let at = ffi::point();
        let blanks = indent::indentation(&text[at..]).len();
        ffi::delete_text(at, at + blanks);
        ffi::insert_text(&format!("\n{base}"));
        ffi::set_point(at);
    }
    ffi::end_undo_group();
}

fn indent_step() -> usize {
    indent::step(ffi::shell_variable("INKLINE_INDENT").as_deref())
}

/// Moves the line the cursor is on back one step when it starts with a word
/// that closes a block, and returns the new cursor position. A line already
/// less indented than a new line after the code above it would be has moved
/// out before, so it stays. An `INKLINE_INDENT` of 0 leaves lines where they
/// are.
fn move_out(line: &str, point: usize) -> Option<usize> {
    let step = indent_step();
    if step == 0 {
        return None;
    }
    let start = lines::line_start(line, point);
    let end = lines::line_end(line, point);
    let remove = indent::outdent(&line[start..end], step);
    if remove == 0 || (start > 0 && !starts_code(&line[..start - 1])) {
        return None;
    }
    if start > 0 {
        // The last line above that starts in code, not in a string or a
        // here-document body.
        let mut above = start - 1;
        loop {
            let s = lines::line_start(line, above);
            if s == 0 || starts_code(&line[..s - 1]) {
                break;
            }
            above = s - 1;
        }
        if lines::width(indent::indentation(&line[start..end]))
            < lines::width(&indent::for_new_line(line, above, step))
        {
            return None;
        }
    }
    ffi::delete_text(start, start + remove);
    let point = if point >= start + remove {
        point - remove
    } else {
        start
    };
    ffi::set_point(point);
    Some(point)
}

/// Whether a line starting after `before` is code, not text inside a string
/// or a here-document body. A comment ends with its line, so the next line is
/// code.
fn starts_code(before: &str) -> bool {
    !syntax::heredoc_open(before) && !syntax::quote_open(before)
}

/// Whether the command still fits on the screen with `more` rows added at
/// `point`: readline cannot draw a command taller than the screen.
fn fits_more_rows(line: &str, point: usize, more: usize) -> bool {
    let longer = format!("{}{}{}", &line[..point], "\n".repeat(more), &line[point..]);
    fits_on_screen(&longer)
}

/// Whether readline can draw `text` after the prompt without scrolling.
fn fits_on_screen(text: &str) -> bool {
    let (rows, cols) = ffi::screen_size();
    render::rows(prompt_width(), text, cols) <= rows
}
