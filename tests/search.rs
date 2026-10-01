//! The line readline's incremental search (`C-r`, `C-s`) finds is drawn in
//! colour, with the match marked, after readline's own search prompt.

#[path = "support/common.rs"]
mod common;

use common::*;

/// What a terminal shows for SGR 90, the default `suggestion` colour.
const GREY: Color = Color::Idx(8);

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

/// Whether every row under the cursor's is blank: no menu and no message.
fn nothing_below(screen: &vt100::Screen) -> bool {
    let (rows, _) = screen.size();
    (screen.cursor_position().0 + 1..rows).all(|row| row_text(screen, row).is_empty())
}

/// Starts a shell with `history` and `rc`, types `keys` and waits until the
/// cursor row reads `row` with `echo` in the command colour.
fn searched(history: Vec<&'static str>, rc: &str, keys: &str, row: &str) -> (Shell, vt100::Screen) {
    let mut sh = Shell::start(Options {
        history,
        rc: rc.into(),
        ..Options::default()
    });
    sh.send(keys);
    let s = sh.wait_for("the found line in colour", |s| {
        cursor_row(s) == row && fg_is(s, "echo", Color::Idx(2))
    });
    (sh, s)
}

/// The found line has its colours and the match is in reverse video; the
/// search prompt stays as readline draws it, with no menu or grey text.
#[test]
fn reverse_search_colours_the_line_and_marks_the_match() {
    let (sh, _) = searched(
        vec!["echo hello"],
        "",
        "\x12hel",
        "(reverse-i-search)`hel': echo hello",
    );
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "(reverse-i-search)`hel': echo hello");
    assert_eq!(fg(&s, "echo"), Color::Idx(2));
    assert!(every_cell(&s, "hel", vt100::Cell::inverse), "{}", dump(&s));
    assert!(!any_cell(&s, "lo", vt100::Cell::inverse), "{}", dump(&s));
    assert!(!any_cell(&s, "echo ", vt100::Cell::inverse), "{}", dump(&s));
    // The prompt is readline's, in no colour.
    assert!(every_cell(&s, "(reverse-i-search)`hel': ", |c| {
        c.fgcolor() == Color::Default && !c.inverse()
    }));
    assert!(!any_on_screen(&s, |c| c.fgcolor() == GREY), "{}", dump(&s));
    assert!(nothing_below(&s), "{}", dump(&s));
}

/// `C-s` searches forward, once the terminal passes it on.
#[test]
fn forward_search_colours_the_line_and_marks_the_match() {
    let (sh, _) = searched(
        vec!["echo hello", "ls", "echo help"],
        "stty -ixon\n",
        "\x12hel\x12\x13",
        "(i-search)`hel': echo hello",
    );
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "(i-search)`hel': echo hello");
    assert!(every_cell(&s, "hel", vt100::Cell::inverse), "{}", dump(&s));
    assert!(!any_cell(&s, "lo", vt100::Cell::inverse), "{}", dump(&s));
    assert!(!any_on_screen(&s, |c| c.fgcolor() == GREY), "{}", dump(&s));
    assert!(nothing_below(&s), "{}", dump(&s));
}

/// Without readline's active region there is nothing to mark.
#[test]
fn no_mark_without_the_active_region() {
    let (sh, _) = searched(
        vec!["echo hello"],
        "bind 'set enable-active-region off'\n",
        "\x12hel",
        "(reverse-i-search)`hel': echo hello",
    );
    let s = sh.settle();
    assert!(!any_on_screen(&s, vt100::Cell::inverse), "{}", dump(&s));
    assert_eq!(fg(&s, "echo"), Color::Idx(2));
}

/// The `search-match` colour marks the match.
#[test]
fn the_match_takes_the_search_match_colour() {
    let (sh, _) = searched(
        vec!["echo hello"],
        "inkline eval '(setq inkline-colors (quote ((search-match . \"underline\"))))' >/dev/null\n",
        "\x12hel",
        "(reverse-i-search)`hel': echo hello",
    );
    let s = sh.wait_for("the match underlined", |s| underlined(s, "hel"));
    assert!(!any_on_screen(&s, vt100::Cell::inverse), "{}", dump(&s));
    assert!(!any_cell(&s, "lo", vt100::Cell::underline), "{}", dump(&s));
}

/// A found entry of more than one line is coloured on each of its rows.
#[test]
fn a_multi_line_entry_is_coloured_on_each_row() {
    let mut sh = Shell::start(Options {
        history: vec!["true one\necho hello"],
        ..Options::default()
    });
    sh.send("\x12hel");
    let s = sh.wait_for("both rows in colour", |s| {
        has_row(s, "(reverse-i-search)`hel': true one")
            && cursor_row(s) == "echo hello"
            && fg_is(s, "true", Color::Idx(2))
            && fg_is(s, "echo", Color::Idx(2))
    });
    assert!(every_cell(&s, "hel", vt100::Cell::inverse), "{}", dump(&s));
    assert!(
        !any_cell(&s, "true one", vt100::Cell::inverse),
        "{}",
        dump(&s)
    );
}

/// Once the search ends, the line is drawn as at any other time.
#[test]
fn the_line_is_drawn_as_before_once_the_search_ends() {
    let (mut sh, _) = searched(
        vec!["echo hello"],
        "",
        "\x12hel",
        "(reverse-i-search)`hel': echo hello",
    );
    // `C-e` ends the search and moves to the end of the line.
    sh.send("\x05");
    let s = sh.wait_for("the line after the prompt", |s| {
        cursor_row(s) == "$ echo hello" && fg_is(s, "echo", Color::Idx(2))
    });
    assert!(!any_on_screen(&s, vt100::Cell::inverse), "{}", dump(&s));
}

/// A non-incremental search (`M-p`) reads its text on a line of its own,
/// which stays as readline draws it, as does the line it finds.
#[test]
fn non_incremental_search_is_left_to_readline() {
    let opts = || Options {
        history: vec!["echo hello"],
        ..Options::default()
    };
    let mut with = Shell::start(opts());
    let mut plain = Shell::start(Options {
        inkline: false,
        ..opts()
    });
    for sh in [&mut with, &mut plain] {
        sh.send("\x1bphel");
    }
    wait_same(&with, &plain, "typing the search text");
    let s = with.screen();
    assert!(
        every_cell(&s, "hel", |c| c.fgcolor() == Color::Default),
        "{}",
        dump(&s)
    );
    for sh in [&mut with, &mut plain] {
        sh.send(ENTER);
    }
    wait_same(&with, &plain, "the search");
    let (w, p) = (with.screen(), plain.screen());
    assert_eq!(
        cell(&w, "hel").map(|c| (c.inverse(), c.fgcolor())),
        cell(&p, "hel").map(|c| (c.inverse(), c.fgcolor())),
    );
}

/// The error underlined on the typed line goes while a search shows it.
#[test]
fn no_error_underline_while_searching() {
    let mut sh = Shell::start(Options::default());
    sh.send("echo hello ) ");
    sh.wait_for("the error underlined", |s| underlined(s, ")"));
    sh.send("\x12hel");
    let s = sh.wait_for("the found line in colour", |s| {
        cursor_row(s) == "(reverse-i-search)`hel': echo hello )" && fg_is(s, "echo", Color::Idx(2))
    });
    assert!(!any_underlined(&s), "{}", dump(&s));
    let s = sh.settle();
    assert!(!any_underlined(&s), "{}", dump(&s));
    assert!(nothing_below(&s), "{}", dump(&s));
}

/// With a prompt of two lines, in colour, readline's search prompt takes the
/// place of its last line, and the found line is drawn after it.
#[test]
fn a_two_line_prompt_gives_way_to_the_search_prompt() {
    let mut sh = Shell::start(Options {
        history: vec!["echo hello"],
        rc: "PS1='\\[\\e[1;34m\\]top\\[\\e[0m\\]\\n\\[\\e[32m\\]>\\[\\e[0m\\] '\n".into(),
        prompt: ">",
        ..Options::default()
    });
    sh.send("\x12hel");
    let s = sh.wait_for("the found line in colour", |s| {
        cursor_row(s) == "(reverse-i-search)`hel': echo hello" && fg_is(s, "echo", Color::Idx(2))
    });
    assert!(every_cell(&s, "hel", vt100::Cell::inverse), "{}", dump(&s));
    assert!(!any_cell(&s, "lo", vt100::Cell::inverse), "{}", dump(&s));
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "(reverse-i-search)`hel': echo hello");
    assert_eq!(fg(&s, "echo"), Color::Idx(2));
}

/// Starts the same setup with and without inkline, and for each of `steps`
/// types it into both and waits until the screens match. Returns both.
fn same_as_plain(opts: impl Fn() -> Options, steps: &[&str]) -> (Shell, Shell) {
    let mut with = Shell::start(opts());
    let mut plain = Shell::start(Options {
        inkline: false,
        ..opts()
    });
    for keys in steps {
        with.send(keys);
        plain.send(keys);
        wait_same(&with, &plain, &format!("typing {keys:?}"));
    }
    (with, plain)
}

/// A newline typed into the search text (`C-v C-j`) is part of readline's
/// search prompt, which readline measures whole, not from its last newline
/// as a prompt of its own: the line is left to readline to draw.
#[test]
fn a_newline_in_the_search_text_is_left_to_readline() {
    same_as_plain(
        || Options {
            history: vec!["true one\necho hello"],
            ..Options::default()
        },
        &["\x12one", "\x16\x0a", "ec"],
    );
}

/// A wide character of the search text that does not fit at the end of a
/// row starts the next one, leaving the last column blank; the line found
/// starts after the search prompt, where readline drew it.
#[test]
fn a_search_prompt_wrapping_a_wide_character() {
    let (with, _plain) = same_as_plain(
        || Options {
            history: vec!["echo 日本語日本語日本語日本語日本語 tail"],
            cols: 40,
            ..Options::default()
        },
        &["\x12日本語日本語日本語日本"],
    );
    let s = with.wait_for("the found line in colour", |s| {
        has_row(s, "本': echo 日本語日本語日本語日本語日本語") && fg_is(s, "echo", Color::Idx(2))
    });
    assert_eq!(find(&s, "echo"), Some((1, 5)), "{}", dump(&s));
}

/// A mode server's notice waits until the search has ended, even when it
/// comes while the search shows the line typing paused on.
#[test]
fn no_mode_server_notice_while_searching() {
    let dir = tempfile::tempdir().unwrap();
    let (quit, gone) = (dir.path().join("quit"), dir.path().join("gone"));
    // A server for `csvm` that never answers, and exits once `quit` exists;
    // and one for `nosuch` that is not there.
    let server = format!(
        "echo 'inkline-mode 1'; until [ -e '{}' ]; do sleep 0.02; done; : > '{}'",
        quit.display(),
        gone.display()
    );
    let mut sh = Shell::start(Options {
        history: vec!["nosuch; csvm a"],
        init_el: Some(format!(
            "(inkline-define-mode 'csvm-mode '(\"/bin/bash\" \"-c\" \"{server}\")) \
             (inkline-define-mode 'nosuch-mode '(\"no-such-program-xyz\")) \
             (setq inkline-command-mode-alist \
               '((\"csvm\" . csvm-mode) (\"nosuch\" . nosuch-mode)))\n"
        )),
        ..Options::default()
    });
    // Typing pauses on the line, for the notice that `nosuch` has no
    // server.
    sh.send("nosuch; csvm a");
    sh.wait_for("the first notice", |s| {
        has_row(s, "inkline: mode nosuch-mode: off (not found)")
    });
    sh.send("\x12");
    sh.wait_for("the search", |s| {
        cursor_row(s) == "(reverse-i-search)`': nosuch; csvm a"
    });
    std::fs::write(&quit, "").unwrap();
    poll(|| gone.exists().then_some(())).expect("the server exits");
    std::thread::sleep(std::time::Duration::from_millis(100));
    sh.send("c");
    sh.wait_for("the search going on", |s| {
        cursor_row(s) == "(reverse-i-search)`c': nosuch; csvm a"
    });
    let notice = "inkline: mode csvm-mode: off (exited)";
    let s = sh.settle();
    assert!(!has_row(&s, notice), "{}", dump(&s));
    assert!(nothing_below(&s), "{}", dump(&s));
    // `C-e` ends the search.
    sh.send("\x05");
    sh.wait_for("the notice", |s| {
        cursor_row(s) == "$ nosuch; csvm a" && has_row(s, notice)
    });
}
