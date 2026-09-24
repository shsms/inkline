#[path = "support/common.rs"]
mod common;

use common::*;

fn with_history(history: Vec<&'static str>) -> Options {
    Options {
        history,
        ..Options::default()
    }
}

fn showing(opts: Options, keys: &str, row: &str) -> Shell {
    let mut sh = Shell::start(opts);
    sh.send(keys);
    sh.wait_for("the suggestion", |s| cursor_row(s) == row);
    sh
}

#[test]
fn suggests_the_newest_matching_entry() {
    let sh = showing(
        with_history(vec!["git stash", "git status"]),
        "git st",
        "$ git status",
    );
    let s = sh.screen();
    assert_eq!(fg(&s, "atus"), Color::Idx(8));
    assert_eq!(s.cursor_position(), (0, 8));
}

#[test]
fn hidden_when_the_cursor_moves_left() {
    let mut sh = showing(with_history(vec!["git status"]), "git st", "$ git status");
    sh.send("\x02");
    sh.wait_for("no suggestion", |s| cursor_row(s) == "$ git st");
}

#[test]
fn nothing_without_a_match() {
    let mut sh = Shell::start(with_history(vec!["git status"]));
    sh.send("zzz");
    assert_eq!(cursor_row(&sh.settle()), "$ zzz");
}

#[test]
fn erased_after_enter() {
    let mut sh = showing(
        with_history(vec!["echo hello-world"]),
        "echo hel",
        "$ echo hello-world",
    );
    sh.send("\r");
    let s = sh.wait_for("the output", |s| has_row(s, "hel"));
    assert_eq!(row_text(&s, 0), "$ echo hel");
}

#[test]
fn erased_after_enter_in_same_burst() {
    let mut sh = Shell::start(with_history(vec!["echo hello-world"]));
    sh.send("echo hel\r");
    let s = sh.wait_for("the output", |s| has_row(s, "hel"));
    assert_eq!(row_text(&s, 0), "$ echo hel");
}

#[test]
fn erased_after_ctrl_c() {
    let mut sh = showing(
        with_history(vec!["echo hello-world"]),
        "echo hel",
        "$ echo hello-world",
    );
    sh.send("\x03");
    let s = sh.wait_for("a new prompt", |s| {
        s.cursor_position().0 == 1 && cursor_row(s) == "$"
    });
    assert!(row_text(&s, 0).starts_with("$ echo hel"));
    // `^C` is echoed over the first two cells of the suggestion.
    assert!(!row_text(&s, 0).contains("world"));
}

#[test]
fn erased_after_ctrl_o() {
    let mut sh = showing(
        with_history(vec!["echo hello-world"]),
        "echo hel",
        "$ echo hello-world",
    );
    sh.send("\x0f");
    let s = sh.wait_for("the output", |s| has_row(s, "hel"));
    assert_eq!(row_text(&s, 0), "$ echo hel");
}

#[test]
fn erased_before_completion_listing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("zz1"), "").unwrap();
    std::fs::write(dir.path().join("zz2"), "").unwrap();
    let opts = Options {
        history: vec!["ls zz-long-suggestion"],
        cwd: Some(dir.path().to_path_buf()),
        ..Options::default()
    };
    let mut sh = showing(opts, "ls z", "$ ls zz-long-suggestion");
    sh.send("\t");
    sh.wait_for("the common prefix", |s| s.cursor_position() == (0, 7));
    // The TAB after a partial completion completes again; the next one lists.
    sh.send("\t\t");
    let s = sh.wait_for("the listing", |s| {
        find(s, "zz1").is_some() && s.cursor_position().0 > 1
    });
    assert_eq!(row_text(&s, 0), "$ ls zz");
}

/// A background job ending interrupts the wait for a key; the suggestion stays.
#[test]
fn kept_when_a_background_job_ends() {
    let mut sh = Shell::start(with_history(vec!["echo hello-world"]));
    sh.send("sleep 0.5 &\r");
    sh.wait_for("the next prompt", |s| s.cursor_position().0 == 2);
    sh.send("echo hel");
    sh.wait_for("the suggestion", |s| cursor_row(s) == "$ echo hello-world");
    std::thread::sleep(std::time::Duration::from_secs(1));
    assert_eq!(cursor_row(&sh.screen()), "$ echo hello-world");
    sh.send("\x05\r");
    sh.wait_for("the output", |s| has_row(s, "hello-world"));
}

#[test]
fn none_while_searching() {
    let mut sh = Shell::start(with_history(vec!["echo hello-world"]));
    sh.send("\x12hel");
    let s = sh.wait_for("the search", |s| {
        cursor_row(s) == "(reverse-i-search)`hel': echo hello-world"
    });
    assert_eq!(cursor_row(&sh.settle()), cursor_row(&s));
}

#[test]
fn none_while_reading_a_count() {
    let mut sh = showing(
        with_history(vec!["echo hello-world"]),
        "echo hel",
        "$ echo hello-world",
    );
    sh.send("\x1b3");
    sh.wait_for("the count prompt", |s| cursor_row(s) == "(arg: 3) echo hel");
}

#[test]
fn multi_line_history_entry() {
    showing(
        with_history(vec!["for i in 1\ndo echo $i; done"]),
        "for i",
        "$ for i in 1",
    );
}

/// When the line exactly fills a row, the suggestion starts at column 0 of the
/// next row, which is where readline leaves the cursor after Enter.
#[test]
fn erased_after_enter_in_same_burst_at_row_end() {
    let mut sh = Shell::start(Options {
        cols: 20,
        // Longer than the output, so an unerased suggestion shows after it.
        history: vec!["echo aaaaaaaaaaaaa-and-much-more-text"],
        ..Options::default()
    });
    sh.send("echo aaaaaaaaaaaaa\r");
    let s = sh.wait_for("the output", |s| has_row(s, "aaaaaaaaaaaaa"));
    assert_eq!(row_text(&s, 0), "$ echo aaaaaaaaaaaaa");
}

const LOOP: &str = "for x in a b; do\n    echo $x\ndone";

#[test]
fn a_multi_line_entry_is_suggested_whole() {
    let mut sh = Shell::start(Options {
        history: vec![LOOP],
        ..Options::default()
    });
    sh.send("for x");
    let s = sh.wait_for("the suggestion", |s| {
        has_row(s, "    echo $x") && has_row(s, "done")
    });
    assert_eq!(s.cursor_position(), (0, 7));
    assert_eq!(fg(&s, "done"), Color::Idx(8));
}

#[test]
fn accepting_takes_every_line() {
    let mut sh = Shell::start(Options {
        history: vec![LOOP],
        ..Options::default()
    });
    sh.send("for x");
    sh.wait_for("the suggestion", |s| has_row(s, "done"));
    sh.send("\x05");
    sh.wait_for("the accepted text in colour", |s| {
        fg_is(s, "done", Color::Idx(5))
    });
}

#[test]
fn erased_when_enter_comes_in_the_same_burst() {
    let mut sh = Shell::start(Options {
        history: vec!["echo hi\necho there"],
        ..Options::default()
    });
    sh.send("echo h\r");
    let s = sh.wait_for("the output", |s| has_row(s, "h"));
    sh.settle();
    assert!(find(&s, "there").is_none(), "{}", dump(&s));
}

#[test]
fn scrolls_at_the_bottom_and_comes_back() {
    let mut sh = Shell::start(Options {
        rows: 6,
        history: vec![LOOP],
        ..Options::default()
    });
    sh.send("\r\r\r\r\r\r");
    sh.wait_for("the prompt on the last row", |s| s.cursor_position().0 == 5);
    sh.send("for x");
    sh.wait_for("the suggestion", |s| has_row(s, "done"));
    sh.send(" ");
    sh.wait_for("the cursor after the space", |s| {
        cursor_row(s).starts_with("$ for x in a b; do") && s.cursor_position().1 == 8
    });
}

#[test]
fn a_long_suggestion_is_cut() {
    let mut sh = Shell::start(Options {
        rc: "INKLINE_SUGGESTION_LINES=3\n".into(),
        history: vec!["echo 1\necho 2\necho 3\necho 4\necho 5"],
        ..Options::default()
    });
    sh.send("echo 1");
    let s = sh.wait_for("the cut suggestion", |s| {
        find(s, "… 2 more lines").is_some()
    });
    assert!(has_row(&s, "echo 2"));
    assert!(find(&s, "echo 4").is_none());
}

#[test]
fn redrawn_once_after_a_resize() {
    let mut sh = Shell::start(Options {
        history: vec![LOOP],
        ..Options::default()
    });
    sh.send("for x");
    sh.wait_for("the suggestion", |s| has_row(s, "done"));
    sh.resize(24, 60);
    let s = sh.settle();
    let dones = (0..24).filter(|&row| row_text(&s, row) == "done").count();
    assert_eq!(dones, 1, "{}", dump(&s));
}

#[test]
fn every_row_erased_when_the_line_stops_matching() {
    let mut sh = Shell::start(Options {
        history: vec![LOOP],
        ..Options::default()
    });
    sh.send("for x");
    sh.wait_for("the suggestion", |s| has_row(s, "done"));
    sh.send("z");
    sh.wait_for("the typed key", |s| cursor_row(s) == "$ for xz");
    let s = sh.settle();
    assert!(find(&s, "done").is_none(), "{}", dump(&s));
}
