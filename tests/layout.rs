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

#[test]
fn the_layout_is_bound_on_load() {
    let mut sh = Shell::start(Options {
        history: vec!["git status"],
        ..Options::default()
    });
    sh.send("git st\x05");
    sh.wait_for("C-e took the suggestion", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position().1 == 12
    });
    sh.send("\x15echo (");
    sh.wait_for("pairing", |s| cursor_row(s) == "$ echo ()");
}

#[test]
fn layout_leaves_inputrc_keys_alone() {
    let mut sh = Shell::start(Options {
        inputrc: Some("\"\\C-k\": transpose-chars\n\"\\e[1~\": beginning-of-line\n".into()),
        ..Options::default()
    });
    sh.send("ab\x0b");
    sh.wait_for("C-k transposes", |s| cursor_row(s) == "$ ba");
    sh.send("\x15inkline keys\r");
    sh.wait_for("the left-alone line", |s| {
        (0..s.size().0).any(|r| {
            row_text(s, r).starts_with("C-k (\\C-k)")
                && row_text(s, r)
                    .contains("left alone for kill-to-line-end: bound to transpose-chars")
        })
    });
}

#[test]
fn layout_leaves_inputrc_keys_after_an_earlier_bind() {
    let mut sh = Shell::start(Options {
        inputrc: Some("\"\\C-k\": transpose-chars\n".into()),
        before_inkline: "bind 'set completion-ignore-case on'\n".into(),
        ..Options::default()
    });
    sh.send("ab\x0b");
    sh.wait_for("C-k transposes", |s| cursor_row(s) == "$ ba");
}

#[test]
fn history_search_on_up_becomes_line_or_search() {
    let mut sh = Shell::start(Options {
        inputrc: Some("\"\\e[A\": history-search-backward\n".into()),
        history: vec!["echo one", "ls two", "echo two"],
        ..Options::default()
    });
    // The suggestion is "echo two"; the second Up skips "ls two".
    sh.send("ec\x1b[A\x1b[A");
    sh.wait_for("the search", |s| cursor_row(s) == "$ echo one");
    // The search leaves the cursor after "ec": clear the whole line.
    sh.send("\x01\x0binkline keys\r");
    sh.wait_for("the binding", |s| {
        (0..s.size().0).any(|r| {
            row_text(s, r).starts_with("<up>")
                && row_text(s, r).ends_with("previous-line-or-search")
        })
    });
}

#[test]
fn unbind_defaults_gives_back_a_group() {
    let mut sh = Shell::start(Options {
        init_el: Some("(inkline-unbind-defaults '(pairing))\n".into()),
        ..Options::default()
    });
    sh.send("echo (");
    sh.wait_for("no pair", |s| cursor_row(s) == "$ echo (");
}

#[test]
fn a_later_bind_wins_over_the_layout() {
    let mut sh = Shell::start(Options {
        rc: "bind '\"\\C-k\": kill-line'\n".into(),
        ..Options::default()
    });
    // On a two-line command, kill-line takes the rest of the command, and
    // inkline's kill-to-line-end only the rest of the first line.
    sh.send(&format!("ab{CTRL_J}cd\x10\x01\x06\x0b"));
    sh.wait_for("kill-line", |s| {
        row_text(s, 0) == "$ a" && !has_row(s, "cd")
    });
}

#[test]
fn unbind_defaults_takes_a_single_group() {
    let mut sh = Shell::start(Options {
        init_el: Some("(inkline-unbind-defaults 'pairing)\n".into()),
        ..Options::default()
    });
    sh.send("echo (");
    sh.wait_for("no pair", |s| cursor_row(s) == "$ echo (");
}

#[test]
fn layout_takes_a_home_key_bound_to_beginning_of_line_in_inputrc() {
    let mut sh = Shell::start(Options {
        inputrc: Some("\"\\e[1~\": beginning-of-line\n".into()),
        ..Options::default()
    });
    // line-start goes to the start of the second line, not of the command.
    sh.send(&format!("ab{CTRL_J}cd\x1b[1~X"));
    sh.wait_for("line-start", |s| {
        row_text(s, 0) == "$ ab" && row_text(s, 1).ends_with("Xcd")
    });
}

#[test]
fn unbind_defaults_rejects_a_dotted_list() {
    let out = bash_command()
        .arg("-c")
        .arg(format!(
            "enable -f {} inkline; inkline eval \"(inkline-unbind-defaults '(pairing . x))\"; echo rc=$?",
            so_path().display()
        ))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "rc=1\n");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("(pairing . x): not a list of layout groups"),
        "{err}"
    );
}

#[test]
fn unbind_defaults_rejects_a_circular_list() {
    let out = bash_command()
        .arg("-c")
        .arg(format!(
            "enable -f {} inkline; inkline eval \"(let ((l (list 'pairing))) (setcdr l l) (inkline-unbind-defaults l))\"; echo rc=$?",
            so_path().display()
        ))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "rc=1\n");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(": not a list of layout groups"), "{err}");
}

#[test]
fn a_terminal_key_bound_after_unbind_defaults_keeps_its_binding() {
    for (key, byte) in [
        ("C-u", "\x15"),
        ("C-v", "\x16"),
        ("C-w", "\x17"),
        ("DEL", "\x7f"),
    ] {
        let mut sh = Shell::start(Options {
            init_el: Some(format!(
                "(inkline-unbind-defaults)\n(keymap-global-set \"{key}\" 'beginning-of-line)\n"
            )),
            ..Options::default()
        });
        sh.send(&format!("abc{byte}X"));
        sh.wait_for(key, |s| cursor_row(s) == "$ Xabc");
    }
}

#[test]
fn a_terminal_key_bound_before_unbind_defaults_keeps_its_binding() {
    let mut sh = Shell::start(Options {
        init_el: Some(
            "(keymap-global-set \"C-w\" 'beginning-of-line)\n(inkline-unbind-defaults)\n".into(),
        ),
        ..Options::default()
    });
    sh.send("abc\x17X");
    sh.wait_for("C-w", |s| cursor_row(s) == "$ Xabc");
}

#[test]
fn unbind_defaults_leaves_the_other_groups() {
    let mut sh = Shell::start(Options {
        init_el: Some("(inkline-unbind-defaults '(pairing))\n".into()),
        ..Options::default()
    });
    sh.send("bind -q accept-suggestion\r");
    sh.wait_for("still bound", |s| {
        (0..s.size().0).any(|r| row_text(s, r).starts_with("accept-suggestion can be invoked via"))
    });
}

#[test]
fn unbind_defaults_with_no_groups_gives_back_every_key() {
    let mut sh = Shell::start(Options {
        init_el: Some("(inkline-unbind-defaults)\n".into()),
        ..Options::default()
    });
    sh.send("echo bound=$(inkline keys | grep -vc 'left alone'); bind -v | grep tty-special\r");
    sh.wait_for("nothing bound", |s| {
        has_row(s, "bound=0") && has_row(s, "set bind-tty-special-chars on")
    });
}

#[test]
fn unset_after_binding_a_layout_key_again_puts_back_readline_s_binding() {
    let mut sh = Shell::start(Options {
        init_el: Some(
            "(keymap-global-set \"C-k\" 'transpose-chars)\n(keymap-global-unset \"C-k\")\n".into(),
        ),
        ..Options::default()
    });
    sh.send("bind -q kill-line\r");
    sh.wait_for("kill-line", |s| {
        has_row(s, "kill-line can be invoked via \"\\C-k\".")
    });
}
