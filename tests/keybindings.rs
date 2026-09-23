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
