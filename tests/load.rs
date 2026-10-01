#[path = "support/common.rs"]
mod common;

use std::path::PathBuf;

use common::*;

#[test]
fn loads_and_switches_on_and_off() {
    let script = format!(
        "enable -f {} inkline && inkline status && inkline off && inkline && inkline on && inkline status; inkline bogus; echo rc=$?",
        so_path().display()
    );
    let out = bash_command().arg("-c").arg(script).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "inkline: on\ninit.el: not read (not an interactive shell with line editing on)\nbash completion: on\n\
         inkline: off\ninit.el: not read (not an interactive shell with line editing on)\nbash completion: on\n\
         inkline: on\ninit.el: not read (not an interactive shell with line editing on)\nbash completion: on\nrc=2\n"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: inkline [on|off|status|"));
}

#[test]
fn write_errors_do_not_kill_the_shell() {
    let script = format!(
        "enable -f {} inkline; inkline status > /dev/full; echo status=$?; inkline bogus 2> /dev/full; echo bogus=$?",
        so_path().display()
    );
    let out = bash_command().arg("-c").arg(script).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "status=1\nbogus=2\n");
    assert!(out.status.success(), "bash exited with {:?}", out.status);
}

/// The bashes older than 5.3 to try, with their versions:
/// `$INKLINE_TEST_OLDER_BASH` when it is set; else those `make` built under
/// `target/` for testing, and the one on `PATH`.
fn older_bashes() -> Vec<(PathBuf, (u32, u32))> {
    if let Some(bash) = std::env::var_os("INKLINE_TEST_OLDER_BASH") {
        let bash = std::path::absolute(bash).unwrap();
        let version = version_of(&bash);
        assert!(
            version < (5, 3),
            "INKLINE_TEST_OLDER_BASH is bash {}.{}, not older than 5.3",
            version.0,
            version.1
        );
        return vec![(bash, version)];
    }
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target");
    let mut bashes: Vec<PathBuf> = std::fs::read_dir(target)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path().join("bin/bash"))
        .filter(|bash| bash.is_file())
        .collect();
    bashes.push(PathBuf::from("bash"));
    bashes
        .into_iter()
        .map(|bash| {
            let version = version_of(&bash);
            (bash, version)
        })
        .filter(|(_, version)| *version < (5, 3))
        .collect()
}

/// A bash older than 5.3 refuses the library: inkline says why in one line,
/// `enable -f` fails, no `inkline` builtin is left, and an interactive shell
/// that tried to load it goes on running commands. Skipped when no such bash
/// is at hand, unless `$INKLINE_TEST_REQUIRE_OLDER` is set.
#[test]
fn refuses_bash_older_than_5_3() {
    let older = older_bashes();
    if older.is_empty() {
        let why = "no bash older than 5.3 on PATH or under target/; build one with \
                   `scripts/build-bash.sh 5.2`, or set INKLINE_TEST_OLDER_BASH";
        assert!(
            std::env::var_os("INKLINE_TEST_REQUIRE_OLDER").is_none(),
            "INKLINE_TEST_REQUIRE_OLDER is set, but there is {why}"
        );
        eprintln!("skipped: {why}");
        return;
    }
    for (bash, (major, minor)) in older {
        let refusal = format!("inkline: needs bash 5.3 or later; this is bash {major}.{minor}");
        let script = format!(
            "enable -f {} inkline; echo rc=$?; type -t inkline || echo none",
            so_path().display()
        );
        let out = bash_command_at(&bash)
            .arg("-c")
            .arg(script)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "rc=1\nnone\n",
            "{bash:?}: {stderr}"
        );
        let ours: Vec<&str> = stderr
            .lines()
            .filter(|line| line.starts_with("inkline:"))
            .collect();
        assert_eq!(ours, [refusal.as_str()], "{bash:?}: {stderr}");

        let mut sh = Shell::start(Options {
            bash: Some(bash.clone()),
            inkline: false,
            rc: format!("enable -f {} inkline\n", so_path().display()),
            ..Options::default()
        });
        sh.send("echo ok");
        let screen = sh.settle();
        assert!(has_row(&screen, &refusal), "{bash:?}:\n{}", dump(&screen));
        // Nothing of inkline's is hooked into readline: the line is not
        // highlighted.
        assert_eq!(fg(&screen, "echo"), Color::Default, "{bash:?}");
        sh.send(ENTER);
        sh.wait_for(&format!("{bash:?} to run a command"), |s| {
            has_row(s, "ok") && cursor_row(s) == "$"
        });
    }
}
