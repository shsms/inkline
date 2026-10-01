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

/// Readline's search takes only `backward-delete-char` as deleting a
/// character of the search text: during `C-r`, `DEL` deletes one and the
/// search goes on. Once the search ends, `DEL` deletes a pair again.
#[test]
fn backspace_deletes_a_search_character() {
    let mut sh = Shell::start(Options {
        history: vec!["echo hello", "echo help"],
        ..Options::default()
    });
    sh.send("\x12help");
    sh.wait_for("the search", |s| {
        cursor_row(s) == "(reverse-i-search)`help': echo help"
    });
    sh.send("\x7f");
    sh.wait_for("a character less", |s| {
        cursor_row(s) == "(reverse-i-search)`hel': echo help"
    });
    sh.send("lo");
    sh.wait_for("the search going on", |s| {
        cursor_row(s) == "(reverse-i-search)`hello': echo hello"
    });
    // `C-g` ends the search with the line as it was.
    sh.send("\x07");
    sh.wait_for("the search ended", |s| cursor_row(s) == "$");
    sh.send("x(\x7f");
    assert_eq!(cursor_row(&sh.settle()), "$ x");
}

/// A `C-c` during a search leaves `DEL` deleting a pair.
#[test]
fn backspace_deletes_a_pair_after_a_search_is_interrupted() {
    let mut sh = Shell::start(Options {
        history: vec!["echo hello"],
        ..Options::default()
    });
    sh.send("\x12hel");
    sh.wait_for("the search", |s| {
        cursor_row(s) == "(reverse-i-search)`hel': echo hello"
    });
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| {
        s.cursor_position().0 > 0 && cursor_row(s) == "$"
    });
    sh.send("x(\x7f");
    assert_eq!(cursor_row(&sh.settle()), "$ x");
}

/// A search ended by RET runs the line found before another key is read:
/// a `bind` of `DEL` in that line stays.
#[test]
fn a_bind_run_from_a_search_keeps_its_key() {
    let mut sh = Shell::start(Options {
        history: vec!["bind '\"\\C-?\": backward-delete-char'"],
        ..Options::default()
    });
    sh.send("\x12bind");
    sh.wait_for("the search", |s| {
        cursor_row(s).starts_with("(reverse-i-search)`bind'")
    });
    sh.send("\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 > 0 && cursor_row(s) == "$"
    });
    sh.send("bind -p | grep C-?\r");
    sh.wait_for("readline's binding", |s| {
        has_row(s, "\"\\C-?\": backward-delete-char")
    });
}

/// As above, with the pairing group unbound from a search: `DEL` is
/// readline's again, and inkline lists it no more.
#[test]
fn unbinding_pairing_from_a_search_gives_back_del() {
    let mut sh = Shell::start(Options {
        history: vec!["inkline eval \"(inkline-unbind-defaults 'pairing)\""],
        ..Options::default()
    });
    sh.send("\x12unbind");
    sh.wait_for("the search", |s| {
        cursor_row(s).starts_with("(reverse-i-search)`unbind'")
    });
    sh.send("\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 > 0 && cursor_row(s) == "$"
    });
    sh.send("echo keys=$(inkline keys | grep -c '^DEL '); bind -p | grep C-?\r");
    sh.wait_for("readline's binding", |s| {
        has_row(s, "keys=0") && has_row(s, "\"\\C-?\": backward-delete-char")
    });
}

/// A macro's `DEL` during a search deletes a search character, as a typed
/// one does.
#[test]
fn backspace_in_a_macro_deletes_a_search_character() {
    let mut sh = Shell::start(Options {
        history: vec!["echo hello", "echo help"],
        inputrc: Some("\"\\C-xr\": \"\\C-rhelp\\C-?lo\"\n".into()),
        ..Options::default()
    });
    sh.send("\x18r");
    sh.wait_for("the search going on", |s| {
        cursor_row(s) == "(reverse-i-search)`hello': echo hello"
    });
}

/// A `bind` of `DEL` in `PROMPT_COMMAND`, run after a `C-c` ended a
/// search, stays.
#[test]
fn a_bind_after_a_search_is_interrupted_keeps_its_key() {
    let mut sh = Shell::start(Options {
        history: vec!["echo hello"],
        rc: "PROMPT_COMMAND='(( ++prompts == 2 )) && bind \"\\\"\\\\C-?\\\": backward-delete-char\"'\n"
            .into(),
        ..Options::default()
    });
    sh.send("\x12hel");
    sh.wait_for("the search", |s| {
        cursor_row(s) == "(reverse-i-search)`hel': echo hello"
    });
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| {
        s.cursor_position().0 > 0 && cursor_row(s) == "$"
    });
    sh.send("bind -p | grep C-?\r");
    sh.wait_for("readline's binding", |s| {
        has_row(s, "\"\\C-?\": backward-delete-char")
    });
}
