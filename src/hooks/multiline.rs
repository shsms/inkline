//! The readline commands for a command that spans several lines.

use std::ffi::c_int;
use std::sync::atomic::Ordering;

use super::{STATE, guard, status_of};
use crate::args;
use crate::ffi;
use crate::indent;
use crate::lines;
use crate::mode_server::{self, protocol::Depths};
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

/// What Enter or `C-j` does once `guard` has returned.
enum Then {
    /// Nothing more: the key has done its work, with this result.
    Done(c_int),
    /// Run the line, after the accept hook (`super::accept_line`).
    Accept,
}

/// Does what `then` says. `super::accept_line` is called after the key's
/// `guard` has returned, as bash may jump from its end to a new prompt.
fn finish(then: Then, count: c_int, key: c_int) -> c_int {
    match then {
        Then::Done(result) => result,
        Then::Accept => super::accept_line(count, key),
    }
}

/// Enter: adds a line to an unfinished command, and accepts any other.
pub(super) extern "C" fn accept_or_newline(count: c_int, key: c_int) -> c_int {
    let then = guard(
        || {
            // A C-c that came with more keys is still waiting for bash to act
            // on it, which bash does once the line is accepted. Bash throws
            // that line away, so the accept hook does not run.
            if ffi::interrupted() {
                return Then::Done(ffi::accept_line(count, key));
            }
            let Some(line) = active_line() else {
                return Then::Accept;
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
                    return Then::Accept;
                }
                new_line(&line, point, below);
                return Then::Done(0);
            }
            let status = STATE.with_borrow_mut(|s| status_of(s, &line));
            if status == Status::Unfinished {
                if !fits_more_rows(&line, point, 1) {
                    return Then::Accept;
                }
                new_line(&line, point, false);
                return Then::Done(0);
            }
            if !ffi::input_waiting() && move_out(&line, point).is_some() {
                // Readline does not redraw before it accepts a line.
                super::repaint_now();
            }
            Then::Accept
        },
        || Then::Done(ffi::accept_line(count, key)),
    );
    finish(then, count, key)
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
    let then = guard(
        || {
            // A C-c that came with more keys is still waiting for bash to act
            // on it, which bash does once the line is accepted. Bash throws
            // that line away, so the accept hook does not run.
            if ffi::interrupted() {
                return Then::Done(ffi::accept_line(count, key));
            }
            if ffi::replaying_macro() {
                return Then::Accept;
            }
            let Some(line) = active_line() else {
                return Then::Accept;
            };
            new_line(&line, ffi::point(), false);
            Then::Done(0)
        },
        || Then::Done(ffi::accept_line(count, key)),
    );
    finish(then, count, key)
}

/// Inserts a newline at `point` as one undo step with what goes with it: the
/// line left moves out when it starts with a closing word, the spaces and
/// tabs around the cursor go, and the new line gets its indentation.
///
/// Inside a quoted argument of a command that has a highlight helper, the
/// helper says how deep both lines are (see `in_script`); without its answer,
/// the new line gets the indentation of the cursor's line, or one step in from
/// the command's line when the cursor is on the line the script starts on.
/// Between an empty pair of such quotes, the new line is one step in from
/// the command's line, and, when the screen has room for both lines, the
/// closing quote goes on a line of its own below it, as indented as the
/// command's line.
///
/// None of this happens when more input is already waiting (pasted text keeps
/// its own spacing) or `inkline-indent` is 0. With `close_below`, the text
/// after the cursor goes on a line of its own below the new one, as indented
/// as the cursor's line.
fn new_line(line: &str, point: usize, close_below: bool) {
    let step = indent_step();
    ffi::begin_undo_group();
    let mut indentation = String::new();
    let mut point = point;
    let mut blanks = point..point;
    // The indentation of the line below the new one that the text after the
    // cursor goes on, when it gets a line of its own.
    let mut below = None;
    if step > 0 && !ffi::input_waiting() {
        point = move_out(line, point).unwrap_or(point);
        let text = ffi::line().unwrap_or_default();
        if starts_code(&text[..point]) {
            // The indentation comes from the line as it was, so a line split
            // right after its own indentation keeps it.
            indentation = indent::for_new_line(&text, point, step);
            blanks = lines::blanks_around(&text, point);
        } else if let Some(script) = in_script(&text, point) {
            let split = indent::script_line(
                &text,
                point,
                script.command_line,
                script.first_line,
                script.depths,
                step,
            );
            if let Some(to) = &split.moved_out {
                point = set_indentation(&text, point, to);
            }
            let text = ffi::line().unwrap_or_default();
            indentation = split.new_line;
            blanks = lines::blanks_around(&text, point);
            if script.empty_pair && fits_more_rows(&text, point, 2) {
                below = Some(indent::indentation(&text[script.command_line..]).to_owned());
            }
        }
    }
    let inserted = format!("\n{indentation}");
    ffi::insert_text(&inserted);
    // The blanks go after the newline is in: undo then takes the newline out
    // last, which puts the cursor back where it was.
    let after = point + inserted.len();
    if blanks.end > point {
        ffi::delete_text(after, after + (blanks.end - point));
    }
    if blanks.start < point {
        ffi::delete_text(blanks.start, point);
        point = blanks.start;
        ffi::set_point(point + inserted.len());
    }
    if close_below {
        // The cursor's line now ends at `point`, where the newline went in.
        let text = ffi::line().unwrap_or_default();
        below = Some(indent::indentation(&text[lines::line_start(&text, point)..point]).to_owned());
    }
    if let Some(base) = below {
        let text = ffi::line().unwrap_or_default();
        let at = ffi::point();
        let blanks = indent::indentation(&text[at..]).len();
        ffi::delete_text(at, at + blanks);
        ffi::insert_text(&format!("\n{base}"));
        ffi::set_point(at);
    }
    ffi::end_undo_group();
}

/// A new line's place inside a quoted argument of a command that has a
/// highlight helper.
struct InScript {
    /// Where the line that holds the command's name starts.
    command_line: usize,
    /// Where the line the argument starts on starts: the script's first
    /// line.
    first_line: usize,
    /// The depths the helper gave; `None` when it was not asked or gave
    /// none in time.
    depths: Option<Depths>,
    /// Whether the cursor is between the argument's opening quote and the
    /// one that closes it, with only blanks between them.
    empty_pair: bool,
}

/// Whether the cursor at `point` in `text` is inside a quoted argument of
/// a command that has a highlight helper, found as for colours. If so, asks
/// the helper how deep the lines are (`mode_server::indent`, waiting up to
/// `mode_server::INDENT_WAIT`), unless the argument is `raw`, it is an empty
/// pair of quotes, or Lisp is running. `None` when the cursor is not inside
/// such an argument: the bash rules apply.
fn in_script(text: &str, point: usize) -> Option<InScript> {
    if !mode_server::any_registered() {
        return None;
    }
    let quote = syntax::open_quote(&text[..point])?;
    // `STATE` is borrowed only for the parse: never while waiting on the
    // helper.
    let tree = STATE.with_borrow_mut(|s| s.lexer.tree(text))?;
    let commands = args::commands(&tree, text, mode_server::is_registered);
    let (command, index) = args::with_quote(&commands, quote)?;
    let command_line = lines::line_start(text, command.args[0].start()?);
    let arg = &command.args[index];
    let first_line = lines::line_start(text, arg.start()?);
    let empty_pair = empty_quote_pair(text, point, quote);
    let depths = if empty_pair || arg.raw || crate::lisp::RUNNING.load(Ordering::Relaxed) {
        None
    } else {
        let cwd = ffi::shell_variable("PWD").unwrap_or_default().into_bytes();
        mode_server::indent(
            &command.name,
            &mode_server::request(cwd, command),
            (index, arg.offset_at(point)),
            mode_server::INDENT_WAIT,
            ffi::signal_to_act_on,
        )
    };
    Some(InScript {
        command_line,
        first_line,
        depths,
        empty_pair,
    })
}

/// Whether the cursor is between the quote mark at byte `quote` and the one
/// that closes it, with only spaces and tabs between them, such as pairing
/// leaves after `csvm "`.
fn empty_quote_pair(text: &str, point: usize, quote: usize) -> bool {
    let mark = text.as_bytes()[quote];
    text[quote + 1..point].trim_matches([' ', '\t']).is_empty()
        && text[point..]
            .trim_start_matches([' ', '\t'])
            .as_bytes()
            .first()
            == Some(&mark)
}

/// Replaces the indentation of the line `point` is on with `to`, and
/// returns where the cursor is then. The cursor must be past the
/// indentation.
fn set_indentation(text: &str, point: usize, to: &str) -> usize {
    let start = lines::line_start(text, point);
    let old = indent::indentation(&text[start..]).len();
    ffi::delete_text(start, start + old);
    ffi::set_point(start);
    ffi::insert_text(to);
    let point = point - old + to.len();
    ffi::set_point(point);
    point
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
