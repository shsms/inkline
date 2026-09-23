//! Each keystroke's output is one synchronized update (DEC private mode 2026),
//! so terminals that support it show a single frame per key instead of the
//! erased suggestion and readline's plain echo in between.

#[path = "support/common.rs"]
mod common;

use common::*;

const BEGIN: &[u8] = b"\x1b[?2026h";
const END: &[u8] = b"\x1b[?2026l";

#[test]
fn a_keystroke_is_one_update() {
    let mut sh = Shell::start(Options {
        history: vec!["git status"],
        ..Options::default()
    });
    sh.send("git st");
    sh.wait_for("the suggestion", |s| cursor_row(s) == "$ git status");
    sh.settle();
    sh.take_output();
    sh.send("a");
    sh.wait_for("the next suggestion", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 9)
    });
    sh.settle();
    let out = sh.take_output();
    let shown = String::from_utf8_lossy(&out).into_owned();
    assert!(out.starts_with(BEGIN), "{shown:?}");
    assert!(out.ends_with(END), "{shown:?}");
    assert_eq!(
        (count_bytes(&out, BEGIN), count_bytes(&out, END)),
        (1, 1),
        "{shown:?}"
    );
}

#[test]
fn the_update_ends_before_the_command_runs() {
    let mut sh = Shell::start(Options {
        history: vec!["echo hello-world"],
        ..Options::default()
    });
    sh.send("echo hel");
    sh.wait_for("the suggestion", |s| cursor_row(s) == "$ echo hello-world");
    sh.settle();
    sh.take_output();
    sh.send("\r");
    sh.wait_for("the output", |s| has_row(s, "hel"));
    sh.settle();
    let out = sh.take_output();
    let shown = String::from_utf8_lossy(&out).into_owned();
    let end = find_bytes(&out, END).expect(&shown);
    let output = find_bytes(&out, b"hel\r\n").expect(&shown);
    assert!(end < output, "{shown:?}");
    assert_eq!(
        count_bytes(&out, BEGIN),
        count_bytes(&out, END),
        "{shown:?}"
    );
}

/// A `bind -x` command runs in the middle of readline's key handling. Its
/// output must not be held back until the key is done, however long it runs.
#[test]
fn bind_x_output_is_not_held_back() {
    let mut sh = Shell::start(Options {
        rc: "bind -x '\"\\C-t\": echo bound-marker'\n".into(),
        ..Options::default()
    });
    sh.send("ls");
    sh.wait_for("the line", |s| fg_is(s, "ls", Color::Idx(2)));
    sh.settle();
    sh.take_output();
    sh.send("\x14");
    sh.wait_for("the output", |s| has_row(s, "bound-marker"));
    sh.settle();
    assert_no_update_open_at(&sh.take_output(), b"bound-marker\r\n");
}

/// Asserts that every update opened in `out` before `marker` was closed.
fn assert_no_update_open_at(out: &[u8], marker: &[u8]) {
    let shown = String::from_utf8_lossy(out);
    let before = &out[..find_bytes(out, marker).expect(&shown)];
    assert_eq!(
        count_bytes(before, BEGIN),
        count_bytes(before, END),
        "{shown:?}"
    );
}

/// A `WINCH` trap runs while readline handles a resize. Its output must not be
/// held back.
#[test]
fn winch_trap_output_is_not_held_back() {
    let mut sh = Shell::start(Options {
        rc: "trap 'echo winch-trap' WINCH\n".into(),
        ..Options::default()
    });
    sh.send("ls");
    sh.wait_for("the line", |s| fg_is(s, "ls", Color::Idx(2)));
    sh.settle();
    sh.take_output();
    sh.resize(24, 60);
    sh.wait_for_output("the trap", b"winch-trap");
    sh.settle();
    assert_no_update_open_at(&sh.take_output(), b"winch-trap");
}
