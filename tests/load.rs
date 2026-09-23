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
        "inkline: on\ninkline: off\ninkline: on\nrc=2\n"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: inkline [on|off|status]"));
}
