#[path = "support/common.rs"]
mod common;

use common::*;

/// Types `keys` into a fresh shell; returns the line and the cursor column.
fn after(keys: &str) -> (String, u16) {
    let mut sh = Shell::start(Options::default());
    sh.send(keys);
    let s = sh.settle();
    (cursor_row(&s), s.cursor_position().1)
}

#[test]
fn inserts_pairs() {
    assert_eq!(after("echo ("), ("$ echo ()".into(), 8));
    assert_eq!(after("echo \"x\""), ("$ echo \"x\"".into(), 10));
    assert_eq!(after("echo (a)"), ("$ echo (a)".into(), 10));
}

#[test]
fn plain_insert_where_pairing_would_get_in_the_way() {
    assert_eq!(after("echo don't").0, "$ echo don't");
    assert_eq!(after("echo \\(").0, "$ echo \\(");
    assert_eq!(after("echo \x1b3(").0, "$ echo (((");
    assert_eq!(after("echo # (").0, "$ echo # (");
    assert_eq!(after("echo \"a(").0, "$ echo \"a(\"");
}

#[test]
fn backspace_and_undo_remove_the_pair() {
    assert_eq!(after("echo (\x7f"), ("$ echo".into(), 7));
    assert_eq!(after("echo (\x1f"), ("$ echo".into(), 7));
}

#[test]
fn pasted_text_is_not_paired() {
    assert_eq!(after("echo \x1b[200~(x\x1b[201~").0, "$ echo (x");
}

#[test]
fn plain_after_enable_d() {
    let mut sh = Shell::start(Options::default());
    sh.send("enable -d inkline\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 == 1 && cursor_row(s) == "$"
    });
    sh.send("echo (");
    assert_eq!(cursor_row(&sh.settle()), "$ echo (");
}
