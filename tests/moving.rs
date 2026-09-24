//! Moving around a command that spans several lines.

#[path = "support/common.rs"]
mod common;

use common::*;

const LOOP: &str = "for x in a\ndo echo $x\ndone";

/// Types `lines` joined by M-Enter and waits for the cursor on the last.
fn block(sh: &mut Shell, lines: &[&str]) {
    sh.send(&lines.join(ALT_ENTER));
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
        rc: "INKLINE_HISTORY_CURSOR=end\n".into(),
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
