//! Enter adds a line to an unfinished command and runs a finished one;
//! `C-j` adds a line even to a finished command, and Alt+Enter sends the
//! command as it is.

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

/// With pairing, `{` brings its `}`: Enter between them opens the block and
/// puts the `}` on a line of its own.
#[test]
fn enter_between_braces_opens_the_block() {
    let mut sh = Shell::start(Options::default());
    sh.send("f() {\r");
    sh.wait_for("the new line", |s| {
        s.cursor_position() == (1, 4) && row_text(s, 0) == "$ f() {" && row_text(s, 2) == "}"
    });
    sh.send("echo hi\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position() == (3, 2) && row_text(s, 1) == "    echo hi"
    });
    sh.send("f\r");
    sh.wait_for("the output", |s| row_text(s, 4) == "hi");
}

#[test]
fn enter_between_parentheses_opens_the_list() {
    let mut sh = Shell::start(Options::default());
    sh.send("a=(\r");
    sh.wait_for("the new line", |s| {
        s.cursor_position() == (1, 4) && row_text(s, 0) == "$ a=(" && row_text(s, 2) == ")"
    });
    sh.send("x y\r");
    sh.wait_for("the next prompt", |s| s.cursor_position() == (3, 2));
    sh.send("echo ${#a[@]}\r");
    sh.wait_for("the output", |s| row_text(s, 4) == "2");
}

#[test]
fn enter_inside_a_command_substitution_opens_it() {
    let mut sh = Shell::start(Options::default());
    sh.send("x=$(\r");
    sh.wait_for("the new line", |s| {
        s.cursor_position() == (1, 4) && row_text(s, 2) == ")"
    });
}

/// Inside a block that is still open, the closer still goes on a line of its
/// own, so the lines typed next stay inside the pair.
#[test]
fn enter_between_parentheses_inside_a_block_opens_the_list() {
    let mut sh = Shell::start(Options::default());
    sh.send("if true; then\r");
    sh.wait_for("the first new line", |s| s.cursor_position() == (1, 4));
    sh.send("a=(\r");
    sh.wait_for("the list opened", |s| {
        row_text(s, 0) == "$ if true; then"
            && row_text(s, 1) == "    a=("
            && s.cursor_position() == (2, 8)
            && row_text(s, 3) == "    )"
    });
    sh.send("x y");
    sh.wait_for("the words", |s| row_text(s, 2) == "        x y");
    sh.send(TO_THE_CLOSER);
    sh.wait_for("the closer's line", |s| s.cursor_position() == (3, 5));
    sh.send("\r");
    sh.wait_for("the line after the closer", |s| {
        row_text(s, 3) == "    )" && s.cursor_position() == (4, 4)
    });
    sh.send("fi");
    sh.wait_for("the closing word", |s| cursor_row(s).trim() == "fi");
    sh.send("\r");
    sh.wait_for("the next prompt", |s| {
        row_text(s, 3) == "    )" && row_text(s, 4) == "fi" && cursor_row(s) == "$"
    });
    sh.send("echo ${#a[@]}\r");
    sh.wait_for("the output", |s| has_row(s, "2"));
}

/// C-f over the newline and `    )` or `    }`, to the end of the closer's
/// line.
const TO_THE_CLOSER: &str = "\x06\x06\x06\x06\x06\x06";

#[test]
fn enter_on_the_closing_brace_keeps_it_in_place() {
    let mut sh = Shell::start(Options::default());
    sh.send("if true; then\r");
    sh.wait_for("the first new line", |s| s.cursor_position() == (1, 4));
    sh.send("f() {\r");
    sh.wait_for("the block opened", |s| {
        s.cursor_position() == (2, 8) && row_text(s, 3) == "    }"
    });
    sh.send("echo hi");
    sh.wait_for("the body", |s| row_text(s, 2) == "        echo hi");
    sh.send(TO_THE_CLOSER);
    sh.wait_for("the closer's line", |s| s.cursor_position() == (3, 5));
    sh.send("\r");
    sh.wait_for("the line after the closer", |s| {
        row_text(s, 3) == "    }" && s.cursor_position() == (4, 4)
    });
}

/// Enter on the closer's line of a finished command runs it as it is.
#[test]
fn enter_on_the_closer_of_a_finished_command_keeps_it_in_place() {
    let mut sh = Shell::start(Options::default());
    sh.send("f() {\r");
    sh.wait_for("the block opened", |s| s.cursor_position() == (1, 4));
    sh.send("a=(\r");
    sh.wait_for("the list opened", |s| {
        s.cursor_position() == (2, 8) && row_text(s, 3) == "    )"
    });
    sh.send("x");
    sh.wait_for("the word", |s| row_text(s, 2) == "        x");
    sh.send(TO_THE_CLOSER);
    sh.wait_for("the closer's line", |s| s.cursor_position() == (3, 5));
    sh.send("\r");
    sh.wait_for("the next prompt", |s| {
        row_text(s, 3) == "    )" && row_text(s, 4) == "}" && cursor_row(s) == "$"
    });
}

/// A here-document body above the closer does not count as its depth.
#[test]
fn enter_on_the_closer_after_a_here_document_keeps_it_in_place() {
    let mut sh = Shell::start(Options::default());
    sh.send("for x in a; do\r");
    sh.wait_for("the first new line", |s| s.cursor_position() == (1, 4));
    sh.send("f() {\r");
    sh.wait_for("the block opened", |s| {
        s.cursor_position() == (2, 8) && row_text(s, 3) == "    }"
    });
    sh.send("cat <<EOF\r");
    sh.wait_for("the body line", |s| s.cursor_position() == (3, 0));
    sh.send("hi\r");
    sh.wait_for("the terminator line", |s| s.cursor_position() == (4, 0));
    sh.send("EOF");
    sh.wait_for("the terminator", |s| row_text(s, 4) == "EOF");
    sh.send(TO_THE_CLOSER);
    sh.wait_for("the closer's line", |s| s.cursor_position() == (5, 5));
    sh.send("\r");
    sh.wait_for("the line after the closer", |s| {
        row_text(s, 5) == "    }" && s.cursor_position().0 == 6
    });
}

#[test]
fn ctrl_j_on_the_closer_keeps_it_in_place() {
    let mut sh = Shell::start(Options::default());
    sh.send("if true; then\r");
    sh.wait_for("the first new line", |s| s.cursor_position() == (1, 4));
    sh.send("a=(\r");
    sh.wait_for("the list opened", |s| {
        s.cursor_position() == (2, 8) && row_text(s, 3) == "    )"
    });
    sh.send("x");
    sh.wait_for("the word", |s| row_text(s, 2) == "        x");
    sh.send(TO_THE_CLOSER);
    sh.wait_for("the closer's line", |s| s.cursor_position() == (3, 5));
    sh.send(CTRL_J);
    sh.wait_for("the line after the closer", |s| {
        row_text(s, 3) == "    )" && s.cursor_position() == (4, 4)
    });
}

#[test]
fn enter_inside_a_command_substitution_inside_a_loop_opens_it() {
    let mut sh = Shell::start(Options::default());
    sh.send("for f in a; do\r");
    sh.wait_for("the first new line", |s| s.cursor_position() == (1, 4));
    sh.send("x=$(\r");
    sh.wait_for("the substitution opened", |s| {
        row_text(s, 1) == "    x=$(" && s.cursor_position() == (2, 8) && row_text(s, 3) == "    )"
    });
}

#[test]
fn enter_between_braces_inside_a_loop_opens_the_group() {
    let mut sh = Shell::start(Options::default());
    sh.send("while :; do\r");
    sh.wait_for("the first new line", |s| s.cursor_position() == (1, 4));
    sh.send("{\r");
    sh.wait_for("the group opened", |s| {
        row_text(s, 1) == "    {" && s.cursor_position() == (2, 8) && row_text(s, 3) == "    }"
    });
}

#[test]
fn enter_between_parentheses_inside_a_block_with_inkline_indent_0() {
    let mut sh = Shell::start(Options {
        rc: "inkline eval '(setq inkline-indent 0)' >/dev/null\n".into(),
        ..Options::default()
    });
    sh.send("if true; then\r");
    sh.wait_for("the first new line", |s| s.cursor_position() == (1, 0));
    sh.send("a=(\r");
    sh.wait_for("the list opened", |s| {
        row_text(s, 1) == "a=(" && s.cursor_position() == (2, 0) && row_text(s, 3) == ")"
    });
}

/// With pairing, a quote or a pair with text in it is closed already, so
/// Enter runs the command wherever the cursor is.
#[test]
fn enter_inside_a_closed_quote_runs_the_command() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo \"hi\r");
    sh.wait_for("the output", |s| {
        row_text(s, 0) == "$ echo \"hi\"" && row_text(s, 1) == "hi" && cursor_row(s) == "$"
    });
    sh.send("echo \"\r");
    sh.wait_for("the empty output", |s| {
        s.cursor_position().0 == 4 && row_text(s, 3).is_empty() && cursor_row(s) == "$"
    });
}

#[test]
fn enter_inside_a_filled_command_substitution_runs_the_command() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo $(date\r");
    sh.wait_for("the output", |s| {
        row_text(s, 0) == "$ echo $(date)"
            && !row_text(s, 1).is_empty()
            && s.cursor_position() == (2, 2)
    });
}

/// `{` and `[` after a command's name are plain words, so Enter between the
/// pair runs the command.
#[test]
fn enter_between_a_pair_in_an_argument_runs_the_command() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo {\r");
    sh.wait_for("the braces", |s| {
        row_text(s, 0) == "$ echo {}" && row_text(s, 1) == "{}" && cursor_row(s) == "$"
    });
    sh.send("echo [\r");
    sh.wait_for("the brackets", |s| {
        row_text(s, 2) == "$ echo []" && row_text(s, 3) == "[]" && cursor_row(s) == "$"
    });
}

/// When the text before the pair is already wrong, Enter runs the command,
/// so bash reports the error.
#[test]
fn enter_between_braces_after_a_wrong_word_runs_the_command() {
    let mut sh = Shell::start(Options::default());
    sh.send("fi {\r");
    sh.wait_for("bash's error", |s| {
        row_text(s, 0) == "$ fi {}" && find(s, "syntax error near unexpected token").is_some()
    });
}

/// Inside a here-document the pair is text: Enter adds a plain line.
#[test]
fn enter_between_braces_inside_a_here_document_adds_a_plain_line() {
    let mut sh = Shell::start(Options::default());
    sh.send("cat <<EOF\r");
    sh.wait_for("the body line", |s| s.cursor_position() == (1, 0));
    sh.send("f() {\r");
    sh.wait_for("the next body line", |s| {
        row_text(s, 1) == "f() {" && s.cursor_position() == (2, 0) && row_text(s, 2) == "}"
    });
}

/// Only closers after the cursor make an empty pair: with more text after
/// them, Enter runs the command.
#[test]
fn enter_between_parentheses_with_text_after_them_runs_the_command() {
    let mut sh = Shell::start(Options::default());
    sh.send("a=(\x05 && echo x\x01\x06\x06\x06\r");
    sh.wait_for("the output", |s| {
        row_text(s, 0) == "$ a=() && echo x" && row_text(s, 1) == "x" && cursor_row(s) == "$"
    });
}

#[test]
fn one_undo_removes_the_opened_block() {
    let mut sh = Shell::start(Options::default());
    sh.send("f() {\r");
    sh.wait_for("the new line", |s| s.cursor_position() == (1, 4));
    sh.send("\x1f");
    sh.wait_for("the single line", |s| {
        s.cursor_position() == (0, 7) && cursor_row(s) == "$ f() {}" && row_text(s, 1).is_empty()
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

/// `C-j` adds a line to a finished command too.
#[test]
fn ctrl_j_always_adds_a_line() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo one");
    sh.send(CTRL_J);
    sh.wait_for("the new line", |s| s.cursor_position() == (1, 0));
    sh.send("echo two\r");
    sh.wait_for("both outputs", |s| has_row(s, "one") && has_row(s, "two"));
}

/// `C-j` indents the new line as Enter does.
#[test]
fn ctrl_j_adds_an_indented_line() {
    let mut sh = Shell::start(Options::default());
    sh.send("for x in a; do\r");
    sh.wait_for("the new line", |s| s.cursor_position() == (1, 4));
    sh.send(&format!("echo $x{CTRL_J}"));
    sh.wait_for("the next line", |s| s.cursor_position() == (2, 4));
}

#[test]
fn alt_enter_sends_the_command_as_it_is() {
    let mut sh = Shell::start(Options::default());
    sh.send(&format!("for x in a; do{ALT_ENTER}"));
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
    sh.send(&format!("echo \x16'a{ALT_ENTER}"));
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
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 == 1 && cursor_row(s) == "$"
    });
    sh.send("for x in a; do\r");
    sh.wait_for("the continuation prompt", |s| cursor_row(s) == ">");
}

#[test]
fn inkline_indent_sets_the_step() {
    let mut sh = Shell::start(Options {
        rc: "inkline eval '(setq inkline-indent 2)' >/dev/null\n".into(),
        ..Options::default()
    });
    sh.send("for x in a; do\r");
    sh.wait_for("two spaces", |s| s.cursor_position() == (1, 2));
    // Bash acts on C-c only once its read of a key is interrupted, so the
    // next keys go in a write of their own.
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    sh.send("inkline eval '(setq inkline-indent 0)' >/dev/null\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 == 3 && cursor_row(s) == "$"
    });
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

/// `inkline-indent` 0 also leaves a closing word's tab alone.
#[test]
fn no_outdent_with_inkline_indent_0() {
    let mut sh = Shell::start(Options {
        rc: "inkline eval '(setq inkline-indent 0)' >/dev/null\n".into(),
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

/// Keys typed after a `C-c` that arrive in two reads all go to the
/// interrupted line, as with readline's own key reader: none of them start
/// the next line.
#[test]
fn ctrl_c_with_keys_after_it_in_two_reads() {
    let mut sh = Shell::start(Options::default());
    for (first, rest) in [("ech", "o hi"), ("e", "cho hi")] {
        sh.send("ab");
        sh.wait_for("the typing", |s| cursor_row(s) == "$ ab");
        sh.send(&format!("\x03{first}"));
        std::thread::sleep(std::time::Duration::from_millis(300));
        sh.send(&format!("{rest}\r"));
        sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
        let s = sh.settle();
        let split = format!(
            "bash: {}: command not found",
            rest.split(' ').next().unwrap()
        );
        assert!(!has_row(&s, &split), "{}", dump(&s));
    }
}

#[test]
fn ctrl_c_with_keys_after_it_ending_in_insert_newline() {
    let mut sh = Shell::start(Options::default());
    sh.send("for x in a; do\r");
    sh.wait_for("the new line", |s| s.cursor_position() == (1, 4));
    sh.send(&format!("\x03echo hi{CTRL_J}"));
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
}

#[test]
fn alt_hash_comments_every_line() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo a");
    sh.send(CTRL_J);
    sh.send("echo SHOULDNOTRUN");
    sh.send("\x1b#");
    let s = sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 == 2 && cursor_row(s) == "$"
    });
    assert!(has_row(&s, "#echo SHOULDNOTRUN"), "{}", dump(&s));
    assert!(!has_row(&s, "SHOULDNOTRUN"), "{}", dump(&s));
}

/// Where inkline adds no lines, `C-j` accepts the line.
#[test]
fn ctrl_j_in_read_e_finishes_the_read() {
    let mut sh = Shell::start(Options::default());
    sh.send("read -e v\r");
    sh.wait_for("read", |s| s.cursor_position() == (1, 0));
    sh.send(&format!("for y in{CTRL_J}"));
    sh.send("echo \"[$v]\"\r");
    sh.wait_for("the value", |s| has_row(s, "[for y in]"));
}

#[test]
fn ctrl_j_at_the_continuation_prompt_accepts() {
    let mut sh = Shell::start(Options::default());
    sh.send("for x in a b; do");
    sh.send(ALT_ENTER);
    sh.wait_for("the continuation prompt", |s| cursor_row(s) == ">");
    sh.send(&format!("echo $x{CTRL_J}"));
    sh.wait_for("the next continuation prompt", |s| {
        s.cursor_position().0 == 2 && cursor_row(s) == ">"
    });
    sh.send(&format!("done{CTRL_J}"));
    sh.wait_for("the output", |s| has_row(s, "a") && has_row(s, "b"));
}

#[test]
fn ctrl_j_after_inkline_off_accepts() {
    let mut sh = Shell::start(Options::default());
    sh.send("inkline off\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 == 1 && cursor_row(s) == "$"
    });
    sh.send(&format!("for x in a; do{CTRL_J}"));
    sh.wait_for("the continuation prompt", |s| cursor_row(s) == ">");
}

/// A macro's `\n` runs its command, as with readline's own `C-j`.
#[test]
fn a_macro_ending_in_ctrl_j_runs_its_command() {
    let mut sh = Shell::start(Options {
        rc: "bind '\"\\C-xr\": \"echo hi\\n\"'\n".into(),
        ..Options::default()
    });
    sh.send("\x18r");
    sh.wait_for("the output", |s| row_text(s, 1) == "hi");
}

/// Where inkline adds no lines, `insert-newline` accepts the line on any key.
#[test]
fn another_key_accepts_after_inkline_off() {
    let mut sh = Shell::start(Options {
        rc: "bind '\"\\C-xn\": insert-newline'\n".into(),
        ..Options::default()
    });
    sh.send("inkline off\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 == 1 && cursor_row(s) == "$"
    });
    sh.send("for x in a; do\x18n");
    sh.wait_for("the continuation prompt", |s| cursor_row(s) == ">");
}

/// The terminal turns an Enter typed while a command runs into `C-j`, so it
/// adds a line to the next command, as in pasted text.
#[test]
fn enter_typed_while_a_command_runs_adds_a_line() {
    let mut sh = Shell::start(Options::default());
    // `started` shows only once readline has given the terminal back.
    sh.send("echo started; sleep 0.5\r");
    sh.wait_for("the command running", |s| row_text(s, 1) == "started");
    sh.send("echo hi\r");
    sh.wait_for("the next prompt", |s| row_text(s, 3) == "$ echo hi");
    let s = sh.settle();
    assert_eq!(s.cursor_position(), (4, 0), "{}", dump(&s));
    assert!(!has_row(&s, "hi"), "{}", dump(&s));
}
