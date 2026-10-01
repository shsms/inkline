#[path = "support/common.rs"]
mod common;

use common::*;

/// Key sequences run from the end of `echo alpha beta gamma`. There is no
/// history, so no suggestion is ever showing and every inkline command must
/// behave like the readline command it replaces.
const SEQUENCES: &[(&str, &str)] = &[
    ("M-DEL", "\x1b\x7f"),
    ("C-w", "\x17"),
    ("C-w C-w C-y M-y", "\x17\x17\x19\x1by"),
    ("C-w C-_", "\x17\x1f"),
    ("C-b C-b C-t", "\x02\x02\x14"),
    ("M-b M-t", "\x1bb\x1bt"),
    ("C-a M-u M-f M-l M-c", "\x01\x1bu\x1bf\x1bl\x1bc"),
    ("C-a M-f C-k", "\x01\x1bf\x0b"),
    (
        "C-a C-f C-f C-@ C-e C-x C-x",
        "\x01\x06\x06\x00\x05\x18\x18",
    ),
    ("M-3 C-b", "\x1b3\x02"),
    ("C-a C-f M-f C-e", "\x01\x06\x1bf\x05"),
    ("C-a Right End", "\x01\x1b[C\x1b[F"),
    ("C-a M-2 C-f", "\x01\x1b2\x06"),
    ("C-e at the end", "\x05"),
    ("DEL", "\x7f"),
    ("M-2 DEL", "\x1b2\x7f"),
    ("C-a C-f DEL", "\x01\x06\x7f"),
    ("C-a M-f M-f C-x DEL", "\x01\x1bf\x1bf\x18\x7f"),
    ("C-a M-f M-- C-x DEL", "\x01\x1bf\x1b-\x18\x7f"),
    ("Home End", "\x1b[H\x1b[F"),
];

#[test]
fn key_sequences_match_plain_bash() {
    let mut with = Shell::start(Options {
        rows: 60,
        history: vec![],
        ..Options::default()
    });
    let mut plain = Shell::start(Options {
        rows: 60,
        inkline: false,
        ..Options::default()
    });
    for (name, keys) in SEQUENCES {
        // The text settles before the keys go out, so readline never sees them
        // in one burst.
        for sh in [&mut with, &mut plain] {
            sh.send("echo alpha beta gamma");
        }
        with.settle();
        plain.settle();
        for sh in [&mut with, &mut plain] {
            sh.send(keys);
        }
        wait_same(&with, &plain, name);
        for sh in [&mut with, &mut plain] {
            sh.send("\x03");
        }
        wait_same(&with, &plain, &format!("C-c after {name}"));
    }
}

/// Counts typed with `C-u` (from `echo alpha beta gamma`), run with
/// inkline's `numeric-argument` and with readline's own `universal-argument`.
const COUNTS: &[(&str, &str)] = &[
    ("C-u C-b", "\x15\x02"),
    ("C-u C-u C-b", "\x15\x15\x02"),
    ("C-u 2 C-u 3", "\x152\x153"),
    ("M-2 C-u 3", "\x1b2\x153"),
    ("C-u 2 C-u C-u x", "\x152\x15\x15x"),
    ("C-u 2 C-u C-u 3 x", "\x152\x15\x153x"),
    ("C-u 2 C-u C-u C-b", "\x152\x15\x15\x02"),
    ("M-2 C-u C-u C-b", "\x1b2\x15\x15\x02"),
    ("C-u 2 C-r al C-u x", "\x152\x12al\x15x"),
    ("C-u 1 2 C-b", "\x1512\x02"),
    ("C-u - 2 C-b", "\x15-2\x02"),
    ("C-u 2 C-b C-u - C-f", "\x152\x02\x15-\x06"),
    ("C-u C-g x", "\x15\x07x"),
    ("C-u (", "\x15("),
    ("M-1 ( M-1 ) C-b C-u DEL", "\x1b1(\x1b1)\x02\x15\x7f"),
    ("M-1 ) C-b C-u )", "\x1b1)\x02\x15)"),
    ("C-u \"", "\x15\""),
    ("C-a C-u C-k", "\x01\x15\x0b"),
];

#[test]
fn numeric_argument_counts_as_universal_argument_does() {
    let mut with = Shell::start(Options {
        rows: 60,
        ..Options::default()
    });
    let mut plain = Shell::start(Options {
        rows: 60,
        inkline: false,
        inputrc: Some("set bind-tty-special-chars off\n\"\\C-u\": universal-argument\n".into()),
        ..Options::default()
    });
    for (name, keys) in COUNTS {
        for sh in [&mut with, &mut plain] {
            sh.send("echo alpha beta gamma");
        }
        with.settle();
        plain.settle();
        for sh in [&mut with, &mut plain] {
            sh.send(keys);
        }
        wait_same(&with, &plain, name);
        for sh in [&mut with, &mut plain] {
            sh.send("\x03");
        }
        wait_same(&with, &plain, &format!("C-c after {name}"));
    }
}

/// While `C-u` after a count's digits waits for its key, the count shows in
/// place of the prompt, as with readline's `universal-argument`.
#[test]
fn c_u_after_digits_shows_the_count_while_it_waits() {
    let mut sh = Shell::start(Options::default());
    sh.send("\x152\x15");
    sh.wait_for("the count", |s| cursor_row(s) == "(arg: 2)");
    sh.send("x");
    sh.wait_for("the count's keys", |s| cursor_row(s) == "$ xx");
    sh.send("\x18\x7f\x15-2\x15");
    sh.wait_for("the count below 0", |s| cursor_row(s) == "(arg: -2)");
}

/// While `C-u` after a count's digits waits for its key, the line shows no
/// suggestion and no menu, as while readline reads a count.
#[test]
fn c_u_after_digits_hides_the_suggestion_while_it_waits() {
    let mut sh = Shell::start(Options {
        history: vec!["abcdef"],
        ..Options::default()
    });
    sh.send("ab");
    sh.wait_for("the suggestion", |s| cursor_row(s) == "$ abcdef");
    sh.send("\x152\x15");
    sh.wait_for("the count", |s| cursor_row(s).starts_with("(arg: 2)"));
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "(arg: 2) ab");
    assert_eq!(row_text(&s, s.cursor_position().0 + 1), "");
}
