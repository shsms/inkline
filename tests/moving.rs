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

/// Down past the newest match gives back the typed line with its undo
/// list: undo takes the typing off in one step and never shows a history
/// entry.
#[test]
fn undo_after_down_back_to_the_typed_text_takes_the_typing_off() {
    let mut sh = Shell::start(substring_keys(vec!["echo stat", "pwd"]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the match", |s| cursor_row(s) == "$ echo stat");
    sh.send(DOWN);
    sh.wait_for("the typed text back", |s| {
        cursor_row(s) == "$ stat" && s.cursor_position() == (0, 6)
    });
    sh.send("\x1f");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$", "{}", dump(&s));
    assert_eq!(s.cursor_position(), (0, 2), "{}", dump(&s));
}

/// A count goes that many matches back: two Ups from `git st` give
/// `git status`, then `git stash`, and so does `M-2 Up`.
#[test]
fn a_count_goes_that_many_matches_back() {
    let mut sh = Shell::start(substring_keys(vec!["git stash", "git status", "git st"]));
    sh.send("git st");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 8));
    sh.send("\x1b2");
    sh.send(UP);
    sh.wait_for("the second match", |s| {
        cursor_row(s) == "$ git stash" && s.cursor_position() == (0, 11)
    });
}

/// Once the search has ended, Down on the line it found walks history
/// from that entry: to the next newer one.
#[test]
fn down_on_a_found_line_after_the_search_ended_walks_on() {
    let mut sh = Shell::start(substring_keys(vec!["git status", "ls", "echo stat", "pwd"]));
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    sh.wait_for("the match", |s| {
        cursor_row(s) == "$ echo stat" && s.cursor_position() == (0, 11)
    });
    sh.send("\x02");
    sh.wait_for("the cursor back one", |s| s.cursor_position() == (0, 10));
    sh.send(DOWN);
    sh.wait_for("the next entry", |s| cursor_row(s) == "$ pwd");
    sh.send(DOWN);
    sh.wait_for("the typed text back", |s| cursor_row(s) == "$ stat");
}

/// A shell with `C-x k` bound to the prefix search and `C-x p` to the
/// substring search.
fn prefix_and_substring_keys() -> Options {
    Options {
        rc: "bind '\"\\C-xk\": previous-line-or-search'\n\
             bind '\"\\C-xp\": previous-line-or-substring-search'\n"
            .into(),
        history: vec!["git status", "xgit", "git log"],
        ..Options::default()
    }
}

/// On a line the prefix search of the Up key found, the substring key
/// (`C-p`) starts a new search for the text before the cursor, as on a line
/// it found itself, on every readline. Walking history from the match would
/// give `ls`: readline 8.3 leaves its history place on a prefix match, older
/// readline does not.
#[test]
fn the_substring_key_searches_again_on_a_line_the_prefix_key_found() {
    let mut sh = Shell::start(Options {
        inputrc: Some(
            "\"\\e[A\": history-search-backward\n\"\\e[B\": history-search-forward\n".into(),
        ),
        history: vec!["git status", "ls", "git log"],
        ..Options::default()
    });
    sh.send("git");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.send(UP);
    sh.wait_for("the prefix match", |s| cursor_row(s) == "$ git log");
    sh.send("\x10");
    sh.wait_for("the older match", |s| cursor_row(s) == "$ git status");
}

/// A prefix search that starts on a line the substring key found finds an
/// older entry; the substring key then searches older than that entry, not
/// older than the one the prefix search started on. Before readline 8.3
/// the prefix search puts its history place back where it was, on
/// `git log`, from where the substring key would find `echo git log`.
#[test]
fn the_substring_key_searches_older_than_a_prefix_match_from_a_found_line() {
    let mut sh = Shell::start(Options {
        inputrc: Some(
            "\"\\e[A\": history-search-backward\n\"\\e[B\": history-search-forward\n".into(),
        ),
        history: vec!["git log --all", "git log -p", "echo git log", "git log"],
        ..Options::default()
    });
    sh.send("git lo");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 8));
    sh.send("\x07");
    sh.send("\x10");
    sh.wait_for("the substring match", |s| cursor_row(s) == "$ git log");
    sh.send(UP);
    sh.wait_for("the prefix match", |s| cursor_row(s) == "$ git log -p");
    sh.send("\x10");
    sh.wait_for("the match older than it", |s| {
        cursor_row(s) == "$ git log --all"
    });
}

/// After a prefix search that went on to an older copy of an entry it
/// found before, the substring key searches older than that older copy.
#[test]
fn the_substring_key_searches_older_than_a_repeated_prefix_match() {
    let mut sh = Shell::start(Options {
        inputrc: Some(
            "\"\\e[A\": history-search-backward\n\"\\e[B\": history-search-forward\n".into(),
        ),
        history: vec!["echo git log", "git log X", "git log Y", "git log X"],
        ..Options::default()
    });
    sh.send("git log");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 9));
    sh.send("\x07");
    sh.send(UP);
    sh.wait_for("the newest match", |s| cursor_row(s) == "$ git log X");
    sh.send(UP);
    sh.wait_for("the next match", |s| cursor_row(s) == "$ git log Y");
    sh.send(UP);
    sh.wait_for("the oldest prefix match", |s| {
        cursor_row(s) == "$ git log X"
    });
    sh.send("\x10");
    sh.wait_for("the match older than it", |s| {
        cursor_row(s) == "$ echo git log"
    });
}

/// The prefix key after the substring key starts its own search, for the
/// text before the cursor: nothing older starts with `git log`, so the
/// line stays. Going on with a prefix search would walk to `xgit`.
#[test]
fn the_prefix_key_does_not_go_on_with_a_substring_search() {
    let mut sh = Shell::start(prefix_and_substring_keys());
    sh.send("git");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.send("\x18p");
    sh.wait_for("the substring match", |s| {
        cursor_row(s) == "$ git log" && s.cursor_position() == (0, 9)
    });
    sh.send("\x18k");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ git log", "{}", dump(&s));
}

/// The substring key after the prefix key starts its own search, for the
/// text before the cursor, which the prefix search left after `git`: it
/// finds `xgit`. Going on with the prefix search would find `git status`.
#[test]
fn the_substring_key_does_not_go_on_with_a_prefix_search() {
    let mut sh = Shell::start(prefix_and_substring_keys());
    sh.send("git");
    sh.wait_for("the typed text", |s| s.cursor_position() == (0, 5));
    sh.send("\x07");
    sh.send("\x18k");
    sh.wait_for("the prefix match", |s| {
        cursor_row(s) == "$ git log" && s.cursor_position() == (0, 5)
    });
    sh.send("\x18p");
    sh.wait_for("the substring match", |s| cursor_row(s) == "$ xgit");
}

/// With readline's `search-ignore-case` on (readline 8.3 and later), the
/// search ignores case, as readline's own substring search does; older
/// readline has no such setting, and the search keeps to case.
#[test]
fn search_ignore_case_finds_entries_in_any_case() {
    let mut options = substring_keys(vec!["git STATUS", "ls"]);
    options
        .rc
        .push_str("bind 'set search-ignore-case on' 2>/dev/null\n");
    let mut sh = Shell::start(options);
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    if bash_version() >= (5, 3) {
        sh.wait_for("the match", |s| cursor_row(s) == "$ git STATUS");
    } else {
        let s = sh.settle();
        assert_eq!(cursor_row(&s), "$ stat", "{}", dump(&s));
    }
}

/// An entry that is not valid UTF-8 is searched too.
#[test]
fn up_finds_an_entry_that_is_not_utf8() {
    let mut options = substring_keys(vec!["ls"]);
    options
        .rc
        .push_str("history -s $'echo \\xff stat'\nhistory -s pwd\n");
    let mut sh = Shell::start(options);
    sh.send("stat");
    sh.wait_for("the typed text", |s| cursor_row(s) == "$ stat");
    sh.send(UP);
    let s = sh.settle();
    assert!(cursor_row(&s).starts_with("$ echo "), "{}", dump(&s));
    assert!(cursor_row(&s).ends_with(" stat"), "{}", dump(&s));
}
