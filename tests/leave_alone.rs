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
