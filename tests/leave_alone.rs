//! Setups where inkline must leave readline's own drawing alone: the screen has
//! to match plain bash exactly.

#[path = "support/common.rs"]
mod common;

use common::*;

/// Starts the same setup with and without inkline.
fn with_and_without(opts: impl Fn() -> Options) -> (Shell, Shell) {
    let with = Shell::start(opts());
    let plain = Shell::start(Options {
        inkline: false,
        ..opts()
    });
    (with, plain)
}

/// Starts the same setup with and without inkline, types `keys` into both and
/// checks the screens match.
fn same_as_plain(opts: impl Fn() -> Options, keys: &str) {
    let (mut with, mut plain) = with_and_without(opts);
    with.send(keys);
    plain.send(keys);
    wait_same(&with, &plain, &format!("typing {keys:?}"));
}

#[test]
fn show_mode_in_prompt() {
    same_as_plain(
        || Options {
            rc: "bind 'set show-mode-in-prompt on'\n".into(),
            prompt: "@",
            ..Options::default()
        },
        "ls -l \"abc\"",
    );
}

#[test]
fn mark_modified_lines() {
    same_as_plain(
        || Options {
            rc: "bind 'set mark-modified-lines on'\n".into(),
            history: vec!["echo one two"],
            ..Options::default()
        },
        "\x10x",
    );
}

#[test]
fn prompt_escapes_without_brackets() {
    same_as_plain(
        || Options {
            rc: "PS1='\\e[32m$ \\e[0m'\n".into(),
            ..Options::default()
        },
        "ls -l \"abc\"",
    );
}

#[test]
fn terminal_without_cursor_up() {
    same_as_plain(
        || Options {
            term: "no-such-term",
            cols: 30,
            ..Options::default()
        },
        // No keys inkline binds (such as `"`): a bound key ends readline's
        // batched typing, and sideways scrolling depends on the redraws.
        &format!("echo {} end", "b".repeat(44)),
    );
}

#[test]
fn search_highlight_is_kept() {
    let opts = || Options {
        history: vec!["echo hello-world"],
        ..Options::default()
    };
    let (mut with, mut plain) = with_and_without(opts);
    with.send("\x12hel");
    plain.send("\x12hel");
    let w = with.settle();
    let p = plain.settle();
    assert_eq!(cursor_row(&w), cursor_row(&p));
    assert_eq!(
        cell(&w, "hel").map(|c| c.inverse()),
        cell(&p, "hel").map(|c| c.inverse()),
        "search match highlight"
    );
}

#[test]
fn no_internal_error_in_a_non_utf8_locale() {
    let mut sh = Shell::start(Options {
        lang: "C",
        ..Options::default()
    });
    sh.send("echo \u{e9}\x02");
    let s = sh.settle();
    assert!(
        find(&s, "internal error").is_none(),
        "screen:\n{}",
        dump(&s)
    );
}

#[test]
fn non_utf8_locale() {
    same_as_plain(
        || Options {
            lang: "C",
            ..Options::default()
        },
        "echo \u{e9}\x02x",
    );
}

/// Options with `history` and a `show` command, which reads a line with echo
/// off and prints it in brackets.
fn reading_silently(history: &'static [&'static str]) -> impl Fn() -> Options {
    move || Options {
        rc: "show() { read -e -s -p 'pw: ' x; printf '[%s]\\n' \"$x\"; }\n".into(),
        history: history.to_vec(),
        ..Options::default()
    }
}

/// Runs `show`, types `keys` and Enter, and waits for the `n`th printed value.
fn show(sh: &mut Shell, keys: &str, n: usize) {
    sh.send(&format!("show\r{keys}\r"));
    sh.wait_for("the value and the next prompt", |s| {
        let values = (0..s.size().0)
            .filter(|&row| row_text(s, row).starts_with('['))
            .count();
        values == n && cursor_row(s) == "$"
    });
}

/// `read -s` turns off the terminal's echo: readline shows only the prompt, and
/// neither the typed text nor a suggestion may appear. `C-e` only moves to the
/// end, so a history entry cannot complete the hidden text.
#[test]
fn read_silent() {
    same_after(
        reading_silently(&["echo secretword"]),
        |sh| show(sh, "echo secr\x05", 1),
        "reading with echo off",
    );
}

/// While echo is off, brackets, quotes and Backspace edit the text as readline
/// does on its own, so `read -e -s` stores what the user typed.
#[test]
fn read_silent_without_pairing() {
    same_after(
        reading_silently(&[]),
        |sh| {
            // An opener, a quote, a closer before the same closer, and
            // Backspace between an opener and its closer.
            for (n, keys) in (1..).zip(["a(b", "say \"hi", "h)\x02)", "g()\x02\x7f"]) {
                show(sh, keys, n);
            }
        },
        "reading with echo off",
    );
}

/// Options with `rc` and a history entry that suggests after `echo hel`.
fn setup(rc: &'static str) -> impl Fn() -> Options {
    move || Options {
        rc: format!("{rc}\n"),
        history: vec!["echo hello-world"],
        ..Options::default()
    }
}

/// Starts the same setup with and without inkline, runs `steps` in both and
/// checks the screens match.
fn same_after(opts: impl Fn() -> Options, steps: impl Fn(&mut Shell), what: &str) {
    let (mut with, mut plain) = with_and_without(opts);
    steps(&mut with);
    steps(&mut plain);
    wait_same(&with, &plain, what);
}

/// Types `echo hel`, which shows a suggestion with `setup`'s history.
fn type_before_a_signal(sh: &mut Shell) {
    sh.send("echo hel");
    sh.wait_for("the line", |s| find(s, "echo hel").is_some());
    sh.settle();
}

/// Starts a short background job and waits for the next prompt.
fn start_job(sh: &mut Shell) {
    sh.send("sleep 0.3 &\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 == 2 && cursor_row(s) == "$"
    });
}

/// With `set -b`, bash prints a job's notice while waiting for a key, and the
/// cursor is no longer where readline thinks it is; the keys after it must be
/// drawn as readline draws them.
#[test]
fn job_notice_with_set_b() {
    let (mut with, mut plain) = with_and_without(setup("set -b"));
    for sh in [&mut with, &mut plain] {
        start_job(sh);
        // Clears the job number, which differs between the shells.
        sh.send("\x0cecho hel");
    }
    with.wait_for("the notice", |s| find(s, "Done").is_some());
    wait_same(&with, &plain, "the notice");
    with.send("l");
    plain.send("l");
    wait_same(&with, &plain, "a key after the notice");
}

/// A `WINCH` trap runs while readline handles a resize, and what it prints moves
/// the cursor; the keys after it must be drawn as readline draws them.
#[test]
fn resize_with_a_winch_trap() {
    same_after(
        setup("trap 'echo winch-trap' WINCH"),
        |sh| {
            type_before_a_signal(sh);
            sh.resize(24, 60);
            sh.wait_for("the trap", |s| find(s, "winch-trap").is_some());
            sh.send("l");
        },
        "a key after the trap",
    );
}

/// Readline restores the terminal while it handles `SIGQUIT`, and then bash
/// runs the trap, which prints on the same line.
#[test]
fn quit_trap_that_prints() {
    same_after(
        setup("trap 'echo quit-trap' QUIT"),
        |sh| {
            type_before_a_signal(sh);
            sh.send("\x1c");
            sh.wait_for("the trap", |s| find(s, "quit-trap").is_some());
            sh.send("l");
        },
        "a key after the trap",
    );
}

/// After a job notice overwrote the suggestion, the accept keys do what
/// readline's commands do.
#[test]
fn accept_after_a_job_notice() {
    same_after(
        setup("set -b"),
        |sh| {
            start_job(sh);
            // A count keeps the stored suggestion past the redraw.
            sh.send("\x0cecho hel\x1b3");
            sh.wait_for("the notice", |s| find(s, "Done").is_some());
            sh.send("\x06x");
        },
        "C-f after the notice",
    );
}

/// The next line after a job notice is drawn by inkline again.
#[test]
fn coloured_again_after_a_job_notice() {
    let mut sh = Shell::start(Options {
        rc: "set -b\n".into(),
        ..Options::default()
    });
    start_job(&mut sh);
    sh.send("echo x");
    sh.wait_for("the notice", |s| find(s, "Done").is_some());
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    sh.send("ls");
    sh.wait_for("colours", |s| fg_is(s, "ls", Color::Idx(2)));
}

/// A job notice while typing a multi-line command: the rest is drawn as
/// readline draws it.
#[test]
fn job_notice_during_a_multi_line_command() {
    let (mut with, mut plain) = with_and_without(setup("set -b"));
    for sh in [&mut with, &mut plain] {
        start_job(sh);
        sh.send(&format!("\x0cfor x in a; do{LITERAL_NEWLINE}    echo"));
    }
    with.wait_for("the notice", |s| find(s, "Done").is_some());
    wait_same(&with, &plain, "the notice");
    for sh in [&mut with, &mut plain] {
        sh.send(" $x");
    }
    wait_same(&with, &plain, "keys after the notice");
}
