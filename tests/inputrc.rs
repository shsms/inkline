#[path = "support/common.rs"]
mod common;

use common::*;

const INPUTRC: &str = "\"\\C-xa\": accept-suggestion\n";

#[test]
fn inputrc_bindings_work_when_inkline_loads_first() {
    // The harness's rc runs `enable -f` before any `bind`, and bash reads
    // INPUTRC on the first `bind`.
    let mut sh = Shell::start(Options {
        history: vec!["git status"],
        inputrc: Some(INPUTRC.into()),
        ..Options::default()
    });
    sh.send("git st");
    sh.wait_for("the suggestion", |s| cursor_row(s) == "$ git status");
    sh.send("\x18a\x02");
    sh.wait_for("the accepted line", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 11)
    });
}

#[test]
fn an_unknown_command_in_inputrc_unbinds_the_key() {
    let inputrc = "\"\\C-e\": accept-suggestion\n";
    let mut sh = Shell::start(Options {
        inkline: false,
        inputrc: Some(inputrc.into()),
        ..Options::default()
    });
    sh.send("abc\x01\x05X");
    assert_eq!(cursor_row(&sh.settle()), "$ Xabc");
}
