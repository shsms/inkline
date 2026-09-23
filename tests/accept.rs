#[path = "support/common.rs"]
mod common;

use common::*;

fn ready() -> Shell {
    let mut sh = Shell::start(Options {
        history: vec!["git status --short"],
        ..Options::default()
    });
    sh.send("git st");
    sh.wait_for("the suggestion", |s| {
        cursor_row(s) == "$ git status --short"
    });
    sh
}

/// Sends `keys`, then C-b so the suggestion hides, and returns the line.
fn line_after(keys: &str) -> String {
    let mut sh = ready();
    sh.send(keys);
    sh.settle();
    sh.send("\x02");
    cursor_row(&sh.settle())
}

#[test]
fn ctrl_f_takes_one_character() {
    assert_eq!(line_after("\x06"), "$ git sta");
}

#[test]
fn right_arrow_takes_one_character() {
    assert_eq!(line_after("\x1b[C"), "$ git sta");
}

#[test]
fn a_count_takes_several_characters() {
    assert_eq!(line_after("\x1b3\x06"), "$ git statu");
}

#[test]
fn meta_f_takes_a_word() {
    assert_eq!(line_after("\x1bf"), "$ git status");
}

#[test]
fn a_count_takes_several_words() {
    assert_eq!(line_after("\x1b2\x1bf"), "$ git status --short");
}

#[test]
fn ctrl_e_and_end_take_everything() {
    assert_eq!(line_after("\x05"), "$ git status --short");
    assert_eq!(line_after("\x1b[F"), "$ git status --short");
}

#[test]
fn undo_removes_accepted_text() {
    assert_eq!(line_after("\x05\x1f"), "$ git st");
}

#[test]
fn keys_move_as_usual_without_a_suggestion() {
    for (keys, want) in [
        ("\x01\x06X", "$ gXit st"),
        ("\x01\x1bfX", "$ gitX st"),
        ("\x01\x05X", "$ git stX"),
    ] {
        let mut sh = ready();
        sh.send(keys);
        assert_eq!(cursor_row(&sh.settle()), want, "keys {keys:?}");
    }
}
