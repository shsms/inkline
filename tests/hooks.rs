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

#[test]
fn a_line_start_insert_is_one_undo_step() {
    let mut sh = shell(r#"(add-hook 'inkline-line-start-functions (lambda () (insert "ls ")))"#);
    sh.wait_for("the insert", |s| cursor_row(s) == "$ ls");
    let s = sh.settle();
    assert_eq!(s.cursor_position().1, 5, "{}", dump(&s));
    sh.send("\x1f");
    sh.wait_for("undone", |s| cursor_row(s) == "$");
}

#[test]
fn line_start_functions_see_the_text_c_o_brings() {
    let mut sh = Shell::start(Options {
        init_el: Some(
            r#"(add-hook 'inkline-line-start-functions (lambda () (message "start:%s" (buffer-string))))"#
                .into(),
        ),
        history: vec!["echo one", "echo two"],
        ..Options::default()
    });
    sh.send(&format!("{UP}{UP}\x0f"));
    sh.wait_for("the next line", |s| has_row(s, "start:echo two"));
}

#[test]
fn a_looping_line_start_function_is_skipped_by_the_next_shell() {
    let home = tempfile::tempdir().unwrap();
    let opts = |init_el: Option<&str>| Options {
        home: Some(home.path().to_owned()),
        init_el: init_el.map(str::to_owned),
        ..Options::default()
    };
    let stuck = Shell::spawn(opts(Some(
        "(add-hook 'inkline-line-start-functions (lambda () (while t)))\n",
    )));
    let state = home.path().join(".local/state/inkline");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !std::fs::read_dir(&state)
        .into_iter()
        .flatten()
        .any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with("hooks.")))
    {
        assert!(std::time::Instant::now() < deadline, "no marker");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    drop(stuck);
    let sh = Shell::start(opts(None));
    let s = sh.settle();
    assert!(
        find(&s, "did not finish in an earlier shell").is_some(),
        "{}",
        dump(&s)
    );
}

#[test]
fn a_failing_line_start_function_is_undone_and_shown() {
    let sh = shell(
        r#"(add-hook 'inkline-line-start-functions (lambda () (insert "ls")))
           (add-hook 'inkline-line-start-functions (lambda () (insert "zz") (error "bad")) t)
           (add-hook 'inkline-line-start-functions (lambda () (user-error "no")) t)"#,
    );
    sh.wait_for("the message", |s| {
        has_row(s, "inkline: lambda: bad; inkline: lambda: no")
    });
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ ls", "{}", dump(&s));
}

/// The changes of all line-start functions are one undo step.
#[test]
fn two_line_start_inserts_are_one_undo_step() {
    let mut sh = shell(
        r#"(add-hook 'inkline-line-start-functions (lambda () (insert "ls ")))
           (add-hook 'inkline-line-start-functions (lambda () (insert "-l ")) t)"#,
    );
    sh.wait_for("both inserts", |s| cursor_row(s) == "$ ls -l");
    sh.send("\x1f");
    sh.wait_for("both undone", |s| cursor_row(s) == "$");
}

/// An internal error in a line-start function: the notice, then the line drawn
/// again below it.
#[cfg(debug_assertions)]
#[test]
fn a_panic_in_a_line_start_function_draws_the_line_below_the_notice() {
    let mut sh = shell(r#"(add-hook 'inkline-line-start-functions (lambda () (inkline--panic)))"#);
    sh.wait_for("the notice", |s| {
        has_row(s, "inkline: internal error, turned off")
    });
    let s = sh.settle();
    assert_eq!(
        rows(&s),
        ["$", "inkline: internal error, turned off", "$"],
        "{}",
        dump(&s)
    );
    assert_eq!(s.cursor_position(), (2, 2), "{}", dump(&s));
    sh.send("ab");
    sh.wait_for("typing on", |s| cursor_row(s) == "$ ab");
}
