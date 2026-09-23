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

#[test]
fn read_with_a_timeout_times_out() {
    let mut sh = Shell::start(Options::default());
    sh.send("read -e -t 1 x; echo \"status $?\"\r");
    sh.wait_for("the timeout", |s| has_row(s, "status 142"));
}

/// Readline stops reading when SIGTERM arrives; bash 5.0 then exits as at the
/// end of input, later versions carry on. inkline's wait for a key must not
/// change that.
#[test]
fn sigterm_while_waiting_acts_as_in_plain_bash() {
    let exits = |inkline| {
        let mut sh = Shell::start(Options {
            inkline,
            ..Options::default()
        });
        sh.send("echo hi");
        sh.wait_for("the line", |s| cursor_row(s) == "$ echo hi");
        sh.settle();
        sh.signal(libc::SIGTERM);
        sh.exits()
    };
    assert_eq!(exits(true), exits(false));
}
