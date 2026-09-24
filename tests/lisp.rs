#[path = "support/common.rs"]
mod common;

use common::*;

#[test]
fn init_el_is_read_at_start() {
    let mut sh = Shell::start(Options {
        init_el: Some("(setq inkline-colors \"command=35\")\n".into()),
        ..Options::default()
    });
    sh.send("ls");
    sh.wait_for("the new colour", |s| fg_is(s, "ls", Color::Idx(5)));
    sh.send("\x15inkline status\r");
    sh.wait_for("the status", |s| {
        (0..s.size().0).any(|r| {
            row_text(s, r).starts_with("init.el: ") && row_text(s, r).ends_with("(loaded)")
        })
    });
}

#[test]
fn a_broken_init_el_is_reported_with_its_line() {
    let sh = Shell::start(Options {
        init_el: Some("(setq inkline-indent 2)\n(no-such-function)\n".into()),
        ..Options::default()
    });
    let s = sh.settle();
    assert!(
        (0..s.size().0).any(|r| row_text(&s, r).contains("init.el:2: ")),
        "{}",
        dump(&s)
    );
}

#[test]
fn a_group_writable_init_el_is_not_read() {
    use std::os::unix::fs::PermissionsExt;
    let cfg = tempfile::tempdir().unwrap();
    let dir = cfg.path().join("inkline");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = dir.join("init.el");
    std::fs::write(&file, "(setq inkline-indent 2)").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o664)).unwrap();
    let mut sh = Shell::start(Options {
        before_inkline: format!("XDG_CONFIG_HOME={}\n", cfg.path().display()),
        ..Options::default()
    });
    let refused = format!(
        "inkline: {}: not read: writable by group or others",
        file.display()
    );
    sh.wait_for("the refusal", |s| {
        (0..s.size().0).any(|r| row_text(s, r) == refused)
    });
    sh.send("inkline status\r");
    sh.wait_for("the status", |s| {
        (0..s.size().0).any(|r| row_text(s, r).ends_with("(skipped: writable by group or others)"))
    });
}

#[test]
fn non_interactive_shells_read_no_init_el() {
    let out = bash_command()
        .arg("-c")
        .arg(format!(
            "enable -f {} inkline; inkline status",
            so_path().display()
        ))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "inkline: on\ninit.el: not read (not an interactive shell with line editing on)\n"
    );
}

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

/// A shell whose `init.el` panics (`inkline--panic` is only in debug
/// builds), once the panic is reported.
#[cfg(debug_assertions)]
fn start_with_a_panicking_init_el(home: &std::path::Path) -> Shell {
    let sh = Shell::start(Options {
        home: Some(home.to_owned()),
        init_el: Some("(inkline--panic)\n".into()),
        ..Options::default()
    });
    sh.wait_for("the panic", |s| {
        has_row(s, "inkline: internal error, turned off")
    });
    sh
}

#[cfg(debug_assertions)]
#[test]
fn enable_after_a_panic_in_init_el_switches_on() {
    let home = tempfile::tempdir().unwrap();
    let mut sh = start_with_a_panicking_init_el(home.path());
    sh.send(&format!(
        "enable -f {} inkline; inkline status\r",
        so_path().display()
    ));
    let s = sh.wait_for("the status", |s| {
        has_row(s, "inkline: on") || has_row(s, "inkline: off")
    });
    assert!(has_row(&s, "inkline: on"), "{}", dump(&s));
}

#[cfg(debug_assertions)]
#[test]
fn a_panic_in_init_el_shows_in_status() {
    let home = tempfile::tempdir().unwrap();
    let mut sh = start_with_a_panicking_init_el(home.path());
    sh.send("inkline status\r");
    sh.wait_for("the status", |s| {
        has_row(s, "inkline: off")
            && (0..s.size().0).any(|r| row_text(s, r).ends_with("(error: internal error)"))
    });
}

#[test]
fn an_old_inkline_variable_is_reported() {
    let home = tempfile::tempdir().unwrap();
    let sh = Shell::start(Options {
        cols: 200,
        home: Some(home.path().to_owned()),
        before_inkline: "INKLINE_INDENT=2\n".into(),
        ..Options::default()
    });
    let warning = format!(
        "inkline: INKLINE_INDENT is no longer read; set inkline-indent in {}",
        home.path().join(".config/inkline/init.el").display()
    );
    sh.wait_for("the warning", |s| has_row(s, &warning));
}

/// The rows of the screen once a shell started with `init_el` settles.
fn start_up_rows(init_el: &str) -> Vec<String> {
    let sh = Shell::start(Options {
        cols: 200,
        init_el: Some(init_el.into()),
        ..Options::default()
    });
    let s = sh.settle();
    (0..s.size().0).map(|r| row_text(&s, r)).collect()
}

#[test]
fn a_parse_error_in_init_el_is_reported_with_its_line() {
    let rows = start_up_rows("(setq inkline-indent 2)\n(setq a\n");
    assert!(
        rows.iter().any(|r| r.ends_with("init.el:2: Unclosed list")),
        "{rows:#?}"
    );
}

#[test]
fn a_compile_error_in_init_el_is_reported_with_its_line() {
    let rows = start_up_rows("(setq inkline-indent 2)\n(defun f ()\n  (car 1 2))\n");
    assert!(
        rows.iter()
            .any(|r| r.ends_with("init.el:3: Too many arguments (in (car 1 2))")),
        "{rows:#?}"
    );
}

#[test]
fn init_el_is_found_through_xdg_config_home() {
    use std::os::unix::fs::PermissionsExt;
    let cfg = tempfile::tempdir().unwrap();
    let dir = cfg.path().join("inkline");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = dir.join("init.el");
    std::fs::write(&file, "(setq inkline-indent 3)\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut sh = Shell::start(Options {
        cols: 200,
        before_inkline: format!("XDG_CONFIG_HOME={}\n", cfg.path().display()),
        ..Options::default()
    });
    sh.send("inkline status; inkline eval inkline-indent\r");
    let loaded = format!("init.el: {} (loaded)", file.display());
    sh.wait_for("the status and the value", |s| {
        has_row(s, &loaded) && has_row(s, "3")
    });
}

/// `XDG_CONFIG_HOME/inkline/init.el` as a link to `dotfiles/init.el`, as
/// dotfile managers set it up, with the link's directory at `link_mode` and
/// the file's at `real_mode`; with `middle_mode`, the link goes through a
/// second link, `shared/init.el`, in a directory with that mode. Returns the
/// link, the file, and the screen's rows once the status and
/// `inkline-indent` are shown.
fn status_through_a_link(
    link_mode: u32,
    middle_mode: Option<u32>,
    real_mode: u32,
) -> (std::path::PathBuf, std::path::PathBuf, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;
    let cfg = tempfile::tempdir().unwrap();
    let dir = cfg.path().join("inkline");
    let real = cfg.path().join("dotfiles");
    for d in [&dir, &real] {
        std::fs::create_dir_all(d).unwrap();
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let file = real.join("init.el");
    std::fs::write(&file, "(setq inkline-indent 3)\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = dir.join("init.el");
    match middle_mode {
        Some(mode) => {
            let shared = cfg.path().join("shared");
            std::fs::create_dir_all(&shared).unwrap();
            std::os::unix::fs::symlink("../dotfiles/init.el", shared.join("init.el")).unwrap();
            std::os::unix::fs::symlink("../shared/init.el", &link).unwrap();
            std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        None => std::os::unix::fs::symlink(&file, &link).unwrap(),
    }
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(link_mode)).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(real_mode)).unwrap();
    let mut sh = Shell::start(Options {
        cols: 200,
        before_inkline: format!("XDG_CONFIG_HOME={}\n", cfg.path().display()),
        ..Options::default()
    });
    sh.send("inkline status; inkline eval inkline-indent\r");
    let s = sh.wait_for("the value", |s| has_row(s, "3") || has_row(s, "4"));
    (
        link,
        file,
        (0..s.size().0).map(|r| row_text(&s, r)).collect(),
    )
}

#[test]
fn init_el_through_a_link_is_read() {
    let (link, _, rows) = status_through_a_link(0o700, Some(0o700), 0o700);
    let loaded = format!("init.el: {} (loaded)", link.display());
    assert!(
        rows.contains(&loaded) && rows.contains(&"3".to_owned()),
        "{rows:#?}"
    );
}

#[test]
fn init_el_through_a_link_to_a_group_writable_directory_is_not_read() {
    let (link, file, rows) = status_through_a_link(0o700, None, 0o770);
    let skipped = format!(
        "init.el: {} (skipped: {}: its directory is writable by group or others)",
        link.display(),
        file.display()
    );
    assert!(
        rows.contains(&skipped) && rows.contains(&"4".to_owned()),
        "{rows:#?}"
    );
}

#[test]
fn init_el_through_a_link_in_a_group_writable_directory_is_not_read() {
    let (link, _, rows) = status_through_a_link(0o770, None, 0o700);
    let skipped = format!(
        "init.el: {} (skipped: its directory is writable by group or others)",
        link.display()
    );
    assert!(
        rows.contains(&skipped) && rows.contains(&"4".to_owned()),
        "{rows:#?}"
    );
}

#[test]
fn init_el_through_a_link_in_a_group_writable_directory_on_the_way_is_not_read() {
    let (link, _, rows) = status_through_a_link(0o700, Some(0o770), 0o700);
    let middle = link.parent().unwrap().join("../shared/init.el");
    let skipped = format!(
        "init.el: {} (skipped: {}: its directory is writable by group or others)",
        link.display(),
        middle.display()
    );
    assert!(
        rows.contains(&skipped) && rows.contains(&"4".to_owned()),
        "{rows:#?}"
    );
}

#[test]
fn getenv_reads_bash_variables() {
    let (out, _) = run(r#"FOO=bar; inkline eval '(list (getenv "HOME") (getenv "FOO"))'"#);
    let home = bash_command()
        .get_envs()
        .find(|(k, _)| *k == "HOME")
        .and_then(|(_, v)| v)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(out, format!("(\"{home}\" \"bar\")\n"));
}

#[test]
fn status_shows_an_init_el_error_without_the_path_again() {
    let home = tempfile::tempdir().unwrap();
    let mut sh = Shell::start(Options {
        cols: 200,
        home: Some(home.path().to_owned()),
        init_el: Some("(setq inkline-indent 2)\n(car 1 2)\n".into()),
        ..Options::default()
    });
    sh.send("inkline status\r");
    let status = format!(
        "init.el: {} (error: line 2: Too many arguments (in (car 1 2)))",
        home.path().join(".config/inkline/init.el").display()
    );
    sh.wait_for("the status", |s| has_row(s, &status));
}
