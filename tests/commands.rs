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
