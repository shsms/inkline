#[path = "support/common.rs"]
mod common;

use common::*;

const INIT: &str = r#"
(defun my-upcase-line () (interactive)
  (let ((text (upcase (buffer-string)))) (erase-buffer) (insert text)))
(keymap-global-set "C-x u" 'my-upcase-line)
(defun boom () (insert "zz") (error "nope %d" 7))
(keymap-global-set "C-x b" 'boom)
(defun show-arg () (insert (format "%s" current-prefix-arg)))
(keymap-global-set "C-x a" 'show-arg)
(keymap-global-set "C-x l" (lambda () (insert "L")))
(keymap-global-set "C-x f" 'forward-char)
"#;

fn shell() -> Shell {
    Shell::start(Options {
        init_el: Some(INIT.into()),
        ..Options::default()
    })
}

#[test]
fn a_lisp_command_changes_the_line_in_one_undo_step() {
    let mut sh = shell();
    sh.send("abc\x18u");
    sh.wait_for("the upcased line", |s| cursor_row(s) == "$ ABC");
    sh.send("\x1f");
    sh.wait_for("one undo", |s| cursor_row(s) == "$ abc");
}

#[test]
fn a_failing_command_changes_nothing() {
    let mut sh = shell();
    sh.send("abc\x02\x18b");
    sh.wait_for("the error", |s| has_row(s, "inkline: boom: nope 7"));
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ abc");
    assert_eq!(s.cursor_position().1, 4);
}

#[test]
fn the_count_is_current_prefix_arg() {
    let mut sh = shell();
    sh.send("\x1b3\x18a \x18a");
    sh.wait_for("both", |s| cursor_row(s) == "$ 3 nil");
}

#[test]
fn lambdas_and_readline_names() {
    let mut sh = shell();
    sh.send("xy\x01\x18l\x18fQ");
    sh.wait_for("both commands", |s| cursor_row(s) == "$ LxQy");
}

#[test]
fn a_named_lisp_command_can_be_bound_with_bind() {
    let mut sh = Shell::start(Options {
        init_el: Some(INIT.into()),
        rc: "bind '\"\\C-xw\": my-upcase-line'\n".into(),
        ..Options::default()
    });
    sh.send("abc\x18w");
    sh.wait_for("upcased", |s| cursor_row(s) == "$ ABC");
}

#[test]
fn inkline_keys_shows_lisp_commands() {
    let mut sh = shell();
    sh.send("inkline keys\r");
    sh.wait_for("the list", |s| {
        (0..s.size().0).any(|r| {
            row_text(s, r).starts_with("C-x u") && row_text(s, r).ends_with("my-upcase-line")
        }) && (0..s.size().0)
            .any(|r| row_text(s, r).starts_with("C-x l") && row_text(s, r).ends_with("(lambda)"))
    });
}

#[test]
fn a_command_that_changes_nothing_adds_no_undo_step() {
    let mut sh = Shell::start(Options {
        init_el: Some("(keymap-global-set \"C-x g\" (lambda () (insert \"\")))\n".into()),
        ..Options::default()
    });
    sh.send("abc");
    sh.wait_for("the typing", |s| cursor_row(s) == "$ abc");
    sh.send("\x18g\x1f");
    sh.wait_for("the typing undone", |s| cursor_row(s) == "$");
}

/// A key that cannot be bound gives its named command no readline name.
#[test]
fn a_refused_key_adds_no_readline_name() {
    let mut sh = Shell::start(Options {
        init_el: Some(
            "(defun never-bound () (insert \"n\"))\n(condition-case nil (keymap-global-set \"C-a x\" 'never-bound) (error nil))\n"
                .into(),
        ),
        ..Options::default()
    });
    sh.send("bind -l | grep -c never-bound\r");
    sh.wait_for("the count", |s| has_row(s, "0"));
}

const INIT_CALLS: &str = r#"
(defun twice-forward () (call-interactively 'forward-char) (call-interactively 'forward-char))
(keymap-global-set "C-x t" 'twice-forward)
(defun try-yank () (call-interactively 'yank) (insert "not reached"))
(keymap-global-set "C-x y" 'try-yank)
(defun insert-key () (call-interactively 'self-insert))
(keymap-global-set "C-x i" 'insert-key)
(defun upcase-via-call () (call-interactively 'my-upcase-line))
(keymap-global-set "C-x v" 'upcase-via-call)
(defvar cleanup-text "none")
(defun guarded-yank ()
  (unwind-protect
      (condition-case nil (call-interactively 'yank) (error (insert "caught")))
    (setq cleanup-text "cleaned")))
(keymap-global-set "C-x p" 'guarded-yank)
(keymap-global-set "C-x o" (lambda () (insert cleanup-text)))
(keymap-global-set "C-x k" (lambda () (copy-region-as-kill 1 3)))
(defun insert-then-previous () (insert "zz") (call-interactively 'previous-history))
(keymap-global-set "C-x h" 'insert-then-previous)
(keymap-global-set "C-x c" (lambda () (let ((current-prefix-arg 3)) (call-interactively 'self-insert))))
(defun undo-via-call () (call-interactively 'undo))
(keymap-global-set "C-x z" 'undo-via-call)
(defun insert-undo-fail () (insert "zz") (call-interactively 'undo) (error "after undo"))
(keymap-global-set "C-x j" 'insert-undo-fail)
(defun undo-fail () (call-interactively 'undo) (error "after undo"))
(keymap-global-set "C-x n" 'undo-fail)
(defun take-suggestion () (call-interactively 'accept-suggestion))
(keymap-global-set "C-x m" 'take-suggestion)
(defun previous-then-undo () (insert "zz") (call-interactively 'previous-history) (call-interactively 'undo))
(keymap-global-set "C-x r" 'previous-then-undo)
(defun undo-nothing-fail ()
  (insert "zz") (let ((current-prefix-arg 0)) (call-interactively 'undo)) (error "after undo"))
(keymap-global-set "C-x d" 'undo-nothing-fail)
(defun vi-undo-via-call () (call-interactively 'vi-undo))
(keymap-global-set "C-x x" 'vi-undo-via-call)
(defun move-then-undo () (call-interactively 'forward-char) (call-interactively 'undo))
(keymap-global-set "C-x w" 'move-then-undo)
(defun previous-then-change ()
  (call-interactively 'previous-history) (end-of-line) (insert " a") (goto-char 1) (insert "b "))
(keymap-global-set "C-x e" 'previous-then-change)
(defun previous-then-fail () (call-interactively 'previous-history) (insert "zz") (error "after move"))
(keymap-global-set "C-x g" 'previous-then-fail)
"#;

fn calls_shell() -> Shell {
    Shell::start(Options {
        init_el: Some(format!("{INIT}{INIT_CALLS}")),
        history: vec!["echo one"],
        ..Options::default()
    })
}

#[test]
fn readline_and_lisp_commands_through_call_interactively() {
    let mut sh = calls_shell();
    sh.send("abcd\x01\x18tX");
    sh.wait_for("two forward-chars", |s| cursor_row(s) == "$ abXcd");
    sh.send("\x05\x15ab\x18v");
    sh.wait_for("a Lisp command by name", |s| cursor_row(s) == "$ AB");
    sh.send("\x15\x18i");
    sh.wait_for("the key that ran the command", |s| cursor_row(s) == "$ i");
}

#[test]
fn current_prefix_arg_is_the_count() {
    let mut sh = calls_shell();
    sh.send("\x18c");
    sh.wait_for("three keys", |s| cursor_row(s) == "$ ccc");
    sh.send("\x15\x1b4\x18i");
    sh.wait_for("the typed count", |s| cursor_row(s) == "$ iiii");
}

#[test]
fn yank_with_an_empty_kill_ring_is_a_quiet_quit() {
    let mut sh = calls_shell();
    sh.send("ab\x18y");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ ab");
    assert!(
        (0..s.size().0).all(|r| !row_text(&s, r).starts_with("inkline:")),
        "{}",
        dump(&s)
    );
    sh.send("\x18u");
    sh.wait_for("still working", |s| cursor_row(s) == "$ AB");
    sh.send("\x15inkline status\r");
    sh.wait_for("on", |s| has_row(s, "inkline: on"));
}

#[test]
fn quit_passes_condition_case_and_runs_unwind_protect() {
    let mut sh = calls_shell();
    sh.send("ab\x18p");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ ab");
    sh.send("\x18o");
    sh.wait_for("the cleanup ran", |s| cursor_row(s) == "$ abcleaned");
}

#[test]
fn copy_region_as_kill_fills_the_kill_ring() {
    let mut sh = calls_shell();
    sh.send("abcd\x02\x02\x18k");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ abcd");
    assert_eq!(s.cursor_position().1, 4);
    sh.send("\x05\x19");
    sh.wait_for("the copy yanked", |s| cursor_row(s) == "$ abcdab");
}

#[test]
fn a_command_that_moves_to_another_history_line_leaves_undo_working() {
    let mut sh = calls_shell();
    sh.send("ab\x18h");
    sh.wait_for("the history line", |s| cursor_row(s) == "$ echo one");
    sh.send("X");
    sh.wait_for("typing", |s| cursor_row(s) == "$ echo oneX");
    sh.send("\x1f");
    sh.wait_for("typing undone", |s| cursor_row(s) == "$ echo one");
    sh.send("\x18u");
    sh.wait_for("upcased", |s| cursor_row(s) == "$ ECHO ONE");
    sh.send("\x1f");
    sh.wait_for("one undo step", |s| cursor_row(s) == "$ echo one");
    sh.send("\x0e");
    sh.wait_for("the line left behind", |s| cursor_row(s) == "$ abzz");
    sh.send("\x1f");
    sh.wait_for("its insert undone", |s| cursor_row(s) == "$ ab");
}

#[test]
fn call_interactively_needs_a_line_being_edited() {
    let mut sh = calls_shell();
    sh.send("inkline eval \"(call-interactively 'forward-char)\"\r");
    sh.wait_for("the error", |s| {
        has_row(s, "inkline: no line is being edited")
    });
}

/// A kill backward, from a position after the other, goes in front of the
/// previous kill, as readline's `backward-kill-word` and Emacs do; a kill
/// forward goes after it.
#[test]
fn a_backward_kill_goes_in_front_of_the_previous_kill() {
    let mut sh = Shell::start(Options {
        init_el: Some(
            r#"
(keymap-global-set "C-x w" (lambda () (delete-char -1 t)))
(keymap-global-set "C-x e" (lambda () (kill-region (point) (- (point) 2))))
(keymap-global-set "C-x g" (lambda () (copy-region-as-kill (point) (- (point) 2))))
(keymap-global-set "C-x q" (lambda () (kill-region (- (point) 2) (point))))
"#
            .into(),
        ),
        ..Options::default()
    });
    for (keys, killed, yanked) in [
        ("foo bar\x1b\x7f\x18w", "$ foo", "$ foo bar"),
        ("abxybar\x02\x02\x02\x0b\x18e", "$ ab", "$ abxybar"),
        ("abxybar\x02\x02\x02\x0b\x18q", "$ ab", "$ abbarxy"),
        ("abxybar\x02\x02\x02\x0b\x18g\x05", "$ abxy", "$ abxyxybar"),
    ] {
        sh.send(keys);
        sh.wait_for("the kill", |s| {
            cursor_row(s) == killed && s.cursor_position().1 as usize == killed.len()
        });
        sh.send("\x19");
        sh.wait_for("the yank", |s| cursor_row(s) == yanked);
        sh.send("\x15");
        sh.wait_for("an empty line", |s| cursor_row(s) == "$");
    }
}

#[test]
fn undo_from_a_command_takes_back_whole_steps() {
    let mut sh = calls_shell();
    sh.send("ab\x18u");
    sh.wait_for("upcased", |s| cursor_row(s) == "$ AB");
    sh.send("\x18z");
    sh.wait_for("the upcase undone", |s| cursor_row(s) == "$ ab");
    sh.send("\x1f");
    sh.wait_for("the typing undone", |s| cursor_row(s) == "$");
}

/// A readline command that changed nothing leaves no empty step for a
/// later `undo` in the same command to take back.
#[test]
fn undo_after_a_readline_command_that_changed_nothing() {
    let mut sh = calls_shell();
    sh.send("ab");
    sh.wait_for("the typing", |s| cursor_row(s) == "$ ab");
    sh.send("\x01\x18w");
    sh.wait_for("the typing undone", |s| cursor_row(s) == "$");
}

/// The changes a command makes after it moved to another history line
/// undo as one step there.
#[test]
fn changes_after_a_history_move_undo_as_one_step() {
    let mut sh = calls_shell();
    sh.send("\x18e");
    sh.wait_for("the history line changed", |s| {
        cursor_row(s) == "$ b echo one a"
    });
    sh.send("\x1f");
    sh.wait_for("the changes undone", |s| cursor_row(s) == "$ echo one");
}

/// A command that fails after it moved to another history line keeps
/// what it did there.
#[test]
fn a_command_failing_after_a_history_move_keeps_its_changes() {
    let mut sh = calls_shell();
    sh.send("\x18g");
    sh.wait_for("the error", |s| {
        has_row(s, "inkline: previous-then-fail: after move")
    });
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ echo onezz", "{}", dump(&s));
}

#[test]
fn a_command_failing_after_undo_keeps_earlier_steps() {
    let mut sh = calls_shell();
    sh.send("ab\x18j");
    sh.wait_for("the error", |s| {
        has_row(s, "inkline: insert-undo-fail: after undo")
    });
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ ab");
    // An undo of an earlier step stays done: readline has no redo.
    sh.send("\x18u");
    sh.wait_for("upcased", |s| cursor_row(s) == "$ AB");
    sh.send("\x18n");
    sh.wait_for("the error", |s| {
        has_row(s, "inkline: undo-fail: after undo")
    });
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ ab");
    sh.send("\x1f");
    sh.wait_for("the typing undone", |s| cursor_row(s) == "$");
}

#[test]
fn inklines_own_commands_run_through_call_interactively() {
    let mut sh = calls_shell();
    sh.send("ec");
    sh.wait_for("the suggestion", |s| cursor_row(s) == "$ echo one");
    sh.send("\x18mX");
    sh.wait_for("the suggestion taken", |s| cursor_row(s) == "$ echo oneX");
}

#[test]
fn a_history_move_leaves_no_undo_group_open_for_vi_mode() {
    // Leaving vi insert mode closes every undo group readline counts as
    // open. A group counted by mistake would add an extra step end there,
    // and `u` would then take back more than one step.
    let mut sh = Shell::start(Options {
        init_el: Some(format!("{INIT}{INIT_CALLS}")),
        history: vec!["echo one"],
        rc: "bind '\"\\C-xV\": vi-editing-mode'\n\
             bind -m vi-command '\"\\C-e\": emacs-editing-mode'\n\
             bind 'set keyseq-timeout 20'\n"
            .into(),
        ..Options::default()
    });
    // Enter vi command mode once, since the first time frees the undo list,
    // then go back to emacs mode with C-e.
    sh.send("\x18V");
    sh.settle();
    sh.send("\x1b");
    past_escape_wait(&sh);
    sh.send("\x05ab\x18h");
    sh.wait_for("the history line", |s| cursor_row(s) == "$ echo one");
    sh.send("X\x18u");
    sh.wait_for("upcased", |s| cursor_row(s) == "$ ECHO ONEX");
    // Entering vi insert mode opens a group that ESC closes: the first `u`
    // takes back that empty group, the second the upcase.
    sh.send("\x18V");
    sh.settle();
    sh.send("\x1b");
    past_escape_wait(&sh);
    sh.send("uu");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ echo oneX");
}

/// Waits until readline has taken a lone ESC as a key of its own, not the
/// start of a longer key sequence (`keyseq-timeout` is 20 ms here).
fn past_escape_wait(sh: &Shell) {
    std::thread::sleep(std::time::Duration::from_millis(200));
    sh.settle();
}

#[test]
fn undo_after_a_history_move_takes_back_one_step_there() {
    let mut sh = calls_shell();
    sh.send("\x10X\x18u");
    sh.wait_for("two steps on the history line", |s| {
        cursor_row(s) == "$ ECHO ONEX"
    });
    sh.send("\x0e");
    sh.wait_for("the new line", |s| cursor_row(s) == "$");
    sh.send("\x18r");
    sh.wait_for("only the upcase undone", |s| cursor_row(s) == "$ echo oneX");
}

#[test]
fn an_undo_with_count_0_keeps_the_step_open() {
    let mut sh = calls_shell();
    sh.send("ab\x18d");
    sh.wait_for("the error", |s| {
        has_row(s, "inkline: undo-nothing-fail: after undo")
    });
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ ab");
    sh.send("\x1f");
    sh.wait_for("the typing undone", |s| cursor_row(s) == "$");
}

#[test]
fn vi_undo_from_a_command_takes_back_whole_steps() {
    // readline names `vi-undo` from 8.1 (bash 5.1).
    if bash_version() < (5, 1) {
        return;
    }
    let mut sh = calls_shell();
    sh.send("ab\x18u");
    sh.wait_for("upcased", |s| cursor_row(s) == "$ AB");
    sh.send("\x18x");
    sh.wait_for("the upcase undone", |s| cursor_row(s) == "$ ab");
    sh.send("\x1f");
    sh.wait_for("the typing undone", |s| cursor_row(s) == "$");
}

const INIT_MSG: &str = r#"
(defun say () (message "hi %s" "there"))
(keymap-global-set "C-x m" 'say)
(keymap-global-set "C-x c" (lambda () (message "first") (insert (message "hi")) (message nil)))
(keymap-global-set "C-x p" (lambda () (print 'sym)))
(keymap-global-set "C-x s" (lambda () (setq inkline-indent 99)))
(keymap-global-set "C-x e" (lambda () (setq inkline-indent 99) (error "oops")))
"#;

fn message_shell(rc: &str) -> Shell {
    Shell::start(Options {
        init_el: Some(format!("{INIT}{INIT_MSG}")),
        rc: rc.into(),
        ..Options::default()
    })
}

/// The text of the row under the cursor's row.
fn row_below(s: &vt100::Screen) -> String {
    row_text(s, s.cursor_position().0 + 1)
}

#[test]
fn message_shows_under_the_line_until_the_next_key() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!("{INIT}{INIT_MSG}")),
        ..Options::default()
    });
    sh.send("ab\x02\x18m");
    sh.wait_for("the message", |s| row_below(s) == "hi there");
    sh.send("X");
    sh.wait_for("gone, text kept", |s| {
        cursor_row(s) == "$ aXb" && row_below(s).is_empty()
    });
}

#[test]
fn a_command_error_shows_under_the_line() {
    let mut sh = Shell::start(Options {
        init_el: Some(INIT.into()),
        ..Options::default()
    });
    sh.send("abc\x18b");
    sh.wait_for("the error", |s| row_below(s) == "inkline: boom: nope 7");
}

#[test]
fn message_nil_clears_and_message_returns_the_text() {
    let mut sh = message_shell("");
    sh.send("ab\x18c");
    sh.wait_for("the text", |s| cursor_row(s) == "$ abhi");
    let s = sh.settle();
    assert!(row_below(&s).is_empty(), "{}", dump(&s));
}

#[test]
fn print_shows_under_the_line_while_editing() {
    let mut sh = message_shell("");
    sh.send("ab\x18p");
    sh.wait_for("the printed value", |s| {
        cursor_row(s) == "$ ab" && row_below(s) == "sym"
    });
}

#[test]
fn outside_editing_print_goes_to_stdout_and_message_to_stderr() {
    let mut sh = message_shell("");
    sh.send("inkline eval '(progn (princ \"a\") (prin1 \"b\") (print 1) nil)' 2>/dev/null\r");
    sh.wait_for("the printed text", |s| has_row(s, "a\"b\"1"));
    sh.send("inkline eval '(message \"to %s\" \"err\")' >/dev/null\r");
    sh.wait_for("the message", |s| has_row(s, "to err"));
}

#[test]
fn a_message_on_the_last_row_scrolls_the_screen() {
    let mut sh = message_shell("");
    sh.send("seq 40\r");
    sh.wait_for("the prompt at the bottom", |s| {
        s.cursor_position().0 == 23 && cursor_row(s) == "$"
    });
    sh.send("ab\x02\x18m");
    sh.wait_for("the message", |s| {
        s.cursor_position().0 == 22 && cursor_row(s) == "$ ab" && row_below(s) == "hi there"
    });
    sh.send("X");
    sh.wait_for("gone, text kept", |s| {
        cursor_row(s) == "$ aXb" && row_below(s).is_empty() && has_row(s, "40")
    });
}

#[test]
fn a_bad_setting_from_a_command_shows_under_the_line() {
    let mut sh = message_shell("");
    sh.send("ab\x18s");
    sh.wait_for("the problem", |s| {
        row_below(s) == "inkline: inkline-indent: expected a number from 0 to 16"
    });
}

/// A command's error and the bad settings it left show together.
#[test]
fn an_error_and_a_bad_setting_show_together() {
    let mut sh = message_shell("");
    sh.send("ab\x18e");
    sh.wait_for("both", |s| {
        row_below(s)
            == "inkline: lambda: oops; inkline: inkline-indent: expected a number from 0 to 16"
    });
}

/// Where readline draws the line alone, or inkline is off, the message goes
/// on a row of its own above a new prompt.
#[test]
fn the_message_goes_above_the_prompt_where_inkline_does_not_draw() {
    for rc in ["bind 'set mark-modified-lines on'\n", "inkline off\n"] {
        let mut sh = message_shell(rc);
        sh.send("ab\x18m");
        sh.wait_for("the message above", |s| {
            let row = s.cursor_position().0;
            cursor_row(s) == "$ ab" && row > 0 && row_text(s, row - 1) == "hi there"
        });
        sh.send("X");
        sh.wait_for("typing goes on", |s| cursor_row(s) == "$ abX");
    }
}

/// A message above the prompt goes below every row of a line that takes
/// more than one, and on a row of its own.
#[test]
fn the_message_above_goes_below_a_line_of_several_rows() {
    let mut sh = Shell::start(Options {
        cols: 20,
        init_el: Some(format!("{INIT}{INIT_MSG}")),
        rc: "inkline off\n".into(),
        ..Options::default()
    });
    sh.send("echo 123456789012345678901234567890");
    sh.wait_for("the typing", |s| cursor_row(s) == "45678901234567890");
    sh.send("\x01\x18m");
    let s = sh.wait_for("the message", |s| {
        s.cursor_position() == (3, 2) && has_row(s, "hi there")
    });
    assert_eq!(row_text(&s, 0), "$ echo 1234567890123", "{}", dump(&s));
    assert_eq!(row_text(&s, 1), "45678901234567890", "{}", dump(&s));
    assert_eq!(row_text(&s, 2), "hi there", "{}", dump(&s));
    assert_eq!(row_text(&s, 3), "$ echo 1234567890123", "{}", dump(&s));
    assert_eq!(row_text(&s, 4), "45678901234567890", "{}", dump(&s));
}

#[test]
fn a_message_shows_under_an_empty_line_and_after_a_resize() {
    let mut sh = message_shell("");
    sh.send("\x18m");
    sh.wait_for("the message", |s| {
        cursor_row(s) == "$" && row_below(s) == "hi there"
    });
    sh.send("echo 12345678");
    sh.wait_for("the typing", |s| {
        cursor_row(s) == "$ echo 12345678" && row_below(s).is_empty()
    });
    sh.send("\x18m");
    sh.wait_for("the message", |s| row_below(s) == "hi there");
    // Narrower, the line takes two rows: the message goes under the second.
    sh.resize(24, 12);
    sh.wait_for("the message again", |s| {
        let row = s.cursor_position().0;
        cursor_row(s) == "678"
            && row_text(s, row - 1) == "$ echo 12345"
            && row_below(s) == "hi there"
    });
}

const INIT_ASK: &str = r#"
(defun ask () (if (y-or-n-p "Sure? ") (insert "yes") (insert "no")))
(keymap-global-set "C-x q" 'ask)
(keymap-global-set "C-x s" (lambda () (call-interactively 'reverse-search-history) (insert "after")))
(keymap-global-set "C-x v" (lambda () (call-interactively 'character-search) (insert "after")))
(keymap-global-set "C-x w" (lambda () (insert "zz") (y-or-n-p "Q? ") (insert "after")))
(keymap-global-set "C-x x"
  (lambda ()
    (catch 'inkline--quit (y-or-n-p "Q? "))
    (call-interactively 'reverse-search-history)
    (insert "after")))
"#;

fn ask_shell() -> Shell {
    Shell::start(Options {
        init_el: Some(format!("{INIT}{INIT_ASK}")),
        ..Options::default()
    })
}

#[test]
fn y_or_n_p_asks_under_the_line() {
    let mut sh = ask_shell();
    sh.send("\x18q");
    sh.wait_for("the question", |s| row_below(s) == "Sure? (y or n)");
    sh.send("y");
    sh.wait_for("the answer", |s| {
        cursor_row(s) == "$ yes" && row_below(s).is_empty()
    });
}

#[test]
fn y_or_n_p_rings_and_asks_again_on_another_key() {
    let mut sh = ask_shell();
    sh.send("\x18q");
    sh.wait_for("the question", |s| row_below(s) == "Sure? (y or n)");
    sh.send("x");
    let s = sh.settle();
    assert_eq!(row_below(&s), "Sure? (y or n)", "{}", dump(&s));
    sh.send("N");
    sh.wait_for("the answer", |s| {
        cursor_row(s) == "$ no" && row_below(s).is_empty()
    });
}

#[test]
fn c_g_at_y_or_n_p_quits_quietly() {
    let mut sh = ask_shell();
    sh.send("ab\x18q");
    sh.wait_for("the question", |s| row_below(s).starts_with("Sure?"));
    sh.send("\x07");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ ab");
    assert!(row_below(&s).is_empty(), "{}", dump(&s));
}

#[test]
fn c_c_at_y_or_n_p_gives_a_new_prompt() {
    let mut sh = ask_shell();
    sh.send("ab\x18q");
    sh.wait_for("the question", |s| row_below(s).starts_with("Sure?"));
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    sh.send("\x18q");
    sh.wait_for("asks again", |s| row_below(s).starts_with("Sure?"));
    sh.send("n");
    sh.wait_for("answered", |s| cursor_row(s) == "$ no");
    sh.send("\x15cd\x18u");
    sh.wait_for("commands still run", |s| cursor_row(s) == "$ CD");
    sh.send("\x15inkline status\r");
    sh.wait_for("on", |s| has_row(s, "inkline: on"));
}

#[test]
fn a_key_typed_ahead_is_not_an_answer() {
    let mut sh = ask_shell();
    sh.send("\x18qy");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ y");
}

/// A key typed ahead that readline read while matching a longer key
/// sequence, and put back, is not an answer either.
#[test]
fn a_key_readline_put_back_is_not_an_answer() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!("{INIT}{INIT_ASK}")),
        rc: "bind '\"\\C-xqz\": \"Z\"'\n".into(),
        ..Options::default()
    });
    sh.send("\x18qy");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ y", "{}", dump(&s));
}

/// A question asked on the last key of macro text is asked, and the keys
/// of macro text answer it.
#[test]
fn y_or_n_p_asks_from_macro_text() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!("{INIT}{INIT_ASK}")),
        rc: "bind '\"\\C-t\": \"a\\C-xq\"'\n\
             bind '\"\\C-xz\": \"c\\C-xqy\"'\n"
            .into(),
        ..Options::default()
    });
    sh.send("\x14");
    sh.wait_for("the question", |s| row_below(s) == "Sure? (y or n)");
    sh.send("y");
    sh.wait_for("the answer", |s| cursor_row(s) == "$ ayes");
    sh.send("\x15\x18z");
    sh.wait_for("the answer from the macro", |s| cursor_row(s) == "$ cyes");
}

#[test]
fn y_or_n_p_works_only_in_a_command() {
    let mut sh = ask_shell();
    sh.send("inkline eval '(y-or-n-p \"x\")'\r");
    sh.wait_for("the error", |s| {
        (0..s.size().0)
            .any(|r| row_text(s, r).starts_with("inkline: y-or-n-p works only in a command"))
    });
}

/// A readline command run from Lisp that reads keys gets `C-c` too: the
/// Lisp command stops, and bash gives a new prompt. Undo still works there,
/// also after a command that changed the line before it asked. A command
/// that catches the quit runs no readline command after it.
#[test]
fn c_c_in_a_readline_command_run_from_lisp_gives_a_new_prompt() {
    for (key, reading) in [
        ("\x18s", "(reverse-i-search)"),
        ("\x18v", "$ ab"),
        ("\x18w", "$ abzz"),
        ("\x18x", "$ ab"),
    ] {
        let mut sh = ask_shell();
        sh.send("ab");
        sh.wait_for("the typing", |s| cursor_row(s) == "$ ab");
        sh.send(key);
        sh.wait_for("reading keys", |s| cursor_row(s).starts_with(reading));
        // character-search shows nothing new: give it time to start reading.
        sh.settle();
        sh.send("\x03");
        sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
        let s = sh.settle();
        assert!(!has_row(&s, "after"), "{}", dump(&s));
        sh.send("cd\x18u");
        sh.wait_for("commands still run", |s| cursor_row(s) == "$ CD");
        sh.send("\x1f");
        sh.wait_for("undo", |s| cursor_row(s) == "$ cd");
    }
}

#[test]
fn y_or_n_p_asks_above_the_line_where_inkline_does_not_draw() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!("{INIT}{INIT_ASK}")),
        rc: "inkline off\n".into(),
        ..Options::default()
    });
    sh.send("\x18q");
    sh.wait_for("the question above", |s| {
        let row = s.cursor_position().0;
        cursor_row(s) == "$" && row > 0 && row_text(s, row - 1) == "Sure? (y or n)"
    });
    sh.send("y");
    sh.wait_for("the answer", |s| cursor_row(s) == "$ yes");
    // `C-c` while Lisp reads a key still waits for Lisp to stop, with
    // inkline off too: in `y-or-n-p`, and in a readline command Lisp ran.
    sh.send("\x15\x18q");
    sh.wait_for("the question above", |s| {
        let row = s.cursor_position().0;
        cursor_row(s) == "$" && row > 0 && row_text(s, row - 1) == "Sure? (y or n)"
    });
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| {
        cursor_row(s) == "$" && has_row(s, "$ ^C")
    });
    sh.send("\x18s");
    sh.wait_for("the search", |s| {
        cursor_row(s).starts_with("(reverse-i-search)")
    });
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    let s = sh.settle();
    assert!(!has_row(&s, "after"), "{}", dump(&s));
    sh.send("inkline eval '(+ 40 2)'\r");
    sh.wait_for("Lisp still runs", |s| has_row(s, "42"));
}

/// A readline command run from Lisp can run a key's command itself (here
/// `universal-argument` runs the key after it). When that key is a Lisp
/// command, it cannot run while the first one does; the `C-c` still waits
/// for the first one to end, and Lisp works at the new prompt.
#[test]
fn c_c_in_a_key_a_readline_command_runs_leaves_lisp_working() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!(
            "{INIT}{}",
            r#"
(keymap-global-set "C-g" (lambda () (insert "G")))
(keymap-global-set "C-x n" (lambda () (call-interactively 'universal-argument) (insert "after")))
"#
        )),
        ..Options::default()
    });
    sh.send("ab\x18n");
    sh.settle();
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    let s = sh.settle();
    assert!(!has_row(&s, "after"), "{}", dump(&s));
    sh.send("cd\x18u");
    sh.wait_for("commands still run", |s| cursor_row(s) == "$ CD");
}

/// A readline command inside `save-excursion` does not move its saved
/// point: when the command shortened the line, point comes back at the
/// end of the line, or at the start of the character the saved point is
/// now inside.
#[test]
fn save_excursion_around_a_readline_command_that_shortens_the_line() {
    let mut sh = Shell::start(Options {
        init_el: Some(
            r#"
(keymap-global-set "C-x e"
  (lambda () (save-excursion (call-interactively 'unix-line-discard)) (insert "X")))
(keymap-global-set "C-x d"
  (lambda ()
    (save-excursion (call-interactively 'beginning-of-line) (call-interactively 'delete-char))
    (insert "X")))
"#
            .into(),
        ),
        ..Options::default()
    });
    sh.send("abcdef\x18e");
    sh.wait_for("the line", |s| cursor_row(s) == "$ X");
    sh.send("Y");
    sh.wait_for("typing goes on", |s| {
        cursor_row(s) == "$ XY" && s.cursor_position().1 == 4
    });
    sh.send("\x15xx\u{e9}\u{e9}\x02\x18d");
    sh.wait_for("the line", |s| cursor_row(s) == "$ x\u{e9}X\u{e9}");
    sh.send("Y");
    sh.wait_for("typing goes on", |s| cursor_row(s) == "$ x\u{e9}XY\u{e9}");
}

/// A readline command run from Lisp can run shell code that jumps back to
/// bash's own top level: the Lisp command stops, and once Lisp has
/// returned, bash makes that jump.
#[test]
fn a_shell_error_in_a_readline_command_run_from_lisp_gives_a_new_prompt() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!(
            "{INIT}{}",
            r#"
(keymap-global-set "C-x e"
  (lambda () (call-interactively 'shell-expand-line) (insert "after")))
"#
        )),
        ..Options::default()
    });
    sh.send("echo ${zz:?boom}");
    sh.wait_for("the typing", |s| cursor_row(s) == "$ echo ${zz:?boom}");
    sh.send("\x18e");
    sh.wait_for("a new prompt", |s| {
        cursor_row(s) == "$" && (0..s.size().0).any(|r| row_text(s, r).ends_with("zz: boom"))
    });
    let s = sh.settle();
    assert!(!has_row(&s, "after"), "{}", dump(&s));
    sh.send("cd\x18u");
    sh.wait_for("commands still run", |s| cursor_row(s) == "$ CD");
    sh.send("\x15inkline eval '(+ 40 2)'\r");
    sh.wait_for("Lisp still runs", |s| has_row(s, "42"));
}

/// Shell code that a Lisp command runs while inkline is off can switch
/// inkline on (here `edit-and-execute-command` runs the line). inkline is
/// then on with its own key reader, which the syntax-error underline after
/// a pause needs, and `inkline off` afterwards puts readline's back: keys,
/// Lisp and the shell keep working.
#[test]
fn inkline_on_from_a_lisp_command_while_off() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!(
            "{INIT}{}",
            r#"
(keymap-global-set "C-x e" (lambda () (call-interactively 'edit-and-execute-command)))
"#
        )),
        rc: "inkline eval '(setq inkline-colors \"error=4\")' >/dev/null\n\
             inkline off\nexport VISUAL=true\n"
            .into(),
        ..Options::default()
    });
    sh.send("inkline on\x18e");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 > 0 && cursor_row(s) == "$"
    });
    sh.send("inkline status | head -1\r");
    sh.wait_for("inkline on", |s| {
        has_row(s, "inkline: on") && cursor_row(s) == "$"
    });
    sh.send("echo ) x");
    sh.wait_for("the underline", |s| underlined(s, ")"));
    sh.send("\x15inkline off\r");
    sh.settle();
    sh.send("ab\x18u");
    sh.wait_for("the Lisp command", |s| cursor_row(s) == "$ AB");
    sh.send("\x15inkline eval '(+ 40 2)'\r");
    sh.wait_for("Lisp still runs", |s| {
        has_row(s, "42") && cursor_row(s) == "$"
    });
}

/// A readline command that Lisp runs can run a key's Lisp command, which
/// cannot run while the first one does. With inkline off, that key leaves
/// inkline's key reader in place, so a `C-c` at a question the first
/// command asks afterwards still waits for Lisp to stop.
#[test]
fn a_nested_lisp_key_keeps_the_key_reader_with_inkline_off() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!(
            "{INIT}{}",
            r#"
(keymap-global-set "C-x g"
  (lambda ()
    (call-interactively 'universal-argument)
    (insert (if (y-or-n-p "Sure? ") "yes" "no"))))
"#
        )),
        rc: "inkline off\nbind '\"\\C-t\": \"\\C-xg\\C-xl\"'\n".into(),
        ..Options::default()
    });
    sh.send("\x14");
    sh.wait_for("the question", |s| {
        (0..s.size().0).any(|r| row_text(s, r).starts_with("Sure? (y or n)"))
    });
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    sh.send("inkline eval '(+ 40 2)'\r");
    sh.wait_for("Lisp still runs", |s| {
        has_row(s, "42") && cursor_row(s) == "$"
    });
}

/// Shell code that a Lisp command runs can switch inkline off, or remove
/// it with `enable -d`, and the command can then still read a key. inkline's
/// key reader stays in place until the command has returned, so a `C-c` at
/// the question still waits for Lisp to stop, and Lisp keeps working.
#[test]
fn inkline_off_from_a_lisp_command_that_asks_afterwards() {
    let enable = format!("enable -f {} inkline", so_path().display());
    for (off, on) in [
        ("inkline off", "inkline on"),
        ("enable -d inkline", enable.as_str()),
    ] {
        let mut sh = Shell::start(Options {
            init_el: Some(format!(
                "{INIT}{}",
                r#"
(keymap-global-set "C-x e"
  (lambda ()
    (call-interactively 'edit-and-execute-command)
    (insert (if (y-or-n-p "Sure? ") "yes" "no"))))
"#
            )),
            rc: "export VISUAL=true\n".into(),
            ..Options::default()
        });
        sh.send(&format!("{off}\x18e"));
        sh.wait_for("the question", |s| {
            (0..s.size().0).any(|r| row_text(s, r).starts_with("Sure? (y or n)"))
        });
        sh.send("\x03");
        sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
        sh.send(&format!("{on}; inkline eval '(+ 40 2)'\r"));
        sh.wait_for("Lisp still runs", |s| {
            has_row(s, "42") && cursor_row(s) == "$"
        });
        sh.send("ab\x18u");
        sh.wait_for("the Lisp command", |s| cursor_row(s) == "$ AB");
    }
}

/// A `C-c` while a Lisp command runs without reading a key gives a new
/// prompt once the command has returned, and the line is not run.
#[test]
fn c_c_while_a_command_computes_gives_a_new_prompt() {
    let mut sh = Shell::start(Options {
        init_el: Some(
            r#"
(keymap-global-set "C-x z"
  (lambda () (let ((i 0)) (while (< i 6000000) (setq i (1+ i)))) (insert "done")))
"#
            .into(),
        ),
        ..Options::default()
    });
    sh.send("ab\x18z");
    std::thread::sleep(std::time::Duration::from_millis(100));
    // The command is still running.
    let s = sh.screen();
    assert_eq!(cursor_row(&s), "$ ab", "{}", dump(&s));
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| {
        s.cursor_position().0 > 0 && cursor_row(s) == "$"
    });
    sh.send("echo hi\r");
    sh.wait_for("the new line run", |s| {
        has_row(s, "hi") && cursor_row(s) == "$"
    });
    let s = sh.settle();
    assert!(!has_row(&s, "$ abdoneecho hi"), "{}", dump(&s));
}

/// `read -e` in shell code that a Lisp command runs reads a line of its
/// own: a `C-c` there stops the shell code as it does without inkline,
/// also in a search on that line, and once Lisp has returned, bash gives
/// a new prompt. Lisp and the shell keep working, with inkline on or off.
#[test]
fn c_c_at_read_e_in_shell_code_run_from_lisp() {
    for (rc, search) in [("", false), ("inkline off\n", false), ("", true)] {
        let mut sh = Shell::start(Options {
            init_el: Some(format!(
                "{INIT}{}",
                r#"
(keymap-global-set "C-x e"
  (lambda () (call-interactively 'edit-and-execute-command) (insert "after")))
"#
            )),
            rc: format!("{rc}export VISUAL=true\n"),
            ..Options::default()
        });
        sh.send("read -e -p 'name? ' x; echo \"ran:[$x]\"\x18e");
        sh.wait_for("the nested prompt", |s| cursor_row(s) == "name?");
        sh.send("abc");
        sh.wait_for("the typing", |s| cursor_row(s) == "name? abc");
        if search {
            sh.send("\x12a");
            sh.wait_for("the search", |s| {
                cursor_row(s).starts_with("(reverse-i-search)")
            });
        }
        sh.send("\x03");
        sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
        let s = sh.settle();
        assert!(
            !(0..s.size().0).any(|r| row_text(&s, r).starts_with("ran:")),
            "{}",
            dump(&s)
        );
        assert!(!has_row(&s, "$ after"), "{}", dump(&s));
        sh.send("inkline eval '(+ 40 2)'\r");
        sh.wait_for("Lisp still runs", |s| {
            has_row(s, "42") && cursor_row(s) == "$"
        });
        sh.send("ab\x18u");
        sh.wait_for("the Lisp command", |s| cursor_row(s) == "$ AB");
    }
}

/// A timed-out `read -e -t` in shell code that a Lisp command runs (here
/// a completion function) leaves a line of its own by a jump. A question
/// the command asks afterwards quits on `C-c`, and Lisp and the shell
/// keep working.
#[test]
fn a_question_after_a_timed_out_read_e_leaves_lisp_working() {
    // bash 5.0 leaves its line broken after such a read (`C-c` gives no new
    // prompt), with or without Lisp.
    if bash_version() < (5, 1) {
        return;
    }
    let mut sh = Shell::start(Options {
        init_el: Some(format!(
            "{INIT}{}",
            r#"
(keymap-global-set "C-x q"
  (lambda () (call-interactively 'complete) (insert (if (y-or-n-p "Q? ") "yes" "no"))))
"#
        )),
        rc: "f() { read -e -t 1 -p 'n? ' y; COMPREPLY=(foobar); }; complete -F f zz\n".into(),
        ..Options::default()
    });
    sh.send("zz ");
    sh.wait_for("the typing", |s| cursor_row(s) == "$ zz");
    sh.send("\x18q");
    sh.wait_for("the question", |s| {
        (0..s.size().0).any(|r| row_text(s, r).contains("Q? (y or n)"))
    });
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    sh.send("inkline eval '(+ 40 2)'\r");
    sh.wait_for("Lisp still runs", |s| {
        has_row(s, "42") && cursor_row(s) == "$"
    });
    sh.send("ab\x18u");
    sh.wait_for("the Lisp command", |s| cursor_row(s) == "$ AB");
}

/// A question on a `read -e -t` line whose time runs out while it waits:
/// the read ends (in bash 5.0 once the question is answered, else at
/// once), Lisp keeps working, and inkline's suggestions still show on the
/// next line.
#[test]
fn a_question_on_a_line_that_times_out() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!("{INIT}{INIT_ASK}")),
        history: vec!["git status"],
        ..Options::default()
    });
    sh.send("read -e -t 1 -p 'x? ' v; echo \"rc=$?\"\r");
    sh.wait_for("the read", |s| cursor_row(s) == "x?");
    sh.send("\x18q");
    if bash_version() < (5, 1) {
        // bash 5.0 times the read out in its own signal check, which runs
        // once Lisp has stopped.
        sh.wait_for("the question", |s| has_row(s, "Sure? (y or n)"));
        std::thread::sleep(std::time::Duration::from_millis(1200));
        sh.send("y");
    }
    sh.wait_for("the timeout", |s| {
        (0..s.size().0).any(|r| row_text(s, r).starts_with("rc=")) && cursor_row(s) == "$"
    });
    sh.send("git st");
    sh.wait_for("the suggestion", |s| cursor_row(s) == "$ git status");
    sh.send("\x15inkline eval '(+ 40 2)'\r");
    sh.wait_for("Lisp still runs", |s| {
        has_row(s, "42") && cursor_row(s) == "$"
    });
}

/// A trap that runs while a Lisp command asks on a `read -e` line prints
/// once the command has returned, as it would at readline's own prompt, and
/// the question leaves nothing behind on the screen.
#[test]
fn a_trap_during_a_question_leaves_no_question_behind() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!("{INIT}{INIT_ASK}")),
        rc: "trap 'echo got-usr1' USR1\n".into(),
        ..Options::default()
    });
    sh.send("read -e -p 'x? ' v; echo \"v=$v\"\r");
    sh.wait_for("the read", |s| cursor_row(s) == "x?");
    sh.send("abc\x18q");
    sh.wait_for("the question", |s| {
        (0..s.size().0).any(|r| row_text(s, r).starts_with("Sure? (y or n)"))
    });
    sh.take_output();
    sh.signal(libc::SIGUSR1);
    std::thread::sleep(std::time::Duration::from_millis(300));
    sh.send("y");
    sh.wait_for("the trap", |s| {
        (0..s.size().0).any(|r| row_text(s, r).ends_with("got-usr1"))
    });
    // The trap's output is not held back behind an open synchronized update.
    let out = sh.take_output();
    let trap = find(&out, b"got-usr1").expect("the trap's output");
    if let Some(begin) = rfind(&out[..trap], b"\x1b[?2026h") {
        assert!(
            find(&out[begin..trap], b"\x1b[?2026l").is_some(),
            "{:?}",
            String::from_utf8_lossy(&out)
        );
    }
    sh.send("\r");
    sh.wait_for("the read done", |s| {
        has_row(s, "v=abcyes") && cursor_row(s) == "$"
    });
    let s = sh.settle();
    assert!(
        !(0..s.size().0).any(|r| {
            let row = row_text(&s, r);
            row.contains("or n)") || row.contains("yesabc")
        }),
        "{}",
        dump(&s)
    );
}

/// A trap waiting for its signal at readline's own prompt runs only once
/// the line is done, so inkline keeps drawing the line and its suggestion.
#[test]
fn a_trap_waiting_at_the_prompt_keeps_the_suggestion() {
    let mut sh = Shell::start(Options {
        rc: "trap 'echo got-usr1' USR1\n".into(),
        history: vec!["echo hello world"],
        ..Options::default()
    });
    sh.send("ec");
    sh.wait_for("the suggestion", |s| cursor_row(s) == "$ echo hello world");
    sh.signal(libc::SIGUSR1);
    std::thread::sleep(std::time::Duration::from_millis(300));
    sh.send("ho ");
    sh.wait_for("the suggestion still", |s| {
        cursor_row(s) == "$ echo hello world"
    });
}

/// Where `needle` first starts in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Where `needle` last starts in `haystack`.
fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}
