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
