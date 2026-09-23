#[path = "support/common.rs"]
mod common;

use common::*;

#[test]
fn interactive_bash_runs_commands_with_inkline_loaded() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo hi\r");
    sh.wait_for("the output", |s| has_row(s, "hi"));
    sh.send("inkline status\r");
    sh.wait_for("the status", |s| has_row(s, "inkline: on"));
}
