#[path = "support/common.rs"]
mod common;

use common::*;

#[test]
fn loads_and_switches_on_and_off() {
    let script = format!(
        "enable -f {} inkline && inkline status && inkline off && inkline && inkline on && inkline status; inkline bogus; echo rc=$?",
        so_path().display()
    );
    let out = bash_command().arg("-c").arg(script).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "inkline: on\ninit.el: not read (not an interactive shell with line editing on)\n\
         inkline: off\ninit.el: not read (not an interactive shell with line editing on)\n\
         inkline: on\ninit.el: not read (not an interactive shell with line editing on)\nrc=2\n"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: inkline [on|off|status|"));
}

#[test]
fn write_errors_do_not_kill_the_shell() {
    let script = format!(
        "enable -f {} inkline; inkline status > /dev/full; echo status=$?; inkline bogus 2> /dev/full; echo bogus=$?",
        so_path().display()
    );
    let out = bash_command().arg("-c").arg(script).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "status=1\nbogus=2\n");
    assert!(out.status.success(), "bash exited with {:?}", out.status);
}
