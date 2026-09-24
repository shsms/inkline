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

/// Adds a line. It accepts the line instead in three cases: while a `C-c`
/// waits for bash, where inkline adds no lines, and while readline replays
/// a macro, so a `\n` in a macro runs the command.
pub(super) extern "C" fn insert_newline(count: c_int, key: c_int) -> c_int {
    guard(
        || {
            // A C-c that came with more keys is still waiting for bash to act
            // on it, which bash does once the line is accepted.
            if ffi::interrupted() {
                return ffi::accept_line(count, key);
            }
            if ffi::replaying_macro() {
                return ffi::accept_line(count, key);
            }
            let Some(line) = active_line() else {
                return ffi::accept_line(count, key);
            };
            new_line(&line, ffi::point(), false);
            0
        },
        || ffi::accept_line(count, key),
    )
}

/// Inserts a newline at `point` as one undo step with what goes with it: the
/// line left moves out when it starts with a closing word, and the new line
/// gets its indentation. Neither happens when more input is already waiting
/// (pasted text keeps its own spacing) or `inkline-indent` is 0. With
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
    crate::lisp::settings::indent()
}

/// Moves the line the cursor is on back one step when it starts with a word
/// that closes a block, and returns the new cursor position. A line already
/// less indented than a new line after the code above it would be has moved
/// out before, so it stays. An `inkline-indent` of 0 leaves lines where they
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

/// Where Up and Down go past the first or last line.
#[derive(Clone, Copy)]
enum Fallback {
    History,
    /// readline's history search for entries starting with the text before
    /// the cursor.
    Search,
}

pub(super) extern "C" fn previous_line_or_history(count: c_int, key: c_int) -> c_int {
    vertical(count, key, true, Fallback::History)
}

pub(super) extern "C" fn next_line_or_history(count: c_int, key: c_int) -> c_int {
    vertical(count, key, false, Fallback::History)
}

pub(super) extern "C" fn previous_line_or_search(count: c_int, key: c_int) -> c_int {
    vertical(count, key, true, Fallback::Search)
}

pub(super) extern "C" fn next_line_or_search(count: c_int, key: c_int) -> c_int {
    vertical(count, key, false, Fallback::Search)
}

/// Whether `f` is one of the Up and Down commands, so a run of them keeps
/// its column.
fn is_vertical(f: Option<ffi::CommandFn>) -> bool {
    let ours: [ffi::CommandFn; 4] = [
        previous_line_or_history,
        next_line_or_history,
        previous_line_or_search,
        next_line_or_search,
    ];
    f.is_some_and(|f| ours.iter().any(|&o| std::ptr::fn_addr_eq(o, f)))
}

/// Moves the cursor `count` lines up or down, keeping its column. Past the
/// first or last line it runs `fallback` for the lines left over.
///
/// Readline's own history search decides whether to continue the last search
/// or start a new one by checking `rl_last_func`, which after this command's
/// own dispatch holds this command, never the search function it called
/// directly; without `continuing_search` a run of `-or-search` presses would
/// restart the search from the newest entry every time instead of moving
/// through the matches. `search_continues` is only trusted when the last key
/// ran one of these four commands, the same condition `goal_column` uses.
fn vertical(count: c_int, key: c_int, up: bool, fallback: Fallback) -> c_int {
    let (count, up) = if count < 0 {
        (-count, !up)
    } else {
        (count, up)
    };
    let is_run = is_vertical(ffi::last_command());
    let continuing_search =
        is_run && STATE.with_borrow(|s| s.search_continues) && matches!(fallback, Fallback::Search);
    let leave = move |n: c_int| {
        STATE.with_borrow_mut(|s| s.search_continues = matches!(fallback, Fallback::Search));
        if continuing_search {
            ffi::continue_history_search();
        }
        match (up, fallback) {
            (true, Fallback::History) => ffi::previous_history(n, key),
            (false, Fallback::History) => ffi::next_history(n, key),
            (true, Fallback::Search) => ffi::history_search_backward(n, key),
            (false, Fallback::Search) => ffi::history_search_forward(n, key),
        }
    };
    guard(
        || {
            let Some(line) = active_line() else {
                return leave(count);
            };
            let prompt = prompt_width();
            let mut point = ffi::point();
            let goal = STATE.with_borrow_mut(|s| {
                if !is_run {
                    s.goal_column = None;
                }
                *s.goal_column
                    .get_or_insert_with(|| lines::column(&line, point, prompt))
            });
            for moved in 0..count {
                let next = if up {
                    lines::up(&line, point, goal, prompt)
                } else {
                    lines::down(&line, point, goal, prompt)
                };
                let Some(next) = next else {
                    let result = leave(count - moved);
                    // With no older entry the line stays as it is, and so
                    // does the cursor.
                    if up
                        && matches!(fallback, Fallback::History)
                        && ffi::line().as_deref() != Some(line.as_str())
                    {
                        open_at_start();
                    }
                    return result;
                };
                point = next;
            }
            STATE.with_borrow_mut(|s| s.search_continues = false);
            ffi::set_point(point);
            0
        },
        || leave(count),
    )
}

pub(super) extern "C" fn line_start(count: c_int, key: c_int) -> c_int {
    on_line(count, key, ffi::beginning_of_line, |line, point| {
        ffi::set_point(lines::line_start(line, point));
    })
}

pub(super) extern "C" fn line_end(count: c_int, key: c_int) -> c_int {
    end_of_line(count, key)
}

/// `line-end`; also what `accept-suggestion` does without a suggestion.
pub(super) fn end_of_line(count: c_int, key: c_int) -> c_int {
    on_line(count, key, ffi::end_of_line, |line, point| {
        ffi::set_point(lines::line_end(line, point));
    })
}

pub(super) extern "C" fn kill_to_line_end(count: c_int, key: c_int) -> c_int {
    on_line(count, key, ffi::kill_line, |line, point| {
        let kill = lines::kill_forward(line, point);
        ffi::kill_text(kill.start, kill.end);
        ffi::set_point(kill.start);
    })
}

pub(super) extern "C" fn kill_to_line_start(count: c_int, key: c_int) -> c_int {
    on_line(count, key, ffi::unix_line_discard, |line, point| {
        let kill = lines::kill_backward(line, point);
        ffi::kill_text(kill.end, kill.start);
        ffi::set_point(kill.start);
    })
}

/// Runs `edit` on the line and cursor. Readline's `fallback` runs instead
/// when the multi-line commands are off, the command has one line, or a
/// count was typed, so single-line editing stays readline's.
fn on_line(
    count: c_int,
    key: c_int,
    fallback: fn(c_int, c_int) -> c_int,
    edit: impl FnOnce(&str, usize),
) -> c_int {
    guard(
        || match active_line() {
            Some(line) if line.contains('\n') && !ffi::explicit_count() => {
                edit(&line, ffi::point());
                0
            }
            _ => fallback(count, key),
        },
        || fallback(count, key),
    )
}

/// Like readline's `insert-comment`, for every line: puts `comment-begin`
/// (`#`) in front of each line and accepts the command, so none of it runs.
/// With a count it takes the comment off instead when every line has it.
pub(super) extern "C" fn comment_lines(count: c_int, key: c_int) -> c_int {
    guard(
        || {
            let Some(line) = active_line().filter(|l| l.contains('\n')) else {
                return ffi::insert_comment(count, key);
            };
            let begin = ffi::variable(c"comment-begin").unwrap_or_else(|| "#".to_owned());
            let commented = lines::comment(&line, &begin, ffi::explicit_count());
            ffi::begin_undo_group();
            ffi::delete_text(0, line.len());
            ffi::set_point(0);
            ffi::insert_text(&commented);
            ffi::end_undo_group();
            super::repaint_now();
            ffi::accept_line(1, c_int::from(b'\n'))
        },
        || ffi::insert_comment(count, key),
    )
}

/// Puts the cursor at the start of a multi-line history entry just recalled,
/// so the next Up leaves it at once. `inkline-history-cursor` set to `end`,
/// readline's `history-preserve-point` and an entry taller than the screen
/// keep readline's placement.
fn open_at_start() {
    let Some(line) = ffi::line() else { return };
    if !line.contains('\n')
        || crate::lisp::settings::history_cursor_end()
        || ffi::variable_on(c"history-preserve-point")
    {
        return;
    }
    if fits_on_screen(&line) {
        ffi::set_point(0);
    }
}
