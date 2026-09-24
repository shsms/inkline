//! Enter adds a line to an unfinished command and runs a finished one;
//! M-Enter always adds a line.

#[path = "support/common.rs"]
mod common;

use common::*;

#[test]
fn enter_on_an_unfinished_command_adds_an_indented_line() {
    let mut sh = Shell::start(Options::default());
    sh.send("for x in a b; do\r");
    sh.wait_for("the new line", |s| {
        s.cursor_position() == (1, 4) && row_text(s, 0) == "$ for x in a b; do"
    });
    sh.send("echo $x\r");
    sh.wait_for("the next line", |s| {
        s.cursor_position() == (2, 4) && row_text(s, 1) == "    echo $x"
    });
    sh.send("done\r");
    sh.wait_for("the output", |s| {
        row_text(s, 2) == "done" && has_row(s, "a") && has_row(s, "b")
    });
}

#[test]
fn enter_runs_a_wrong_command() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo )\r");
    sh.wait_for("bash's error", |s| {
        find(s, "syntax error near unexpected token").is_some()
    });
}

/// Without pairing, `{` comes alone and Enter adds a line as for any
/// unfinished command.
#[test]
fn enter_after_an_unpaired_brace() {
    let mut sh = Shell::start(Options {
        rc: "bind '\"{\": self-insert'\nbind '\"(\": self-insert'\n".into(),
        ..Options::default()
    });
    sh.send("f() {\r");
    sh.wait_for("the new line", |s| {
        s.cursor_position() == (1, 4) && row_text(s, 2).is_empty()
    });
    sh.send("echo hi\r}\r");
    sh.wait_for("the next prompt", |s| s.cursor_position() == (3, 2));
    sh.send("f\r");
    sh.wait_for("the output", |s| row_text(s, 4) == "hi");
}

#[test]
fn alt_enter_always_adds_a_line() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo one");
    sh.send(ALT_ENTER);
    sh.wait_for("the new line", |s| s.cursor_position() == (1, 0));
    sh.send("echo two\r");
    sh.wait_for("both outputs", |s| has_row(s, "one") && has_row(s, "two"));
}

#[test]
fn ctrl_j_sends_the_command_as_it_is() {
    let mut sh = Shell::start(Options::default());
    sh.send("for x in a; do\n");
    sh.wait_for("the continuation prompt", |s| cursor_row(s) == ">");
}

#[test]
fn one_undo_removes_the_newline_and_indentation() {
    let mut sh = Shell::start(Options::default());
    sh.send("for x in a; do\r");
    sh.wait_for("the new line", |s| s.cursor_position() == (1, 4));
    sh.send("\x1f");
    sh.wait_for("the single line", |s| {
        s.cursor_position() == (0, 16) && row_text(s, 1).is_empty()
    });
}

#[test]
fn pasted_text_keeps_its_own_indentation() {
    let mut sh = Shell::start(Options {
        rc: "bind 'set enable-bracketed-paste off'\n".into(),
        ..Options::default()
    });
    sh.send("for x in a b; do\r  echo $x\rdone\r");
    let s = sh.wait_for("the output", |s| has_row(s, "a") && has_row(s, "b"));
    assert!(has_row(&s, "  echo $x"), "{}", dump(&s));
}

#[test]
fn enter_at_the_continuation_prompt_accepts() {
    let mut sh = Shell::start(Options::default());
    // C-v keeps `'` from being paired.
    sh.send("echo \x16'a\n");
    sh.wait_for("the continuation prompt", |s| cursor_row(s) == ">");
    sh.send("b'\r");
    sh.wait_for("the output", |s| has_row(s, "a") && has_row(s, "b"));
}

#[test]
fn stops_adding_lines_before_the_screen_is_full() {
    let mut sh = Shell::start(Options {
        rows: 4,
        ..Options::default()
    });
    sh.send("if true; then\r");
    sh.wait_for("line 2", |s| s.cursor_position() == (1, 4));
    sh.send("a\r");
    sh.wait_for("line 3", |s| s.cursor_position() == (2, 4));
    sh.send("b\r");
    sh.wait_for("line 4", |s| s.cursor_position() == (3, 4));
    sh.send("c\r");
    sh.wait_for("bash's continuation prompt", |s| cursor_row(s) == ">");
}

#[test]
fn plain_enter_with_horizontal_scroll_mode() {
    let mut sh = Shell::start(Options {
        rc: "bind 'set horizontal-scroll-mode on'\n".into(),
        ..Options::default()
    });
    sh.send("for x in a; do\r");
    sh.wait_for("the continuation prompt", |s| cursor_row(s) == ">");
}

#[test]
fn plain_enter_in_read_e() {
    let mut sh = Shell::start(Options::default());
    sh.send("read -e v\r");
    sh.wait_for("read", |s| s.cursor_position() == (1, 0));
    sh.send("for y in\r");
    sh.send("echo \"[$v]\"\r");
    sh.wait_for("the value", |s| has_row(s, "[for y in]"));
}

#[test]
fn plain_enter_after_inkline_off() {
    let mut sh = Shell::start(Options::default());
    sh.send("inkline off\r");
    sh.wait_for("the next prompt", |s| s.cursor_position().0 == 1);
    sh.send("for x in a; do\r");
    sh.wait_for("the continuation prompt", |s| cursor_row(s) == ">");
}

#[test]
fn inkline_indent_sets_the_step() {
    let mut sh = Shell::start(Options {
        rc: "INKLINE_INDENT=2\n".into(),
        ..Options::default()
    });
    sh.send("for x in a; do\r");
    sh.wait_for("two spaces", |s| s.cursor_position() == (1, 2));
    // Bash acts on C-c only once its read of a key is interrupted, so the
    // next keys go in a write of their own.
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    sh.send("INKLINE_INDENT=0\r");
    sh.wait_for("the next prompt", |s| s.cursor_position().0 == 3);
    sh.send("for x in a; do\r");
    sh.wait_for("no indentation", |s| {
        let (row, col) = s.cursor_position();
        col == 0 && row_text(s, row.saturating_sub(1)).ends_with("do")
    });
}

/// A 500-line paste, taller than the screen, leaves typing responsive.
#[test]
fn a_long_paste_stays_responsive() {
    let mut sh = Shell::start(Options::default());
    let body: String = (0..500)
        .map(|i| format!("    echo \"line {i}\" | tr a b\n"))
        .collect();
    sh.send(&format!("\x1b[200~for x in 1; do\n{body}\x1b[201~"));
    sh.settle();
    let start = std::time::Instant::now();
    sh.send("done");
    // Readline cannot draw a command taller than the screen: the word goes
    // over what the row showed.
    sh.wait_for("the typed word", |s| {
        let (row, col) = s.cursor_position();
        s.contents_between(row, 0, row, col).ends_with("done")
    });
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

/// `C-c` puts the next prompt below the whole block, as plain bash does.
#[test]
fn ctrl_c_matches_plain_bash() {
    let mut with = Shell::start(Options::default());
    let mut plain = Shell::start(Options {
        inkline: false,
        ..Options::default()
    });
    with.send("for x in a; do\r");
    with.wait_for("the new line", |s| s.cursor_position() == (1, 4));
    with.send("echo");
    plain.send(&format!("for x in a; do{LITERAL_NEWLINE}    echo"));
    wait_same(&with, &plain, "typing");
    with.send("\x03");
    plain.send("\x03");
    wait_same(&with, &plain, "C-c");
}

/// A comment ends with its line: the next line is indented, and a closing
/// word after a comment moves out.
#[test]
fn a_comment_does_not_stop_indentation() {
    let mut sh = Shell::start(Options::default());
    sh.send("for x in a; do # c\r");
    sh.wait_for("the new line", |s| s.cursor_position() == (1, 4));
    sh.send("echo $x # c\r");
    sh.wait_for("the next line", |s| s.cursor_position() == (2, 4));
    sh.send("done\r");
    sh.wait_for("the output", |s| {
        row_text(s, 2) == "done" && has_row(s, "a")
    });
}

/// A closing word that already moved out stays where it is on a second Enter.
#[test]
fn a_closing_word_moves_out_once() {
    let mut sh = Shell::start(Options::default());
    sh.send("while :; do\r");
    sh.wait_for("the first new line", |s| s.cursor_position() == (1, 4));
    sh.send("if x; then\r");
    sh.wait_for("the second new line", |s| s.cursor_position() == (2, 8));
    sh.send("y\r");
    sh.wait_for("the third new line", |s| s.cursor_position() == (3, 8));
    sh.send("fi\r");
    sh.wait_for("the closing word moved out", |s| {
        row_text(s, 3) == "    fi" && s.cursor_position() == (4, 4)
    });
    // C-b back to the end of the `fi` line.
    sh.send("\x02\x02\x02\x02\x02");
    sh.wait_for("the end of the fi line", |s| s.cursor_position() == (3, 6));
    sh.send("\r");
    sh.wait_for("the new line", |s| {
        row_text(s, 3) == "    fi" && s.cursor_position() == (4, 4)
    });
}

/// Without pairing, a `)` typed on a line of its own moves out, and the next
/// line keeps its indentation.
#[test]
fn a_closing_parenthesis_typed_by_hand() {
    let mut sh = Shell::start(Options {
        rc: "bind '\"(\": self-insert'\n".into(),
        ..Options::default()
    });
    sh.send("if true; then\r");
    sh.wait_for("the first new line", |s| s.cursor_position() == (1, 4));
    sh.send("a=(\r");
    sh.wait_for("the list opened", |s| s.cursor_position() == (2, 8));
    sh.send("x y\r");
    sh.wait_for("the next word line", |s| s.cursor_position() == (3, 8));
    sh.send(")\r");
    sh.wait_for("the list closed", |s| {
        row_text(s, 3) == "    )" && s.cursor_position() == (4, 4)
    });
    sh.send("fi\r");
    sh.wait_for("the next prompt", |s| {
        row_text(s, 4) == "fi" && cursor_row(s) == "$"
    });
    sh.send("echo ${#a[@]}\r");
    sh.wait_for("the output", |s| has_row(s, "2"));
}

#[test]
fn a_subshell_closed_by_hand() {
    let mut sh = Shell::start(Options {
        rc: "bind '\"(\": self-insert'\n".into(),
        ..Options::default()
    });
    sh.send("if true; then\r");
    sh.wait_for("the first new line", |s| s.cursor_position() == (1, 4));
    sh.send("(\r");
    sh.wait_for("the subshell opened", |s| s.cursor_position() == (2, 8));
    sh.send("echo in\r");
    sh.wait_for("the next line", |s| s.cursor_position() == (3, 8));
    sh.send(")\r");
    sh.wait_for("the subshell closed", |s| {
        row_text(s, 3) == "    )" && s.cursor_position() == (4, 4)
    });
}

#[test]
fn no_indentation_inside_a_string() {
    let mut sh = Shell::start(Options::default());
    sh.send("for x in a; do\r");
    sh.wait_for("the new line", |s| s.cursor_position() == (1, 4));
    // C-v keeps `"` from being paired.
    sh.send("echo \x16\"a\r");
    sh.wait_for("the line in the string", |s| s.cursor_position() == (2, 0));
}

#[test]
fn no_indentation_inside_a_here_document() {
    let mut sh = Shell::start(Options::default());
    sh.send("for x in a; do\r");
    sh.wait_for("the new line", |s| s.cursor_position() == (1, 4));
    sh.send("cat <<EOF\r");
    sh.wait_for("the body line", |s| s.cursor_position() == (2, 0));
}

/// `INKLINE_INDENT=0` also leaves a closing word's tab alone.
#[test]
fn no_outdent_with_inkline_indent_0() {
    let mut sh = Shell::start(Options {
        rc: "INKLINE_INDENT=0\n".into(),
        ..Options::default()
    });
    sh.send("for x in a; do\r");
    sh.wait_for("the new line", |s| s.cursor_position() == (1, 0));
    sh.send(&format!("{LITERAL_TAB}done\r"));
    sh.wait_for("the next prompt", |s| cursor_row(s) == "$");
    sh.send("fc -ln -1 | cat -A\r");
    let s = sh.wait_for("the command", |s| find(s, "done$").is_some());
    assert!(find(&s, "^Idone$").is_some(), "{}", dump(&s));
}

/// A closing word in pasted text keeps its spacing, as the paste is still
/// coming in when its Enter runs.
#[test]
fn pasted_closing_word_keeps_its_spacing() {
    let mut sh = Shell::start(Options {
        rc: "bind 'set enable-bracketed-paste off'\n".into(),
        ..Options::default()
    });
    sh.send("if true; then\r    echo a\r    fi\recho b\r");
    let s = sh.wait_for("the output", |s| has_row(s, "a") && has_row(s, "b"));
    assert!(has_row(&s, "    fi"), "{}", dump(&s));
}

/// A `C-c` that arrives with more keys ends the block at the next Enter, as
/// bash acts on it only once the line is accepted.
#[test]
fn ctrl_c_with_keys_after_it() {
    let mut sh = Shell::start(Options::default());
    sh.send("for x in a; do\r");
    sh.wait_for("the new line", |s| s.cursor_position() == (1, 4));
    sh.send("\x03echo hi\r");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
}
