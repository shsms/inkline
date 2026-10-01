//! Each keystroke's output is one synchronized update (DEC private mode 2026),
//! so terminals that support it show a single frame per key instead of the
//! erased suggestion and readline's plain echo in between.

#[path = "support/common.rs"]
mod common;

use common::*;

#[test]
fn a_keystroke_is_one_update() {
    let mut sh = Shell::start(Options {
        history: vec!["git status"],
        ..Options::default()
    });
    sh.send("git st");
    sh.wait_for("the suggestion", |s| cursor_row(s) == "$ git status");
    sh.settle();
    sh.take_output();
    sh.send("a");
    sh.wait_for("the next suggestion", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 9)
    });
    sh.settle();
    let out = sh.take_output();
    let shown = String::from_utf8_lossy(&out).into_owned();
    assert!(out.starts_with(BEGIN_UPDATE), "{shown:?}");
    assert!(out.ends_with(END_UPDATE), "{shown:?}");
    assert_eq!(
        (
            count_bytes(&out, BEGIN_UPDATE),
            count_bytes(&out, END_UPDATE)
        ),
        (1, 1),
        "{shown:?}"
    );
}

#[test]
fn the_update_ends_before_the_command_runs() {
    let mut sh = Shell::start(Options {
        history: vec!["echo hello-world"],
        ..Options::default()
    });
    sh.send("echo hel");
    sh.wait_for("the suggestion", |s| cursor_row(s) == "$ echo hello-world");
    sh.settle();
    sh.take_output();
    sh.send("\r");
    sh.wait_for("the output", |s| has_row(s, "hel"));
    sh.settle();
    let out = sh.take_output();
    let shown = String::from_utf8_lossy(&out).into_owned();
    let end = find_bytes(&out, END_UPDATE).expect(&shown);
    let output = find_bytes(&out, b"hel\r\n").expect(&shown);
    assert!(end < output, "{shown:?}");
    assert_eq!(
        count_bytes(&out, BEGIN_UPDATE),
        count_bytes(&out, END_UPDATE),
        "{shown:?}"
    );
}

/// A `bind -x` command runs in the middle of readline's key handling. Its
/// output must not be held back until the key is done, however long it runs.
#[test]
fn bind_x_output_is_not_held_back() {
    let mut sh = Shell::start(Options {
        rc: "bind -x '\"\\C-t\": echo bound-marker'\n".into(),
        ..Options::default()
    });
    sh.send("ls");
    sh.wait_for("the line", |s| fg_is(s, "ls", Color::Idx(2)));
    sh.settle();
    sh.take_output();
    sh.send("\x14");
    sh.wait_for("the output", |s| has_row(s, "bound-marker"));
    sh.settle();
    assert_no_update_open_at(&sh.take_output(), b"bound-marker\r\n");
}

/// Asserts that every update opened in `out` before `marker` was closed.
fn assert_no_update_open_at(out: &[u8], marker: &[u8]) {
    let shown = String::from_utf8_lossy(out);
    let before = &out[..find_bytes(out, marker).expect(&shown)];
    assert_eq!(
        count_bytes(before, BEGIN_UPDATE),
        count_bytes(before, END_UPDATE),
        "{shown:?}"
    );
}

/// A `WINCH` trap runs while readline handles a resize. Its output must not be
/// held back.
#[test]
fn winch_trap_output_is_not_held_back() {
    let mut sh = Shell::start(Options {
        rc: "trap 'echo winch-trap' WINCH\n".into(),
        ..Options::default()
    });
    sh.send("ls");
    sh.wait_for("the line", |s| fg_is(s, "ls", Color::Idx(2)));
    sh.settle();
    sh.take_output();
    sh.resize(24, 60);
    sh.wait_for_output("the trap", b"winch-trap");
    sh.settle();
    assert_no_update_open_at(&sh.take_output(), b"winch-trap");
}

/// What `term` shows, its rows joined by newlines.
fn shown(term: &Term) -> String {
    term.screen().contents()
}

/// A frame split across reads at any two points is shown whole or not at
/// all.
#[test]
fn the_harness_never_shows_part_of_a_frame() {
    let old = b"old-line";
    let frame = [BEGIN_UPDATE, b"\x1b[H\x1b[2Jnew-one\r\nnew-two", END_UPDATE].concat();
    let mut whole = Term::new(4, 20);
    whole.process(old);
    let old_screen = shown(&whole);
    whole.process(&frame);
    let new_screen = shown(&whole);
    for i in 0..=frame.len() {
        for j in i..=frame.len() {
            let mut term = Term::new(4, 20);
            term.process(old);
            for read in [&frame[..i], &frame[i..j], &frame[j..]] {
                term.process(read);
                let screen = shown(&term);
                assert!(
                    screen == old_screen || screen == new_screen,
                    "split at {i} and {j}: {screen:?}"
                );
            }
            assert_eq!(shown(&term), new_screen, "split at {i} and {j}");
        }
    }
}

/// Text read with the end of an update is shown at once.
#[test]
fn the_harness_shows_text_after_the_end_at_once() {
    let mut term = Term::new(4, 20);
    term.process(&[BEGIN_UPDATE, b"held"].concat());
    assert_eq!(shown(&term), "");
    term.process(&[END_UPDATE, b"-then-more"].concat());
    assert_eq!(shown(&term), "held-then-more");
}

/// A second begin inside an update changes nothing, and the first end
/// closes the update.
#[test]
fn the_harness_ignores_a_nested_begin() {
    let mut term = Term::new(4, 20);
    term.process(b"before");
    term.process(&[BEGIN_UPDATE, b"-one"].concat());
    term.process(&[BEGIN_UPDATE, b"-two"].concat());
    assert_eq!(shown(&term), "before");
    term.process(END_UPDATE);
    assert_eq!(shown(&term), "before-one-two");
    term.process(b"-three");
    assert_eq!(shown(&term), "before-one-two-three");
}

/// An update left open stops hiding the screen after `UPDATE_TIMEOUT`.
#[test]
fn the_harness_shows_an_update_left_open_after_the_timeout() {
    let mut term = Term::new(4, 20);
    term.process(&[BEGIN_UPDATE, b"held"].concat());
    assert_eq!(shown(&term), "");
    std::thread::sleep(UPDATE_TIMEOUT);
    assert_eq!(shown(&term), "held");
}
