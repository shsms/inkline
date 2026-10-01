//! The region between the mark and the cursor, active as in Emacs's
//! transient mark mode: `C-SPC` sets the mark, the region is drawn in
//! reverse video, and any other edit or `C-g` ends it.

#[path = "support/common.rs"]
mod common;

use common::*;

const C_SPC: &str = "\x00";
const C_G: &str = "\x07";
const C_X_C_X: &str = "\x18\x18";

/// Whether each cell of `needle`, which must be ASCII, passes `test`.
fn every_cell(screen: &vt100::Screen, needle: &str, test: impl Fn(&vt100::Cell) -> bool) -> bool {
    let Some((row, col)) = find(screen, needle) else {
        return false;
    };
    (0..needle.len() as u16).all(|i| screen.cell(row, col + i).is_some_and(&test))
}

/// Whether any cell of `needle`, which must be ASCII, passes `test`.
fn any_cell(screen: &vt100::Screen, needle: &str, test: impl Fn(&vt100::Cell) -> bool) -> bool {
    let Some((row, col)) = find(screen, needle) else {
        return false;
    };
    (0..needle.len() as u16).any(|i| screen.cell(row, col + i).is_some_and(&test))
}

/// Whether a cell anywhere on the screen passes `test`.
fn any_on_screen(screen: &vt100::Screen, test: impl Fn(&vt100::Cell) -> bool) -> bool {
    let (rows, cols) = screen.size();
    (0..rows).any(|row| (0..cols).any(|col| screen.cell(row, col).is_some_and(&test)))
}

/// The row under the cursor's.
fn below(s: &vt100::Screen) -> String {
    row_text(s, s.cursor_position().0 + 1)
}

/// Types `text` and waits for it on the line. A `C-@` must come in a
/// later read: readline reads the keys after a typed letter itself, to
/// insert them all at once, and loses a `C-@` (a 0 byte) it reads there.
fn type_text(sh: &mut Shell, text: &str) {
    sh.send(text);
    sh.wait_for("the typed text", |s| cursor_row(s) == format!("$ {text}"));
}

/// A shell with `echo hello world` in history, `echo hello` typed and its
/// menu showing.
fn typed_with_menu() -> Shell {
    let mut sh = Shell::start(Options {
        history: vec!["echo hello world"],
        ..Options::default()
    });
    sh.send("echo hello");
    sh.wait_for("the menu", |s| below(s) == "h  echo hello world");
    sh
}

/// `C-SPC` then moves draws the region reversed, with no menu and no grey
/// text.
#[test]
fn ctrl_space_draws_the_region_as_the_cursor_moves() {
    let mut sh = typed_with_menu();
    sh.send(C_SPC);
    sh.send("\x02\x02\x02");
    let s = sh.wait_for("the region", |s| {
        s.cursor_position() == (0, 9) && every_cell(s, "llo", vt100::Cell::inverse)
    });
    assert_eq!(cursor_row(&s), "$ echo hello", "no grey text: {}", dump(&s));
    assert!(
        !any_cell(&s, "echo he", vt100::Cell::inverse),
        "{}",
        dump(&s)
    );
    assert_eq!(below(&s), "", "no menu: {}", dump(&s));
}

/// `C-SPC` again with the mark at the cursor ends the region; the menu
/// comes back.
#[test]
fn ctrl_space_twice_ends_the_region() {
    let mut sh = typed_with_menu();
    sh.send(C_SPC);
    sh.wait_for("no menu", |s| below(s).is_empty());
    sh.send(C_SPC);
    sh.wait_for("the menu again", |s| below(s) == "h  echo hello world");
}

/// `C-x C-x` swaps the cursor and the mark and keeps the region; with no
/// active region it makes it active.
#[test]
fn ctrl_x_ctrl_x_swaps_and_activates() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo hello\x01");
    sh.wait_for("the cursor at the start", |s| s.cursor_position() == (0, 2));
    // Set the mark here, then end the region at once.
    sh.send(&format!("{C_SPC}{C_SPC}\x05"));
    let s = sh.wait_for("the cursor at the end", |s| s.cursor_position() == (0, 12));
    assert!(!any_on_screen(&s, vt100::Cell::inverse), "{}", dump(&s));
    sh.send(C_X_C_X);
    sh.wait_for("swapped, region active", |s| {
        s.cursor_position() == (0, 2) && every_cell(s, "echo hello", vt100::Cell::inverse)
    });
    sh.send(C_X_C_X);
    sh.wait_for("swapped back, still active", |s| {
        s.cursor_position() == (0, 12) && every_cell(s, "echo hello", vt100::Cell::inverse)
    });
    // readline's own `exchange-point-and-mark` highlights only until the
    // next key; this region stays.
    sh.send("\x02");
    sh.wait_for("the region after a move", |s| {
        s.cursor_position() == (0, 11)
            && every_cell(s, "echo hell", vt100::Cell::inverse)
            && !s.cell(0, 11).is_some_and(vt100::Cell::inverse)
    });
}

/// `C-x C-x` after the line got shorter than where the mark was set lands
/// at the end: readline keeps the mark within the line as text is deleted.
/// A mark past the end cannot be made from keys; `region`'s clamps for it
/// are only there to be safe.
#[test]
fn ctrl_x_ctrl_x_after_the_line_got_shorter_lands_at_the_end() {
    let mut sh = Shell::start(Options::default());
    type_text(&mut sh, "abcdef");
    sh.send(&format!("{C_SPC}{C_G}"));
    sh.send("\x7f\x7f\x01");
    sh.wait_for("shorter, cursor at the start", |s| {
        cursor_row(s) == "$ abcd" && s.cursor_position() == (0, 2)
    });
    sh.send(C_X_C_X);
    sh.wait_for("the cursor at the end", |s| {
        s.cursor_position() == (0, 6) && every_cell(s, "abcd", vt100::Cell::inverse)
    });
}

/// A typed letter goes in at the cursor and ends the region.
#[test]
fn typing_ends_the_region() {
    let mut sh = Shell::start(Options::default());
    type_text(&mut sh, "echo hello");
    sh.send(&format!("{C_SPC}\x02\x02"));
    sh.wait_for("the region", |s| every_cell(s, "lo", vt100::Cell::inverse));
    sh.send("X");
    sh.wait_for("the letter", |s| cursor_row(s) == "$ echo helXlo");
    let s = sh.settle();
    assert!(!any_on_screen(&s, vt100::Cell::inverse), "{}", dump(&s));
}

/// `C-g` ends the region and leaves the line and the cursor.
#[test]
fn ctrl_g_ends_the_region_only() {
    let mut sh = Shell::start(Options {
        rc: "bind 'set bell-style audible'\n".into(),
        ..Options::default()
    });
    type_text(&mut sh, "echo hello");
    sh.send(&format!("{C_SPC}\x02\x02"));
    sh.wait_for("the region", |s| every_cell(s, "lo", vt100::Cell::inverse));
    sh.take_output();
    sh.send(C_G);
    let s = sh.wait_for("no region", |s| !any_on_screen(s, vt100::Cell::inverse));
    assert_eq!(cursor_row(&s), "$ echo hello");
    assert_eq!(s.cursor_position(), (0, 10));
    let out = sh.take_output();
    assert!(
        find_bytes(&out, b"\x07").is_none(),
        "no abort's bell: {out:?}"
    );
}

/// Enter runs the line, which stays on screen without the highlight.
#[test]
fn enter_runs_the_line_without_the_region() {
    let mut sh = Shell::start(Options::default());
    type_text(&mut sh, "echo hello");
    sh.send(&format!("{C_SPC}\x01"));
    sh.wait_for("the region", |s| {
        every_cell(s, "echo hello", vt100::Cell::inverse)
    });
    sh.send("\r");
    let s = sh.wait_for("the output", |s| {
        has_row(s, "hello") && cursor_row(s) == "$"
    });
    assert!(!any_on_screen(&s, vt100::Cell::inverse), "{}", dump(&s));
}

/// The line drawn again without the region on Enter shows no grey text
/// either: it would flash before the command's output.
#[test]
fn enter_draws_no_grey_text() {
    let mut sh = Shell::start(Options {
        history: vec!["echo hello world"],
        ..Options::default()
    });
    sh.send("echo hello");
    sh.wait_for("the grey text", |s| cursor_row(s) == "$ echo hello world");
    sh.send("\x01");
    sh.wait_for("the cursor at the start", |s| s.cursor_position() == (0, 2));
    sh.send(&format!("{C_SPC}\x05"));
    sh.wait_for("the region", |s| {
        s.cursor_position() == (0, 12) && every_cell(s, "echo hello", vt100::Cell::inverse)
    });
    sh.take_output();
    sh.send("\r");
    sh.wait_for("the output", |s| {
        has_row(s, "hello") && cursor_row(s) == "$"
    });
    let out = sh.take_output();
    assert!(find_bytes(&out, b"world").is_none(), "{out:?}");
}

/// A new line starts with no region: a region left active on an empty
/// line by `C-c` is gone, so `C-g` on the next line is readline's `abort`,
/// which rings the bell.
#[test]
fn a_new_line_starts_without_the_region() {
    let mut sh = Shell::start(Options {
        rc: "bind 'set bell-style audible'\n".into(),
        ..Options::default()
    });
    sh.wait_for("the prompt", |s| cursor_row(s) == "$");
    // `C-b` at the start rings the bell once readline has read `C-SPC`;
    // a `C-c` before that would throw `C-SPC` away unread.
    sh.take_output();
    sh.send(&format!("{C_SPC}\x02"));
    sh.wait_for_output("C-b's bell", b"\x07");
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| {
        s.cursor_position().0 > 0 && cursor_row(s) == "$"
    });
    sh.take_output();
    sh.send(C_G);
    sh.wait_for_output("abort's bell", b"\x07");
}

/// With inkline off, `C-SPC` is readline's `set-mark`: no region is
/// active, so DEL deletes one character.
#[test]
fn ctrl_space_with_inkline_off_starts_no_region() {
    let mut sh = Shell::start(Options {
        rc: "inkline off\n".into(),
        ..Options::default()
    });
    type_text(&mut sh, "echo hello");
    sh.send(C_SPC);
    sh.send(" world");
    sh.wait_for("the line", |s| cursor_row(s) == "$ echo hello world");
    sh.send("\x7f");
    sh.wait_for("one character gone", |s| {
        cursor_row(s) == "$ echo hello worl"
    });
}

/// The `region` colour can be changed, and an empty one draws nothing.
#[test]
fn the_region_colour_can_be_changed_and_turned_off() {
    let mut sh = Shell::start(Options {
        rc: "inkline eval '(setq inkline-colors (quote ((region . \"underline\"))))' >/dev/null\n"
            .into(),
        ..Options::default()
    });
    type_text(&mut sh, "echo hello");
    sh.send(&format!("{C_SPC}\x02\x02"));
    let s = sh.wait_for("the region underlined", |s| underlined(s, "lo"));
    assert!(!any_on_screen(&s, vt100::Cell::inverse), "{}", dump(&s));
    let mut sh = Shell::start(Options {
        rc: "inkline eval '(setq inkline-colors (quote ((region . \"\"))))' >/dev/null\n".into(),
        ..Options::default()
    });
    type_text(&mut sh, "echo hello");
    sh.send(&format!("{C_SPC}\x02\x02"));
    sh.wait_for("the cursor moved", |s| s.cursor_position() == (0, 10));
    let s = sh.settle();
    assert!(!any_on_screen(&s, vt100::Cell::inverse), "{}", dump(&s));
    assert!(!any_underlined(&s), "{}", dump(&s));
}

/// The highlight covers wide characters whole.
#[test]
fn the_region_covers_wide_characters() {
    let mut sh = Shell::start(Options::default());
    type_text(&mut sh, "echo 日本");
    sh.send(&format!("{C_SPC}\x02\x02"));
    let s = sh.wait_for("the region", |s| {
        s.cursor_position() == (0, 7) && cell(s, "日").is_some_and(|c| c.inverse())
    });
    assert!(cell(&s, "本").is_some_and(|c| c.inverse()), "{}", dump(&s));
    assert!(!any_cell(&s, "echo", vt100::Cell::inverse), "{}", dump(&s));
}
