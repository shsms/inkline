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

const C_W: &str = "\x17";
const C_Y: &str = "\x19";

/// `C-w` kills the region; `C-y` yanks it back.
#[test]
fn ctrl_w_kills_the_region() {
    let mut sh = Shell::start(Options::default());
    type_text(&mut sh, "echo hello world");
    sh.send(&format!("{C_SPC}\x1bb\x1bb"));
    sh.wait_for("the region", |s| {
        every_cell(s, "hello world", vt100::Cell::inverse)
    });
    sh.send(C_W);
    let s = sh.wait_for("killed", |s| cursor_row(s) == "$ echo");
    assert_eq!(s.cursor_position(), (0, 7));
    sh.settle();
    assert!(!any_on_screen(&sh.screen(), vt100::Cell::inverse));
    sh.send(&format!("\x01{C_Y}"));
    sh.wait_for("yanked at the start", |s| {
        cursor_row(s) == "$ hello worldecho"
    });
}

/// With no active region, `C-w` kills the word before the cursor, as
/// bash's `unix-word-rubout`.
#[test]
fn ctrl_w_without_a_region_kills_a_word() {
    let mut sh = Shell::start(Options::default());
    sh.send(&format!("echo hello world{C_W}"));
    sh.wait_for("one word gone", |s| cursor_row(s) == "$ echo hello");
}

/// `M-w` copies the region and leaves the line; the region ends.
#[test]
fn meta_w_copies_the_region() {
    let mut sh = Shell::start(Options::default());
    type_text(&mut sh, "echo hello");
    sh.send(&format!("{C_SPC}\x1bb"));
    sh.wait_for("the region", |s| {
        every_cell(s, "hello", vt100::Cell::inverse)
    });
    sh.send("\x1bw");
    sh.wait_for("the region ended", |s| {
        !any_on_screen(s, vt100::Cell::inverse)
    });
    sh.send(&format!("\x05 {C_Y}"));
    sh.wait_for("yanked", |s| cursor_row(s) == "$ echo hello hello");
}

/// `M-w` with no active region rings the bell and changes nothing.
#[test]
fn meta_w_without_a_region_rings_the_bell() {
    let mut sh = Shell::start(Options {
        rc: "bind 'set bell-style audible'\n".into(),
        ..Options::default()
    });
    sh.send("echo hello");
    sh.wait_for("the line", |s| cursor_row(s) == "$ echo hello");
    sh.take_output();
    sh.send("\x1bw");
    sh.wait_for_output("the bell", b"\x07");
    assert_eq!(cursor_row(&sh.settle()), "$ echo hello");
}

/// DEL and `C-d` delete the region without putting it on the kill ring:
/// `C-y` yanks the kill before.
#[test]
fn del_and_ctrl_d_delete_the_region_off_the_kill_ring() {
    for (name, key) in [("DEL", "\x7f"), ("C-d", "\x04")] {
        let mut sh = Shell::start(Options::default());
        // `C-w` puts `x` on the kill ring.
        sh.send(&format!("x{C_W}"));
        type_text(&mut sh, "echo hello world");
        sh.send(&format!("{C_SPC}\x1bb"));
        sh.wait_for(name, |s| every_cell(s, "world", vt100::Cell::inverse));
        sh.send(key);
        // Rows are read without trailing blanks.
        sh.wait_for(name, |s| {
            cursor_row(s) == "$ echo hello" && s.cursor_position() == (0, 13)
        });
        sh.send(C_Y);
        sh.wait_for(name, |s| cursor_row(s) == "$ echo hello x");
    }
}

/// Undo brings back a deleted region.
#[test]
fn undo_brings_a_deleted_region_back() {
    let mut sh = Shell::start(Options::default());
    type_text(&mut sh, "echo hello world");
    sh.send(&format!("{C_SPC}\x1bb\x7f"));
    sh.wait_for("deleted", |s| {
        cursor_row(s) == "$ echo hello" && s.cursor_position() == (0, 13)
    });
    sh.send("\x1f");
    sh.wait_for("back", |s| cursor_row(s) == "$ echo hello world");
}

/// A count other than 1 before DEL or `C-d` deletes that many characters,
/// not the region, as in Emacs.
#[test]
fn a_count_before_del_or_ctrl_d_deletes_characters() {
    for (name, key, left) in [
        ("DEL", "\x7f", "$ echo hellworld"),
        ("C-d", "\x04", "$ echo hello rld"),
    ] {
        let mut sh = Shell::start(Options::default());
        type_text(&mut sh, "echo hello world");
        sh.send(&format!("{C_SPC}\x1bb"));
        sh.wait_for(name, |s| every_cell(s, "world", vt100::Cell::inverse));
        sh.send(&format!("\x1b2{key}"));
        sh.wait_for(name, |s| cursor_row(s) == left);
    }
}

/// With an active but empty region, DEL and `C-d` delete a character, as
/// in Emacs.
#[test]
fn del_and_ctrl_d_on_an_empty_region_delete_a_character() {
    let mut sh = Shell::start(Options::default());
    sh.send("abcd\x02");
    sh.wait_for("the cursor before d", |s| {
        cursor_row(s) == "$ abcd" && s.cursor_position() == (0, 5)
    });
    sh.send(&format!("{C_SPC}\x7f"));
    sh.wait_for("DEL", |s| cursor_row(s) == "$ abd");
    sh.send(&format!("{C_SPC}\x04"));
    sh.wait_for("C-d", |s| cursor_row(s) == "$ ab");
}

const C_J: &str = "\x0a";

/// On a command of two lines, `C-p` and `C-n` stretch the region between
/// the lines and ring the bell at the first and the last instead of
/// walking history; the arrows do the same.
#[test]
fn up_and_down_stay_within_the_command() {
    for (up, down) in [("\x10", "\x0e"), ("\x1b[A", "\x1b[B")] {
        // Up past the first line searches history for entries holding the
        // text before the cursor, `echo o`: `echo old` matches, so without
        // the region Up would replace the line. An entry that does not
        // match, such as `ls`, would ring the bell anyway.
        let mut sh = Shell::start(Options {
            history: vec!["echo old"],
            rc: "bind 'set bell-style audible'\n".into(),
            ..Options::default()
        });
        sh.send(&format!("echo one{C_J}echo two"));
        sh.wait_for("two lines", |s| s.cursor_position() == (1, 8));
        sh.send(C_SPC);
        sh.send(up);
        // The goal column, 8, counts the prompt on the first line:
        // `$ echo o|ne`.
        sh.wait_for("the region over both lines", |s| {
            s.cursor_position() == (0, 8)
                && every_cell(s, "two", vt100::Cell::inverse)
                && !every_cell(s, "echo one", vt100::Cell::inverse)
        });
        sh.take_output();
        sh.send(up);
        sh.wait_for_output("the bell at the first line", b"\x07");
        let s = sh.settle();
        assert!(has_row(&s, "$ echo one"), "no history walk: {}", dump(&s));
        assert!(every_cell(&s, "two", vt100::Cell::inverse), "{}", dump(&s));
        sh.send(down);
        sh.wait_for("back on the last line", |s| s.cursor_position() == (1, 8));
        sh.take_output();
        sh.send(down);
        sh.wait_for_output("the bell at the last line", b"\x07");
        assert!(has_row(&sh.settle(), "$ echo one"));
    }
}
