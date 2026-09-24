#[path = "support/common.rs"]
mod common;

use common::*;

fn run(script: &str) -> (String, String) {
    let script = format!("enable -f {} inkline; {script}", so_path().display());
    let out = bash_command().arg("-c").arg(script).output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn eval_prints_results_and_errors() {
    let (out, err) = run(
        r#"inkline eval '(+ 1 2)'; inkline eval nil; inkline eval '(car 1 2)'; echo rc=$?; inkline eval; echo usage=$?"#,
    );
    assert_eq!(out, "3\nrc=1\nusage=2\n");
    assert!(
        err.contains("inkline: Too many arguments (in (car 1 2))"),
        "{err}"
    );
}

#[test]
fn load_runs_a_file_and_names_the_failing_line() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good.el");
    std::fs::write(&good, "(defun twice (x) (* 2 x))\n").unwrap();
    let bad = dir.path().join("bad.el");
    std::fs::write(&bad, "(setq a 1)\n(no-such-function)\n").unwrap();
    let (out, err) = run(&format!(
        "inkline load {}; inkline eval '(twice 4)'; inkline load {}; echo rc=$?",
        good.display(),
        bad.display()
    ));
    assert_eq!(out, "8\nrc=1\n");
    assert!(
        err.contains(&format!("inkline: {}:2: ", bad.display())),
        "{err}"
    );
}
