#[path = "support/common.rs"]
mod common;

use common::*;

#[test]
fn keymap_global_set_binds_every_sequence_and_unset_puts_it_back() {
    let mut sh = Shell::start(Options {
        init_el: Some("(keymap-global-set \"C-x C-a\" 'beginning-of-line)\n".into()),
        ..Options::default()
    });
    sh.send("abc\x18\x01X");
    sh.wait_for("the binding", |s| cursor_row(s) == "$ Xabc");
    sh.send("\x15inkline eval '(keymap-global-unset \"C-x C-a\")'\r");
    sh.wait_for("the next prompt", |s| cursor_row(s) == "$");
    sh.send("abc\x18\x01X");
    // C-x C-a is unbound again: readline rings the bell and inserts nothing.
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ abcX");
}

#[test]
fn keymap_global_set_over_a_macro_puts_the_macro_back() {
    let mut sh = Shell::start(Options {
        rc: "bind '\"\\C-xm\": \"macro text\"'\ninkline eval \"(keymap-global-set \\\"C-x m\\\" 'beginning-of-line)\" >/dev/null\ninkline eval '(keymap-global-unset \"C-x m\")' >/dev/null\n".into(),
        ..Options::default()
    });
    sh.send("\x18m");
    sh.wait_for("the macro", |s| cursor_row(s) == "$ macro text");
}

#[test]
fn unknown_commands_and_keys_are_errors() {
    let out = bash_command()
        .arg("-c")
        .arg(format!(
            "enable -f {} inkline; inkline eval \"(keymap-global-set \\\"C-t\\\" 'no-such-command)\"; inkline eval \"(keymap-global-set \\\"M-<up>\\\" 'beginning-of-line)\"",
            so_path().display()
        ))
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("no-such-command: not a readline or inkline command"),
        "{err}"
    );
    assert!(err.contains("modifiers on <up> are not supported"), "{err}");
}

#[test]
fn inkline_keys_lists_bindings() {
    let mut sh = Shell::start(Options {
        init_el: Some("(keymap-global-set \"C-x C-a\" 'beginning-of-line)\n".into()),
        ..Options::default()
    });
    sh.send("inkline keys\r");
    sh.wait_for("the list", |s| {
        (0..s.size().0).any(|r| {
            row_text(s, r).starts_with("C-x C-a") && row_text(s, r).ends_with("beginning-of-line")
        })
    });
}

#[test]
fn unset_puts_back_a_bind_made_after_inkline_bound_the_key() {
    let mut sh = Shell::start(Options {
        init_el: Some("(keymap-global-set \"C-x C-a\" 'beginning-of-line)\n".into()),
        rc: "bind '\"\\C-x\\C-a\": end-of-line'\ninkline eval \"(keymap-global-set \\\"C-x C-a\\\" 'beginning-of-line)\" >/dev/null\ninkline eval '(keymap-global-unset \"C-x C-a\")' >/dev/null\n".into(),
        ..Options::default()
    });
    sh.send("abc\x01\x18\x01X");
    sh.wait_for("end-of-line", |s| cursor_row(s) == "$ abcX");
}

#[test]
fn keymap_global_set_refuses_a_key_after_one_that_runs_a_command() {
    let out = bash_command()
        .arg("-c")
        .arg(format!(
            "enable -f {} inkline; inkline eval '(keymap-global-set \"C-a C-b\" (quote end-of-line))'; echo rc=$?; bind -p | grep -F '\"\\C-a'",
            so_path().display()
        ))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "rc=1\n\"\\C-a\": beginning-of-line\n"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("C-a C-b: starts with a key that runs a command"),
        "{err}"
    );
}

/// `<home> x` stands for four sequences; with `\e[H` unbound the first is
/// allowed, but `\eOH x` starts with a key that runs a command, so none of the
/// four may be bound.
#[test]
fn keymap_global_set_binds_nothing_when_one_sequence_is_refused() {
    let out = bash_command()
        .arg("-c")
        .arg(format!(
            "bind -r '\\e[H' 2>/dev/null; enable -f {} inkline; inkline eval '(keymap-global-set \"<home> x\" (quote end-of-line))' 2>/dev/null; echo rc=$?; bind -p 2>/dev/null | grep -cF 'Hx'",
            so_path().display()
        ))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "rc=1\n0\n");
}
