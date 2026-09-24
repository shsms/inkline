#[path = "support/common.rs"]
mod common;

use common::*;

#[test]
fn bound_keys_keep_working_after_enable_d() {
    let mut sh = Shell::start(Options {
        history: vec!["git status"],
        ..Options::default()
    });
    sh.send("enable -d inkline\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 == 1 && cursor_row(s) == "$"
    });
    sh.send("git st");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git st");
    assert_eq!(fg(&s, "git"), Color::Default);
    // C-e and C-f are still bound to inkline's commands, which now only run
    // readline's own. Without the library staying loaded, bash crashes here.
    sh.send("\x05\x01\x06X");
    sh.wait_for("plain editing", |s| cursor_row(s) == "$ gXit st");
}

#[test]
fn loads_again_after_enable_d() {
    let mut sh = Shell::start(Options::default());
    sh.send(&format!(
        "enable -d inkline; enable -f {} inkline; inkline status\r",
        so_path().display()
    ));
    sh.wait_for("the status", |s| has_row(s, "inkline: on"));
    sh.send("ls");
    sh.wait_for("colours", |s| {
        cursor_row(s) == "$ ls" && fg_is(s, "ls", Color::Idx(2))
    });
}

/// Readline keeps pointers to inkline's commands after `enable -d`, so the
/// library must stay mapped even when nothing else happens to hold it.
#[test]
fn library_stays_loaded_after_enable_d() {
    let script = format!(
        // The trailing `:` keeps bash from exec-ing grep in its own place.
        "enable -f {} inkline; enable -d inkline; grep -c libinkline /proc/$$/maps; :",
        so_path().display()
    );
    let out = bash_command().arg("-c").arg(script).output().unwrap();
    let mappings: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    assert!(mappings > 0, "libinkline.so was unmapped by enable -d");
}

#[test]
fn a_second_enable_f_only_switches_on() {
    let mut sh = Shell::start(Options {
        init_el: Some("(message-not-defined-yet)\n".into()),
        ..Options::default()
    });
    sh.take_output();
    sh.send(&format!(
        "enable -d inkline; enable -f {} inkline; echo done\r",
        so_path().display()
    ));
    sh.wait_for("done", |s| has_row(s, "done"));
    let out = String::from_utf8_lossy(&sh.take_output()).into_owned();
    assert!(!out.contains("init.el:1"), "init.el was read again: {out}");
}

#[test]
fn enter_is_accept_line_after_enable_d() {
    let mut sh = Shell::start(Options::default());
    sh.send("enable -d inkline\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 == 1 && cursor_row(s) == "$"
    });
    sh.send("for x in a; do\r");
    sh.wait_for("the continuation prompt", |s| cursor_row(s) == ">");
}
