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
