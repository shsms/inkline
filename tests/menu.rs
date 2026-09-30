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
const SHIFT_TAB: &str = "\x1b[Z";

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

/// A shell with `git st` typed and three history rows: `git status`,
/// `git stash`, `git stage`, top to bottom.
fn three_items() -> Shell {
    menu_showing(
        with_history(vec!["git stage", "git stash", "git status"]),
        "git st",
        "h  git status",
    )
}

#[test]
fn no_row_is_highlighted_before_a_move() {
    let sh = two_items();
    let s = sh.screen();
    assert!(!picked(&s, 1) && !picked(&s, 2), "{}", dump(&s));
    assert_eq!(
        cursor_row(&s),
        "$ git status",
        "the grey text is the top row"
    );
    assert_eq!(s.cursor_position(), (0, 8));
}

#[test]
fn ctrl_n_and_ctrl_p_write_each_row_into_the_line() {
    let mut sh = two_items();
    sh.send(C_N);
    sh.wait_for("the top row written", |s| {
        picked(s, 1) && cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    sh.send(C_N);
    sh.wait_for("the second row written", |s| {
        picked(s, 2)
            && !picked(s, 1)
            && cursor_row(s) == "$ git stash"
            && s.cursor_position() == (0, 11)
    });
    sh.send(C_N);
    sh.wait_for("back at the top", |s| {
        picked(s, 1) && cursor_row(s) == "$ git status"
    });
    sh.send(C_P);
    sh.wait_for("up wraps to the bottom", |s| {
        picked(s, 2) && cursor_row(s) == "$ git stash"
    });
}

#[test]
fn ctrl_p_starts_at_the_bottom() {
    let mut sh = two_items();
    sh.send(C_P);
    sh.wait_for("the bottom written", |s| {
        picked(s, 2) && cursor_row(s) == "$ git stash"
    });
}

/// A count moves that many rows; the first row down is the top one.
#[test]
fn a_count_moves_the_pick_that_many_rows() {
    let mut sh = three_items();
    sh.send(&format!("\x1b2{C_N}"));
    sh.wait_for("the second row", |s| {
        picked(s, 2) && !picked(s, 1) && cursor_row(s) == "$ git stash"
    });
}

/// Keys sent together chain: each acts on the rows of the first move.
#[test]
fn keys_sent_together_move_through_the_rows() {
    let mut sh = Shell::start(with_history(vec!["git stage", "git stash", "git status"]));
    sh.send(&format!("git st{C_N}{C_N}\t"));
    sh.wait_for("the third row", |s| {
        cursor_row(s) == "$ git stage" && s.cursor_position() == (0, 11)
    });
}

#[test]
fn a_kept_row_is_one_undo_step() {
    let mut sh = two_items();
    sh.send(&format!("{C_N}{C_N}\r"));
    sh.wait_for("the row kept", |s| {
        cursor_row(s) == "$ git stash" && s.cursor_position() == (0, 11)
    });
    sh.send(UNDO);
    sh.wait_for("the typed text back", |s| {
        cursor_row(s).starts_with("$ git st") && s.cursor_position() == (0, 8)
    });
}

/// Tab moves to the top row when there are more rows; the next Tab goes on.
#[test]
fn tab_moves_through_the_rows() {
    let mut sh = two_items();
    sh.send("\t");
    sh.wait_for("the top row", |s| {
        picked(s, 1) && cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    sh.send("\t");
    sh.wait_for("the second row", |s| {
        picked(s, 2) && cursor_row(s) == "$ git stash"
    });
}

/// With one row, Tab writes it and does not start moving.
#[test]
fn tab_with_one_row_writes_it() {
    let mut sh = menu_showing(with_history(vec!["git status"]), "git st", "h  git status");
    sh.send("\t");
    let s = sh.wait_for("the row written", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    assert!(!picked(&s, 1), "{}", dump(&s));
}

/// While moving, Tab with one row moves onto that same row: the line stays
/// and the row stays picked.
#[test]
fn tab_while_moving_with_one_row_stays_on_it() {
    let mut sh = menu_showing(with_history(vec!["git status"]), "git st", "h  git status");
    sh.send(C_N);
    sh.wait_for("the row written", |s| {
        picked(s, 1) && cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    sh.send("\t");
    let s = sh.settle();
    assert!(picked(&s, 1), "{}", dump(&s));
    assert_eq!(cursor_row(&s), "$ git status", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 12), "{}", dump(&s));
    // Still moving: C-g takes the row back.
    sh.send(C_G);
    sh.wait_for("the typed text back", |s| s.cursor_position() == (0, 8));
}

/// A shell in a directory holding `zzfile`, with `history`.
fn with_zzfile(dir: &tempfile::TempDir, history: Vec<&'static str>) -> Options {
    std::fs::write(dir.path().join("zzfile"), "").unwrap();
    Options {
        history,
        cwd: Some(dir.path().to_path_buf()),
        ..Options::default()
    }
}

#[test]
fn ctrl_g_then_tab_completes_as_bash_does() {
    let dir = tempfile::tempdir().unwrap();
    let mut sh = menu_showing(with_zzfile(&dir, vec!["ls zz-old"]), "ls z", "h  ls zz-old");
    sh.send(&format!("{C_G}\t"));
    sh.wait_for("the file name", |s| {
        cursor_row(s).starts_with("$ ls zzfile")
    });
}

/// With no menu, Tab completes as bash does, and a second Tab lists the
/// choices. The typed `zzf` is all the files have in common, so the first
/// Tab leaves the line as it is: readline lists on a second Tab only after
/// one that changed nothing.
#[test]
fn ctrl_g_then_two_tabs_list_the_choices() {
    let dir = tempfile::tempdir().unwrap();
    let opts = with_zzfile(&dir, vec!["ls zzf-old"]);
    std::fs::write(dir.path().join("zzfoo"), "").unwrap();
    let mut sh = menu_showing(opts, "ls zzf", "h  ls zzf-old");
    sh.send(&format!("{C_G}\t"));
    sh.wait_for("no menu", |s| {
        cursor_row(s) == "$ ls zzf" && row_text(s, 1).is_empty()
    });
    sh.send("\t");
    sh.wait_for("the listing", |s| {
        find(s, "zzfoo").is_some() && find(s, "zzfile").is_some()
    });
}

#[test]
fn enter_keeps_the_row_without_running() {
    let mut sh = menu_showing(
        with_history(vec!["echo one-two", "echo oh"]),
        "echo o",
        "h  echo oh",
    );
    sh.send(C_P);
    sh.wait_for("the bottom written", |s| {
        picked(s, 2) && cursor_row(s) == "$ echo one-two"
    });
    sh.send("\r");
    let s = sh.wait_for("the row kept", |s| {
        cursor_row(s) == "$ echo one-two" && !picked(s, 2)
    });
    assert_eq!(
        s.cursor_position(),
        (0, 14),
        "the line did not run: {}",
        dump(&s)
    );
    sh.send("\r");
    sh.wait_for("the output", |s| row_text(s, 1) == "one-two");
}

/// With no move, Enter runs the line as typed.
#[test]
fn enter_with_no_move_runs_the_line_as_typed() {
    let mut sh = menu_showing(
        with_history(vec!["echo one-two", "echo oh"]),
        "echo o",
        "h  echo oh",
    );
    sh.send("\r");
    sh.wait_for("the output", |s| row_text(s, 1) == "o");
}

/// `M-RET` while moving runs the line as written.
#[test]
fn alt_enter_while_moving_runs_the_written_line() {
    let mut sh = menu_showing(
        with_history(vec!["echo one-two", "echo oh"]),
        "echo o",
        "h  echo oh",
    );
    sh.send(C_P);
    sh.wait_for("the bottom written", |s| cursor_row(s) == "$ echo one-two");
    sh.send(ALT_ENTER);
    sh.wait_for("the output", |s| {
        has_row(s, "one-two") && row_text(s, 1) == "one-two"
    });
}

/// Where Enter is readline's `accept-line` (the multi-line group unbound),
/// it runs the written row at once.
#[test]
fn enter_as_accept_line_runs_the_written_row() {
    let mut sh = menu_showing(
        with_init(
            "(inkline-unbind-defaults 'multi-line)",
            vec!["echo one-two", "echo oh"],
        ),
        "echo o",
        "h  echo oh",
    );
    sh.send(C_P);
    sh.wait_for("the bottom written", |s| cursor_row(s) == "$ echo one-two");
    sh.send("\r");
    sh.wait_for("the output", |s| row_text(s, 1) == "one-two");
}

#[test]
fn ctrl_g_while_moving_puts_back_the_typed_text() {
    let mut sh = two_items();
    sh.send(&format!("{C_N}{C_N}"));
    sh.wait_for("the second row", |s| cursor_row(s) == "$ git stash");
    sh.send(C_G);
    let s = sh.wait_for("the typed text, no menu", |s| {
        cursor_row(s) == "$ git st" && s.cursor_position() == (0, 8) && row_text(s, 1).is_empty()
    });
    assert!(!picked(&s, 1), "{}", dump(&s));
}

/// A letter after a move goes after the row, and the menu for the new text
/// shows.
#[test]
fn a_letter_after_a_move_keeps_the_row() {
    let mut sh = menu_showing(
        with_history(vec!["git status -s", "git status"]),
        "git st",
        "h  git status",
    );
    sh.send(C_N);
    sh.wait_for("the top row", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    sh.send(" ");
    sh.wait_for("the new menu", |s| {
        row_text(s, 1) == "h  git status -s" && !picked(s, 1) && s.cursor_position() == (0, 13)
    });
}

/// A key that changes nothing still ends moving: `C-g` after it does not
/// take the row back.
#[test]
fn a_key_that_changes_nothing_ends_moving() {
    let mut sh = two_items();
    sh.send(C_N);
    sh.wait_for("the top row", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    // `C-f` at the end of the line, with no grey text, moves nothing.
    sh.send("\x06");
    sh.settle();
    sh.send(C_G);
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git status", "{}", dump(&s));
}

/// A Lisp command can move, after a draw it made; the row it wrote is kept
/// once it returns.
#[test]
fn a_lisp_command_moves_after_its_own_question() {
    let init = r#"(keymap-global-set "C-x w"
  (lambda () (interactive)
    (y-or-n-p "Q? ")
    (call-interactively 'menu-next)))"#;
    let mut sh = menu_showing(
        with_init(init, vec!["git stash", "git status"]),
        "git st",
        "h  git status",
    );
    sh.send("\x18w");
    sh.wait_for("the question", |s| has_row(s, "Q? (y or n)"));
    sh.send("n");
    sh.wait_for("the top row written", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
}

/// A Lisp command that changes the line and then moves leaves one undo
/// step: the next `C-n` does not undo part of it, and `C-_` takes all of
/// it back.
#[test]
fn a_lisp_command_that_edits_then_moves_leaves_a_whole_line() {
    let init = r#"(keymap-global-set "C-x w"
  (lambda () (interactive)
    (insert "a")
    (y-or-n-p "Q? ")
    (call-interactively 'menu-next)))"#;
    let mut sh = menu_showing(
        with_init(init, vec!["git stash", "git status"]),
        "git st",
        "h  git status",
    );
    sh.send("\x18w");
    sh.wait_for("the question", |s| has_row(s, "Q? (y or n)"));
    sh.send("n");
    sh.wait_for("the top row written", |s| cursor_row(s) == "$ git status");
    sh.send(C_N);
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git status", "{}", dump(&s));
    sh.send(UNDO);
    sh.wait_for("the typed text back", |s| {
        cursor_row(s).starts_with("$ git st") && s.cursor_position() == (0, 8)
    });
}

/// An after-change function that changes the line after a move ends
/// moving there. The function acts only after a move, so it would not put
/// its change back after a `C-g` that took it back.
#[test]
fn an_after_change_edit_ends_moving() {
    let init = r#"(add-hook 'inkline-after-change-functions
  (lambda (_b _e _l)
    (when (eq this-command 'menu-next)
      (goto-char (point-max))
      (insert "!"))))"#;
    let mut sh = menu_showing(
        with_init(init, vec!["git stash", "git status"]),
        "git st",
        "h  git status",
    );
    sh.send(C_N);
    sh.wait_for("the row and the hook's change", |s| {
        cursor_row(s) == "$ git status!"
    });
    sh.send(C_G);
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git status!", "{}", dump(&s));
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
    sh.send(C_N);
    sh.wait_for("the top row written", |s| {
        picked(s, row + 1) && cursor_row(s) == "name? echo help-me"
    });
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
    sh.wait_for("the menu", |s| row_text(s, 2) == "l  switch");
    sh.send(C_N);
    let s = sh.wait_for("the pick", |s| picked(s, 2));
    let col = s.cursor_position().1;
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

/// `menu-next` works on any key: with a menu it moves, and with none the
/// key does what it did before (`C-t` swaps two characters).
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
    sh.wait_for("the top row", |s| {
        picked(s, 1) && cursor_row(s) == "$ git status"
    });
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
    sh.send("\t");
    sh.wait_for("the item taken", |s| cursor_row(s) == "$ Echo Hello");
}

/// With history left out of the menu, the grey text still shows the newest
/// history match, and a history entry does not hide a Lisp item that gives
/// the same line.
#[test]
fn menu_sources_limit_the_menu_but_not_the_grey_text() {
    let init = format!("{WORDS}\n(setq inkline-menu-sources '(lisp))");
    let history = vec!["git switch", "git status"];
    let mut sh = menu_showing(with_init(&init, history), "git s", "l  switch");
    let s = sh.screen();
    assert_eq!(cursor_row(&s), "$ git status");
    assert_eq!(row_text(&s, 2), "l  show");
    assert_eq!(row_text(&s, 3), "");
    assert!(!picked(&s, 1), "{}", dump(&s));
    // Tab moves to the menu's top row, not the grey text's item.
    sh.send("\t");
    sh.wait_for("the item in the line", |s| cursor_row(s) == "$ git switch");
}

#[test]
fn the_menu_waits_for_enough_typed_characters() {
    let init = format!("{WORDS}\n(setq inkline-menu-min-chars 2)");
    let mut sh = Shell::start(with_init(&init, vec![]));
    sh.send("git s");
    let s = sh.settle();
    assert_eq!(row_text(&s, 1), "", "{}", dump(&s));
    sh.send("w");
    sh.wait_for("the menu", |s| row_text(s, 1) == "l  switch");
}

/// Moving the cursor without changing the text hides the menu, so `C-n` and
/// `C-p` then move between the lines of a command; typing brings it back.
#[test]
fn moving_the_cursor_hides_the_menu_until_the_text_changes() {
    // `\x0a` is C-j: it adds a line to the command. The second line has no
    // prompt, so the cursor's column there is after the `s` of the first.
    let mut sh = menu_showing(with_init(WORDS, vec![]), "echo s\x0aecho  sh", "l  show");
    sh.send("\x1b[A");
    sh.wait_for("the first line", |s| s.cursor_position().0 == 0);
    let s = sh.settle();
    assert!(!has_row(&s, "l  s"), "{}", dump(&s));
    sh.send(C_N);
    sh.wait_for("the second line", |s| s.cursor_position().0 == 1);
    let s = sh.settle();
    assert!(!has_row(&s, "l  s"), "{}", dump(&s));
    sh.send(C_P);
    sh.wait_for("the first line again", |s| s.cursor_position().0 == 0);
    sh.send(C_N);
    sh.wait_for("the second line again", |s| s.cursor_position().0 == 1);
    sh.send("o");
    sh.wait_for("the menu again", |s| row_text(s, 2) == "l  show");
}

#[test]
fn menu_on_move_keeps_the_menu_while_the_cursor_moves() {
    let init = format!("{WORDS}\n(setq inkline-menu-on-move t)");
    let mut sh = menu_showing(with_init(&init, vec![]), "echo s\x0aecho  sh", "l  show");
    sh.send("\x1b[A");
    sh.wait_for("the first line and its menu", |s| {
        s.cursor_position().0 == 0 && has_row(s, "l  switch")
    });
}

#[test]
fn shift_tab_moves_up_through_the_rows() {
    let mut sh = two_items();
    sh.send(SHIFT_TAB);
    sh.wait_for("the bottom row", |s| {
        picked(s, 2) && cursor_row(s) == "$ git stash"
    });
    sh.send(SHIFT_TAB);
    sh.wait_for("the top row", |s| {
        picked(s, 1) && cursor_row(s) == "$ git status"
    });
}

/// With no menu, Shift-Tab does what it did before inkline bound it: here
/// nothing, so the line stays.
#[test]
fn shift_tab_without_a_menu_changes_nothing() {
    let mut sh = Shell::start(with_history(vec!["echo old"]));
    sh.send("zzz");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ zzz");
    sh.send(SHIFT_TAB);
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ zzz", "{}", dump(&s));
}

/// With no menu, Shift-Tab runs what inputrc bound it to before inkline
/// did.
#[test]
fn shift_tab_without_a_menu_runs_its_own_binding() {
    let mut sh = Shell::start(Options {
        inputrc: Some("\"\\e[Z\": backward-char\n".into()),
        history: vec!["echo old"],
        ..Options::default()
    });
    sh.send("zzz");
    sh.wait_for("the typed text", |s| {
        cursor_row(s) == "$ zzz" && s.cursor_position() == (0, 5)
    });
    sh.send(SHIFT_TAB);
    sh.wait_for("the cursor back one", |s| {
        cursor_row(s) == "$ zzz" && s.cursor_position() == (0, 4)
    });
}
