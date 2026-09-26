#[path = "support/common.rs"]
mod common;

use common::*;

fn with_history(history: Vec<&'static str>) -> Options {
    Options {
        history,
        ..Options::default()
    }
}

fn with_init(init: &str, history: Vec<&'static str>) -> Options {
    Options {
        init_el: Some(init.to_owned()),
        history,
        ..Options::default()
    }
}

/// Starts a shell with `opts`, types `keys`, and waits for `row` on the
/// row below the cursor.
fn menu_showing(opts: Options, keys: &str, row: &str) -> Shell {
    let mut sh = Shell::start(opts);
    sh.send(keys);
    sh.wait_for("the menu", |s| {
        row_text(s, s.cursor_position().0 + 1) == row
    });
    sh
}

#[test]
fn the_menu_lists_history_under_the_line() {
    let sh = menu_showing(
        with_history(vec!["git stash", "git status", "ls"]),
        "git st",
        "h  git status",
    );
    let s = sh.screen();
    assert_eq!(
        cursor_row(&s),
        "$ git status",
        "the grey text is the top item"
    );
    assert_eq!(row_text(&s, 2), "h  git stash");
    assert_eq!(row_text(&s, 3), "");
    assert_eq!(s.cursor_position(), (0, 8));
}

#[test]
fn no_menu_on_an_empty_line_or_without_a_match() {
    let mut sh = Shell::start(with_history(vec!["git status"]));
    let s = sh.settle();
    assert_eq!(row_text(&s, 1), "");
    sh.send("zzz");
    let s = sh.settle();
    assert_eq!(row_text(&s, 1), "");
}

#[test]
fn erased_after_enter() {
    let mut sh = menu_showing(
        with_history(vec!["echo hello-world"]),
        "echo hel",
        "h  echo hello-world",
    );
    sh.send("\r");
    let s = sh.wait_for("the output", |s| has_row(s, "hel"));
    assert_eq!(row_text(&s, 0), "$ echo hel");
    assert!(!has_row(&s, "h  echo hello-world"), "{}", dump(&s));
}

#[test]
fn erased_after_ctrl_c() {
    let mut sh = menu_showing(
        with_history(vec!["echo hello-world"]),
        "echo hel",
        "h  echo hello-world",
    );
    sh.send("\x03");
    let s = sh.wait_for("a new prompt", |s| {
        s.cursor_position().0 == 1 && cursor_row(s) == "$"
    });
    assert_eq!(row_text(&s, 2), "", "{}", dump(&s));
}

/// With no grey text after the cursor, the menu's rows are still erased
/// when the line stops matching.
#[test]
fn erased_without_grey_text() {
    let mut sh = menu_showing(
        with_init("(setq inkline-completion-style 'fuzzy)", vec!["git status"]),
        "gst",
        "h  git status",
    );
    sh.send("z");
    sh.wait_for("the menu gone", |s| {
        cursor_row(s) == "$ gstz" && row_text(s, 1).is_empty()
    });
}

#[test]
fn show_menu_nil_keeps_just_the_grey_text() {
    let mut sh = Shell::start(with_init(
        "(setq inkline-show-menu nil)",
        vec!["git status"],
    ));
    sh.send("git st");
    let s = sh.wait_for("the grey text", |s| cursor_row(s) == "$ git status");
    assert_eq!(row_text(&s, 1), "");
}

#[test]
fn show_suggestion_nil_keeps_just_the_menu() {
    let sh = menu_showing(
        with_init("(setq inkline-show-suggestion nil)", vec!["git status"]),
        "git st",
        "h  git status",
    );
    assert_eq!(cursor_row(&sh.screen()), "$ git st");
}

/// Completes the lowercase word before the cursor. It only reads the line:
/// moving point is refused while the completion functions run.
const WORDS: &str = r#"(add-hook 'inkline-completion-functions
  (lambda ()
    (let ((start (point)))
      (while (let ((c (char-before start))) (and c (<= 97 c) (<= c 122)))
        (setq start (- start 1)))
      (list start (point) '("switch" "show")))))"#;

#[test]
fn a_lisp_item_shows_with_its_letter() {
    let sh = menu_showing(with_init(WORDS, vec![]), "git s", "l  switch");
    let s = sh.screen();
    assert_eq!(row_text(&s, 2), "l  show");
    assert_eq!(cursor_row(&s), "$ git switch");
}

#[test]
fn a_failing_completion_function_is_removed() {
    let mut sh = Shell::start(with_init(
        "(add-hook 'inkline-completion-functions (lambda () (error \"boom\")))",
        vec![],
    ));
    sh.send("x");
    sh.wait_for("the note", |s| {
        has_row(
            s,
            "inkline: lambda: boom (removed from inkline-completion-functions)",
        )
    });
    sh.send("\r");
    // An Enter typed while `x` still runs would only add a line.
    sh.wait_for("a new prompt", |s| {
        s.cursor_position().0 > 0 && cursor_row(s) == "$"
    });
    sh.send("inkline eval '(length inkline-completion-functions)'\r");
    sh.wait_for("the empty hook", |s| has_row(s, "0"));
}

/// A `C-c` while a completion function computes gives a new prompt once the
/// function has returned, and the typed line does not run.
#[test]
fn ctrl_c_during_a_slow_completion_function_gives_a_new_prompt() {
    let mut sh = Shell::start(with_init(
        r#"(add-hook 'inkline-completion-functions
  (lambda ()
    (when (equal (buffer-string) "x")
      (let ((i 0)) (while (< i 4500000) (setq i (1+ i)))))
    nil))"#,
        vec![],
    ));
    sh.send("x");
    std::thread::sleep(std::time::Duration::from_millis(200));
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| {
        s.cursor_position().0 > 0 && cursor_row(s) == "$"
    });
    sh.send("echo alive\r");
    let s = sh.wait_for("the shell answers", |s| has_row(s, "alive"));
    assert!(find(&s, "command not found").is_none(), "{}", dump(&s));
}

#[test]
fn a_short_screen_keeps_the_command_in_view() {
    let mut sh = Shell::start(Options {
        rows: 4,
        history: vec!["x1", "x2", "x3", "x4", "x5"],
        ..Options::default()
    });
    sh.send("x");
    let s = sh.wait_for("the menu", |s| has_row(s, "h  x5"));
    assert!(has_row(&s, "$ x5"), "the command stays: {}", dump(&s));
    assert!(has_row(&s, "   … 3 more"), "{}", dump(&s));
}

#[test]
fn a_resize_redraws_the_menu() {
    let mut sh = menu_showing(
        with_history(vec!["echo a-rather-long-history-line-that-will-be-cut"]),
        "echo a",
        "h  echo a-rather-long-history-line-that-will-be-cut",
    );
    sh.resize(24, 30);
    let s = sh.wait_for("the cut row", |s| {
        has_row(s, "h  echo a-rather-long-histor…")
    });
    assert!(!has_row(&s, "will-be-cut"), "{}", dump(&s));
}

#[test]
fn fuzzy_style_matches_with_gaps() {
    let sh = menu_showing(
        with_init("(setq inkline-completion-style 'fuzzy)", vec!["git status"]),
        "gst",
        "h  git status",
    );
    assert_eq!(
        cursor_row(&sh.screen()),
        "$ gst",
        "no grey text for a gapped match"
    );
}

#[test]
fn a_completion_function_that_changes_the_line_is_removed() {
    let mut sh = Shell::start(Options {
        cols: 120,
        ..with_init(
            "(add-hook 'inkline-completion-functions (lambda () (insert \"x\")))",
            vec![],
        )
    });
    sh.send("ab");
    let s = sh.wait_for("the note", |s| {
        has_row(
            s,
            "inkline: lambda: the line cannot be changed here \
             (removed from inkline-completion-functions)",
        )
    });
    assert_eq!(cursor_row(&s), "$ ab", "the line is as typed");
    sh.send("\x15inkline eval '(length inkline-completion-functions)'\r");
    sh.wait_for("the empty hook", |s| has_row(s, "0"));
}

/// A menu is kept only while the line's text and cursor stay: after the
/// line is emptied and the same text comes back, the menu is made again.
/// Here the matching style changes while the line is empty, and only a new
/// menu uses it.
#[test]
fn a_menu_is_made_again_when_the_text_comes_back() {
    let mut sh = Shell::start(with_init(
        "(keymap-global-set \"C-x f\" (lambda () (setq inkline-completion-style 'fuzzy)))",
        vec!["git status"],
    ));
    sh.send("gst");
    let s = sh.wait_for("the typed text", |s| cursor_row(s) == "$ gst");
    assert_eq!(row_text(&s, 1), "", "no prefix match: {}", dump(&s));
    sh.send("\x15\x18f\x19");
    sh.wait_for("the fuzzy menu", |s| {
        cursor_row(s) == "$ gst" && row_text(s, 1) == "h  git status"
    });
}

/// With both the menu and the grey text off, no items are gathered: the
/// completion functions are not asked.
#[test]
fn nothing_is_gathered_with_the_menu_and_grey_text_off() {
    let mut sh = Shell::start(with_init(
        "(setq inkline-show-menu nil inkline-show-suggestion nil)
         (defvar asked \"not asked\")
         (add-hook 'inkline-completion-functions
           (lambda () (setq asked \"asked\") nil))",
        vec![],
    ));
    sh.send("x");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ x");
    sh.settle();
    sh.send("\x15inkline eval asked\r");
    sh.wait_for("the answer", |s| has_row(s, "\"not asked\""));
}

/// A menu drawn while a Lisp command runs, here under `y-or-n-p`'s question,
/// has no items from the Lisp hooks; the draw after the command has them.
#[test]
fn a_menu_drawn_while_lisp_runs_is_not_kept() {
    let init = format!(
        r#"{WORDS}
(keymap-global-set "C-x w" (lambda () (interactive) (insert " s") (y-or-n-p "Q? ")))"#
    );
    let mut sh = Shell::start(with_init(&init, vec![]));
    sh.send("git");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ git");
    sh.send("\x18w");
    sh.wait_for("the question", |s| has_row(s, "Q? (y or n)"));
    sh.send("n");
    sh.wait_for("the Lisp items", |s| {
        cursor_row(s) == "$ git switch" && row_text(s, 1) == "l  switch"
    });
}

const C_N: &str = "\x0e";
const C_P: &str = "\x10";

/// Whether row `row` is drawn in reverse video.
fn picked(s: &vt100::Screen, row: u16) -> bool {
    s.cell(row, 0).is_some_and(|c| c.inverse())
}

fn two_items() -> Shell {
    menu_showing(
        with_history(vec!["git stash", "git status"]),
        "git st",
        "h  git status",
    )
}

#[test]
fn ctrl_n_and_ctrl_p_move_the_pick() {
    let mut sh = two_items();
    assert!(!picked(&sh.screen(), 1));
    sh.send(C_N);
    sh.wait_for("the top picked", |s| picked(s, 1) && !picked(s, 2));
    sh.send(C_N);
    let s = sh.wait_for("the second picked", |s| picked(s, 2));
    assert_eq!(
        cursor_row(&s),
        "$ git stash",
        "the grey text follows the pick"
    );
    sh.send(C_P);
    sh.wait_for("back to the top", |s| picked(s, 1));
}

/// A count moves the pick that many rows, though the menu is off the screen
/// while the count is typed.
#[test]
fn a_count_moves_the_pick_that_many_rows() {
    let mut sh = two_items();
    sh.send(&format!("\x1b2{C_N}"));
    sh.wait_for("the second picked", |s| {
        picked(s, 2) && !picked(s, 1) && cursor_row(s) == "$ git stash"
    });
}

#[test]
fn ctrl_p_starts_at_the_bottom() {
    let mut sh = two_items();
    sh.send(C_P);
    sh.wait_for("the bottom picked", |s| picked(s, 2));
}

#[test]
fn ctrl_p_walks_history_without_a_menu() {
    let mut sh = Shell::start(with_init(
        "(setq inkline-show-menu nil)",
        vec!["echo old", "ls"],
    ));
    sh.send("echo");
    sh.wait_for("the grey text", |s| cursor_row(s) == "$ echo old");
    sh.send(C_P);
    sh.wait_for("the newest entry", |s| cursor_row(s) == "$ ls");
}

#[test]
fn a_users_own_binding_is_left_alone() {
    let mut sh = Shell::start(Options {
        before_inkline: "bind '\"\\C-n\": backward-char'\n".into(),
        history: vec!["git status"],
        ..Options::default()
    });
    sh.send("git st");
    sh.wait_for("the menu", |s| row_text(s, 1) == "h  git status");
    sh.send(C_N);
    sh.wait_for("the cursor moved back", |s| s.cursor_position() == (0, 7));
}

#[test]
fn up_moves_between_lines_while_the_menu_shows() {
    // `\x0a` is C-j: it adds a line to the command.
    let mut sh = menu_showing(with_init(WORDS, vec![]), "echo a\x0aecho s", "l  switch");
    sh.send("\x1b[A");
    sh.wait_for("the cursor on the first line", |s| {
        s.cursor_position().0 == 0
    });
}

/// Moving the pick is not a move between lines: the next Up keeps the
/// cursor's own column, not one from an earlier run of Up and Down.
#[test]
fn up_after_a_pick_keeps_the_cursors_column() {
    let mut sh = Shell::start(with_init(WORDS, vec![]));
    sh.send("echo aaaaaaaaaaaa\x0aecho");
    sh.wait_for("two lines", |s| s.cursor_position().0 == 1);
    sh.send("\x1b[A");
    sh.wait_for("the first line", |s| s.cursor_position().0 == 0);
    sh.send("\x1b[B");
    sh.wait_for("the second line", |s| s.cursor_position().0 == 1);
    sh.send(" s");
    let s = sh.wait_for("the menu", |s| row_text(s, 2) == "l  switch");
    let col = s.cursor_position().1;
    sh.send(C_N);
    sh.wait_for("the pick", |s| picked(s, 2));
    sh.send("\x1b[A");
    let s = sh.wait_for("the first line", |s| s.cursor_position().0 == 0);
    assert_eq!(s.cursor_position(), (0, col), "{}", dump(&s));
}
