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
const C_G: &str = "\x07";
const UNDO: &str = "\x1f";

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
fn enter_takes_the_picked_item_and_a_second_enter_runs_it() {
    let mut sh = menu_showing(
        with_history(vec!["echo one-two"]),
        "echo o",
        "h  echo one-two",
    );
    sh.send(C_N);
    sh.wait_for("the pick", |s| picked(s, 1));
    sh.send("\r");
    // The grey text already shows the item, so wait for the cursor too.
    sh.wait_for("the item in the line", |s| {
        cursor_row(s) == "$ echo one-two" && s.cursor_position() == (0, 14)
    });
    let s = sh.settle();
    assert_eq!(s.cursor_position(), (0, 14), "not run yet: {}", dump(&s));
    sh.send("\r");
    sh.wait_for("the output", |s| {
        has_row(s, "one-two") && row_text(s, 1) == "one-two"
    });
}

/// Keys typed ahead in one burst still take the picked item: the line does
/// not run.
#[test]
fn a_typed_ahead_pick_and_enter_take_the_item() {
    let mut sh = Shell::start(with_history(vec!["git stash", "git status"]));
    sh.send(&format!("git st{C_N}\r"));
    sh.wait_for("the item in the line", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    let s = sh.settle();
    assert_eq!(s.cursor_position(), (0, 12), "not run: {}", dump(&s));
    assert!(find(&s, "command not found").is_none(), "{}", dump(&s));
}

#[test]
fn tab_takes_the_picked_item_and_undo_takes_it_back() {
    let mut sh = two_items();
    sh.send(&format!("{C_N}{C_N}\t"));
    sh.wait_for("the item in the line", |s| {
        cursor_row(s) == "$ git stash" && s.cursor_position() == (0, 11)
    });
    sh.send(UNDO);
    sh.wait_for("the line back", |s| s.cursor_position() == (0, 8));
}

#[test]
fn tab_without_a_pick_completes_as_bash_does() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("zzfile"), "").unwrap();
    let opts = Options {
        history: vec!["ls zz-old"],
        cwd: Some(dir.path().to_path_buf()),
        ..Options::default()
    };
    let mut sh = menu_showing(opts, "ls z", "h  ls zz-old");
    sh.send("\t");
    sh.wait_for("the file name", |s| {
        cursor_row(s).starts_with("$ ls zzfile")
    });
}

/// The menu keys work in a Lisp command after a draw it made, here under
/// `y-or-n-p`'s question: the kept menu stays in use while Lisp runs.
#[test]
fn a_lisp_command_picks_after_its_own_question() {
    let init = r#"(keymap-global-set "C-x w"
  (lambda () (interactive)
    (y-or-n-p "Q? ")
    (call-interactively 'menu-next)
    (call-interactively 'menu-next)))"#;
    let mut sh = menu_showing(
        with_init(init, vec!["git stash", "git status"]),
        "git st",
        "h  git status",
    );
    sh.send("\x18w");
    sh.wait_for("the question", |s| has_row(s, "Q? (y or n)"));
    sh.send("n");
    sh.wait_for("the second item picked", |s| picked(s, 2));
    sh.send("\t");
    sh.wait_for("the item in the line", |s| {
        cursor_row(s) == "$ git stash" && s.cursor_position() == (0, 11)
    });
}

/// The menu keys work in a line that `read -e` reads under shell code a Lisp
/// command runs, where every draw happens while Lisp runs.
#[test]
fn the_menu_keys_work_in_read_e_under_lisp() {
    let mut sh = Shell::start(Options {
        init_el: Some(
            r#"(keymap-global-set "C-x e"
  (lambda () (call-interactively 'edit-and-execute-command)))"#
                .to_owned(),
        ),
        rc: "export VISUAL=true\n".into(),
        history: vec!["echo hello-world", "echo help-me"],
        ..Options::default()
    });
    sh.send("read -e -p 'name? ' x; echo \"ran:[$x]\"\x18e");
    sh.wait_for("the nested prompt", |s| cursor_row(s) == "name?");
    sh.send("echo hel");
    let s = sh.wait_for("the menu", |s| {
        row_text(s, s.cursor_position().0 + 1) == "h  echo help-me"
    });
    let row = s.cursor_position().0;
    sh.send(&format!("{C_N}{C_N}"));
    sh.wait_for("the second item picked", |s| picked(s, row + 2));
    sh.send("\t");
    sh.wait_for("the item in the line", |s| {
        cursor_row(s) == "name? echo hello-world"
    });
}

#[test]
fn ctrl_g_hides_the_menu_until_the_line_changes() {
    let mut sh = two_items();
    sh.send(C_G);
    let s = sh.wait_for("no menu", |s| row_text(s, 1).is_empty());
    assert_eq!(cursor_row(&s), "$ git st", "no grey text either");
    sh.send("a");
    sh.wait_for("the menu again", |s| {
        cursor_row(s) == "$ git status" && row_text(s, 1) == "h  git status"
    });
}

/// The hide lasts only until the text changes: typing a character and
/// deleting it brings the menu back for the same text.
#[test]
fn ctrl_g_hide_ends_when_the_text_changes_and_comes_back() {
    let mut sh = two_items();
    sh.send(C_G);
    sh.wait_for("no menu", |s| row_text(s, 1).is_empty());
    sh.send("u");
    sh.wait_for("the typed text", |s| cursor_row(s).starts_with("$ git stu"));
    sh.send("\x7f");
    sh.wait_for("the menu again", |s| {
        cursor_row(s) == "$ git status" && row_text(s, 1) == "h  git status"
    });
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

/// `C-g` with no menu is readline's `abort`, which rings the bell, and the
/// shell goes on.
#[test]
fn ctrl_g_without_a_menu_aborts_and_the_shell_goes_on() {
    let mut sh = Shell::start(Options {
        rc: "bind 'set bell-style audible'\n".into(),
        ..Options::default()
    });
    sh.send("zzz");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ zzz");
    assert_eq!(row_text(&sh.settle(), 1), "", "no menu");
    sh.take_output();
    sh.send(C_G);
    sh.wait_for_output("the bell", b"\x07");
    sh.send("\x15echo ok\r");
    sh.wait_for("the output", |s| has_row(s, "ok"));
}

/// A line brought back from history shows no menu, so `C-p` and `C-n` keep
/// walking history even where each line starts an older entry.
#[test]
fn ctrl_p_and_ctrl_n_walk_history_past_lines_that_start_older_ones() {
    let mut sh = Shell::start(with_history(vec!["ls -la -h", "ls -la", "ls"]));
    sh.settle();
    for (row, col) in [("$ ls", 4), ("$ ls -la", 8), ("$ ls -la -h", 11)] {
        sh.send(C_P);
        let s = sh.wait_for(row, |s| {
            cursor_row(s) == row && s.cursor_position() == (0, col)
        });
        assert_eq!(row_text(&s, 1), "", "no menu: {}", dump(&s));
    }
    sh.send(C_N);
    sh.wait_for("the newer entry", |s| {
        cursor_row(s) == "$ ls -la" && s.cursor_position() == (0, 8)
    });
}

#[test]
fn a_recalled_line_shows_no_menu_until_it_changes() {
    let mut sh = Shell::start(with_history(vec!["git status", "git st"]));
    sh.settle();
    sh.send(C_P);
    sh.wait_for("the entry", |s| {
        cursor_row(s) == "$ git st" && s.cursor_position() == (0, 8)
    });
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git st", "no grey text: {}", dump(&s));
    assert_eq!(row_text(&s, 1), "", "no menu: {}", dump(&s));
    sh.send("a");
    sh.wait_for("the menu again", |s| {
        cursor_row(s) == "$ git status" && row_text(s, 1) == "h  git status"
    });
}

/// Whether `C-p` walked history from `git st`, the line a search left: to
/// the entry before it or, where readline puts its history place back after
/// a search (before 8.3), to the newest entry. No menu row is picked.
fn walked_back_from_the_search(s: &vt100::Screen) -> bool {
    let row = cursor_row(s);
    (row == "$ ls" || row == "$ git status")
        && s.cursor_position() == (0, row.len() as u16)
        && !picked(s, 1)
}

/// A line that Up found with `previous-line-or-search` counts as brought
/// back from history: after `C-e` it shows no menu and no grey text, and
/// `C-p` walks history. inkline binds Up to `previous-line-or-search` where
/// inputrc binds it to `history-search-backward`.
#[test]
fn a_line_found_with_previous_line_or_search_shows_no_menu() {
    let mut sh = Shell::start(Options {
        inputrc: Some("\"\\e[A\": history-search-backward\n".into()),
        history: vec!["git status", "git st", "ls"],
        ..Options::default()
    });
    sh.send("git s");
    sh.wait_for("the menu", |s| row_text(s, 1) == "h  git st");
    sh.send("\x1b[A");
    sh.wait_for("the found entry", |s| {
        cursor_row(s) == "$ git st" && row_text(s, 1).is_empty()
    });
    sh.send("\x05");
    sh.wait_for("the cursor at the end", |s| s.cursor_position() == (0, 8));
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git st", "no grey text: {}", dump(&s));
    assert_eq!(row_text(&s, 1), "", "no menu: {}", dump(&s));
    sh.send(C_P);
    sh.wait_for("an older entry", walked_back_from_the_search);
}

/// The same for a line that `M-p` (`non-incremental-reverse-search-history`)
/// found.
#[test]
fn a_line_found_with_a_non_incremental_search_shows_no_menu() {
    let mut sh = Shell::start(with_history(vec!["git status", "git st", "ls"]));
    sh.settle();
    sh.send("\x1bp");
    sh.settle();
    sh.send("git st\r");
    sh.wait_for("the found entry", |s| {
        cursor_row(s) == "$ git st" && s.cursor_position() == (0, 2)
    });
    sh.send("\x05");
    sh.wait_for("the cursor at the end", |s| s.cursor_position() == (0, 8));
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git st", "no grey text: {}", dump(&s));
    assert_eq!(row_text(&s, 1), "", "no menu: {}", dump(&s));
    sh.send(C_P);
    sh.wait_for("an older entry", walked_back_from_the_search);
}

/// A search with Up (`previous-line-or-search`, as above) that finds nothing
/// leaves the typed line, and its menu, as they were.
#[test]
fn a_search_that_finds_nothing_keeps_the_menu() {
    let mut sh = Shell::start(Options {
        inputrc: Some("\"\\e[A\": history-search-backward\n".into()),
        ..with_init(WORDS, vec!["ls"])
    });
    sh.send("echo s");
    sh.wait_for("the menu", |s| row_text(s, 1) == "l  switch");
    sh.send("\x1b[A");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ echo switch", "{}", dump(&s));
    assert_eq!(row_text(&s, 1), "l  switch", "{}", dump(&s));
}

/// `menu-next` works on any key: with a menu it moves the pick, and with none
/// the key does what it did before (`C-t` swaps two characters).
#[test]
fn menu_next_on_another_key_keeps_that_keys_own_job() {
    let mut sh = menu_showing(
        with_init(
            "(keymap-global-set \"C-t\" 'menu-next)",
            vec!["git stash", "git status"],
        ),
        "git st",
        "h  git status",
    );
    sh.send("\x14");
    sh.wait_for("the top picked", |s| picked(s, 1));
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    sh.send("xy\x14");
    sh.wait_for("the two characters swapped", |s| cursor_row(s) == "$ yx");
}

/// The README's `M-p`: with no menu it runs readline's non-incremental
/// history search. The line the search finds shows no menu (here the Lisp
/// items, which show wherever the search leaves the cursor), so a second
/// `M-p` searches again.
#[test]
fn menu_previous_on_m_p_runs_the_non_incremental_search() {
    let init = format!("{WORDS}\n(keymap-global-set \"M-p\" 'menu-previous)");
    let mut sh = Shell::start(with_init(&init, vec!["ls", "echo one"]));
    sh.send("\x1bp");
    sh.wait_for("the search prompt", |s| cursor_row(s) == "$ :");
    sh.send("one\r");
    sh.wait_for("the found line", |s| cursor_row(s) == "$ echo one");
    let s = sh.settle();
    assert_eq!(row_text(&s, 1), "", "{}", dump(&s));
    sh.send("\x1bp");
    sh.wait_for("the search prompt again", |s| cursor_row(s) == "$ :");
}

/// A key that ran `history-search-backward` moves through the matches on
/// each press with no menu, as `<up>` does when bound to it.
#[test]
fn menu_previous_on_a_history_search_key_moves_through_the_matches() {
    let mut sh = Shell::start(Options {
        inputrc: Some("\"\\ep\": history-search-backward\n".into()),
        ..with_init(
            "(keymap-global-set \"M-p\" 'menu-previous)",
            vec!["git status", "git stash", "ls"],
        )
    });
    sh.send("git st");
    sh.wait_for("the menu", |s| row_text(s, 1) == "h  git stash");
    sh.send("\x07");
    sh.wait_for("the menu hidden", |s| row_text(s, 1).is_empty());
    sh.send("\x1bp");
    sh.wait_for("the newest match", |s| cursor_row(s) == "$ git stash");
    sh.send("\x1bp");
    sh.wait_for("the match before it", |s| cursor_row(s) == "$ git status");
}

/// Keys that ran readline's substring searches also move through the
/// matches on each press with no menu, in both directions, as when both are
/// bound to the searches. Another key in between, even one that runs
/// `menu-previous` to move through history, starts a new search.
#[test]
fn menu_keys_on_substring_search_keys_move_through_the_matches() {
    let mut sh = Shell::start(Options {
        inputrc: Some(
            "\"\\ep\": history-substring-search-backward
\"\\en\": history-substring-search-forward
"
            .into(),
        ),
        ..with_init(
            "(keymap-global-set \"M-p\" 'menu-previous)
             (keymap-global-set \"M-n\" 'menu-next)
             (setq inkline-history-cursor 'end)",
            vec!["x stat", "zzz", "git status", "a stat", "ls"],
        )
    });
    sh.send("stat\x07");
    sh.settle();
    sh.send("\x1bp");
    sh.wait_for("the newest match", |s| cursor_row(s) == "$ a stat");
    sh.send("\x1bp");
    sh.wait_for("the match before it", |s| cursor_row(s) == "$ git status");
    sh.send("\x1bn");
    sh.wait_for("the newer match again", |s| cursor_row(s) == "$ a stat");
    sh.send("\x1bp");
    sh.wait_for("the older match again", |s| cursor_row(s) == "$ git status");
    // A new search for the whole line finds nothing older.
    sh.send("\x05\x1bp");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git status", "{}", dump(&s));
    // `C-p` moves through history; the new search after it finds nothing
    // older either.
    sh.send("\x10");
    sh.wait_for("another entry", |s| cursor_row(s) != "$ git status");
    let moved = cursor_row(&sh.settle());
    sh.send("\x1bp");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), moved, "{}", dump(&s));
}

/// With no menu, a key that had macro text types it.
#[test]
fn menu_next_on_a_macro_key_types_the_macro() {
    let mut sh = Shell::start(Options {
        inputrc: Some("\"\\C-t\": \"MAC\"\n".into()),
        ..with_init("(keymap-global-set \"C-t\" 'menu-next)", vec![])
    });
    sh.send("xy\x14");
    sh.wait_for("the macro typed", |s| cursor_row(s) == "$ xyMAC");
}

/// With no menu, a key that had nothing moves between the lines of a
/// command.
#[test]
fn menu_keys_that_had_nothing_move_between_lines() {
    let mut sh = Shell::start(with_init(
        "(keymap-global-set \"C-x n\" 'menu-next)
         (keymap-global-set \"C-x p\" 'menu-previous)",
        vec![],
    ));
    // `\x0a` is C-j: it adds a line to the command.
    sh.send("echo a\x0aecho b");
    sh.wait_for("the second line", |s| s.cursor_position().0 == 1);
    sh.send("\x18p");
    sh.wait_for("the first line", |s| s.cursor_position().0 == 0);
    sh.send("\x18n");
    sh.wait_for("the second line again", |s| s.cursor_position().0 == 1);
}

/// A key that already ran `menu-next` before inkline bound it to
/// `menu-next` moves a line or through history with no menu, as a key that
/// had nothing.
#[test]
fn menu_next_on_a_key_that_already_ran_it_does_not_loop() {
    let mut sh = Shell::start(Options {
        rc: "bind '\"\\C-t\": menu-next'
inkline eval \"(keymap-global-set \\\"C-t\\\" 'menu-next)\"
"
        .into(),
        ..Options::default()
    });
    sh.send("xy\x14z");
    sh.wait_for("the shell still up", |s| cursor_row(s) == "$ xyz");
}

/// Ignoring case, an item of another case is listed, with no grey text, and
/// taking it puts the item's own case in the line.
#[test]
fn ignoring_case_lists_items_of_another_case() {
    let mut sh = menu_showing(
        with_init(
            "(setq inkline-completion-ignore-case t)",
            vec!["Echo Hello"],
        ),
        "echo h",
        "h  Echo Hello",
    );
    assert_eq!(cursor_row(&sh.screen()), "$ echo h");
    sh.send(&format!("{C_N}\t"));
    sh.wait_for("the item taken", |s| cursor_row(s) == "$ Echo Hello");
}
