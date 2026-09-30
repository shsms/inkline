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
}

#[test]
fn at_a_line_edge_the_kills_join_lines() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo b"]);
    sh.send("\x01\x15");
    sh.wait_for("joined by C-u", |s| row_text(s, 0) == "$ echo aecho b");
    sh.send("\x1f");
    sh.wait_for("undone", |s| row_text(s, 1) == "echo b");
    sh.send(UP);
    sh.send("\x05\x0b");
    sh.wait_for("joined by C-k", |s| row_text(s, 0) == "$ echo aecho b");
}

#[test]
fn up_can_search_history() {
    let mut sh = Shell::start(Options {
        rc: "bind '\"\\e[A\": previous-line-or-search'\n".into(),
        history: vec!["git status", "ls", "git log"],
        ..Options::default()
    });
    sh.send("git");
    sh.wait_for("the text", |s| cursor_row(s).starts_with("$ git"));
    sh.send(UP);
    sh.wait_for("the newest match", |s| row_text(s, 0) == "$ git log");
    sh.send(UP);
    sh.wait_for("the next match", |s| row_text(s, 0) == "$ git status");
}

/// A shell whose Up and Down run the substring search commands.
fn substring_keys(history: Vec<&'static str>) -> Options {
    Options {
        rc: "bind '\"\\e[A\": previous-line-or-substring-search'
bind '\"\\e[B\": next-line-or-substring-search'
"
        .into(),
        history,
        ..Options::default()
    }
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
