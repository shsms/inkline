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
        rc: "inkline eval '(setq inkline-colors \"error=4\")' >/dev/null\n".into(),
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
