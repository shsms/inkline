//! Highlight helpers: starting them, their status, and what turns one off.

#[path = "support/common.rs"]
mod common;

use common::*;

/// An `init.el` line that registers the fake helper for `csvm`, doing `mode`.
pub fn fake(mode: &str) -> String {
    format!(
        "(inkline-highlight-arguments \"csvm\" (list \"{}/tests/data/fake-highlight\" \"{mode}\"))\n",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// An rc line for plain underlines, which the test terminal can see.
const PLAIN_UNDERLINE: &str = "inkline eval '(setq inkline-colors \"error=4\")' >/dev/null\n";

/// Whether `needle`, which must be ASCII, is on row `row` with every cell
/// underlined.
fn underlined_on_row(screen: &vt100::Screen, row: u16, needle: &str) -> bool {
    let Some(col) = row_text(screen, row).find(needle) else {
        return false;
    };
    (col..col + needle.len()).all(|c| screen.cell(row, c as u16).is_some_and(|c| c.underline()))
}

/// Runs `inkline status` from the prompt and waits for `want` among its rows
/// and for the next prompt.
fn status_row(sh: &mut Shell, want: &str) {
    sh.wait_for("the prompt", |s| cursor_row(s).starts_with('$'));
    sh.send("\x15inkline status\r");
    sh.wait_for(want, |s| has_row(s, want) && cursor_row(s) == "$");
}

/// Runs `command` from the prompt and waits for the next prompt, so the
/// keys sent after it reach readline rather than the running command.
fn run(sh: &mut Shell, command: &str) {
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mark = format!(
        "ran {}",
        RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    sh.send(&format!("\x15{command}; echo {mark}\r"));
    sh.wait_for(&mark, |s| {
        let row = s.cursor_position().0;
        cursor_row(s) == "$" && row > 0 && row_text(s, row - 1) == mark
    });
}

/// Runs `inkline status` from the prompt and returns the rows it printed
/// after `inkline: on`.
fn status_rows(sh: &mut Shell) -> Vec<String> {
    run(sh, "inkline status; echo status done");
    let s = sh.screen();
    let rows: Vec<String> = (0..s.size().0).map(|r| row_text(&s, r)).collect();
    let start = rows.iter().rposition(|r| r == "inkline: on").unwrap() + 1;
    let end = rows.iter().rposition(|r| r == "status done").unwrap();
    rows[start..end].to_vec()
}

#[test]
fn status_shows_each_helper() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("words")),
        ..Options::default()
    });
    status_row(&mut sh, "highlight csvm: not started");
}

#[test]
fn a_missing_program_is_off_with_its_reason() {
    let mut sh = Shell::start(Options {
        init_el: Some("(inkline-highlight-arguments \"csvm\" '(\"no-such-helper-xyz\"))\n".into()),
        ..Options::default()
    });
    sh.send("csvm 'a'");
    sh.wait_for("the message", |s| {
        has_row(s, "inkline: highlight csvm: off (not found)")
    });
    status_row(&mut sh, "highlight csvm: off (not found)");
}

#[test]
fn a_wrong_first_line_is_off() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("version")),
        ..Options::default()
    });
    sh.send("csvm 'a'");
    sh.wait_for("the message", |s| {
        has_row(s, "inkline: highlight csvm: off (not a highlight helper)")
    });
}

#[test]
fn the_helper_is_not_one_of_bashs_jobs() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("words")),
        ..Options::default()
    });
    sh.send("csvm 'a'");
    sh.wait_for("started", |s| cursor_row(s) == "$ csvm 'a'");
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    sh.send("jobs; wait; echo waited\r");
    sh.wait_for("wait returned", |s| has_row(s, "waited"));
    let s = sh.settle();
    assert!(
        !(0..s.size().0).any(|r| row_text(&s, r).starts_with('[')),
        "{}",
        dump(&s)
    );
    status_row(&mut sh, "highlight csvm: running");
}

#[test]
fn registering_again_and_removing() {
    let mut sh = Shell::start(Options {
        // Wide enough for the type error on one row.
        cols: 120,
        init_el: Some(fake("words")),
        ..Options::default()
    });
    sh.send("csvm 'a'");
    sh.wait_for("typed", |s| cursor_row(s) == "$ csvm 'a'");
    status_row(&mut sh, "highlight csvm: running");
    run(
        &mut sh,
        &format!(
            "inkline eval '{}'",
            fake("version").trim_end().replace('\'', "'\\''")
        ),
    );
    status_row(&mut sh, "highlight csvm: not started");
    run(
        &mut sh,
        "inkline eval '(inkline-highlight-arguments \"csvm\" nil)'",
    );
    let rows = status_rows(&mut sh);
    assert!(!rows.iter().any(|r| r.starts_with("highlight")), "{rows:?}");
    sh.send("inkline eval '(inkline-highlight-arguments 1 nil)'\r");
    sh.wait_for("the type error", |s| {
        has_row(
            s,
            "inkline: Wrong type argument: stringp, 1 (in (inkline-highlight-arguments 1 nil))",
        )
    });
}

#[test]
fn reload_stops_helpers() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("words")),
        ..Options::default()
    });
    // `init.el` does not register this one: only `stop_all` removes it.
    run(
        &mut sh,
        &format!(
            "inkline eval '{}'",
            fake("words").trim_end().replace("\"csvm\"", "\"other\"")
        ),
    );
    status_row(&mut sh, "highlight other: not started");
    sh.send("\x15other 'a'");
    sh.wait_for("typed", |s| cursor_row(s) == "$ other 'a'");
    status_row(&mut sh, "highlight other: running");
    // Keys typed ahead while `inkline reload` runs are sometimes lost: `run`
    // waits for the next prompt.
    run(&mut sh, "inkline reload");
    let rows = status_rows(&mut sh);
    assert_eq!(rows[1..], ["highlight csvm: not started"], "{rows:?}");
}

#[test]
fn a_plain_name_is_found_in_bashs_path() {
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(
        format!("{}/tests/data/fake-highlight", env!("CARGO_MANIFEST_DIR")),
        dir.path().join("csvm-highlight"),
    )
    .unwrap();
    // Not exported: only bash's own `PATH` holds the directory, never the
    // environment of the process.
    let mut sh = Shell::start(Options {
        before_inkline: format!("export -n PATH\nPATH={}:$PATH\n", dir.path().display()),
        init_el: Some(
            "(inkline-highlight-arguments \"csvm\" '(\"csvm-highlight\" \"words\"))\n".into(),
        ),
        ..Options::default()
    });
    sh.send("csvm 'a'");
    sh.wait_for("typed", |s| cursor_row(s) == "$ csvm 'a'");
    status_row(&mut sh, "highlight csvm: running");
}

/// An `init.el` line that registers the fake helper for `csvm`, doing
/// `mode` and logging each request's arguments to `log`.
fn fake_logging(mode: &str, log: &std::path::Path) -> String {
    format!(
        "(inkline-highlight-arguments \"csvm\" (list \"/usr/bin/env\" \"FAKE_LOG={}\" \"{}/tests/data/fake-highlight\" \"{mode}\"))\n",
        log.display(),
        env!("CARGO_MANIFEST_DIR")
    )
}

/// As `fake_logging`, giving the command `colors` as its own.
fn fake_logging_with_colors(mode: &str, log: &std::path::Path, colors: &str) -> String {
    format!(
        "(inkline-highlight-arguments \"csvm\" (list \"/usr/bin/env\" \"FAKE_LOG={}\" \"{}/tests/data/fake-highlight\" \"{mode}\") {colors})\n",
        log.display(),
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Waits until the fake helper has logged `line`.
fn wait_for_log(log: &std::path::Path, line: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let logged = std::fs::read_to_string(log).unwrap_or_default();
        if logged.lines().any(|l| l == line) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!(
        "no {line:?} in the log:\n{}",
        std::fs::read_to_string(log).unwrap_or_default()
    );
}

/// A helper gets the variables bash exports as they are when it starts,
/// not as they were when bash started.
#[test]
fn a_helper_gets_the_shells_exported_variables() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = Shell::start(Options {
        init_el: Some(fake_logging("words", &log)),
        ..Options::default()
    });
    run(&mut sh, "export CSVM_X=1");
    sh.send("\x15csvm 'a'");
    wait_for_log(&log, "CSVM_X:1");
}

/// `enable -d inkline` stops the helpers: each sees the end of its input.
#[test]
fn enable_d_stops_helpers() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = Shell::start(Options {
        init_el: Some(fake_logging("words", &log)),
        ..Options::default()
    });
    sh.send("csvm 'a'");
    wait_for_log(&log, "CSVM_X:unset");
    run(&mut sh, "enable -d inkline");
    wait_for_log(&log, "end");
}

/// Waits up to five seconds for `path` to exist.
fn wait_for_file(path: &std::path::Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "no {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// The helper holds none of bash's other descriptors: once bash closes one,
/// the program at its other end sees the end of its input.
#[test]
fn the_helper_holds_none_of_bashs_descriptors() {
    let dir = tempfile::tempdir().unwrap();
    let done = dir.path().join("done");
    let log = dir.path().join("log");
    let mut sh = Shell::start(Options {
        init_el: Some(fake_logging("words", &log)),
        ..Options::default()
    });
    run(
        &mut sh,
        &format!("exec 4> >(cat >/dev/null; echo >{})", done.display()),
    );
    sh.send("\x15csvm 'a'");
    wait_for_log(&log, "CSVM_X:unset");
    run(&mut sh, "exec 4>&-");
    wait_for_file(&done);
}

#[test]
fn a_script_is_coloured_and_dimmed() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("words")),
        ..Options::default()
    });
    sh.send("csvm 'select a 12 | sort b' x.csv");
    sh.wait_for("colours", |s| fg_is(s, "select", Color::Idx(2)));
    let s = sh.settle();
    assert_eq!(fg(&s, "12"), Color::Idx(6));
    assert_eq!(fg(&s, "sort"), Color::Idx(2));
    assert!(cell(&s, "select").unwrap().dim());
    assert!(!cell(&s, "'select").unwrap().dim(), "the quote mark");
    assert_eq!(fg(&s, "x.csv"), Color::Default);
}

/// A command's own colours go on its script; the keys they leave out come
/// from `inkline-colors`. Registering the same program again keeps the
/// helper running and only changes the colours.
#[test]
fn a_command_can_have_colours_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let own = r#"'((command . "bold magenta") (script . "on grey3"))"#;
    let mut sh = Shell::start(Options {
        init_el: Some(fake_logging_with_colors("words", &log, own)),
        ..Options::default()
    });
    sh.send("csvm 'select a 12' x.csv");
    sh.wait_for("the command's colours", |s| {
        fg_is(s, "select", Color::Idx(5))
    });
    let s = sh.settle();
    let select = cell(&s, "select").unwrap();
    assert!(select.bold() && !select.dim(), "{}", dump(&s));
    assert_eq!(select.bgcolor(), Color::Idx(235), "{}", dump(&s));
    assert_eq!(
        fg(&s, "a 12"),
        Color::Idx(4),
        "variable, from inkline-colors"
    );
    assert_eq!(fg(&s, "12"), Color::Idx(6), "number, from inkline-colors");
    assert_eq!(
        cell(&s, "'select").unwrap().bgcolor(),
        Color::Default,
        "the quote mark is bash's"
    );
    assert_eq!(fg(&s, "x.csv"), Color::Default);
    let again = fake_logging_with_colors("words", &log, r#"'((command . "yellow"))"#);
    run(
        &mut sh,
        &format!("inkline eval '{}'", again.trim_end().replace('\'', "'\\''")),
    );
    sh.send("\x15csvm 'select b'");
    sh.wait_for("the new colours", |s| fg_is(s, "select", Color::Idx(3)));
    let s = sh.settle();
    assert!(
        cell(&s, "select").unwrap().dim(),
        "the script style is inkline-colors' again:\n{}",
        dump(&s)
    );
    let logged = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        logged.lines().filter(|l| l.starts_with("CSVM_X:")).count(),
        1,
        "the helper started once:\n{logged}"
    );
}

#[test]
fn double_quoted_scripts_map_back_to_what_was_typed() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("words")),
        ..Options::default()
    });
    // The helper gets `sélect "a" 7`: its offsets skip the backslashes.
    sh.send("csvm \"sélect \\\"a\\\" 7\"");
    sh.wait_for("colours", |s| fg_is(s, "sélect", Color::Idx(2)));
    let s = sh.settle();
    assert_eq!(fg(&s, "7"), Color::Idx(6));
    assert!(
        !cell(&s, "\\\"a").unwrap().dim(),
        "a removed backslash is not the helper's"
    );
}

#[test]
fn a_bash_variable_inside_the_script_keeps_its_colour() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("words")),
        ..Options::default()
    });
    sh.send("csvm \"select $x 7\"");
    sh.wait_for("colours", |s| fg_is(s, "select", Color::Idx(2)));
    let s = sh.settle();
    assert_eq!(fg(&s, "$x"), Color::Idx(4));
    assert_eq!(fg(&s, "7"), Color::Idx(6));
    assert!(cell(&s, "select").unwrap().dim());
    assert!(!cell(&s, "\"select").unwrap().dim(), "the quote mark");
}

#[test]
fn two_commands_on_one_line() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("words")),
        ..Options::default()
    });
    sh.send("csvm 'one' | csvm 'two'");
    sh.wait_for("both", |s| {
        fg_is(s, "one", Color::Idx(2)) && fg_is(s, "two", Color::Idx(2))
    });
}

/// A new line forgets the replies kept for the one before: the files they
/// speak of may have changed since. The same line recalled is asked about
/// again.
#[test]
fn a_new_line_asks_again() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = Shell::start(Options {
        init_el: Some(fake_logging("words", &log)),
        ..Options::default()
    });
    sh.send("csvm 'a'");
    wait_for_log(&log, "final:a");
    sh.send("\r");
    sh.wait_for("the next prompt", |s| cursor_row(s) == "$");
    sh.send("\x1b[A");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let asked = || {
        let logged = std::fs::read_to_string(&log).unwrap_or_default();
        logged.lines().filter(|l| *l == "final:a").count()
    };
    while asked() < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "asked {} times",
            asked()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// `exec FD>file` after the helper started leaves both the file and the
/// helper working.
fn a_users_descriptor_is_left_alone(fd: u32) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("out");
    let mut sh = Shell::start(Options {
        init_el: Some(fake("words")),
        ..Options::default()
    });
    sh.send("csvm 'select a'");
    sh.wait_for("colours", |s| fg_is(s, "select", Color::Idx(2)));
    run(&mut sh, &format!("exec {fd}>{}", file.display()));
    sh.send("\x15csvm 'sort b'");
    sh.wait_for("colours", |s| fg_is(s, "sort", Color::Idx(2)));
    run(&mut sh, &format!("echo hi >&{fd}"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hi\n");
    status_row(&mut sh, "highlight csvm: running");
}

#[test]
fn a_users_low_descriptor_is_left_alone() {
    a_users_descriptor_is_left_alone(3);
}

/// bash takes a close-on-exec descriptor from 10 up for one of its own and
/// puts it back after `exec 10>file`, so the socket is not on 10 either.
#[test]
fn a_users_descriptor_10_is_left_alone() {
    a_users_descriptor_is_left_alone(10);
}

/// A command that closes inkline's end of the socket owns that descriptor
/// from then on: the helper is turned off, and a file opened there stays
/// open. (bash puts back a close-on-exec descriptor from 10 up after `exec
/// N>file`, taking it for one of its own: it must be closed first.)
#[test]
fn a_socket_taken_over_is_left_to_the_user() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("out");
    let mut sh = Shell::start(Options {
        init_el: Some(fake("words")),
        ..Options::default()
    });
    sh.send("csvm 'select a'");
    sh.wait_for("colours", |s| fg_is(s, "select", Color::Idx(2)));
    // The shell's only socket is inkline's end.
    run(
        &mut sh,
        &format!(
            "for f in /proc/$$/fd/*; do [[ $(readlink $f) == socket:* ]] && n=${{f##*/}}; done; \
             eval \"exec $n>&-; exec $n>{}\"",
            file.display()
        ),
    );
    sh.send("\x15csvm 'sort b'");
    sh.wait_for("off", |s| {
        has_row(s, "inkline: highlight csvm: off (connection lost)")
    });
    run(&mut sh, "echo hi >&$n");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hi\n");
}

#[test]
fn a_helper_that_exits_is_turned_off() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("exit")),
        ..Options::default()
    });
    sh.send("csvm 'a'");
    sh.wait_for("off", |s| {
        has_row(s, "inkline: highlight csvm: off (exited)")
    });
    sh.send(" 'b'");
    sh.send("\x15echo still here\r");
    sh.wait_for("bash is alive", |s| has_row(s, "still here"));
}

#[test]
fn garbage_turns_a_helper_off() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("garbage")),
        ..Options::default()
    });
    sh.send("csvm 'a'");
    sh.wait_for("off", |s| {
        has_row(s, "inkline: highlight csvm: off (bad reply: \"nonsense\")")
    });
}

#[test]
fn a_late_reply_is_painted_without_a_key() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("late")),
        ..Options::default()
    });
    sh.send("csvm 'select a'");
    sh.wait_for("typed", |s| cursor_row(s) == "$ csvm 'select a'");
    // No key after this: the reply comes about 0.5 s later.
    sh.wait_for("the late colours", |s| fg_is(s, "select", Color::Idx(2)));
}

#[test]
fn a_stale_reply_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = Shell::start(Options {
        init_el: Some(fake_logging("late", &log)),
        ..Options::default()
    });
    // The reply for `7` makes its first byte a number.
    sh.send("csvm '7'");
    wait_for_log(&log, "final:7");
    sh.send("\x15csvm 'zz'");
    // The request for `zz` goes out once the reply for `7` has come, and
    // its reply comes about 0.5 s later.
    wait_for_log(&log, "final:zz");
    std::thread::sleep(std::time::Duration::from_millis(150));
    let s = sh.screen();
    assert_eq!(cursor_row(&s), "$ csvm 'zz'");
    assert_eq!(
        fg(&s, "zz"),
        fg(&s, "'zz"),
        "the reply for `7` is not painted on `zz`:\n{}",
        dump(&s)
    );
    sh.wait_for("the new colours", |s| fg_is(s, "zz", Color::Idx(2)));
}

#[test]
fn a_late_reply_waits_for_a_question_to_be_answered() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("late")),
        rc: "complete -W 'a1 a2 a3' csvm\n".into(),
        inputrc: Some("set completion-query-items 2\n".into()),
        ..Options::default()
    });
    sh.send("csvm 'select' a");
    sh.wait_for("typed", |s| cursor_row(s) == "$ csvm 'select' a");
    sh.send("\t\t");
    let question = "Display all 3 possibilities? (y or n)";
    sh.wait_for("the question", |s| cursor_row(s) == question);
    // Well past the reply.
    std::thread::sleep(std::time::Duration::from_millis(800));
    let s = sh.settle();
    assert_eq!(cursor_row(&s), question, "{}", dump(&s));
    assert!(has_row(&s, "$ csvm 'select' a"), "{}", dump(&s));
    assert!(!fg_is(&s, "select", Color::Idx(2)), "{}", dump(&s));
    sh.send("n");
    sh.wait_for("the colours", |s| {
        cursor_row(s) == "$ csvm 'select' a" && fg_is(s, "select", Color::Idx(2))
    });
}

#[test]
fn part_of_a_reply_does_not_hold_up_the_underline() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("split")),
        rc: PLAIN_UNDERLINE.into(),
        ..Options::default()
    });
    // The first span comes during the pause; the rest 2 s later.
    let began = std::time::Instant::now();
    sh.send("csvm 'a b'; echo ) x");
    sh.wait_for("the underline", |s| underlined(s, ")"));
    let took = began.elapsed();
    // The rest of the reply comes 2 s after its first span; the bound
    // leaves room for a slow start.
    assert!(
        took < std::time::Duration::from_millis(1700),
        "took {took:?}"
    );
    sh.wait_for("the colours", |s| fg_is(s, "b", Color::Idx(4)));
}

#[test]
fn an_error_is_underlined_after_the_pause_with_its_message() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("error")),
        rc: PLAIN_UNDERLINE.into(),
        ..Options::default()
    });
    sh.send("csvm 'bad a'");
    // The message has `bad` too: the underline is looked for on the line.
    sh.wait_for("the underline", |s| underlined_on_row(s, 0, "bad"));
    sh.wait_for("the message", |s| has_row(s, "csvm: unknown command 'bad'"));
}

/// The message stays as long as the underline does: a change to the line
/// that keeps the error's place keeps both.
#[test]
fn an_errors_message_stays_with_its_underline() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = Shell::start(Options {
        init_el: Some(fake_logging("error", &log)),
        rc: PLAIN_UNDERLINE.into(),
        ..Options::default()
    });
    sh.send("csvm 'bad a'");
    sh.wait_for("the underline", |s| underlined_on_row(s, 0, "bad"));
    sh.wait_for("the message", |s| has_row(s, "csvm: unknown command 'bad'"));
    sh.send(" x");
    wait_for_log(&log, "final:x");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ csvm 'bad a' x");
    assert!(underlined_on_row(&s, 0, "bad"), "{}", dump(&s));
    assert!(has_row(&s, "csvm: unknown command 'bad'"), "{}", dump(&s));
}

#[test]
fn no_underline_while_typing_the_word() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("error")),
        rc: PLAIN_UNDERLINE.into(),
        ..Options::default()
    });
    sh.send("csvm 'x bad");
    sh.wait_for("colours", |s| fg_is(s, "x", Color::Idx(2)));
    let s = sh.settle();
    assert!(!any_underlined(&s), "{}", dump(&s));
}

#[test]
fn a_bash_error_wins() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("error")),
        rc: PLAIN_UNDERLINE.into(),
        ..Options::default()
    });
    sh.send("csvm 'bad a' | | x");
    sh.wait_for("colours", |s| fg_is(s, "bad", Color::Idx(2)));
    sh.wait_for("bash's underline", any_underlined);
    let s = sh.settle();
    assert!(!underlined(&s, "bad"), "{}", dump(&s));
    assert!(!has_row(&s, "csvm: unknown command 'bad'"));
}

#[test]
fn no_error_in_a_raw_argument() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("error")),
        rc: PLAIN_UNDERLINE.into(),
        ..Options::default()
    });
    sh.send("csvm \"bad $x\" y");
    sh.wait_for("colours", |s| fg_is(s, "bad", Color::Idx(2)));
    // Well past the pause.
    std::thread::sleep(std::time::Duration::from_millis(400));
    let s = sh.settle();
    assert!(!underlined_on_row(&s, 0, "bad"), "{}", dump(&s));
    assert!(!has_row(&s, "csvm: unknown command 'bad'"), "{}", dump(&s));
}

#[test]
fn an_error_with_no_place_shows_only_its_message() {
    let mut sh = Shell::start(Options {
        init_el: Some(fake("noplace")),
        rc: PLAIN_UNDERLINE.into(),
        ..Options::default()
    });
    sh.send("csvm 'a' x");
    sh.wait_for("the message", |s| has_row(s, "csvm: no place"));
    assert!(!any_underlined(&sh.settle()));
}

#[test]
fn a_notice_comes_before_an_errors_message() {
    let mut sh = Shell::start(Options {
        init_el: Some(format!(
            "{}(inkline-highlight-arguments \"nosuch\" '(\"no-such-helper-xyz\"))\n",
            fake("error")
        )),
        rc: PLAIN_UNDERLINE.into(),
        ..Options::default()
    });
    sh.send("nosuch a; csvm 'bad a' x");
    let notice = "inkline: highlight nosuch: off (not found)";
    let error = "csvm: unknown command 'bad'";
    let s = sh.wait_for("the notice", |s| has_row(s, notice));
    assert!(underlined_on_row(&s, 0, "bad"), "{}", dump(&s));
    let s = sh.settle();
    assert!(!has_row(&s, error), "{}", dump(&s));
    // A key that leaves the line as it is takes the notice away.
    sh.send("\x02");
    sh.wait_for("the error's message", |s| {
        has_row(s, error) && !has_row(s, notice) && underlined_on_row(s, 0, "bad")
    });
}

/// The `command` colour, which the fake gives the first word of a stage.
const COMMAND: Color = Color::Idx(2);
/// The `variable` colour, which the fake gives the other words.
const VARIABLE: Color = Color::Idx(4);

/// An `init.el` with `inkline-indent` 2 and the fake helper for `csvm`
/// doing `mode`, logging to `log`.
fn indenting(mode: &str, log: &std::path::Path) -> String {
    format!("(setq inkline-indent 2)\n{}", fake_logging(mode, log))
}

/// Types `csvm 'warm'`, waits for its colours, and empties the line: the
/// helper is then running.
fn warm_up(sh: &mut Shell) {
    sh.send("csvm 'warm'");
    sh.wait_for("the helper's colours", |s| fg_is(s, "warm", COMMAND));
    sh.send("\x01\x0b");
    sh.wait_for("an empty line", |s| cursor_row(s) == "$");
}

/// Sends `keys`, then waits until `word` has the colour `color`: the
/// helper has answered for the line as it is now, so no colour request is
/// in flight when the next key comes.
fn type_then(sh: &mut Shell, keys: &str, word: &str, color: Color) {
    sh.send(keys);
    sh.wait_for(&format!("{word} coloured"), |s| fg_is(s, word, color));
}

/// The text of the first `n` rows.
fn rows(s: &vt100::Screen, n: u16) -> Vec<String> {
    (0..n).map(|r| row_text(s, r)).collect()
}

/// Waits a little for requests to reach the log, then checks that none
/// asked for depths.
fn assert_no_indent_request(log: &std::path::Path) {
    std::thread::sleep(std::time::Duration::from_millis(200));
    let logged = std::fs::read_to_string(log).unwrap_or_default();
    assert!(!logged.lines().any(|l| l.starts_with("at:")), "{logged}");
}

/// Starts a shell with `inkline-indent` 2 and the fake doing `mode`, and
/// warms the helper up. The log's directory must live as long as the
/// shell.
fn indenting_shell(mode: &str) -> (Shell, tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = Shell::start(Options {
        init_el: Some(indenting(mode, &log)),
        ..Options::default()
    });
    warm_up(&mut sh);
    (sh, dir, log)
}

#[test]
fn a_scripts_lines_are_one_step_in() {
    let (mut sh, _dir, log) = indenting_shell("indent");
    type_then(&mut sh, "csvm \"head", "head", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 2));
    wait_for_log(&log, "at:1 4");
    type_then(&mut sh, "| sort x", "sort", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the third line", |s| s.cursor_position() == (2, 2));
    type_then(&mut sh, "| etc", "etc", COMMAND);
    let s = sh.settle();
    assert_eq!(
        rows(&s, 3),
        ["$ csvm \"head", "  | sort x", "  | etc\""],
        "{}",
        dump(&s)
    );
}

#[test]
fn a_line_after_a_pipe_is_one_step_in() {
    let (mut sh, _dir, _log) = indenting_shell("indent");
    type_then(&mut sh, "csvm \"head |", "head", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 2));
    type_then(&mut sh, "sort x", "sort", COMMAND);
    let s = sh.settle();
    assert_eq!(
        rows(&s, 2),
        ["$ csvm \"head |", "  sort x\""],
        "{}",
        dump(&s)
    );
}

/// A script that starts on its own line: C-j between the empty quotes
/// opens them, and the closing quote goes on a line of its own.
#[test]
fn a_script_on_lines_of_its_own() {
    let (mut sh, _dir, _log) = indenting_shell("indent");
    sh.send("csvm \"");
    sh.wait_for("the empty script", |s| cursor_row(s) == "$ csvm \"\"");
    sh.send(CTRL_J);
    sh.wait_for("the first script line", |s| {
        s.cursor_position() == (1, 2) && row_text(s, 2) == "\""
    });
    type_then(&mut sh, "head", "head", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the next line", |s| s.cursor_position() == (2, 2));
    type_then(&mut sh, "| sort x", "sort", COMMAND);
    sh.send(&format!("{DOWN}\x05 data.csv"));
    let s = sh.wait_for("the whole command", |s| row_text(s, 3) == "\" data.csv");
    assert_eq!(
        rows(&s, 4),
        ["$ csvm \"", "  head", "  | sort x", "\" data.csv"],
        "{}",
        dump(&s)
    );
}

#[test]
fn a_group_adds_a_step_and_its_closer_moves_out() {
    let (mut sh, _dir, _log) = indenting_shell("indent");
    type_then(&mut sh, "csvm \"head", "head", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 2));
    type_then(&mut sh, "| join (", "join", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("inside the group", |s| s.cursor_position() == (2, 4));
    type_then(&mut sh, "cols a,b", "cols", VARIABLE);
    sh.send(CTRL_J);
    sh.wait_for("still inside", |s| s.cursor_position() == (3, 4));
    type_then(&mut sh, ") other.csv on a", "other.csv", VARIABLE);
    sh.send(CTRL_J);
    sh.wait_for("after the group", |s| {
        s.cursor_position() == (4, 2) && row_text(s, 3) == "  ) other.csv on a"
    });
    type_then(&mut sh, "| sort x", "sort", COMMAND);
    let s = sh.settle();
    assert_eq!(
        rows(&s, 5),
        [
            "$ csvm \"head",
            "  | join (",
            "    cols a,b",
            "  ) other.csv on a",
            "  | sort x\"",
        ],
        "{}",
        dump(&s)
    );
}

#[test]
fn a_function_body_adds_a_step() {
    let (mut sh, _dir, _log) = indenting_shell("indent");
    sh.send("csvm '");
    sh.wait_for("the empty script", |s| cursor_row(s) == "$ csvm ''");
    sh.send(CTRL_J);
    sh.wait_for("the first script line", |s| {
        s.cursor_position() == (1, 2) && row_text(s, 2) == "'"
    });
    type_then(&mut sh, "fn prep(n) {", "fn", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("inside the body", |s| s.cursor_position() == (2, 4));
    type_then(&mut sh, "rename value=n", "rename", VARIABLE);
    sh.send(CTRL_J);
    sh.wait_for("still inside", |s| s.cursor_position() == (3, 4));
    type_then(&mut sh, "| cols -v metric", "cols", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the closing line", |s| s.cursor_position() == (4, 4));
    sh.send("}");
    sh.wait_for("the brace", |s| row_text(s, 4) == "    }");
    sh.send(CTRL_J);
    sh.wait_for("the brace moved out", |s| {
        s.cursor_position() == (5, 2) && row_text(s, 4) == "  }"
    });
    type_then(&mut sh, "prep(pv)", "prep(pv)", VARIABLE);
    sh.send(&format!("{DOWN}\x05 pv.csv"));
    let s = sh.wait_for("the whole command", |s| row_text(s, 6) == "' pv.csv");
    assert_eq!(
        rows(&s, 7),
        [
            "$ csvm '",
            "  fn prep(n) {",
            "    rename value=n",
            "    | cols -v metric",
            "  }",
            "  prep(pv)",
            "' pv.csv",
        ],
        "{}",
        dump(&s)
    );
}

/// Types `csvm \`, Enter, and a script six spaces in on the next line,
/// then adds a line inside the script.
fn split_after_a_backslash(sh: &mut Shell) {
    sh.send("csvm \\\r");
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 2));
    type_then(sh, "    \"head x", "head", COMMAND);
    sh.send(CTRL_J);
}

/// The line where the script's quote opens is its first line: it never
/// moves, even when the command's name is on a line above it.
#[test]
fn the_line_the_script_starts_on_stays() {
    let (mut sh, _dir, log) = indenting_shell("indent");
    split_after_a_backslash(&mut sh);
    let s = sh.wait_for("the new line", |s| s.cursor_position() == (2, 2));
    wait_for_log(&log, "at:1 6");
    assert_eq!(
        rows(&s, 3),
        ["$ csvm \\", "      \"head x", "  \""],
        "{}",
        dump(&s)
    );
}

/// Without depths, a new line after the script's first line is one step
/// in from the command's line, as it is when the quote opens on that line.
#[test]
fn without_depths_the_line_the_script_starts_on_gives_one_step() {
    let (mut sh, _dir, log) = indenting_shell("words");
    split_after_a_backslash(&mut sh);
    let s = sh.wait_for("the new line", |s| s.cursor_position() == (2, 2));
    assert_eq!(
        rows(&s, 3),
        ["$ csvm \\", "      \"head x", "  \""],
        "{}",
        dump(&s)
    );
    assert_no_indent_request(&log);
}

/// The closing quote of an empty pair goes as far in as the command's
/// line, not the line the quote opens on, and no depths are asked for.
#[test]
fn an_empty_pair_opens_from_the_commands_line() {
    let (mut sh, _dir, log) = indenting_shell("indent");
    sh.send("csvm \\\r");
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 2));
    sh.send("    \"");
    sh.wait_for("the empty script", |s| row_text(s, 1) == "      \"\"");
    sh.send(CTRL_J);
    let s = sh.wait_for("the pair opened", |s| {
        s.cursor_position() == (2, 2) && row_text(s, 3) == "\""
    });
    assert_eq!(
        rows(&s, 4),
        ["$ csvm \\", "      \"", "", "\""],
        "{}",
        dump(&s)
    );
    assert_no_indent_request(&log);
}

/// Opening an empty pair needs no depths, so a helper that cannot indent
/// opens it the same way.
#[test]
fn an_empty_pair_opens_without_depths() {
    let (mut sh, _dir, _log) = indenting_shell("words");
    sh.send("if true; then\r");
    sh.wait_for("inside the block", |s| s.cursor_position() == (1, 2));
    sh.send("csvm \"");
    sh.wait_for("the empty script", |s| row_text(s, 1) == "  csvm \"\"");
    sh.send(CTRL_J);
    let s = sh.wait_for("the pair opened", |s| {
        s.cursor_position() == (2, 4) && row_text(s, 3) == "  \""
    });
    assert_eq!(
        rows(&s, 4),
        ["$ if true; then", "  csvm \"", "", "  \""],
        "{}",
        dump(&s)
    );
}

#[test]
fn one_undo_closes_the_opened_pair() {
    let (mut sh, _dir, _log) = indenting_shell("indent");
    sh.send("csvm \"");
    sh.wait_for("the empty script", |s| cursor_row(s) == "$ csvm \"\"");
    sh.send(CTRL_J);
    sh.wait_for("the pair opened", |s| row_text(s, 2) == "\"");
    sh.send("\x1f");
    sh.wait_for("the pair as it was", |s| {
        cursor_row(s) == "$ csvm \"\"" && s.cursor_position() == (0, 8) && row_text(s, 1).is_empty()
    });
}

/// With room for only one more line, C-j adds one line and leaves the pair
/// closed: readline cannot draw a command taller than the screen.
#[test]
fn an_empty_pair_stays_closed_on_a_full_screen() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = Shell::start(Options {
        rows: 4,
        init_el: Some(indenting("words", &log)),
        ..Options::default()
    });
    warm_up(&mut sh);
    sh.send("if true; then\r");
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 2));
    sh.send("a\r");
    sh.wait_for("the third line", |s| s.cursor_position() == (2, 2));
    sh.send("csvm \"");
    sh.wait_for("the empty script", |s| row_text(s, 2) == "  csvm \"\"");
    sh.send(CTRL_J);
    let s = sh.wait_for("one new line", |s| s.cursor_position() == (3, 4));
    assert_eq!(
        rows(&s, 4),
        ["$ if true; then", "  a", "  csvm \"", "    \""],
        "{}",
        dump(&s)
    );
}

/// An empty pair of quotes in bash code is a plain string: C-j adds a line
/// inside it and leaves the closing quote after the cursor.
#[test]
fn an_empty_bash_string_is_not_opened() {
    let (mut sh, _dir, _log) = indenting_shell("indent");
    sh.send("echo \"");
    sh.wait_for("the empty string", |s| cursor_row(s) == "$ echo \"\"");
    sh.send(CTRL_J);
    let s = sh.wait_for("the new line", |s| s.cursor_position() == (1, 0));
    assert_eq!(rows(&s, 3), ["$ echo \"", "\"", ""], "{}", dump(&s));
}

/// Enter adds a line while the quote is open (C-v keeps `"` from being
/// paired), and moves a closing line out.
#[test]
fn enter_moves_a_closing_line_out() {
    let (mut sh, _dir, _log) = indenting_shell("indent");
    type_then(&mut sh, "csvm \x16\"fn f {", "fn", COMMAND);
    sh.send("\r");
    sh.wait_for("inside the body", |s| s.cursor_position() == (1, 4));
    type_then(&mut sh, "a", "a", VARIABLE);
    sh.send("\r");
    sh.wait_for("still inside", |s| s.cursor_position() == (2, 4));
    sh.send("}\r");
    sh.wait_for("the brace moved out", |s| {
        s.cursor_position() == (3, 2) && row_text(s, 2) == "  }"
    });
}

#[test]
fn one_undo_takes_back_the_new_line_and_the_move_out() {
    let (mut sh, _dir, _log) = indenting_shell("indent");
    type_then(&mut sh, "csvm 'fn f {", "fn", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("inside the body", |s| s.cursor_position() == (1, 4));
    type_then(&mut sh, "a", "a", VARIABLE);
    sh.send(CTRL_J);
    sh.wait_for("still inside", |s| s.cursor_position() == (2, 4));
    sh.send("}");
    sh.wait_for("the brace", |s| row_text(s, 2) == "    }'");
    sh.send(CTRL_J);
    sh.wait_for("the brace moved out", |s| row_text(s, 2) == "  }");
    sh.send("\x1f");
    // Readline's undo leaves the cursor where the first edit of the group
    // was, at the line's start plus its old indentation, so only its row is
    // checked.
    sh.wait_for("the line as it was", |s| {
        row_text(s, 2) == "    }'" && s.cursor_position().0 == 2 && row_text(s, 3).is_empty()
    });
}

/// The spaces and tabs around the cursor go inside a script too.
#[test]
fn the_blanks_around_the_cursor_go_in_a_script() {
    let (mut sh, _dir, _log) = indenting_shell("indent");
    type_then(&mut sh, "csvm \"head   | sort x", "sort", COMMAND);
    sh.send(&"\x02".repeat(11));
    sh.wait_for("the cursor after head", |s| s.cursor_position() == (0, 12));
    sh.send(CTRL_J);
    let s = sh.wait_for("the new line", |s| {
        s.cursor_position() == (1, 2) && row_text(s, 1) == "  | sort x\""
    });
    assert_eq!(row_text(&s, 0), "$ csvm \"head", "{}", dump(&s));
}

/// Without depths in time, a new line after the command's line goes one step
/// in, and a new line after a later line gets that line's indentation.
/// Neither waits for the late depths, which do not turn the helper off.
#[test]
fn no_depths_in_time_keeps_the_line_above() {
    let (mut sh, _dir, log) = indenting_shell("slow-indent");
    type_then(&mut sh, "csvm \"head", "head", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 2));
    wait_for_log(&log, "at:1 4");
    type_then(&mut sh, " | sort x", "sort", COMMAND);
    let began = std::time::Instant::now();
    sh.send(CTRL_J);
    sh.wait_for("the third line", |s| s.cursor_position() == (2, 3));
    let took = began.elapsed();
    assert!(
        took < std::time::Duration::from_millis(450),
        "took {took:?}"
    );
    // C-u would clear only the last line of the command: C-c drops it all.
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    status_row(&mut sh, "highlight csvm: running");
}

#[test]
fn c_c_while_waiting_for_depths() {
    let (mut sh, _dir, _log) = indenting_shell("slow-indent");
    type_then(&mut sh, "csvm \"head", "head", COMMAND);
    sh.send(CTRL_J);
    std::thread::sleep(std::time::Duration::from_millis(30));
    sh.send("\x03");
    sh.wait_for("a new prompt", |s| cursor_row(s) == "$");
    sh.send("echo still here\r");
    sh.wait_for("bash is alive", |s| has_row(s, "still here"));
    status_row(&mut sh, "highlight csvm: running");
}

#[test]
fn a_helper_that_cannot_tell_keeps_the_line_above() {
    let (mut sh, _dir, log) = indenting_shell("indent");
    type_then(&mut sh, "csvm \"nodepth", "nodepth", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 2));
    wait_for_log(&log, "at:1 7");
    type_then(&mut sh, " | x", "x", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the third line", |s| s.cursor_position() == (2, 3));
}

#[test]
fn a_helper_without_indent_is_not_asked_for_depths() {
    let (mut sh, _dir, log) = indenting_shell("words");
    type_then(&mut sh, "csvm \"head", "head", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("one step in", |s| s.cursor_position() == (1, 2));
    type_then(&mut sh, " | sort x", "sort", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the line above's indentation", |s| {
        s.cursor_position() == (2, 3)
    });
    assert_no_indent_request(&log);
}

#[test]
fn a_raw_script_is_not_asked_for_depths() {
    let (mut sh, _dir, log) = indenting_shell("indent");
    type_then(&mut sh, "csvm \"head $x", "head", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("one step in", |s| s.cursor_position() == (1, 2));
    type_then(&mut sh, " | sort", "sort", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the line above's indentation", |s| {
        s.cursor_position() == (2, 3)
    });
    assert_no_indent_request(&log);
}

#[test]
fn pasted_lines_are_not_asked_for_depths() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = Shell::start(Options {
        init_el: Some(indenting("indent", &log)),
        rc: "bind 'set enable-bracketed-paste off'\n".into(),
        ..Options::default()
    });
    warm_up(&mut sh);
    sh.send("csvm 'head\n| sort x'");
    let s = sh.wait_for("the pasted lines", |s| row_text(s, 1) == "| sort x'");
    assert_eq!(row_text(&s, 0), "$ csvm 'head", "{}", dump(&s));
    assert_no_indent_request(&log);
}

#[test]
fn inkline_indent_0_asks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = Shell::start(Options {
        init_el: Some(format!(
            "(setq inkline-indent 0)\n{}",
            fake_logging("indent", &log)
        )),
        ..Options::default()
    });
    warm_up(&mut sh);
    type_then(&mut sh, "csvm \"head", "head", COMMAND);
    sh.send(CTRL_J);
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 0));
    assert_no_indent_request(&log);
}

/// A line added by a Lisp command asks for no depths.
#[test]
fn a_lisp_command_asks_for_no_depths() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let mut sh = Shell::start(Options {
        init_el: Some(format!(
            "{}(defun add-line () (interactive) (call-interactively 'insert-newline))\n\
             (keymap-global-set \"C-x j\" 'add-line)\n",
            indenting("indent", &log)
        )),
        ..Options::default()
    });
    warm_up(&mut sh);
    type_then(&mut sh, "csvm \"head", "head", COMMAND);
    sh.send("\x18j");
    sh.wait_for("the second line", |s| s.cursor_position() == (1, 2));
    assert_no_indent_request(&log);
}
