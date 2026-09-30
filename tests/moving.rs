//! Moving around a command that spans several lines.

#[path = "support/common.rs"]
mod common;

use common::*;

const LOOP: &str = "for x in a\ndo echo $x\ndone";

/// Types `lines` joined by `C-j` and waits for the cursor on the last.
fn block(sh: &mut Shell, lines: &[&str]) {
    sh.send(&lines.join(CTRL_J));
    let last = lines.len() as u16 - 1;
    sh.wait_for("the block", |s| s.cursor_position().0 == last);
}

#[test]
fn up_and_down_move_between_lines() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo bb"]);
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 7));
    sh.send("X");
    sh.wait_for("the insert", |s| row_text(s, 0) == "$ echo Xa");
    sh.send(DOWN);
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 7));
}

#[test]
fn a_run_of_moves_keeps_its_column() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo abcdefghij", "x", "echo 12345678"]);
    sh.send(UP);
    sh.wait_for("the short line", |s| s.cursor_position() == (1, 1));
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 13));
}

#[test]
fn up_on_the_first_line_goes_to_history() {
    let mut sh = Shell::start(Options {
        history: vec!["echo old"],
        ..Options::default()
    });
    block(&mut sh, &["echo new", "x"]);
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position().0 == 0);
    sh.send(UP);
    sh.wait_for("the history entry", |s| {
        row_text(s, 0) == "$ echo old" && s.cursor_position() == (0, 10)
    });
}

/// With no older entry, Up on the first line leaves the cursor where it is.
#[test]
fn up_past_the_oldest_entry_keeps_the_cursor() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo b"]);
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 6));
    sh.send(UP);
    sh.send("X");
    sh.wait_for("the insert", |s| {
        row_text(s, 0) == "$ echoX a" && s.cursor_position() == (0, 7)
    });
}

#[test]
fn a_recalled_multi_line_entry_opens_at_its_start() {
    let mut sh = Shell::start(Options {
        history: vec![LOOP],
        ..Options::default()
    });
    sh.send(UP);
    sh.wait_for("the entry", |s| {
        row_text(s, 2) == "done" && s.cursor_position() == (0, 2)
    });
}

#[test]
fn inkline_history_cursor_end_keeps_readlines_place() {
    let mut sh = Shell::start(Options {
        rc: "inkline eval \"(setq inkline-history-cursor 'end)\" >/dev/null\n".into(),
        history: vec![LOOP],
        ..Options::default()
    });
    sh.send(UP);
    sh.wait_for("the entry", |s| {
        row_text(s, 2) == "done" && s.cursor_position() == (2, 4)
    });
}

#[test]
fn home_and_end_stay_on_the_line() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo b"]);
    sh.send("\x01");
    sh.wait_for("the line's start", |s| s.cursor_position() == (1, 0));
    sh.send("\x05");
    sh.wait_for("the line's end", |s| s.cursor_position() == (1, 6));
}

#[test]
fn kills_stop_at_the_line() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo b"]);
    sh.send("\x01\x0b");
    sh.wait_for("the killed line", |s| {
        row_text(s, 0) == "$ echo a" && row_text(s, 1).is_empty()
    });
    sh.send("\x19");
    sh.wait_for("the yank", |s| row_text(s, 1) == "echo b");
}

#[test]
fn at_a_line_edge_the_kills_join_lines() {
    let mut sh = Shell::start(Options::default());
    block(&mut sh, &["echo a", "echo b"]);
    sh.send("\x01\x15");
    sh.wait_for("joined by C-u", |s| row_text(s, 0) == "$ echo aecho b");
    sh.send("\x1f");
    sh.wait_for("undone", |s| row_text(s, 1) == "echo b");
    sh.send(UP);
    sh.send("\x05\x0b");
    sh.wait_for("joined by C-k", |s| row_text(s, 0) == "$ echo aecho b");
}

/// A shell whose Up and Down run `up` and `down`.
fn arrow_keys(up: &str, down: &str, history: Vec<&'static str>) -> Options {
    Options {
        rc: format!("bind '\"\\e[A\": {up}'\nbind '\"\\e[B\": {down}'\n"),
        history,
        ..Options::default()
    }
}

#[test]
fn up_can_search_history() {
    let mut sh = Shell::start(arrow_keys(
        "previous-line-or-search",
        "next-line-or-search",
        vec!["git status", "ls", "git log"],
    ));
    sh.send("git");
    sh.wait_for("the text", |s| cursor_row(s).starts_with("$ git"));
    sh.send(UP);
    sh.wait_for("the newest match", |s| row_text(s, 0) == "$ git log");
    sh.send(UP);
    sh.wait_for("the next match", |s| row_text(s, 0) == "$ git status");
}

/// A shell whose Up and Down run the substring search commands.
fn substring_keys(history: Vec<&'static str>) -> Options {
    arrow_keys(
        "previous-line-or-substring-search",
        "next-line-or-substring-search",
        history,
    )
}

#[test]
fn up_finds_entries_holding_the_typed_text() {
    let mut sh = Shell::start(substring_keys(vec!["git status", "ls", "echo stat", "pwd"]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the newest match", |s| {
        cursor_row(s) == "$ echo stat" && s.cursor_position() == (0, 11)
    });
    sh.send(UP);
    let s = sh.wait_for("the match before it", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    assert_eq!(row_text(&s, 1), "", "no menu on a found line: {}", dump(&s));
}

/// An entry that is the typed text itself is passed over.
#[test]
fn up_passes_over_an_entry_that_is_the_typed_text() {
    let mut sh = Shell::start(substring_keys(vec!["git status", "git st", "ls"]));
    sh.send("git st");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 8));
    sh.send(UP);
    sh.wait_for("the older match", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
}

/// With only the typed text itself older than a match, Up leaves the match
/// and its cursor as they are.
#[test]
fn up_leaves_the_last_match_when_only_the_typed_text_is_older() {
    let mut sh = Shell::start(substring_keys(vec!["git st", "ls", "git status"]));
    sh.send("git st");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 8));
    sh.send(UP);
    sh.wait_for("the match", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    sh.send(UP);
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git status", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 12), "{}", dump(&s));
    // The typed text shows its grey text and menu again.
    sh.send(DOWN);
    sh.wait_for("the typed text back", |s| {
        s.cursor_position() == (0, 8) && row_text(s, 1) == "h  git status"
    });
}

/// Up whose only match is the typed text itself leaves the typed line as
/// it is, not as a history entry: after more typing, Down leaves it alone.
#[test]
fn down_after_up_found_only_the_typed_text_keeps_the_line() {
    let mut sh = Shell::start(substring_keys(vec!["zzq", "ls"]));
    sh.send("zzq");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ zzq");
    sh.send(UP);
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ zzq", "{}", dump(&s));
    sh.send("x");
    sh.wait_for("the x", |s| cursor_row(s) == "$ zzqx");
    sh.send(DOWN);
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ zzqx", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 6), "{}", dump(&s));
}

#[test]
fn down_past_the_newest_match_gives_back_the_typed_text() {
    let mut sh = Shell::start(substring_keys(vec!["git status", "ls", "echo stat", "pwd"]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the newest match", |s| cursor_row(s) == "$ echo stat");
    sh.send(UP);
    sh.wait_for("the match before it", |s| cursor_row(s) == "$ git status");
    sh.send(DOWN);
    sh.wait_for("the newer match", |s| cursor_row(s) == "$ echo stat");
    sh.send(DOWN);
    sh.wait_for("the typed text back", |s| {
        cursor_row(s) == "$ stat" && s.cursor_position() == (0, 6)
    });
}

#[test]
fn up_on_an_empty_line_walks_history() {
    let mut sh = Shell::start(substring_keys(vec!["echo one", "echo two", "echo three"]));
    sh.settle();
    for row in ["$ echo three", "$ echo two", "$ echo one"] {
        sh.send(UP);
        sh.wait_for(row, |s| cursor_row(s) == row);
    }
}

#[test]
fn a_multi_line_match_opens_at_its_start() {
    let mut sh = Shell::start(substring_keys(vec!["echo older", LOOP]));
    sh.send("echo");
    // The rest of "echo older" shows after the cursor as a suggestion.
    sh.wait_for("the typed text", |s| {
        cursor_row(s).starts_with("$ echo") && s.cursor_position() == (0, 6)
    });
    sh.send(UP);
    sh.wait_for("the loop", |s| {
        row_text(s, 2) == "done" && s.cursor_position() == (0, 2)
    });
    sh.send(UP);
    sh.wait_for("the older match", |s| cursor_row(s) == "$ echo older");
}

/// After Up finds a multi-line entry, Down moves through its lines and
/// past the last one goes on with the search: back to the typed text.
#[test]
fn down_through_a_found_multi_line_entry_goes_on_with_the_search() {
    let mut sh = Shell::start(substring_keys(vec!["echo older", LOOP]));
    sh.send("echo");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 6));
    sh.send(UP);
    sh.wait_for("the loop", |s| {
        row_text(s, 2) == "done" && s.cursor_position() == (0, 2)
    });
    sh.send(DOWN);
    sh.wait_for("the second line", |s| s.cursor_position().0 == 1);
    sh.send(DOWN);
    sh.wait_for("the third line", |s| s.cursor_position().0 == 2);
    sh.send(DOWN);
    sh.wait_for("the typed text back", |s| {
        cursor_row(s).starts_with("$ echo")
            && row_text(s, 1) != "do echo $x"
            && s.cursor_position() == (0, 6)
    });
}

/// Types a two-line command no history entry holds, goes Up to its first
/// line, and Up again, where the search finds nothing; then Down to the
/// last line and Down again. The search left the line as it was, so moving
/// between its lines ends it, and the last Down leaves the cursor where it
/// is.
fn down_after_a_failed_search_stays(options: Options) {
    let mut sh = Shell::start(options);
    block(&mut sh, &["echo a", "echo b"]);
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 6));
    sh.send(UP);
    let s = sh.settle();
    assert_eq!(s.cursor_position(), (0, 6), "{}", dump(&s));
    sh.send(DOWN);
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 6));
    sh.send(DOWN);
    let s = sh.settle();
    assert_eq!(row_text(&s, 0), "$ echo a", "{}", dump(&s));
    assert_eq!(row_text(&s, 1), "echo b", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (1, 6), "{}", dump(&s));
}

#[test]
fn down_after_a_failed_substring_search_stays() {
    down_after_a_failed_search_stays(substring_keys(vec!["ls"]));
}

#[test]
fn down_after_a_failed_prefix_search_stays() {
    down_after_a_failed_search_stays(arrow_keys(
        "previous-line-or-search",
        "next-line-or-search",
        vec!["ls"],
    ));
}

/// On a history entry that was changed, Down walks history: it does not
/// search for the changed text.
#[test]
fn down_on_a_changed_history_entry_walks_history() {
    let mut sh = Shell::start(substring_keys(vec!["echo one", "echo two"]));
    sh.settle();
    sh.send(UP);
    sh.wait_for("the newest entry", |s| cursor_row(s) == "$ echo two");
    sh.send(UP);
    sh.wait_for("the older entry", |s| cursor_row(s) == "$ echo one");
    sh.send("X");
    sh.wait_for("the change", |s| cursor_row(s) == "$ echo oneX");
    sh.send(DOWN);
    sh.wait_for("the newer entry", |s| cursor_row(s) == "$ echo two");
}

#[test]
fn up_goes_to_the_line_above_first() {
    let mut sh = Shell::start(substring_keys(vec!["echo old"]));
    block(&mut sh, &["echo a", "echo b"]);
    sh.send(UP);
    sh.wait_for("the first line", |s| s.cursor_position() == (0, 6));
}

/// Up on a line a search found and that was then changed searches again,
/// for the changed text, as it does on a typed line.
#[test]
fn up_on_a_changed_found_line_searches_again() {
    let mut sh = Shell::start(substring_keys(vec![
        "echo statX older",
        "git status",
        "ls",
        "echo stat",
    ]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the match", |s| cursor_row(s) == "$ echo stat");
    sh.send("X");
    // The rest of "echo statX older" shows after the cursor as a suggestion.
    sh.wait_for("the change", |s| {
        cursor_row(s).starts_with("$ echo statX") && s.cursor_position() == (0, 12)
    });
    // Going on with the search for "stat" would give "git status", and a
    // walk "ls".
    sh.send(UP);
    sh.wait_for("the match for the changed text", |s| {
        cursor_row(s) == "$ echo statX older" && s.cursor_position() == (0, 18)
    });
    // Nothing older holds "echo statX": the line and cursor stay, so Y
    // goes at the end.
    sh.send(UP);
    sh.send("Y");
    sh.wait_for("the line as it was", |s| {
        cursor_row(s) == "$ echo statX olderY" && s.cursor_position() == (0, 19)
    });
}

#[test]
fn up_past_the_oldest_match_keeps_the_cursor() {
    let mut sh = Shell::start(substring_keys(vec!["git status"]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the match", |s| {
        cursor_row(s) == "$ git status" && s.cursor_position() == (0, 12)
    });
    sh.send(UP);
    sh.send("Z");
    sh.wait_for("the cursor where it was", |s| {
        cursor_row(s) == "$ git statusZ" && s.cursor_position() == (0, 13)
    });
}

/// A walk through history places the cursor as readline does, here where
/// `history-preserve-point` keeps it.
#[test]
fn a_walk_keeps_readlines_cursor_place() {
    let mut options = substring_keys(vec!["echo one", "echo two"]);
    options
        .rc
        .push_str("bind 'set history-preserve-point on'\n");
    let mut sh = Shell::start(options);
    sh.settle();
    sh.send(UP);
    sh.wait_for("the newest entry", |s| {
        cursor_row(s) == "$ echo two" && s.cursor_position() == (0, 10)
    });
    sh.send("\x02");
    sh.wait_for("the cursor back one", |s| s.cursor_position() == (0, 9));
    sh.send(UP);
    sh.wait_for("the older entry, cursor kept", |s| {
        cursor_row(s) == "$ echo one" && s.cursor_position() == (0, 9)
    });
}
