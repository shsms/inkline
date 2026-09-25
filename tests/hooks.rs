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

/// `y-or-n-p` works only in a command and in the accept hook.
#[test]
fn y_or_n_p_is_an_error_in_line_start_and_after_change_functions() {
    const ERROR: &str = "y-or-n-p works only in a command or in inkline-accept-functions";
    let mut sh = Shell::start(Options {
        cols: 160,
        init_el: Some(
            r#"(add-hook 'inkline-line-start-functions (lambda () (insert "a") (y-or-n-p "Start? ")))
               (add-hook 'inkline-after-change-functions (lambda (_b _e _l) (y-or-n-p "Change? ")))"#
                .into(),
        ),
        ..Options::default()
    });
    sh.wait_for("the line-start error", |s| {
        has_row(s, &format!("inkline: lambda: {ERROR}"))
    });
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$", "{}", dump(&s));
    sh.send("b");
    sh.wait_for("the after-change removal", |s| {
        has_row(
            s,
            &format!("inkline: lambda: {ERROR} (removed from inkline-after-change-functions)"),
        )
    });
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ b", "{}", dump(&s));
    assert!(!s.contents().contains("? (y or n)"), "{}", dump(&s));
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

const SHOW_CHANGES: &str = r#"(add-hook 'inkline-after-change-functions
  (lambda (beg end len) (message "%s %s %s %s %s" this-command last-command beg end len)))"#;

#[test]
fn after_change_gets_the_part_and_the_commands() {
    let mut sh = Shell::start(Options {
        init_el: Some(SHOW_CHANGES.into()),
        history: vec!["echo old"],
        ..Options::default()
    });
    sh.send("a");
    sh.wait_for("typing", |s| has_row(s, "self-insert nil 1 2 0"));
    sh.send("b");
    sh.wait_for("typing", |s| has_row(s, "self-insert self-insert 2 3 0"));
    // The default layout binds DEL to `delete-pair` and Up to
    // `previous-line-or-history`.
    sh.send("\x7f");
    sh.wait_for("deleting", |s| has_row(s, "delete-pair self-insert 2 2 1"));
    sh.send(UP);
    sh.wait_for("history", |s| {
        has_row(s, "previous-line-or-history delete-pair 1 9 1")
    });
}

#[test]
fn an_abbreviation_is_one_undo_step_after_the_typing() {
    let mut sh = shell(
        r#"(add-hook 'inkline-after-change-functions
             (lambda (_b _e _l) (when (and (eq this-command 'self-insert) (equal (buffer-string) "gco ")) (erase-buffer) (insert "git checkout "))))"#,
    );
    sh.send("gco ");
    sh.wait_for("expanded", |s| cursor_row(s) == "$ git checkout");
    sh.send("\x1f");
    sh.wait_for("the expansion undone", |s| cursor_row(s) == "$ gco");
    let s = sh.settle();
    assert_eq!(s.cursor_position().1, 6, "{}", dump(&s));
}

#[test]
fn a_hook_change_does_not_run_the_hook_again() {
    let mut sh = shell(
        r#"(defvar calls 0)
           (add-hook 'inkline-after-change-functions
             (lambda (_b _e _l) (setq calls (1+ calls))
               (when (equal (buffer-string) "x") (insert "y"))
               (message "calls %d" calls)))"#,
    );
    sh.send("x");
    sh.wait_for("one call", |s| has_row(s, "calls 1"));
    sh.send("z");
    sh.wait_for("two calls", |s| has_row(s, "calls 2"));
    assert_eq!(cursor_row(&sh.settle()), "$ xyz");
}

#[test]
fn a_failing_after_change_function_is_removed() {
    let mut sh = shell(
        r#"(add-hook 'inkline-after-change-functions (lambda (_b _e _l) (insert "q") (error "ac")))"#,
    );
    sh.send("a");
    sh.wait_for("the removal", |s| {
        has_row(
            s,
            "inkline: lambda: ac (removed from inkline-after-change-functions)",
        )
    });
    sh.send("b");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ ab", "{}", dump(&s));
    assert!(!has_row(
        &s,
        "inkline: lambda: ac (removed from inkline-after-change-functions)"
    ));
}

#[test]
fn no_after_change_at_read_e_or_while_searching() {
    let mut sh = Shell::start(Options {
        init_el: Some(SHOW_CHANGES.into()),
        history: vec!["echo abc"],
        ..Options::default()
    });
    sh.send("\x12ab");
    sh.wait_for("the search", |s| cursor_row(s).contains("echo abc"));
    let s = sh.settle();
    assert!(
        !(0..s.size().0).any(|r| row_text(&s, r).starts_with("self-insert")),
        "{}",
        dump(&s)
    );
    sh.send("\x07\x15read -e v\r");
    sh.send("q");
    let s = sh.settle();
    assert!(
        !(0..s.size().0).any(|r| row_text(&s, r).starts_with("self-insert")),
        "{}",
        dump(&s)
    );
}

/// Shell code that fails under an after-change function (`shell-expand-line` on
/// `${zz:?boom}`): bash's error goes after the line, as with readline's own
/// `shell-expand-line`, the new prompt comes below it, and the hook runs again
/// only once the new line changes.
#[test]
fn a_shell_error_in_an_after_change_function_gives_a_new_prompt() {
    let mut sh = shell(
        r#"(add-hook 'inkline-after-change-functions
             (lambda (_b _e _l)
               (when (equal (buffer-string) "echo ${zz:?boom}!")
                 (call-interactively 'shell-expand-line))))
           (add-hook 'inkline-after-change-functions
             (lambda (b e l) (message "ac %s %s %s [%s]" b e l (buffer-string))) t)"#,
    );
    sh.send("echo ${zz:?boom}");
    sh.wait_for("the typing", |s| cursor_row(s) == "$ echo ${zz:?boom}");
    sh.send("!");
    sh.wait_for("a new prompt", |s| {
        cursor_row(s) == "$" && (0..s.size().0).any(|r| row_text(s, r).ends_with("zz: boom"))
    });
    let s = sh.settle();
    assert_eq!(
        rows(&s),
        ["$ echo ${zz:?boom}bash: zz: boom", "$"],
        "{}",
        dump(&s)
    );
    sh.send("x");
    sh.wait_for("the hook on the new line", |s| has_row(s, "ac 1 2 0 [x]"));
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

/// An internal error in an after-change function: the notice, then the line
/// drawn again below it.
#[cfg(debug_assertions)]
#[test]
fn a_panic_in_an_after_change_function_draws_the_line_below_the_notice() {
    let mut sh =
        shell(r#"(add-hook 'inkline-after-change-functions (lambda (_b _e _l) (inkline--panic)))"#);
    sh.send("echo a");
    sh.wait_for("the notice", |s| {
        has_row(s, "inkline: internal error, turned off")
    });
    let s = sh.settle();
    assert_eq!(
        rows(&s),
        ["$", "inkline: internal error, turned off", "$ echo a"],
        "{}",
        dump(&s)
    );
    assert_eq!(s.cursor_position(), (2, 8), "{}", dump(&s));
    sh.send("b");
    sh.wait_for("typing on", |s| cursor_row(s) == "$ echo ab");
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

/// Shell code that an after-change function runs can turn inkline off. Once
/// `inkline on` runs in the middle of a later line, the hook waits for the next
/// line: it does not compare with the line it saw before inkline went off.
#[test]
fn inkline_on_mid_line_does_not_run_after_change_on_an_old_line() {
    let mut sh = Shell::start(Options {
        init_el: Some(
            r#"(add-hook 'inkline-after-change-functions
                 (lambda (b e l)
                   (if (equal (buffer-string) "zz ")
                       (call-interactively 'complete)
                     (message "AC %s %s %s" b e l))))"#
                .into(),
        ),
        rc: r#"f() { inkline off; COMPREPLY=(foo); }; complete -F f zz
bind -x '"\C-xo": inkline on'
"#
        .into(),
        ..Options::default()
    });
    sh.send("zz");
    sh.wait_for("the typing", |s| cursor_row(s) == "$ zz");
    sh.send(" ");
    sh.wait_for("the completion", |s| cursor_row(s) == "$ zz foo");
    sh.send("a\x18o");
    sh.send("b");
    sh.wait_for("the typing", |s| cursor_row(s) == "$ zz foo ab");
    let s = sh.settle();
    assert!(!s.contents().contains("AC "), "{}", dump(&s));
    sh.send("\x15\r");
    sh.wait_for("the next line", |s| {
        cursor_row(s) == "$" && s.cursor_position().0 > 0
    });
    sh.send("x");
    sh.wait_for("the hook on the next line", |s| has_row(s, "AC 1 2 0"));
}
