#[path = "support/common.rs"]
mod common;

use common::*;

fn typed(opts: Options, keys: &str) -> Shell {
    let mut sh = Shell::start(opts);
    sh.send(keys);
    sh
}

#[test]
fn colours_each_kind() {
    let sh = typed(Options::default(), "ls -la \"x\" | grep y # c");
    let s = sh.wait_for("the whole line in colour", |s| {
        cursor_row(s) == "$ ls -la \"x\" | grep y # c" && cell(s, "# c").is_some_and(|c| c.dim())
    });
    assert_eq!(fg(&s, "ls"), Color::Idx(2));
    assert_eq!(fg(&s, "-la"), Color::Idx(6));
    assert_eq!(fg(&s, "\"x\""), Color::Idx(3));
    assert!(cell(&s, "|").unwrap().bold());
    assert_eq!(fg(&s, "grep"), Color::Idx(2));
    assert_eq!(fg(&s, "y "), Color::Default);
    assert!(cell(&s, "# c").unwrap().dim());
}

#[test]
fn keywords_and_variables() {
    let sh = typed(Options::default(), "for x in $HOME; do");
    let s = sh.wait_for("the whole line in colour", |s| {
        cursor_row(s) == "$ for x in $HOME; do" && fg_is(s, "do", Color::Idx(5))
    });
    assert_eq!(fg(&s, "$HOME"), Color::Idx(4));
    assert_eq!(fg(&s, "for"), Color::Idx(5));
    assert_eq!(fg(&s, "in"), Color::Idx(5));
    assert_eq!(fg(&s, "do"), Color::Idx(5));
}

#[test]
fn unknown_commands_are_red() {
    let sh = typed(Options::default(), "nosuchcmdxyz");
    sh.wait_for("red", |s| fg_is(s, "nosuchcmdxyz", Color::Idx(1)));
}

/// Only bash can tell what a quoted command name runs, so it is not marked
/// unknown, not even the part before the quote.
#[test]
fn quoted_command_names_are_not_red() {
    let sh = typed(Options::default(), "l's' x");
    let s = sh.wait_for("the whole line in colour", |s| {
        cursor_row(s) == "$ l's' x" && fg_is(s, "'s'", Color::Idx(3))
    });
    assert_eq!(fg(&s, "l's'"), Color::Idx(2));
}

#[test]
fn functions_aliases_and_builtins_are_known() {
    let opts = Options {
        rc: "myfn() { :; }\nalias myal=ls\n".into(),
        ..Options::default()
    };
    let sh = typed(opts, "myfn; myal; cd");
    let s = sh.wait_for("colours", |s| fg_is(s, "cd", Color::Idx(2)));
    assert_eq!(fg(&s, "myfn"), Color::Idx(2));
    assert_eq!(fg(&s, "myal"), Color::Idx(2));
}

#[test]
fn inkline_colors_overrides_defaults() {
    let opts = Options {
        rc: "inkline eval '(setq inkline-colors \"command=35\")' >/dev/null\n".into(),
        ..Options::default()
    };
    let sh = typed(opts, "ls -l");
    let s = sh.wait_for("the whole line in colour", |s| {
        cursor_row(s) == "$ ls -l" && fg_is(s, "-l", Color::Idx(6))
    });
    assert_eq!(fg(&s, "ls"), Color::Idx(5));
}

#[test]
fn off_and_on() {
    let mut sh = typed(Options::default(), "inkline off\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 == 1 && cursor_row(s) == "$"
    });
    sh.send("ls");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ ls");
    assert_eq!(fg(&s, "ls"), Color::Default);
    sh.send("\x15inkline on\r");
    sh.wait_for("the next prompt", |s| {
        s.cursor_position().0 == 2 && cursor_row(s) == "$"
    });
    sh.send("ls");
    sh.wait_for("colours", |s| fg_is(s, "ls", Color::Idx(2)));
}

#[test]
fn wrapped_line_matches_plain_bash() {
    let line = format!("echo {} \"wrapped\"", "a".repeat(25));
    let with = typed(
        Options {
            cols: 30,
            ..Options::default()
        },
        &line,
    );
    let plain = typed(
        Options {
            cols: 30,
            inkline: false,
            ..Options::default()
        },
        &line,
    );
    let s = with.wait_for("colours", |s| fg_is(s, "\"wrapped\"", Color::Idx(3)));
    assert_eq!(fg(&s, "echo"), Color::Idx(2));
    wait_same(&with, &plain, "typing a wrapped line");
}

/// Readline redraws the line itself after a resize; inkline repaints the
/// colours and the suggestion straight after, without waiting for a key.
#[test]
fn colours_survive_a_resize() {
    let mut sh = typed(
        Options {
            history: vec!["ls \"abc\" -l"],
            ..Options::default()
        },
        "ls \"abc\"",
    );
    sh.wait_for("colours", |s| {
        cursor_row(s) == "$ ls \"abc\" -l" && fg_is(s, "\"abc\"", Color::Idx(3))
    });
    sh.settle();
    sh.take_output();
    sh.resize(24, 40);
    sh.wait_for_output("readline's redraw", b"$ ls \"abc\"");
    let s = sh.wait_for("colours after the resize", |s| {
        cursor_row(s) == "$ ls \"abc\" -l"
            && fg_is(s, "ls", Color::Idx(2))
            && fg_is(s, "\"abc\"", Color::Idx(3))
            && fg_is(s, " -l", Color::Idx(8))
    });
    assert_eq!(s.cursor_position(), (0, 10));
}

#[test]
fn colored_multiline_prompt() {
    let opts = Options {
        rc: "PS1='\\[\\e[1;34m\\]top\\[\\e[0m\\]\\n\\[\\e[32m\\]>\\[\\e[0m\\] '\n".into(),
        prompt: ">",
        ..Options::default()
    };
    let sh = typed(opts, "ls -l");
    let s = sh.wait_for("colours", |s| fg_is(s, "-l", Color::Idx(6)));
    assert_eq!(cursor_row(&s), "> ls -l");
    assert_eq!(find(&s, "ls").map(|(_, col)| col), Some(2));
    assert_eq!(fg(&s, "ls"), Color::Idx(2));
}

#[test]
fn wide_characters() {
    let sh = typed(Options::default(), "echo 日本 \"x\"");
    let s = sh.wait_for("colours", |s| fg_is(s, "\"x\"", Color::Idx(3)));
    assert_eq!(cursor_row(&s), "$ echo 日本 \"x\"");
    assert_eq!(fg(&s, "echo"), Color::Idx(2));
}

#[test]
fn control_characters_left_plain() {
    // C-v C-a inserts a literal ^A, which readline draws as two cells.  (C-q
    // would be taken by the terminal's flow control.)
    let mut with = typed(Options::default(), "echo a\x16\x01b");
    let mut plain = typed(
        Options {
            inkline: false,
            ..Options::default()
        },
        "echo a\x16\x01b",
    );
    wait_same(&with, &plain, "typing a control character");
    with.send("c");
    plain.send("c");
    wait_same(&with, &plain, "typing after a control character");
}

/// `C-o` on a line from history fills the next prompt with the following entry
/// before any key is typed.
#[test]
fn line_filled_in_by_ctrl_o_is_coloured() {
    let mut sh = Shell::start(Options {
        history: vec!["echo one", "echo two"],
        ..Options::default()
    });
    sh.send("\x10\x10");
    sh.wait_for("the older entry", |s| cursor_row(s) == "$ echo one");
    sh.send("\x0f");
    sh.wait_for("the next entry, coloured", |s| {
        s.cursor_position().0 >= 2
            && cursor_row(s) == "$ echo two"
            && fg_is(s, "echo two", Color::Idx(2))
    });
}

#[test]
fn line_filled_in_by_read_i_is_coloured() {
    let mut sh = Shell::start(Options::default());
    sh.send("read -e -i 'ls -l' line\r");
    sh.wait_for("the filled-in line, coloured", |s| {
        cursor_row(s) == "ls -l" && fg_is(s, "ls", Color::Idx(2)) && fg_is(s, "-l", Color::Idx(6))
    });
}

/// Newlines and tabs typed with `C-v` are drawn where readline draws them.
#[test]
fn newlines_and_tabs_match_plain_bash() {
    let keys = format!(
        "for x in a b; do{LITERAL_NEWLINE}{LITERAL_TAB}echo \"日本 $x\"{LITERAL_NEWLINE}done"
    );
    let opts = || Options {
        cols: 30,
        ..Options::default()
    };
    let with = typed(opts(), &keys);
    let plain = typed(
        Options {
            inkline: false,
            ..opts()
        },
        &keys,
    );
    let s = with.wait_for("colours on the last line", |s| {
        fg_is(s, "done", Color::Idx(5))
    });
    assert_eq!(fg(&s, "echo"), Color::Idx(2));
    wait_same(&with, &plain, "typing a multi-line command");
}

/// A long line wraps inside a multi-line command.
#[test]
fn wrapped_rows_inside_a_multi_line_command() {
    let keys = format!("echo {}{LITERAL_NEWLINE}echo \"x\"", "a".repeat(30));
    let opts = || Options {
        cols: 20,
        ..Options::default()
    };
    let with = typed(opts(), &keys);
    let plain = typed(
        Options {
            inkline: false,
            ..opts()
        },
        &keys,
    );
    with.wait_for("colours on the last line", |s| {
        fg_is(s, "\"x\"", Color::Idx(3))
    });
    wait_same(&with, &plain, "typing a wrapped multi-line command");
}
