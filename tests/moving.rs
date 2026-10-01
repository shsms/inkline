//! Moving around a command that spans several lines.

#[path = "support/common.rs"]
mod common;

use common::*;

const LOOP: &str = "for x in a\ndo echo $x\ndone";

/// Types `lines` joined by `C-j` and waits for the cursor on the last.
fn block(sh: &mut Shell, lines: &[&str]) {
    sh.send(&lines.join(CTRL_J));
    let last = lines.len() as u16 - 1;
    sh.wait_for("the block", |s| s.cursor_position().0 == last);
}

#[test]
fn up_and_down_move_between_lines() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo bb"]);
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 7));
    sh.send("X");
    sh.wait_for("the insert", |s| row_text(s, 0) == "$ echo Xa");
    sh.send(DOWN);
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 7));
}

#[test]
fn a_run_of_moves_keeps_its_column() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo abcdefghij", "x", "echo 12345678"]);
    sh.send(UP);
    sh.wait_for("the short line", |s| s.cursor_position() == (1, 1));
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 13));
}

/// A move with a `C-u` count is part of the run: the next Up keeps the
/// column the run started with.
#[test]
fn a_c_u_counted_move_keeps_the_runs_column() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo abcdefghij", "x", "y", "echo 12345678"]);
    sh.send(&format!("\x152{UP}"));
    sh.wait_for("the short line", |s| s.cursor_position() == (1, 1));
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 13));
}

#[test]
fn up_on_the_first_line_goes_to_history() {
    let mut sh = Shell::start(Options {
        history: vec!["echo old"],
        ..Options::default()
    });
    block(&mut sh, &["echo new", "x"]);
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position().0 == 0);
    sh.send(UP);
    sh.wait_for("the history entry", |s| {
        row_text(s, 0) == "$ echo old" && s.cursor_position() == (0, 10)
    });
}

/// With no older entry, Up on the first line leaves the cursor where it is.
#[test]
fn up_past_the_oldest_entry_keeps_the_cursor() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo b"]);
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 6));
    sh.send(UP);
    sh.send("X");
    sh.wait_for("the insert", |s| {
        row_text(s, 0) == "$ echoX a" && s.cursor_position() == (0, 7)
    });
}

#[test]
fn a_recalled_multi_line_entry_opens_at_its_start() {
    let mut sh = Shell::start(Options {
        history: vec![LOOP],
        ..Options::default()
    });
    sh.send(UP);
    sh.wait_for("the entry", |s| {
        row_text(s, 2) == "done" && s.cursor_position() == (0, 2)
    });
}

#[test]
fn inkline_history_cursor_end_keeps_readlines_place() {
    let mut sh = Shell::start(Options {
        rc: "inkline eval \"(setq inkline-history-cursor 'end)\" >/dev/null\n".into(),
        history: vec![LOOP],
        ..Options::default()
    });
    sh.send(UP);
    sh.wait_for("the entry", |s| {
        row_text(s, 2) == "done" && s.cursor_position() == (2, 4)
    });
}

#[test]
fn home_and_end_stay_on_the_line() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo b"]);
    sh.send("\x01");
    sh.wait_for("the line's start", |s| s.cursor_position() == (1, 0));
    sh.send("\x05");
    sh.wait_for("the line's end", |s| s.cursor_position() == (1, 6));
}

/// With a count, `C-a` is readline's `beginning-of-line`, which goes to the
/// start of the whole command; `C-u` alone is a count too.
#[test]
fn c_a_with_a_c_u_count_goes_to_the_start_of_the_command() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo b"]);
    sh.send("\x15\x01");
    sh.wait_for("the command's start", |s| s.cursor_position() == (0, 2));
}

#[test]
fn kills_stop_at_the_line() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo b"]);
    sh.send("\x01\x0b");
    sh.wait_for("the killed line", |s| {
        row_text(s, 0) == "$ echo a" && row_text(s, 1).is_empty()
    });
    sh.send("\x19");
    sh.wait_for("the yank", |s| row_text(s, 1) == "echo b");
    sh.send("\x18\x7f");
    sh.wait_for("killed back to the line's start", |s| {
        row_text(s, 0) == "$ echo a" && row_text(s, 1).is_empty()
    });
}

#[test]
fn at_a_line_edge_the_kills_join_lines() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo b"]);
    sh.send("\x01\x18\x7f");
    sh.wait_for("joined by C-x DEL", |s| row_text(s, 0) == "$ echo aecho b");
    sh.send("\x1f");
    sh.wait_for("undone", |s| row_text(s, 1) == "echo b");
    sh.send(UP);
    sh.send("\x05\x0b");
    sh.wait_for("joined by C-k", |s| row_text(s, 0) == "$ echo aecho b");
}

/// `C-k` with a count of 0 kills back to the start of the current line only,
/// and `C-y` gives the text back.
#[test]
fn c_k_with_a_count_of_0_kills_back_to_the_line_start() {
    for zero in ["\x150", "\x1b0"] {
        let mut sh = Shell::start(Options::default());
        block(&mut sh, &["echo a", "echo bc"]);
        sh.send(&format!("\x02{zero}\x0b"));
        sh.wait_for("killed back", |s| {
            row_text(s, 0) == "$ echo a" && row_text(s, 1) == "c" && s.cursor_position() == (1, 0)
        });
        sh.send("\x19");
        sh.wait_for("the yank", |s| row_text(s, 1) == "echo bc");
    }
}

/// `C-k` with a count of 2 kills to the start of the second line down,
/// newlines and all.
#[test]
fn c_k_with_a_count_kills_to_the_start_of_that_many_lines_down() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo abc", "echo d", "echo e"]);
    sh.send(&format!("{UP}{UP}"));
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 6));
    sh.send("\x152\x0b");
    sh.wait_for("killed down", |s| {
        row_text(s, 0) == "$ echoecho e" && row_text(s, 1).is_empty()
    });
    sh.send("\x19");
    sh.wait_for("the yank", |s| {
        row_text(s, 0) == "$ echo abc" && row_text(s, 1) == "echo d" && row_text(s, 2) == "echo e"
    });
}

/// `C-k` with a count past the last line kills to the end of the command;
/// `C-u` alone is a count of 4.
#[test]
fn c_k_with_a_count_past_the_last_line_kills_to_the_end() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "b", "c", "d", "e", "f"]);
    sh.send(&format!("{UP}{UP}{UP}{UP}{UP}\x01"));
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 2));
    sh.send("\x15\x0b");
    sh.wait_for("four lines killed", |s| {
        row_text(s, 0) == "$ e" && row_text(s, 1) == "f" && row_text(s, 2).is_empty()
    });
    sh.send("\x01\x159\x0b");
    sh.wait_for("the rest killed", |s| {
        row_text(s, 0) == "$" && row_text(s, 1).is_empty()
    });
}

/// `C-k` with a count below 0 kills back from the start of that many lines
/// up, or of the command.
#[test]
fn c_k_with_a_count_below_0_kills_from_lines_up() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo b", "echo cd"]);
    sh.send("\x02\x1b-\x0b");
    sh.wait_for("killed from the line above", |s| {
        row_text(s, 0) == "$ echo a" && row_text(s, 1) == "d" && s.cursor_position() == (1, 0)
    });
    sh.send("\x19");
    sh.wait_for("the yank", |s| {
        row_text(s, 1) == "echo b" && row_text(s, 2) == "echo cd"
    });
    sh.send("\x15-5\x0b");
    sh.wait_for("killed from the start", |s| {
        row_text(s, 0) == "$ d" && row_text(s, 1).is_empty()
    });
}

/// On a command of one line, `C-k` with a count above 0 kills to the end, and
/// with 0 or below back to the start. Kills in a row join, as readline's do.
#[test]
fn c_k_with_a_count_on_one_line() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo abc\x02\x02");
    sh.wait_for("the cursor", |s| s.cursor_position() == (0, 8));
    sh.send("\x152\x0b");
    sh.wait_for("killed to the end", |s| cursor_row(s) == "$ echo a");
    sh.send("\x19");
    sh.wait_for("the yank", |s| cursor_row(s) == "$ echo abc");
    sh.send("\x02\x02\x1b0\x0b");
    sh.wait_for("killed back", |s| {
        cursor_row(s) == "$ bc" && s.cursor_position() == (0, 2)
    });
    // The next kill joins the last: one yank gives back both.
    sh.send("\x0b\x19");
    sh.wait_for("both yanked", |s| cursor_row(s) == "$ echo abc");
    sh.send("\x02\x02\x1b-\x0b");
    sh.wait_for("killed back", |s| cursor_row(s) == "$ bc");
}

/// A shell whose Up and Down run `up` and `down`.
fn arrow_keys(up: &str, down: &str, history: Vec<&'static str>) -> Options {
    Options {
        rc: format!("bind '\"\\e[A\": {up}'\nbind '\"\\e[B\": {down}'\n"),
        history,
        ..Options::default()
    }
}

#[test]
fn up_can_search_history() {
    let mut sh = Shell::start(arrow_keys(
        "previous-line-or-search",
        "next-line-or-search",
        vec!["git status", "ls", "git log"],
    ));
    sh.send("git");
    sh.wait_for("the text", |s| cursor_row(s).starts_with("$ git"));
    sh.send(UP);
    sh.wait_for("the newest match", |s| row_text(s, 0) == "$ git log");
    sh.send(UP);
    sh.wait_for("the next match", |s| row_text(s, 0) == "$ git status");
}

/// A shell whose Up and Down run the substring search commands.
fn substring_keys(history: Vec<&'static str>) -> Options {
    arrow_keys(
        "previous-line-or-substring-search",
        "next-line-or-substring-search",
        history,
    )
}

#[test]
fn up_finds_entries_holding_the_typed_text() {
    let mut sh = Shell::start(substring_keys(vec!["git status", "ls", "echo stat", "pwd"]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the newest match", |s| {
        cursor_row(s) == "$ echo stat" && s.cursor_position() == (0, 11)
    });
    sh.send(UP);
    let s = sh.wait_for("the match before it", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    assert_eq!(row_text(&s, 1), "", "no menu on a found line: {}", dump(&s));
}

/// An entry that is the typed text itself is passed over.
#[test]
fn up_passes_over_an_entry_that_is_the_typed_text() {
    let mut sh = Shell::start(substring_keys(vec!["git status", "git st", "ls"]));
    sh.send("git st");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 8));
    sh.send(UP);
    sh.wait_for("the older match", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
}

/// With only the typed text itself older than a match, Up leaves the match
/// and its cursor as they are.
#[test]
fn up_leaves_the_last_match_when_only_the_typed_text_is_older() {
    let mut sh = Shell::start(substring_keys(vec!["git st", "ls", "git status"]));
    sh.send("git st");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 8));
    sh.send(UP);
    sh.wait_for("the match", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    sh.send(UP);
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git status", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 12), "{}", dump(&s));
    // The typed text shows its grey text and menu again.
    sh.send(DOWN);
    sh.wait_for("the typed text back", |s| {
        s.cursor_position() == (0, 8) && row_text(s, 1) == "h  git status"
    });
}

/// Up whose only match is the typed text itself leaves the typed line as
/// it is, not as a history entry: after more typing, Down leaves it alone.
#[test]
fn down_after_up_found_only_the_typed_text_keeps_the_line() {
    let mut sh = Shell::start(substring_keys(vec!["zzq", "ls"]));
    sh.send("zzq");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ zzq");
    sh.send(UP);
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ zzq", "{}", dump(&s));
    sh.send("x");
    sh.wait_for("the x", |s| cursor_row(s) == "$ zzqx");
    sh.send(DOWN);
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ zzqx", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 6), "{}", dump(&s));
}

#[test]
fn down_past_the_newest_match_gives_back_the_typed_text() {
    let mut sh = Shell::start(substring_keys(vec!["git status", "ls", "echo stat", "pwd"]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the newest match", |s| cursor_row(s) == "$ echo stat");
    sh.send(UP);
    sh.wait_for("the match before it", |s| cursor_row(s) == "$ git status");
    sh.send(DOWN);
    sh.wait_for("the newer match", |s| cursor_row(s) == "$ echo stat");
    sh.send(DOWN);
    sh.wait_for("the typed text back", |s| {
        cursor_row(s) == "$ stat" && s.cursor_position() == (0, 6)
    });
}

#[test]
fn up_on_an_empty_line_walks_history() {
    let mut sh = Shell::start(substring_keys(vec!["echo one", "echo two", "echo three"]));
    sh.settle();
    for row in ["$ echo three", "$ echo two", "$ echo one"] {
        sh.send(UP);
        sh.wait_for(row, |s| cursor_row(s) == row);
    }
}

#[test]
fn a_multi_line_match_opens_at_its_start() {
    let mut sh = Shell::start(substring_keys(vec!["echo older", LOOP]));
    sh.send("echo");
    // The rest of "echo older" shows after the cursor as a suggestion.
    sh.wait_for("the typed text", |s| {
        cursor_row(s).starts_with("$ echo") && s.cursor_position() == (0, 6)
    });
    sh.send(UP);
    sh.wait_for("the loop", |s| {
        row_text(s, 2) == "done" && s.cursor_position() == (0, 2)
    });
    sh.send(UP);
    sh.wait_for("the older match", |s| cursor_row(s) == "$ echo older");
}

/// After Up finds a multi-line entry, Down moves through its lines and
/// past the last one goes on with the search: back to the typed text.
#[test]
fn down_through_a_found_multi_line_entry_goes_on_with_the_search() {
    let mut sh = Shell::start(substring_keys(vec!["echo older", LOOP]));
    sh.send("echo");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 6));
    sh.send(UP);
    sh.wait_for("the loop", |s| {
        row_text(s, 2) == "done" && s.cursor_position() == (0, 2)
    });
    sh.send(DOWN);
    sh.wait_for("the second line", |s| s.cursor_position().0 == 1);
    sh.send(DOWN);
    sh.wait_for("the third line", |s| s.cursor_position().0 == 2);
    sh.send(DOWN);
    sh.wait_for("the typed text back", |s| {
        cursor_row(s).starts_with("$ echo")
            && row_text(s, 1) != "do echo $x"
            && s.cursor_position() == (0, 6)
    });
}

/// Types a two-line command no history entry holds, goes Up to its first
/// line, and Up again, where the search finds nothing; then Down to the
/// last line and Down again. The search left the line as it was, so moving
/// between its lines ends it, and the last Down leaves the cursor where it
/// is.
fn down_after_a_failed_search_stays(options: Options) {
    let mut sh = Shell::start(options);
    block(&mut sh, &["echo a", "echo b"]);
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 6));
    sh.send(UP);
    let s = sh.settle();
    assert_eq!(s.cursor_position(), (0, 6), "{}", dump(&s));
    sh.send(DOWN);
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 6));
    sh.send(DOWN);
    let s = sh.settle();
    assert_eq!(row_text(&s, 0), "$ echo a", "{}", dump(&s));
    assert_eq!(row_text(&s, 1), "echo b", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (1, 6), "{}", dump(&s));
}

#[test]
fn down_after_a_failed_substring_search_stays() {
    down_after_a_failed_search_stays(substring_keys(vec!["ls"]));
}

#[test]
fn down_after_a_failed_prefix_search_stays() {
    down_after_a_failed_search_stays(arrow_keys(
        "previous-line-or-search",
        "next-line-or-search",
        vec!["ls"],
    ));
}

/// On a history entry that was changed, Down walks history: it does not
/// search for the changed text.
#[test]
fn down_on_a_changed_history_entry_walks_history() {
    let mut sh = Shell::start(substring_keys(vec!["echo one", "echo two"]));
    sh.settle();
    sh.send(UP);
    sh.wait_for("the newest entry", |s| cursor_row(s) == "$ echo two");
    sh.send(UP);
    sh.wait_for("the older entry", |s| cursor_row(s) == "$ echo one");
    sh.send("X");
    sh.wait_for("the change", |s| cursor_row(s) == "$ echo oneX");
    sh.send(DOWN);
    sh.wait_for("the newer entry", |s| cursor_row(s) == "$ echo two");
}

#[test]
fn up_goes_to_the_line_above_first() {
    let mut sh = Shell::start(substring_keys(vec!["echo old"]));
    block(&mut sh, &["echo a", "echo b"]);
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 6));
}

/// Up on a line a search found and that was then changed searches again,
/// for the changed text, as it does on a typed line.
#[test]
fn up_on_a_changed_found_line_searches_again() {
    let mut sh = Shell::start(substring_keys(vec![
        "echo statX older",
        "git status",
        "ls",
        "echo stat",
    ]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the match", |s| cursor_row(s) == "$ echo stat");
    sh.send("X");
    // The rest of "echo statX older" shows after the cursor as a suggestion.
    sh.wait_for("the change", |s| {
        cursor_row(s).starts_with("$ echo statX") && s.cursor_position() == (0, 12)
    });
    // Going on with the search for "stat" would give "git status", and a
    // walk "ls".
    sh.send(UP);
    sh.wait_for("the match for the changed text", |s| {
        cursor_row(s) == "$ echo statX older" && s.cursor_position() == (0, 18)
    });
    // Nothing older holds "echo statX": the line and cursor stay, so Y
    // goes at the end.
    sh.send(UP);
    sh.send("Y");
    sh.wait_for("the line as it was", |s| {
        cursor_row(s) == "$ echo statX olderY" && s.cursor_position() == (0, 19)
    });
}

#[test]
fn up_past_the_oldest_match_keeps_the_cursor() {
    let mut sh = Shell::start(substring_keys(vec!["git status"]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the match", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    sh.send(UP);
    sh.send("Z");
    sh.wait_for("the cursor where it was", |s| {
        cursor_row(s) == "$ git statusZ" && s.cursor_position() == (0, 13)
    });
}

/// A walk through history places the cursor as readline does, here where
/// `history-preserve-point` keeps it.
#[test]
fn a_walk_keeps_readlines_cursor_place() {
    let mut options = substring_keys(vec!["echo one", "echo two"]);
    options
        .rc
        .push_str("bind 'set history-preserve-point on'\n");
    let mut sh = Shell::start(options);
    sh.settle();
    sh.send(UP);
    sh.wait_for("the newest entry", |s| {
        cursor_row(s) == "$ echo two" && s.cursor_position() == (0, 10)
    });
    sh.send("\x02");
    sh.wait_for("the cursor back one", |s| s.cursor_position() == (0, 9));
    sh.send(UP);
    sh.wait_for("the older entry, cursor kept", |s| {
        cursor_row(s) == "$ echo one" && s.cursor_position() == (0, 9)
    });
}

/// Down past the newest match gives back the typed line with its undo
/// list: undo takes the typing off in one step and never shows a history
/// entry.
#[test]
fn undo_after_down_back_to_the_typed_text_takes_the_typing_off() {
    let mut sh = Shell::start(substring_keys(vec!["echo stat", "pwd"]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the match", |s| cursor_row(s) == "$ echo stat");
    sh.send(DOWN);
    sh.wait_for("the typed text back", |s| {
        cursor_row(s) == "$ stat" && s.cursor_position() == (0, 6)
    });
    sh.send("\x1f");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 2), "{}", dump(&s));
}

/// A count goes that many matches back: two Ups from `git st` give
/// `git status`, then `git stash`, and so does `M-2 Up`.
#[test]
fn a_count_goes_that_many_matches_back() {
    let mut sh = Shell::start(substring_keys(vec!["git stash", "git status", "git st"]));
    sh.send("git st");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 8));
    sh.send("\x1b2");
    sh.send(UP);
    sh.wait_for("the second match", |s| {
        cursor_row(s) == "$ git stash" && s.cursor_position() == (0, 11)
    });
}

/// Once the search has ended, Down on the line it found walks history
/// from that entry: to the next newer one.
#[test]
fn down_on_a_found_line_after_the_search_ended_walks_on() {
    let mut sh = Shell::start(substring_keys(vec!["git status", "ls", "echo stat", "pwd"]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the match", |s| {
        cursor_row(s) == "$ echo stat" && s.cursor_position() == (0, 11)
    });
    sh.send("\x02");
    sh.wait_for("the cursor back one", |s| s.cursor_position() == (0, 10));
    sh.send(DOWN);
    sh.wait_for("the next entry", |s| cursor_row(s) == "$ pwd");
    sh.send(DOWN);
    sh.wait_for("the typed text back", |s| cursor_row(s) == "$ stat");
}

/// A shell with `C-x k` bound to the prefix search and `C-x p` to the
/// substring search.
fn prefix_and_substring_keys() -> Options {
    Options {
        rc: "bind '\"\\C-xk\": previous-line-or-search'\n\
             bind '\"\\C-xp\": previous-line-or-substring-search'\n"
            .into(),
        history: vec!["git status", "xgit", "git log"],
        ..Options::default()
    }
}

/// Options whose Up and Down keys have readline's prefix search in inputrc,
/// so with no menu they run `previous-line-or-search` and
/// `next-line-or-search`, and `C-p` the substring search.
fn prefix_arrows(history: Vec<&'static str>) -> Options {
    Options {
        inputrc: Some(
            "\"\\e[A\": history-search-backward\n\"\\e[B\": history-search-forward\n".into(),
        ),
        history,
        ..Options::default()
    }
}

/// On a line the prefix search of the Up key found, the substring key
/// (`C-p`) starts a new search for the text before the cursor, as on a line
/// it found itself: it finds `git status`, where walking history from the
/// match would give `ls`.
#[test]
fn the_substring_key_searches_again_on_a_line_the_prefix_key_found() {
    let mut sh = Shell::start(prefix_arrows(vec!["git status", "ls", "git log"]));
    sh.send("git");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.send(UP);
    sh.wait_for("the prefix match", |s| cursor_row(s) == "$ git log");
    sh.send("\x10");
    sh.wait_for("the older match", |s| cursor_row(s) == "$ git status");
}

/// A prefix search that starts on a line the substring key found finds an
/// older entry; the substring key then searches older than that entry, not
/// older than the one the prefix search started on, `git log`, from where
/// it would find `echo git log`.
#[test]
fn the_substring_key_searches_older_than_a_prefix_match_from_a_found_line() {
    let mut sh = Shell::start(prefix_arrows(vec![
        "git log --all",
        "git log -p",
        "echo git log",
        "git log",
    ]));
    sh.send("git lo");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 8));
    sh.send("\x07");
    sh.send("\x10");
    sh.wait_for("the substring match", |s| cursor_row(s) == "$ git log");
    sh.send(UP);
    sh.wait_for("the prefix match", |s| cursor_row(s) == "$ git log -p");
    sh.send("\x10");
    sh.wait_for("the match older than it", |s| {
        cursor_row(s) == "$ git log --all"
    });
}

/// After a prefix search that went on to an older copy of an entry it
/// found before, the substring key searches older than that older copy.
#[test]
fn the_substring_key_searches_older_than_a_repeated_prefix_match() {
    let mut sh = Shell::start(prefix_arrows(vec![
        "echo git log",
        "git log X",
        "git log Y",
        "git log X",
    ]));
    sh.send("git log");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 9));
    sh.send("\x07");
    sh.send(UP);
    sh.wait_for("the newest match", |s| cursor_row(s) == "$ git log X");
    sh.send(UP);
    sh.wait_for("the next match", |s| cursor_row(s) == "$ git log Y");
    sh.send(UP);
    sh.wait_for("the oldest prefix match", |s| {
        cursor_row(s) == "$ git log X"
    });
    sh.send("\x10");
    sh.wait_for("the match older than it", |s| {
        cursor_row(s) == "$ echo git log"
    });
}

/// The prefix key after the substring key starts its own search, for the
/// text before the cursor: nothing older starts with `git log`, so the
/// line stays. Going on with a prefix search would walk to `xgit`.
#[test]
fn the_prefix_key_does_not_go_on_with_a_substring_search() {
    let mut sh = Shell::start(prefix_and_substring_keys());
    sh.send("git");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.send("\x18p");
    sh.wait_for("the substring match", |s| {
        cursor_row(s) == "$ git log" && s.cursor_position() == (0, 9)
    });
    sh.send("\x18k");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git log", "{}", dump(&s));
}

/// The substring key after the prefix key starts its own search, for the
/// text before the cursor, which the prefix search left after `git`, from
/// the line it found: it finds `git`. Going on with the prefix run as a
/// substring search would pass over `git`, the line that run started from,
/// and find nothing.
#[test]
fn the_substring_key_does_not_go_on_with_a_prefix_search() {
    let mut sh = Shell::start(Options {
        history: vec!["git", "git log"],
        ..prefix_and_substring_keys()
    });
    sh.send("git");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.send("\x18k");
    sh.wait_for("the prefix match", |s| {
        cursor_row(s) == "$ git log" && s.cursor_position() == (0, 5)
    });
    sh.send("\x18p");
    sh.wait_for("the substring match", |s| cursor_row(s) == "$ git");
}

/// The substring Down key after the prefix key does not go on with the
/// prefix run either: the substring search's Down never starts one, so it
/// walks history from the found line, to `ls`. Going on with the run would
/// find `git b`.
#[test]
fn the_substring_down_key_walks_on_from_a_prefix_match() {
    let mut options = prefix_and_substring_keys();
    options
        .rc
        .push_str("bind '\"\\C-xn\": next-line-or-substring-search'\n");
    let mut sh = Shell::start(Options {
        history: vec!["git a", "ls", "git b"],
        ..options
    });
    sh.send("git");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.wait_for("no grey text", |s| cursor_row(s) == "$ git");
    sh.send("\x18k");
    sh.wait_for("the newest prefix match", |s| cursor_row(s) == "$ git b");
    sh.send("\x18k");
    sh.wait_for("the older prefix match", |s| cursor_row(s) == "$ git a");
    sh.send("\x18n");
    sh.wait_for("the next entry", |s| cursor_row(s) == "$ ls");
}

/// After the prefix keys went down to a newer match, the substring key
/// searches older than that match: from `git b`, `C-p` finds the `git c`
/// before it, where walking history would give `ls`.
#[test]
fn the_substring_key_searches_older_than_a_prefix_match_down_found() {
    let mut sh = Shell::start(prefix_arrows(vec!["git c", "ls", "git b", "git c"]));
    sh.send("git");
    sh.wait_for("the typed text", |s| cursor_row(s).starts_with("$ git"));
    sh.send("\x07");
    sh.wait_for("no grey text", |s| cursor_row(s) == "$ git");
    sh.send(UP);
    sh.wait_for("the newest match", |s| cursor_row(s) == "$ git c");
    sh.send(UP);
    sh.wait_for("the next match", |s| cursor_row(s) == "$ git b");
    sh.send(UP);
    sh.wait_for("the oldest git c", |s| cursor_row(s) == "$ git c");
    sh.send(DOWN);
    sh.wait_for("the newer match", |s| cursor_row(s) == "$ git b");
    sh.send("\x10");
    sh.wait_for("the match older than it", |s| cursor_row(s) == "$ git c");
}

/// A count goes that many prefix matches back, and the substring key then
/// searches older than the match the count reached.
#[test]
fn the_substring_key_searches_older_than_a_counted_prefix_match() {
    let mut sh = Shell::start(prefix_arrows(vec!["echo git q", "git x", "git y", "git x"]));
    sh.send("git");
    sh.wait_for("the typed text", |s| cursor_row(s).starts_with("$ git"));
    sh.send("\x07");
    sh.wait_for("no grey text", |s| cursor_row(s) == "$ git");
    sh.send("\x1b3");
    sh.send(UP);
    sh.wait_for("the third match", |s| {
        cursor_row(s) == "$ git x" && s.cursor_position() == (0, 5)
    });
    sh.send("\x10");
    sh.wait_for("the match older than it", |s| {
        cursor_row(s) == "$ echo git q"
    });
}

/// On the oldest prefix match, the substring key's new search finds
/// nothing older: the bell rings and the line stays, where walking history
/// would give `ls`.
#[test]
fn the_substring_key_finds_nothing_older_than_the_oldest_prefix_match() {
    let mut sh = Shell::start(prefix_arrows(vec!["ls", "git log -p", "echo git log"]));
    sh.send("git lo");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 8));
    sh.send("\x07");
    sh.wait_for("no grey text", |s| cursor_row(s) == "$ git lo");
    sh.send(UP);
    sh.wait_for("the prefix match", |s| {
        cursor_row(s) == "$ git log -p" && s.cursor_position() == (0, 8)
    });
    sh.send("\x10");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git log -p", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 8), "{}", dump(&s));
}

/// On an entry brought back by walking history, the prefix keys search
/// for the text before the cursor, as readline's prefix search does: after
/// `C-a M-f` on `git log`, Up finds `git status` with the cursor after
/// `git`, where walking history would give `ls`.
#[test]
fn the_prefix_keys_search_on_an_entry_brought_back_by_walking() {
    let mut sh = Shell::start(prefix_arrows(vec!["git status", "ls", "git log"]));
    sh.settle();
    sh.send(UP);
    sh.wait_for("the newest entry", |s| cursor_row(s) == "$ git log");
    sh.send("\x01\x1bf");
    sh.wait_for("the cursor after git", |s| s.cursor_position() == (0, 5));
    sh.send(UP);
    sh.wait_for("the prefix match", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 5)
    });
}

/// On an entry brought back by walking and then changed, the prefix keys
/// search for the changed text: `git log` cut back to `git` finds
/// `git status`, where walking history would give `ls`.
#[test]
fn the_prefix_keys_search_on_a_changed_entry_brought_back_by_walking() {
    let mut sh = Shell::start(prefix_arrows(vec!["git status", "ls", "git log"]));
    sh.settle();
    sh.send(UP);
    sh.wait_for("the newest entry", |s| cursor_row(s) == "$ git log");
    sh.send("\x7f\x7f\x7f\x7f");
    sh.wait_for("the cut", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.wait_for("no grey text", |s| cursor_row(s) == "$ git");
    sh.send(UP);
    sh.wait_for("the prefix match", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 5)
    });
}

/// A prefix match sets the mark at the end of the line, as readline's
/// prefix search does, so `C-x C-x` goes there.
#[test]
fn a_prefix_match_sets_the_mark_at_the_line_end() {
    let mut sh = Shell::start(prefix_arrows(vec!["git status --short"]));
    sh.send("git");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.wait_for("no grey text", |s| cursor_row(s) == "$ git");
    sh.send(UP);
    sh.wait_for("the prefix match", |s| {
        cursor_row(s) == "$ git status --short" && s.cursor_position() == (0, 5)
    });
    sh.send("\x18\x18");
    sh.wait_for("the cursor at the mark", |s| s.cursor_position() == (0, 20));
}

/// The prefix search goes through the matches with each Up, passing over
/// one that repeats the last, with the cursor after the prefix; Down goes
/// back through them and past the newest gives back the typed text with
/// its cursor and undo list: undo takes the typing off and never shows a
/// history entry.
#[test]
fn the_prefix_search_goes_through_the_matches_and_back() {
    let mut sh = Shell::start(arrow_keys(
        "previous-line-or-search",
        "next-line-or-search",
        vec!["git status", "ls", "git log", "git log"],
    ));
    sh.send("git");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.wait_for("no grey text", |s| cursor_row(s) == "$ git");
    sh.send(UP);
    sh.wait_for("the newest match", |s| {
        cursor_row(s) == "$ git log" && s.cursor_position() == (0, 5)
    });
    sh.send(UP);
    sh.wait_for("the older match", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 5)
    });
    sh.send(DOWN);
    sh.wait_for("the newer match", |s| {
        cursor_row(s) == "$ git log" && s.cursor_position() == (0, 5)
    });
    // The typed text may show `git log` as its grey text again, so only
    // undo, below, tells it from the match.
    sh.send(DOWN);
    let s = sh.settle();
    assert!(cursor_row(&s).starts_with("$ git"), "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 5), "{}", dump(&s));
    sh.send("\x1f");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 2), "{}", dump(&s));
}

/// A multi-line entry the prefix search finds has the cursor right after
/// the prefix, as readline's own prefix search puts it.
#[test]
fn a_multi_line_prefix_match_has_the_cursor_after_the_prefix() {
    let mut sh = Shell::start(arrow_keys(
        "previous-line-or-search",
        "next-line-or-search",
        vec![LOOP, "ls"],
    ));
    sh.send("for");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.wait_for("no grey text", |s| cursor_row(s) == "$ for");
    sh.send(UP);
    sh.wait_for("the loop", |s| {
        row_text(s, 2) == "done" && s.cursor_position() == (0, 5)
    });
}

/// An entry that is the typed line itself is passed over by the prefix
/// search too, so Up does not seem to do nothing.
#[test]
fn the_prefix_search_passes_over_an_entry_that_is_the_typed_line() {
    let mut sh = Shell::start(arrow_keys(
        "previous-line-or-search",
        "next-line-or-search",
        vec!["git status", "git st"],
    ));
    sh.send("git st");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 8));
    // `git status` shows as grey text until C-g hides it.
    sh.send("\x07");
    sh.wait_for("no grey text", |s| cursor_row(s) == "$ git st");
    sh.send(UP);
    sh.wait_for("the older match", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 8)
    });
}

/// With nothing before the cursor the prefix keys walk history, and a
/// multi-line entry opens at its start, as `previous-line-or-history`
/// opens it.
#[test]
fn a_multi_line_entry_the_prefix_keys_walk_to_opens_at_its_start() {
    let mut sh = Shell::start(arrow_keys(
        "previous-line-or-search",
        "next-line-or-search",
        vec![LOOP],
    ));
    sh.settle();
    sh.send(UP);
    sh.wait_for("the loop", |s| {
        row_text(s, 2) == "done" && s.cursor_position() == (0, 2)
    });
}

/// With nothing before the cursor, a run of the prefix keys keeps walking
/// history, as readline's prefix search does while its text is empty: Up
/// on a walked-to entry does not search for the whole line, and Down past
/// the newest entry gives back the empty line.
#[test]
fn a_run_of_the_prefix_keys_from_an_empty_line_walks_history() {
    let mut sh = Shell::start(prefix_arrows(vec!["a1", "b2", "c3"]));
    sh.settle();
    for (key, row) in [
        (UP, "$ c3"),
        (UP, "$ b2"),
        (UP, "$ a1"),
        (DOWN, "$ b2"),
        (DOWN, "$ c3"),
        (DOWN, "$"),
    ] {
        sh.send(key);
        sh.wait_for(row, |s| cursor_row(s) == row);
    }
}

/// The empty-line walk goes on across a multi-line entry: Down on the
/// loop's first line moves through its lines and then on to `c3`.
#[test]
fn a_prefix_walk_from_an_empty_line_goes_across_a_multi_line_entry() {
    let mut sh = Shell::start(prefix_arrows(vec!["a1", LOOP, "c3"]));
    sh.settle();
    for (key, row) in [
        (UP, "$ c3"),
        (UP, "$ for x in a"),
        (DOWN, "do echo $x"),
        (DOWN, "done"),
        (DOWN, "$ c3"),
    ] {
        sh.send(key);
        sh.wait_for(row, |s| cursor_row(s) == row);
    }
}

/// Goes to the oldest of `history`'s entries with `M-<`, `C-n` `newer`
/// times, and puts the cursor after the entry's first word.
fn after_the_first_word_of_an_entry(sh: &mut Shell, newer: usize, entry: &str) {
    sh.settle();
    sh.send("\x1b<");
    sh.send(&"\x0e".repeat(newer));
    let row = format!("$ {entry}");
    sh.wait_for(entry, |s| cursor_row(s) == row);
    sh.send("\x01\x1bf");
    sh.wait_for("the cursor after the first word", |s| {
        s.cursor_position() == (0, 5)
    });
}

/// A prefix search that starts on an entry inside history goes through
/// history in order both ways, the entry it started from included, as
/// readline's does; Down past the newest match rings the bell and stays.
#[test]
fn a_prefix_search_from_inside_history_goes_through_it_in_order() {
    let mut sh = Shell::start(Options {
        rc: "bind 'set bell-style audible'\n".into(),
        ..prefix_arrows(vec!["git a", "ls", "git b", "pwd", "git c"])
    });
    after_the_first_word_of_an_entry(&mut sh, 2, "git b");
    for (key, row) in [
        (DOWN, "$ git c"),
        (UP, "$ git b"),
        (UP, "$ git a"),
        (DOWN, "$ git b"),
        (DOWN, "$ git c"),
    ] {
        sh.send(key);
        sh.wait_for(row, |s| {
            cursor_row(s) == row && s.cursor_position() == (0, 5)
        });
    }
    sh.take_output();
    sh.send(DOWN);
    sh.wait_for_output("the bell", b"\x07");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git c", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 5), "{}", dump(&s));
}

/// A prefix search that starts on an entry inside history finds an older
/// entry that repeats it, not `git a`, as readline's does, and Down then
/// goes back to the newer one. Up from there passes over the older `git b`
/// it found before, and Down goes through history in order again: each key
/// moves without the bell.
#[test]
fn a_prefix_search_from_inside_history_finds_a_repeat_of_its_start() {
    let mut sh = Shell::start(Options {
        rc: "bind 'set bell-style audible'\n".into(),
        ..prefix_arrows(vec!["git a", "git b", "ls", "git b", "pwd"])
    });
    after_the_first_word_of_an_entry(&mut sh, 3, "git b");
    for (key, row) in [
        (UP, "$ git b"),
        (DOWN, "$ git b"),
        (UP, "$ git a"),
        (DOWN, "$ git b"),
        (DOWN, "$ git b"),
    ] {
        sh.take_output();
        sh.send(key);
        let s = sh.settle();
        assert_eq!(cursor_row(&s), row, "{}", dump(&s));
        assert_eq!(s.cursor_position(), (0, 5), "{}", dump(&s));
        assert!(!sh.take_output().contains(&0x07), "a bell\n{}", dump(&s));
    }
    // On the newest `git b`, Down finds nothing newer.
    sh.send(DOWN);
    sh.wait_for_output("the bell", b"\x07");
}

/// On the entry a prefix search started from, which Down found again, the
/// substring key (`C-p`) starts a new search for the text before the
/// cursor, as on any line a search found: it finds `git a`, where walking
/// history would give `ls`.
#[test]
fn the_substring_key_searches_again_on_a_start_the_prefix_key_found_again() {
    let mut sh = Shell::start(prefix_arrows(vec!["git a", "git b", "ls", "git b", "pwd"]));
    after_the_first_word_of_an_entry(&mut sh, 3, "git b");
    sh.send(UP);
    sh.wait_for("the older git b", |s| {
        cursor_row(s) == "$ git b" && s.cursor_position() == (0, 5)
    });
    sh.send(DOWN);
    sh.wait_for("the newer git b", |s| {
        cursor_row(s) == "$ git b" && s.cursor_position() == (0, 5)
    });
    sh.send("\x10");
    sh.wait_for("the older match", |s| cursor_row(s) == "$ git a");
}

/// A prefix search that went back by a match to the multi-line entry it
/// started from keeps its run across a move between that entry's lines:
/// Up past the first line goes on with the search and passes over the
/// older `git b` it found before, to `git a`.
#[test]
fn a_prefix_run_back_on_its_start_goes_on_across_its_lines() {
    let mut sh = Shell::start(prefix_arrows(vec![
        "git a", "git b\nx", "ls", "git b\nx", "pwd",
    ]));
    sh.settle();
    sh.send("\x1b<");
    sh.send(&"\x0e".repeat(3));
    sh.wait_for("the second line of entry 3", |s| cursor_row(s) == "x");
    sh.send("\x01\x1b[A\x01\x1bf");
    sh.wait_for("the cursor after git", |s| {
        cursor_row(s) == "$ git b" && s.cursor_position() == (0, 5)
    });
    for (key, row) in [
        (UP, "$ git b"),
        (DOWN, "x"),
        (DOWN, "$ git b"),
        (DOWN, "x"),
        (UP, "$ git b"),
        (UP, "$ git a"),
    ] {
        sh.send(key);
        let s = sh.settle();
        assert_eq!(cursor_row(&s), row, "{}", dump(&s));
    }
}

/// A counted Down that finds fewer newer matches than the count goes to
/// the newest it finds, as readline's prefix search does.
#[test]
fn a_counted_prefix_down_goes_to_the_newest_match_it_finds() {
    let mut sh = Shell::start(prefix_arrows(vec!["git x", "ls", "git y", "pwd"]));
    after_the_first_word_of_an_entry(&mut sh, 0, "git x");
    sh.send("\x1b3");
    sh.send(DOWN);
    sh.wait_for("the newest match", |s| {
        cursor_row(s) == "$ git y" && s.cursor_position() == (0, 5)
    });
}

/// A prefix search that starts on an entry inside history finds newer
/// entries with Down: from `git status`, past `ls`, to `git log`.
#[test]
fn the_prefix_down_key_finds_a_newer_match_from_inside_history() {
    let mut sh = Shell::start(prefix_arrows(vec!["git status", "ls", "git log"]));
    sh.settle();
    sh.send(UP);
    sh.wait_for("the newest entry", |s| cursor_row(s) == "$ git log");
    sh.send("\x10");
    sh.wait_for("the entry before", |s| cursor_row(s) == "$ ls");
    sh.send("\x10");
    sh.wait_for("the oldest entry", |s| cursor_row(s) == "$ git status");
    sh.send("\x01\x1bf");
    sh.wait_for("the cursor after git", |s| s.cursor_position() == (0, 5));
    sh.send(DOWN);
    sh.wait_for("the newer match", |s| {
        cursor_row(s) == "$ git log" && s.cursor_position() == (0, 5)
    });
}

/// Down on a typed line finds nothing newer: the bell rings and the line
/// stays.
#[test]
fn the_prefix_down_key_on_a_typed_line_rings_the_bell() {
    let mut sh = Shell::start(Options {
        rc: "bind 'set bell-style audible'\n".into(),
        ..prefix_arrows(vec!["git log"])
    });
    sh.send("git");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    // C-g hides the menu and the grey text.
    sh.send("\x07");
    sh.wait_for("no menu", |s| {
        cursor_row(s) == "$ git" && row_text(s, 1).is_empty()
    });
    sh.take_output();
    sh.send(DOWN);
    sh.wait_for_output("the bell", b"\x07");
    let s = sh.settle();
    assert!(cursor_row(&s).starts_with("$ git"), "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 5), "{}", dump(&s));
}

/// A prefix search that finds nothing sets the mark at the end of the
/// line, as readline's does, so `C-x C-x` goes there.
#[test]
fn a_prefix_search_that_finds_nothing_sets_the_mark_at_the_line_end() {
    let mut sh = Shell::start(prefix_arrows(vec!["ls"]));
    sh.send("git status");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 12));
    sh.send("\x01\x1bf");
    sh.wait_for("the cursor after git", |s| s.cursor_position() == (0, 5));
    sh.send(UP);
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git status", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 5), "{}", dump(&s));
    sh.send("\x18\x18");
    sh.wait_for("the cursor at the mark", |s| s.cursor_position() == (0, 12));
}

/// With readline's `search-ignore-case` on, the prefix search ignores case,
/// as readline's own does.
#[test]
fn search_ignore_case_finds_prefix_matches_in_any_case() {
    let mut options = arrow_keys(
        "previous-line-or-search",
        "next-line-or-search",
        vec!["GIT status", "ls"],
    );
    options.rc.push_str("bind 'set search-ignore-case on'\n");
    let mut sh = Shell::start(options);
    sh.send("git");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.wait_for("no grey text", |s| cursor_row(s) == "$ git");
    sh.send(UP);
    sh.wait_for("the match", |s| {
        cursor_row(s) == "$ GIT status" && s.cursor_position() == (0, 5)
    });
}

/// With case ignored, the cursor goes after as many characters of the
/// match as the typed text has, so it never lands inside a character whose
/// lowercase form is shorter, such as the Kelvin sign.
#[test]
fn search_ignore_case_puts_the_cursor_after_whole_characters() {
    let mut options = arrow_keys(
        "previous-line-or-search",
        "next-line-or-search",
        vec!["\u{212A}ubectl get pods", "ls"],
    );
    options.rc.push_str("bind 'set search-ignore-case on'\n");
    let mut sh = Shell::start(options);
    sh.send("k");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 3));
    sh.send("\x07");
    sh.wait_for("no grey text", |s| cursor_row(s) == "$ k");
    sh.send(UP);
    sh.wait_for("the match", |s| {
        cursor_row(s) == "$ \u{212A}ubectl get pods" && s.cursor_position() == (0, 3)
    });
    sh.send("X");
    sh.wait_for("X after the Kelvin sign", |s| {
        cursor_row(s) == "$ \u{212A}Xubectl get pods"
    });
}

/// With readline's `search-ignore-case` on, the search ignores case, as
/// readline's own substring search does.
#[test]
fn search_ignore_case_finds_entries_in_any_case() {
    let mut options = substring_keys(vec!["git STATUS", "ls"]);
    options.rc.push_str("bind 'set search-ignore-case on'\n");
    let mut sh = Shell::start(options);
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the match", |s| cursor_row(s) == "$ git STATUS");
}

/// An entry that is not valid UTF-8 is searched too.
#[test]
fn up_finds_an_entry_that_is_not_utf8() {
    let mut options = substring_keys(vec!["ls"]);
    options
        .rc
        .push_str("history -s $'echo \\xff stat'\nhistory -s pwd\n");
    let mut sh = Shell::start(options);
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    let s = sh.settle();
    assert!(cursor_row(&s).starts_with("$ echo "), "{}", dump(&s));
    assert!(cursor_row(&s).ends_with(" stat"), "{}", dump(&s));
}
