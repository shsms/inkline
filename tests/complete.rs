//! Completion on the lines after the first of a multi-line command.

#[path = "support/common.rs"]
mod common;

use common::*;

#[test]
fn command_names_complete_on_a_later_line() {
    let mut sh = Shell::start(Options::default());
    sh.send("true");
    sh.send(ALT_ENTER);
    sh.send("histor\t");
    sh.wait_for("the completed name", |s| row_text(s, 1) == "history");
}

#[test]
fn programmable_completion_on_a_later_line() {
    let mut sh = Shell::start(Options {
        rc: "complete -W 'alpha beta' mycmd\n".into(),
        ..Options::default()
    });
    sh.send("true");
    sh.send(ALT_ENTER);
    sh.send("mycmd al\t");
    sh.wait_for("the completed word", |s| row_text(s, 1) == "mycmd alpha");
}
