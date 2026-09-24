//! A command bash would reject is underlined once typing pauses, never while
//! the word at the cursor is being typed, and only where readline reads a
//! command for bash.

#[path = "support/common.rs"]
mod common;

use common::*;

use std::thread::sleep;
use std::time::Duration;

const END: &[u8] = b"\x1b[?2026l";
const UNDERLINED_PAREN: &[u8] = b"\x1b[4m)";

/// Plain underlines, which the test terminal can see.
fn plain_underline() -> Options {
    Options {
        rc: "INKLINE_COLORS='error=4'\n".into(),
        ..Options::default()
    }
}

/// Waits until the screen is still, then well past the pause, so an underline
/// drawn after the pause is on screen.
fn quiet(sh: &Shell) -> vt100::Screen {
    sh.settle();
    sleep(Duration::from_millis(400));
    sh.settle()
}

/// Asserts that the underline in `out` came in an update of its own, after the
/// update in which readline echoed `echo`, the last key's output.
fn assert_underline_waited(out: &[u8], echo: &[u8]) {
    let shown = String::from_utf8_lossy(out);
    let echoed = find_bytes(out, echo).expect(&shown);
    let key_done = echoed + find_bytes(&out[echoed..], END).expect(&shown);
    let underline = find_bytes(out, UNDERLINED_PAREN).expect(&shown);
    assert!(key_done < underline, "{shown:?}");
}

#[test]
fn a_wrong_command_is_underlined_after_a_pause() {
    let mut sh = Shell::start(plain_underline());
    sh.send("echo ) x");
    sh.wait_for("the underline", |s| underlined(s, ")"));
}

#[test]
fn an_unfinished_command_is_not_underlined() {
    let mut sh = Shell::start(plain_underline());
    sh.send("for x in a; do");
    let s = quiet(&sh);
    assert!(!any_underlined(&s), "{}", dump(&s));
}

#[test]
fn the_word_being_typed_is_not_underlined() {
    let mut sh = Shell::start(plain_underline());
    sh.send("echo a; fi");
    let s = quiet(&sh);
    assert!(!any_underlined(&s), "{}", dump(&s));
    sh.send(" x");
    sh.wait_for("the underline", |s| underlined(s, "fi"));
}

#[test]
fn the_default_underline_is_wavy_and_red() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo ) x");
    sh.wait_for_output("the underline", b"\x1b[4m\x1b[4:3m\x1b[58:5:1m)");
}

#[test]
fn an_empty_error_colour_turns_it_off() {
    let mut sh = Shell::start(Options {
        rc: "INKLINE_COLORS='error='\n".into(),
        ..Options::default()
    });
    sh.send("echo ) x");
    let s = quiet(&sh);
    assert!(!any_underlined(&s), "{}", dump(&s));
}

#[test]
fn not_on_a_continuation_line() {
    let mut sh = Shell::start(plain_underline());
    sh.send("if true\n");
    sh.wait_for("the continuation prompt", |s| cursor_row(s) == ">");
    sh.send("fi x");
    let s = quiet(&sh);
    assert!(!any_underlined(&s), "{}", dump(&s));
}

#[test]
fn not_in_read_e() {
    let mut sh = Shell::start(plain_underline());
    sh.send("read -e v\r");
    sh.wait_for("read", |s| s.cursor_position() == (1, 0));
    sh.send("echo ) x");
    let s = quiet(&sh);
    assert!(!any_underlined(&s), "{}", dump(&s));
}

#[test]
fn the_underline_waits_for_the_pause() {
    let mut sh = Shell::start(plain_underline());
    sh.settle();
    sh.take_output();
    sh.send("echo ) x");
    sh.wait_for_output("the underline", UNDERLINED_PAREN);
    sh.settle();
    // `x` is the last key, and first appears as readline's echo of it.
    assert_underline_waited(&sh.take_output(), b"x");
}

#[test]
fn an_underline_is_forgotten_with_the_line() {
    let mut sh = Shell::start(plain_underline());
    sh.send("echo ) x");
    sh.wait_for("the underline", |s| underlined(s, ")"));
    sh.send("\x15");
    sh.wait_for("the empty line", |s| cursor_row(s) == "$");
    sh.settle();
    sh.take_output();
    // Yanking puts the same line back in one key.
    sh.send("\x19");
    sh.wait_for_output("the underline", UNDERLINED_PAREN);
    sh.settle();
    assert_underline_waited(&sh.take_output(), b"echo ) x");
}

#[test]
fn not_in_read_e_from_prompt_command() {
    let mut sh = Shell::start(Options {
        rc: "INKLINE_COLORS='error=4'\n\
             PROMPT_COMMAND='if [ -n \"$go\" ]; then go=; read -e v; fi'\n"
            .into(),
        ..Options::default()
    });
    sh.send("go=1\r");
    sh.wait_for("read", |s| s.cursor_position() == (1, 0));
    sh.send("echo ) x");
    sh.wait_for("the text", |s| cursor_row(s) == "echo ) x");
    let s = quiet(&sh);
    assert!(!any_underlined(&s), "{}", dump(&s));
    sh.send("\r");
    sh.wait_for("the prompt", |s| cursor_row(s) == "$");
}

#[test]
fn an_empty_substitution_from_pairing_is_not_underlined() {
    let mut sh = Shell::start(plain_underline());
    sh.send("echo $(");
    let s = quiet(&sh);
    assert!(!any_underlined(&s), "{}", dump(&s));
}
