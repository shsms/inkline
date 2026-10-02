//! Completion items from mode servers, in the menu under the line.

#[path = "support/common.rs"]
mod common;

use common::*;

const C_N: &str = "\x0e";

/// A shell whose `csvm` command uses a mode run by `program` (the words of
/// a Lisp list), with `more` added to `init.el` and `history` as its history.
fn start(program: &str, more: &str, history: Vec<&'static str>) -> Shell {
    Shell::start(Options {
        init_el: Some(format!(
            "(inkline-define-mode 'fake-mode '({program}))\n\
             (push '(\"csvm\" . fake-mode) inkline-command-mode-alist)\n{more}"
        )),
        history,
        ..Options::default()
    })
}

/// The fake mode server in mode `does`, as the words of a Lisp list.
fn server(does: &str) -> String {
    format!("\"{}\" \"{does}\"", fake_mode_server().display())
}

/// A shell whose `csvm` command uses the fake mode server in mode `does`,
/// with `more` added to `init.el` and `history` as its history.
fn shell_with(does: &str, more: &str, history: Vec<&'static str>) -> Shell {
    start(&server(does), more, history)
}

fn shell(does: &str) -> Shell {
    shell_with(does, "", Vec::new())
}

/// As `shell_with`, with the server logging each request's arguments and
/// each `:at` to `log` (see `tests/data/fake-mode-server`).
fn shell_logging(
    does: &str,
    log: &std::path::Path,
    more: &str,
    history: Vec<&'static str>,
) -> Shell {
    let program = format!(
        "\"/usr/bin/env\" \"FAKE_LOG={}\" {}",
        log.display(),
        server(does)
    );
    start(&program, more, history)
}

/// Waits until the fake mode server has logged a line that `wanted` takes;
/// all the lines it has logged by then.
fn wait_for_log(log: &std::path::Path, what: &str, wanted: impl Fn(&str) -> bool) -> Vec<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let logged = std::fs::read_to_string(log).unwrap_or_default();
        let lines: Vec<String> = logged.lines().map(str::to_owned).collect();
        if lines.iter().any(|l| wanted(l)) {
            return lines;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no {what} in the log:\n{logged}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn picked(s: &vt100::Screen, row: u16) -> bool {
    s.cell(row, 0).is_some_and(|c| c.inverse())
}

#[test]
fn mode_items_show_with_their_notes() {
    let mut sh = shell("complete");
    // An empty word: every item that fits is listed.
    sh.send("csvm 'sort ");
    // Items pad to the widest shown one, `first name`.
    let s = sh.wait_for("the items", |s| {
        has_row(s, "m  sort        sort the rows")
            && has_row(s, "m  amount      column")
            && has_row(s, "m  first name  column")
    });
    // `it's` cannot go inside single quotes.
    assert!(find(&s, "it's").is_none(), "{}", dump(&s));
}

#[test]
fn a_taken_item_is_quoted_for_where_it_goes() {
    for (typed, want) in [
        ("csvm 'sort fi", "$ csvm 'sort first name'"),
        ("csvm \"sort fi", "$ csvm \"sort first name\""),
        ("csvm sort fi", "$ csvm sort first\\ name"),
    ] {
        let mut sh = shell("complete");
        sh.send(typed);
        // Outside quotes the row shows the item as it goes in: `first\ name`.
        sh.wait_for("the items", |s| find(s, "m  first").is_some());
        sh.send(&format!("{C_N}\t"));
        sh.wait_for("the item taken", |s| cursor_row(s).starts_with(want));
    }
}

#[test]
fn late_items_show_without_a_key() {
    let mut sh = shell("late-complete");
    sh.send("csvm 'sort am");
    // No key after the typing: the reply comes 0.5 s later and is drawn.
    sh.wait_for("the late items", |s| find(s, "m  amount").is_some());
}

/// Tab before the mode server's items have come rings no bell: they show
/// when they come.
#[test]
fn tab_while_items_are_coming_waits_for_them() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!(
            "(inkline-define-mode 'fake-mode '({}))\n\
             (push '(\"csvm\" . fake-mode) inkline-command-mode-alist)\n",
            server("late-complete")
        )),
        rc: "bind 'set bell-style audible'\n".into(),
        ..Options::default()
    });
    sh.send("csvm 'sort am");
    sh.wait_for("the line", |s| find(s, "csvm 'sort am").is_some());
    sh.take_output();
    sh.send("\t");
    sh.wait_for("the late items", |s| find(s, "m  amount").is_some());
    let out = sh.take_output();
    assert!(!out.contains(&0x07), "no bell: {out:?}");
}

/// While moving, the menu keeps the rows it had at the first move: items
/// that come later are not added.
#[test]
fn late_items_wait_while_moving() {
    // Without pairing, the cursor stays at the end of the line, so the
    // history item shows at once.
    let mut sh = shell_with(
        "late-complete",
        "(inkline-unbind-defaults 'pairing)\n",
        vec!["csvm 'sort amended'"],
    );
    sh.send("csvm 'sort am");
    sh.wait_for("the history item", |s| {
        row_text(s, 1) == "h  csvm 'sort amended'"
    });
    sh.send(C_N);
    sh.wait_for("the pick", |s| {
        picked(s, 1) && cursor_row(s) == "$ csvm 'sort amended'"
    });
    // The late reply comes 0.5 s after the request.
    std::thread::sleep(std::time::Duration::from_millis(1000));
    let s = sh.settle();
    assert!(find(&s, "m  amount").is_none(), "{}", dump(&s));
    assert!(picked(&s, 1), "{}", dump(&s));
}

/// Tab with one row starts moving while the mode server's items are still
/// coming, as more rows may come: it does not write the row and stop.
#[test]
fn tab_with_one_row_moves_while_items_are_coming() {
    let mut sh = shell_with(
        "late-complete",
        "(inkline-unbind-defaults 'pairing)\n",
        vec!["csvm 'sort amended'"],
    );
    sh.send("csvm 'sort am");
    sh.wait_for("the history item", |s| {
        row_text(s, 1) == "h  csvm 'sort amended'"
    });
    sh.send("\t");
    sh.wait_for("the row picked", |s| {
        picked(s, 1) && cursor_row(s) == "$ csvm 'sort amended'"
    });
}

#[test]
fn a_server_that_did_not_name_complete_gives_no_items() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = shell_logging("words", &log, "", Vec::new());
    sh.send("csvm 'sort am");
    let s = sh.settle();
    assert!(find(&s, "m  amount").is_none(), "{}", dump(&s));
    // The server reads its requests in order: once it has the colour
    // request for a later line, it has logged any `:complete` sent before.
    sh.send(" x");
    let lines = wait_for_log(&log, "the later line", |l| l.ends_with(":sort am x"));
    assert!(!lines.iter().any(|l| l.starts_with("at:")), "{lines:?}");
}

#[test]
fn a_server_that_named_complete_is_asked_where_the_cursor_is() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = shell_logging("complete", &log, "", Vec::new());
    // The cursor on the command's name: no items are asked for, even once
    // typing pauses.
    sh.send("csvm");
    wait_for_log(&log, "the command's name", |l| l == "final:csvm");
    sh.settle();
    sh.send(" 'sort am");
    sh.wait_for("the items", |s| find(s, "m  amount").is_some());
    // Requests are logged in order, so a `:complete` for the name comes
    // before this one.
    let lines = wait_for_log(&log, "the :at", |l| l == "at:1 7");
    assert!(!lines.iter().any(|l| l.starts_with("at:0 ")), "{lines:?}");
}

#[test]
fn a_line_brought_back_from_history_is_not_asked_for_items() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = shell_logging("complete", &log, "", vec!["csvm 'sort am'"]);
    sh.settle();
    sh.send("\x10");
    sh.wait_for("the entry", |s| cursor_row(s) == "$ csvm 'sort am'");
    // Typing pauses on the line brought back. Once the server has the
    // colour request for a changed line, it has logged any `:complete`
    // sent for the line brought back.
    sh.settle();
    sh.send(" x");
    let lines = wait_for_log(&log, "the changed line", |l| l == "final:x");
    assert!(!lines.iter().any(|l| l == "at:1 7"), "{lines:?}");
}

#[test]
fn items_are_asked_for_once_typing_pauses() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = shell_logging("complete", &log, "", Vec::new());
    sh.send("csvm 'sort am");
    wait_for_log(&log, "the colours", |l| l == "final:sort am");
    // Well within the pause after the last key, the colours have been
    // answered but no items asked for.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let logged = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(!logged.contains("at:"), "{logged}");
    sh.wait_for("the items", |s| find(s, "m  amount").is_some());
    wait_for_log(&log, "the :at", |l| l == "at:1 7");
}

#[test]
fn a_moved_cursor_waits_for_a_new_pause() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = shell_logging("complete", &log, "", Vec::new());
    sh.send("csvm 'sort am");
    wait_for_log(&log, "the :at", |l| l == "at:1 7");
    // C-b moves the cursor before `m`; `z` comes before any pause there.
    sh.send("\x02z");
    let lines = wait_for_log(&log, "the changed line", |l| l == "final:sort azm");
    assert!(!lines.iter().any(|l| l == "at:1 6"), "{lines:?}");
}

/// A move that hides the menu, with the cursor before the line's end where no
/// grey text shows, asks for no items even once typing pauses there.
#[test]
fn a_move_that_hides_the_menu_asks_for_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = shell_logging("complete", &log, "", Vec::new());
    sh.send("csvm 'sort am");
    wait_for_log(&log, "the :at", |l| l == "at:1 7");
    sh.send("\x02");
    sh.settle();
    std::thread::sleep(std::time::Duration::from_millis(200));
    // Requests are logged in order: once the changed line's colours are logged,
    // so is any `:complete` sent before them.
    sh.send("z");
    let lines = wait_for_log(&log, "the changed line", |l| l == "final:sort azm");
    assert!(!lines.iter().any(|l| l == "at:1 6"), "{lines:?}");
}

#[test]
fn the_innermost_command_is_asked() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    // The menu stays while the cursor moves back.
    let mut sh = shell_logging(
        "complete",
        &log,
        "(setq inkline-menu-on-move t)",
        Vec::new(),
    );
    // Both commands use the mode; the cursor goes back into the inner
    // one's argument, at offset 7 of `sort am` and 16 of the outer one's.
    sh.send("csvm \"$(csvm 'sort am')\"\x02\x02\x02");
    sh.wait_for("the items", |s| find(s, "m  amount").is_some());
    let lines = wait_for_log(&log, "the :at", |l| l == "at:1 7");
    assert!(!lines.iter().any(|l| l == "at:1 16"), "{lines:?}");
}

#[test]
fn a_bad_item_turns_the_server_off() {
    let mut sh = shell("bad-complete");
    sh.send("csvm 'x");
    sh.wait_for("the notice", |s| {
        has_row(
            s,
            "inkline: mode fake-mode: off (bad reply: \":item x 0 a\")",
        )
    });
}

#[test]
fn status_shows_complete() {
    let mut sh = shell("complete");
    sh.send("csvm 'x'\x18\x7finkline status\r");
    sh.wait_for("the status", |s| {
        has_row(s, "mode fake-mode (csvm): running (complete)")
    });
}
