#[path = "support/common.rs"]
mod common;

use common::*;

fn shell(init_el: &str) -> Shell {
    Shell::start(Options {
        init_el: Some(init_el.into()),
        ..Options::default()
    })
}

/// Rows of the screen, top to bottom, without trailing blank rows.
fn rows(s: &vt100::Screen) -> Vec<String> {
    let mut rows: Vec<String> = (0..s.size().0).map(|r| row_text(s, r)).collect();
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
}

#[test]
fn a_refused_line_stays_with_the_message() {
    let mut sh = shell(
        r#"(add-hook 'inkline-accept-functions
             (lambda () (when (string-search "rm" (buffer-string)) (user-error "no rm here"))))"#,
    );
    sh.send("rm x\r");
    sh.wait_for("the refusal", |s| has_row(s, "no rm here"));
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ rm x", "{}", dump(&s));
    sh.send("\x15echo ok\r");
    sh.wait_for("the line ran", |s| has_row(s, "ok"));
}

#[test]
fn a_failing_accept_function_is_printed_above_the_output_and_undone() {
    let mut sh = shell(
        r#"(add-hook 'inkline-accept-functions (lambda () (goto-char (point-max)) (insert "zz") (error "bad")))"#,
    );
    sh.send("echo hi\r");
    sh.wait_for("the output", |s| has_row(s, "hi"));
    let s = sh.settle();
    assert_eq!(
        rows(&s)[..3],
        ["$ echo hi", "inkline: lambda: bad", "hi"],
        "{}",
        dump(&s)
    );
}

#[test]
fn a_changed_line_is_redrawn_run_and_kept_in_history() {
    let mut sh = shell(
        r#"(add-hook 'inkline-accept-functions
             (lambda () (when (equal (buffer-string) "x") (erase-buffer) (insert "echo expanded"))))"#,
    );
    sh.send("x\r");
    sh.wait_for("the output", |s| has_row(s, "expanded"));
    let s = sh.settle();
    assert_eq!(
        rows(&s)[..2],
        ["$ echo expanded", "expanded"],
        "{}",
        dump(&s)
    );
    sh.send(UP);
    sh.wait_for("history", |s| cursor_row(s) == "$ echo expanded");
}

#[test]
fn m_ret_and_an_empty_line_run_the_hook() {
    let mut sh = shell(
        r#"(add-hook 'inkline-accept-functions
             (lambda () (if (equal (buffer-string) "") (insert "echo empty")
                          (goto-char (point-max)) (insert " M"))))"#,
    );
    sh.send("\r");
    sh.wait_for("the empty line ran", |s| has_row(s, "empty"));
    sh.send(&format!("echo a{ALT_ENTER}"));
    sh.wait_for("M-RET ran the hook", |s| has_row(s, "a M"));
}

#[test]
fn no_accept_hook_at_read_e_or_when_off() {
    let mut sh = shell(
        r#"(add-hook 'inkline-accept-functions (lambda () (when (equal (buffer-string) "x") (insert "Z"))))"#,
    );
    sh.send("read -e v; echo \"[$v]\"\r");
    sh.wait_for("read -e", |s| {
        has_row(s, "$ read -e v; echo \"[$v]\"") && cursor_row(s).is_empty()
    });
    sh.send("x\r");
    sh.wait_for("read -e got x", |s| has_row(s, "[x]"));
    sh.send("inkline off\r");
    sh.send("x\r");
    sh.wait_for("off", |s| find(s, "x: command not found").is_some());
    let s = sh.settle();
    assert!(find(&s, "xZ").is_none(), "{}", dump(&s));
}

#[test]
fn c_c_in_an_accept_question_gives_a_new_prompt() {
    let mut sh = shell(
        r#"(add-hook 'inkline-accept-functions (lambda () (unless (y-or-n-p "Run? ") (user-error "not run"))))"#,
    );
    sh.send("echo one\r");
    sh.wait_for("the question", |s| has_row(s, "Run? (y or n)"));
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    let s = sh.settle();
    assert!(!has_row(&s, "one"), "{}", dump(&s));
    sh.send("echo two\r");
    sh.wait_for("the question again", |s| {
        has_row(s, "$ echo two") && has_row(s, "Run? (y or n)")
    });
    sh.send("y");
    sh.wait_for("the line ran", |s| has_row(s, "two"));
}

#[test]
fn an_undo_in_an_accept_function_is_an_error() {
    let mut sh =
        shell(r#"(add-hook 'inkline-accept-functions (lambda () (call-interactively 'undo)))"#);
    sh.send("echo hi\r");
    sh.wait_for("the output", |s| has_row(s, "hi"));
    let s = sh.settle();
    assert_eq!(
        rows(&s)[..3],
        [
            "$ echo hi",
            "inkline: lambda: undo cannot run in a hook",
            "hi"
        ],
        "{}",
        dump(&s)
    );
}

#[test]
fn no_accept_hook_on_m_ret_while_a_c_c_waits() {
    let mut sh = shell(
        r#"(defvar runs 0)
           (add-hook 'inkline-accept-functions (lambda () (setq runs (1+ runs))))"#,
    );
    sh.send("sleep 0");
    sh.wait_for("the typing", |s| cursor_row(s) == "$ sleep 0");
    sh.send(&format!("\x03echo hi{ALT_ENTER}"));
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    // The hook runs for this line, so a run for the thrown-away one makes 2.
    sh.send("inkline eval runs\r");
    sh.wait_for("the count", |s| {
        cursor_row(s) == "$" && s.cursor_position().0 > 1
    });
    let s = sh.settle();
    assert!(has_row(&s, "1"), "{}", dump(&s));
    assert!(!has_row(&s, "2"), "{}", dump(&s));
}

/// An accept function's shell code can turn inkline off. The errors of that run
/// then never show, not even under a later line read once inkline is on again.
#[test]
fn accept_errors_are_dropped_when_a_function_turns_inkline_off() {
    let mut sh = Shell::start(Options {
        init_el: Some(
            r#"(add-hook 'inkline-accept-functions
                 (lambda () (when (equal (buffer-string) "zz ") (call-interactively 'complete))))
               (add-hook 'inkline-accept-functions
                 (lambda () (when (equal (buffer-string) "zz foo ") (error "bad"))) t)"#
                .into(),
        ),
        rc: "f() { inkline off; COMPREPLY=(foo); }; complete -F f zz\n".into(),
        ..Options::default()
    });
    sh.send("zz \r");
    sh.wait_for("the line run", |s| {
        s.contents().contains("zz: command not found") && cursor_row(s) == "$"
    });
    sh.send("inkline on; read -e x\r");
    sh.wait_for("read -e", |s| cursor_row(s).is_empty());
    sh.send("y\r");
    sh.wait_for("the next prompt", |s| {
        cursor_row(s) == "$" && s.cursor_position().0 > 2
    });
    let s = sh.settle();
    assert!(!s.contents().contains("bad"), "{}", dump(&s));
}

/// An internal error in an accept function: the notice, and the line runs as
/// typed, without the function's change.
#[cfg(debug_assertions)]
#[test]
fn a_panic_in_an_accept_function_runs_the_line_as_typed() {
    let mut sh = shell(
        r#"(add-hook 'inkline-accept-functions (lambda () (goto-char (point-max)) (insert "zz") (inkline--panic)))"#,
    );
    sh.send("echo hi\r");
    sh.wait_for("the output", |s| has_row(s, "hi") && cursor_row(s) == "$");
    let s = sh.settle();
    assert!(
        has_row(&s, "inkline: internal error, turned off"),
        "{}",
        dump(&s)
    );
    assert!(!s.contents().contains("hizz"), "{}", dump(&s));
    sh.send("echo ok\r");
    sh.wait_for("the next line run", |s| {
        has_row(s, "ok") && cursor_row(s) == "$"
    });
}
