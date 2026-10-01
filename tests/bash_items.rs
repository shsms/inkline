//! bash's own completion in the menu under the line.

#[path = "support/common.rs"]
mod common;

use std::path::Path;

use common::*;

const LEFT: &str = "\x1b[D";

/// The rules the tests complete with.
const RULES: &str = r#"
complete -W 'switch show stash' gg
_fl() {
    local cur=${COMP_WORDS[COMP_CWORD]}
    if [[ $cur == -* ]]; then
        COMPREPLY=($(compgen -W '--verbose --version' -- "$cur"))
    else
        COMPREPLY=($(compgen -W 'build test' -- "$cur"))
    fi
}
complete -F _fl fl
complete -o nospace -W 'key=' kv
zzfunc_one() { :; }
zzfunc_two() { :; }
ZZVAR_ONE=1
_group_to() {
    local p g
    read -r _ _ _ p g _ < /proc/$BASHPID/stat
    echo "$g $p $$" > "$1"
}
"#;

/// A directory holding `alpha.txt`, `my file.txt` and `src/main.rs`.
fn files() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.txt"), "").unwrap();
    std::fs::write(dir.path().join("my file.txt"), "").unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
    dir
}

/// A shell with bash completion on, starting in `dir`, with `RULES` and
/// `rc` in its rc file and `init` as its `init.el` (none when empty).
fn shell_in(dir: &Path, rc: &str, init: &str) -> Shell {
    Shell::start(Options {
        bash_completion: true,
        cwd: Some(dir.to_owned()),
        rc: format!("{RULES}{rc}"),
        init_el: (!init.is_empty()).then(|| init.to_owned()),
        ..Options::default()
    })
}

/// Types `keys` and waits for a row that reads `row`.
fn typed_then(sh: &mut Shell, keys: &str, row: &str) -> vt100::Screen {
    sh.send(keys);
    sh.wait_for(row, |s| has_row(s, row))
}

fn bash_rows(s: &vt100::Screen) -> Vec<String> {
    (0..s.size().0)
        .map(|r| row_text(s, r))
        .filter(|t| t.starts_with("c  "))
        .collect()
}

#[test]
fn a_word_list_rule_gives_c_rows() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "");
    let s = typed_then(&mut sh, "gg s", "c  show");
    assert_eq!(
        bash_rows(&s),
        ["c  show", "c  stash", "c  switch"],
        "{}",
        dump(&s)
    );
}

#[test]
fn a_function_rule_gives_its_words_and_flags_after_a_dash() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "");
    let s = typed_then(&mut sh, "fl ", "c  build");
    assert!(has_row(&s, "c  test"), "{}", dump(&s));
    let s = typed_then(&mut sh, "-", "c  --verbose");
    assert!(!has_row(&s, "c  build"), "{}", dump(&s));
}

#[test]
fn command_names_show_from_one_character() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "");
    let s = typed_then(&mut sh, "zzf", "c  zzfunc_one");
    assert!(has_row(&s, "c  zzfunc_two"), "{}", dump(&s));
    sh.send("\x18\x7fls | ");
    let s = sh.settle();
    assert!(bash_rows(&s).is_empty(), "{}", dump(&s));
}

#[test]
fn files_and_directories_are_taken_as_tab_would() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "");
    typed_then(&mut sh, "cat my", "c  my\\ file.txt");
    sh.send("\t");
    // The space after the name starts a new word, which gets its own items
    // and the grey text of the first.
    sh.wait_for("the file taken", |s| {
        cursor_row(s) == "$ cat my\\ file.txt alpha.txt"
            && s.cursor_position().1 == 19
            && has_row(s, "c  alpha.txt")
    });
    sh.send("\x18\x7fcat s");
    sh.wait_for("the directory", |s| has_row(s, "c  src/"));
    sh.send("\t");
    sh.wait_for("the directory taken and its files", |s| {
        cursor_row(s) == "$ cat src/main.rs"
            && s.cursor_position().1 == 10
            && has_row(s, "c  src/main.rs")
    });
}

/// Tab with one row on a directory writes it with its `/`, and the menu then
/// lists what is inside.
#[test]
fn tab_with_one_row_on_a_directory_lists_what_is_inside() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "");
    let s = typed_then(&mut sh, "cd s", "c  src/");
    assert_eq!(bash_rows(&s), ["c  src/"], "{}", dump(&s));
    sh.send("\t");
    sh.wait_for("the directory written and its files", |s| {
        cursor_row(s).starts_with("$ cd src/")
            && s.cursor_position() == (0, 9)
            && has_row(s, "c  src/main.rs")
    });
}

/// Pairing closed the quote after the cursor; readline's own closing quote
/// takes its place.
#[test]
fn a_quoted_file_name_closes_its_quote() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "");
    typed_then(&mut sh, "cat \"my", "c  my file.txt\"");
    sh.send("\t");
    // The space after the quote starts a new word, with the grey text of
    // its first item.
    sh.wait_for("the quoted name", |s| {
        cursor_row(s) == "$ cat \"my file.txt\" alpha.txt" && s.cursor_position().1 == 20
    });
}

#[test]
fn a_nospace_rule_adds_no_space() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "");
    typed_then(&mut sh, "kv k", "c  key=");
    sh.send("\t");
    // `=` breaks words, so the next word starts right after it and gets
    // `key=` again as its grey text.
    sh.wait_for("no space after the word", |s| {
        cursor_row(s) == "$ kv key=key=" && s.cursor_position().1 == 9
    });
}

#[test]
fn variables_complete_after_a_dollar() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "");
    typed_then(&mut sh, "echo $ZZV", "c  $ZZVAR_ONE");
}

#[test]
fn a_later_line_completes_its_own_word() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "");
    sh.send("true");
    sh.send(CTRL_J);
    typed_then(&mut sh, "gg sw", "c  switch");
}

#[test]
fn the_middle_of_the_line_completes_up_to_the_cursor() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "");
    sh.send("gg  rest");
    sh.send(&LEFT.repeat(5));
    typed_then(&mut sh, "sw", "c  switch");
    sh.send("\t");
    // As Tab, nothing is added after the word when the cursor is not at the
    // end of the line.
    sh.wait_for("the word taken", |s| {
        cursor_row(s) == "$ gg switch rest" && s.cursor_position().1 == 11
    });
}

#[test]
fn turned_off_there_are_no_c_rows() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "(setq inkline-bash-completion nil)");
    sh.send("gg sw");
    let s = sh.settle();
    assert!(bash_rows(&s).is_empty(), "{}", dump(&s));
    assert_eq!(cursor_row(&s), "$ gg sw");
}

#[test]
fn left_out_of_the_menu_bash_still_gives_the_grey_text() {
    let dir = files();
    let mut sh = shell_in(
        dir.path(),
        "",
        "(setq inkline-menu-sources '(history lisp mode))",
    );
    sh.send("gg sw");
    let s = sh.wait_for("the grey text", |s| cursor_row(s) == "$ gg switch");
    assert!(bash_rows(&s).is_empty(), "{}", dump(&s));
}

#[test]
fn a_late_answer_shows_without_a_key() {
    let dir = files();
    let mut sh = shell_in(
        dir.path(),
        "_late() { sleep 0.2; COMPREPLY=(later); }\ncomplete -F _late late\n",
        "",
    );
    typed_then(&mut sh, "late ", "c  later");
}

/// A pipeline in a rule that a signal ends leaves the terminal as readline
/// set it: keys still come one at a time.
#[test]
fn a_rule_s_killed_pipeline_leaves_the_terminal_alone() {
    let dir = files();
    let mut sh = shell_in(
        dir.path(),
        "_p() { yes | head -1 >/dev/null; COMPREPLY=(pout); }\ncomplete -F _p p\n",
        "",
    );
    let s = typed_then(&mut sh, "p ", "c  pout");
    assert!(sh.keys_one_by_one(), "{}", dump(&s));
    sh.send("\x01X");
    sh.wait_for("C-a at once", |s| {
        cursor_row(s) == "$ Xp" && s.cursor_position().1 == 3
    });
}

#[test]
fn asking_again_at_a_pause_shows_the_new_answer() {
    let dir = files();
    let mut sh = shell_in(
        dir.path(),
        "_e() { case $2 in b*) COMPREPLY=(banana);; *) COMPREPLY=(apple);; esac; }\n\
         complete -F _e e\n",
        "",
    );
    typed_then(&mut sh, "e ", "c  apple");
    typed_then(&mut sh, "b", "c  banana");
}

/// An answer that came in time while the key read was not for plain
/// editing is taken at the copy's limit, not counted as timed out.
#[test]
fn an_answer_that_came_during_a_search_is_not_a_time_out() {
    let dir = files();
    let mut sh = shell_in(
        dir.path(),
        "_late() { sleep 0.2; COMPREPLY=(later); }\ncomplete -F _late late\n",
        "(setq inkline-bash-completion-timeout 1000)",
    );
    sh.send("late ");
    sh.send("\x12");
    sh.wait_for("the search", |s| find(s, "reverse-i-search").is_some());
    std::thread::sleep(std::time::Duration::from_millis(1300));
    sh.send("\x07\x18\x7finkline status\r");
    let s = sh.wait_for("the status", |s| find(s, "bash completion:").is_some());
    assert!(has_row(&s, "bash completion: on"), "{}", dump(&s));
}

/// A copy killed at its limit takes the word's `c` rows away at once: the
/// word gets no items from bash now. The rows come from a first answer cut
/// at 1000 matches, which serves the longer word while a copy is asked
/// about it.
#[test]
fn a_time_out_takes_the_rows_away_without_a_key() {
    let dir = files();
    let mut sh = shell_in(
        dir.path(),
        "_s() { [[ $2 == f0* ]] && sleep 5; COMPREPLY=(f{0000..1499}); }\n\
         complete -F _s s\n",
        "(setq inkline-bash-completion-timeout 300)",
    );
    typed_then(&mut sh, "s f", "c  f0000");
    typed_then(&mut sh, "0", "c  f0000");
    let s = sh.wait_for("no c rows", |s| bash_rows(s).is_empty());
    assert_eq!(cursor_row(&s), "$ s f0", "{}", dump(&s));
}

/// An error that makes bash leave a rule for its top level ends the copy
/// alone.
#[test]
fn a_rule_s_error_ends_only_the_copy() {
    let dir = files();
    let mut sh = shell_in(
        dir.path(),
        "_u() { set -u; echo \"$zz_unset\"; COMPREPLY=(never); }\ncomplete -F _u u\n",
        "",
    );
    sh.send("u ");
    let s = sh.settle();
    assert_eq!(cursor_row(&s), "$ u", "{}", dump(&s));
    assert!(
        (1..s.size().0).all(|r| row_text(&s, r).is_empty()),
        "{}",
        dump(&s)
    );
    sh.send("\x18\x7fecho still-$((1 + 1))\r");
    sh.wait_for("the next command", |s| has_row(s, "still-2"));
    assert!(sh.keys_one_by_one());
    sh.send("inkline status\r");
    sh.wait_for("the status", |s| {
        has_row(s, "bash completion: on (1 failed)")
    });
}

/// Waits up to 5 s for `done`.
fn eventually(what: &str, done: impl Fn() -> bool) {
    assert!(poll(|| done().then_some(())).is_some(), "never: {what}");
}

/// The process group, the parent and the shell a rule wrote to `path` with
/// `_group_to` once it started: the group is the copy of the shell's own.
fn written(path: &Path) -> [String; 3] {
    eventually("the rule started", || {
        std::fs::read_to_string(path).is_ok_and(|s| s.ends_with('\n'))
    });
    let text = std::fs::read_to_string(path).unwrap();
    let fields: Vec<_> = text.split_whitespace().map(str::to_owned).collect();
    fields.try_into().unwrap()
}

/// The process group a rule wrote to `path` (see `written`).
fn written_group(path: &Path) -> String {
    let [group, ..] = written(path);
    group
}

/// Whether a process of group `group` still runs. One that has ended but
/// that its parent has not waited for yet does not count.
fn group_alive(group: &str) -> bool {
    std::fs::read_dir("/proc").unwrap().flatten().any(|entry| {
        std::fs::read_to_string(entry.path().join("stat")).is_ok_and(|stat| {
            // After the name in parentheses: the state, the parent, the group.
            let Some((_, rest)) = stat.rsplit_once(')') else {
                return false;
            };
            let mut fields = rest.split_whitespace();
            let state = fields.next();
            fields.nth(1) == Some(group) && state != Some("Z")
        })
    })
}

#[test]
fn a_slow_rule_never_holds_up_typing_and_is_killed_at_its_limit() {
    let dir = files();
    let pids = tempfile::tempdir().unwrap();
    let pid = pids.path().join("slow");
    let mut sh = shell_in(
        dir.path(),
        &format!(
            "_slow() {{ _group_to '{}'; sleep 7.25; COMPREPLY=(late); }}\n\
             complete -F _slow slow\n",
            pid.display()
        ),
        "(setq inkline-bash-completion-timeout 300)",
    );
    let began = std::time::Instant::now();
    sh.send("slow ");
    sh.send("abc");
    sh.wait_for("the keys", |s| cursor_row(s) == "$ slow abc");
    assert!(
        began.elapsed() < std::time::Duration::from_secs(2),
        "typing waited for the rule"
    );
    let group = written_group(&pid);
    eventually("the rule killed", || !group_alive(&group));
    sh.send("\x18\x7finkline status\r");
    sh.wait_for("the status", |s| {
        has_row(s, "bash completion: on (1 timed out)")
    });
}

#[test]
fn copies_end_with_the_line() {
    let dir = files();
    let pids = tempfile::tempdir().unwrap();
    let (first, second) = (pids.path().join("slow1"), pids.path().join("slow2"));
    let mut sh = shell_in(
        dir.path(),
        &format!(
            "_slow1() {{ _group_to '{}'; sleep 7.5; }}\ncomplete -F _slow1 slowa\n\
             _slow2() {{ _group_to '{}'; sleep 7.75; }}\ncomplete -F _slow2 slowb\n",
            first.display(),
            second.display()
        ),
        // Longer than the test waits, so only what the test checks ends it.
        "(setq inkline-bash-completion-timeout 60000)",
    );
    sh.send("slowa ");
    let group = written_group(&first);
    assert!(group_alive(&group), "the first rule is not running");
    sh.send(ENTER);
    eventually("the first rule killed", || !group_alive(&group));
    sh.send("slowb ");
    let group = written_group(&second);
    assert!(group_alive(&group), "the second rule is not running");
    sh.send("\x03");
    eventually("the second rule killed", || !group_alive(&group));
}

/// A copy of the shell is not the shell's child, so the shell's end does
/// not end it: its watcher kills it and what it started.
#[test]
fn a_copy_ends_with_the_shell() {
    let dir = files();
    let pids = tempfile::tempdir().unwrap();
    let pid = pids.path().join("slow");
    let mut sh = shell_in(
        dir.path(),
        &format!(
            "_slow() {{ _group_to '{}'; sleep 7.5; }}\ncomplete -F _slow slow\n",
            pid.display()
        ),
        // Longer than the test waits, so only what the test checks ends it.
        "(setq inkline-bash-completion-timeout 60000)",
    );
    sh.send("slow ");
    let group = written_group(&pid);
    assert!(group_alive(&group), "the rule is not running");
    sh.signal(libc::SIGKILL);
    eventually("the rule killed", || !group_alive(&group));
}

/// A copy of the shell does not run long past its limit while the shell
/// cannot kill it: its watcher kills it a second after.
#[test]
fn a_copy_ends_at_its_limit_while_the_shell_is_stopped() {
    let dir = files();
    let pids = tempfile::tempdir().unwrap();
    let pid = pids.path().join("slow");
    let mut sh = shell_in(
        dir.path(),
        &format!(
            "_slow() {{ _group_to '{}'; sleep 7.5; }}\ncomplete -F _slow slow\n",
            pid.display()
        ),
        "(setq inkline-bash-completion-timeout 300)",
    );
    sh.send("slow ");
    let group = written_group(&pid);
    sh.signal(libc::SIGSTOP);
    eventually("the rule killed", || !group_alive(&group));
    sh.signal(libc::SIGCONT);
}

#[test]
fn the_status_says_whether_bash_completion_is_on() {
    let dir = files();
    let mut sh = shell_in(dir.path(), "", "");
    sh.send("inkline status\r");
    sh.wait_for("on", |s| has_row(s, "bash completion: on"));
    let mut sh = shell_in(dir.path(), "", "(setq inkline-bash-completion nil)");
    sh.send("inkline status\r");
    sh.wait_for("off", |s| has_row(s, "bash completion: off"));
}

#[test]
fn a_rude_rule_leaves_the_screen_and_the_shell_alone() {
    let dir = files();
    let mut sh = shell_in(
        dir.path(),
        "_rude() {\n\
             echo RUDE-OUT; echo RUDE-ERR >&2; echo RUDE-TTY > /dev/tty\n\
             read -r x; cd /; COMPREPLY=(polite)\n\
         }\n\
         complete -F _rude rude\n",
        "",
    );
    let s = typed_then(&mut sh, "rude ", "c  polite");
    assert!(find(&s, "RUDE").is_none(), "{}", dump(&s));
    sh.send("\x18\x7fpwd\r");
    let here = dir.path().canonicalize().unwrap();
    let here = here.to_str().unwrap().to_owned();
    sh.wait_for("the shell's directory", |s| has_row(s, &here));
}

#[test]
fn a_rule_that_exits_runs_no_trap_and_writes_no_history() {
    let dir = files();
    let home = dir.path();
    let mut sh = shell_in(
        home,
        &format!(
            "HISTFILE={home}/hist\n\
             trap ': > {home}/trapped' EXIT\n\
             _quit() {{ _group_to {home}/ran; exit 0; }}\n\
             complete -F _quit quit\n",
            home = home.display()
        ),
        "",
    );
    // bash writes the history file at exit only once a line joined the
    // history.
    sh.send("true\r");
    sh.send("quit ");
    // Once the copy has ended, its pipe is closed, and the next key's draw
    // reads that before it replaces the copy: the failure is counted.
    let group = written_group(&home.join("ran"));
    eventually("the copy ended", || !group_alive(&group));
    sh.send("\x18\x7finkline status\r");
    sh.wait_for("the failure", |s| {
        has_row(s, "bash completion: on (1 failed)")
    });
    // The failure is counted only once the copy's pipe closes, which is
    // after the copy's exit path ran any EXIT trap and wrote any history.
    assert!(!home.join("trapped").exists(), "the copy ran the EXIT trap");
    assert!(
        !home.join("hist").exists(),
        "the copy wrote the history file"
    );
}

/// The first answer is cut at 1000 names; a longer word asks again once
/// typing pauses.
#[test]
fn more_than_a_thousand_files() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..1500 {
        std::fs::write(dir.path().join(format!("f{i:04}")), "").unwrap();
    }
    let mut sh = shell_in(dir.path(), "", "");
    typed_then(&mut sh, "cat f", "c  f0000");
    typed_then(&mut sh, "14", "c  f1400");
}

#[test]
fn a_file_name_that_is_not_utf8_is_left_out() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(std::ffi::OsStr::from_bytes(b"bad\xff")), "").unwrap();
    std::fs::write(dir.path().join("bad-ok"), "").unwrap();
    let mut sh = shell_in(dir.path(), "", "");
    let s = typed_then(&mut sh, "cat bad", "c  bad-ok");
    assert_eq!(bash_rows(&s), ["c  bad-ok"], "{}", dump(&s));
}

/// A copy of the shell that ends is not one of the shell's jobs: under
/// `set -b` nothing is drawn when it is killed at its limit, and the line
/// keeps its grey text and its menu after.
#[test]
fn a_killed_copy_is_not_a_job_that_ended() {
    let dir = files();
    let pids = tempfile::tempdir().unwrap();
    let pid = pids.path().join("slow");
    let mut sh = Shell::start(Options {
        bash_completion: true,
        cwd: Some(dir.path().to_owned()),
        history: vec!["slow abcdef"],
        rc: format!(
            "{RULES}set -b\n\
             _slow() {{ _group_to '{}'; sleep 5; COMPREPLY=(late); }}\n\
             complete -F _slow slow\n",
            pid.display()
        ),
        init_el: Some("(setq inkline-bash-completion-timeout 1000)".to_owned()),
        ..Options::default()
    });
    sh.send("slow ");
    let [group, parent, shell] = written(&pid);
    assert_ne!(parent, shell, "the copy is the shell's child");
    sh.settle();
    sh.take_output();
    eventually("the rule killed", || !group_alive(&group));
    std::thread::sleep(std::time::Duration::from_millis(300));
    let out = sh.take_output();
    assert!(
        out.is_empty(),
        "drawn after the kill: {:?}",
        String::from_utf8_lossy(&out)
    );
    sh.send("abc");
    let s = sh.wait_for("the grey text and the menu", |s| {
        cursor_row(s) == "$ slow abcdef" && has_row(s, "h  slow abcdef")
    });
    let (row, col) = s.cursor_position();
    let grey = s.cell(row, col).map(vt100::Cell::fgcolor);
    assert_eq!(grey, Some(vt100::Color::Idx(8)), "{}", dump(&s));
}

/// A process the shell forks, such as the child of `$(…)` that a `bind -x`
/// key runs, leaves the shell's copy running when it exits.
#[test]
fn a_child_of_the_shell_leaves_its_copy_alone() {
    let dir = files();
    let pids = tempfile::tempdir().unwrap();
    let pid = pids.path().join("slow");
    let mut sh = shell_in(
        dir.path(),
        &format!(
            "_slow() {{ _group_to '{}'; sleep 1.5; COMPREPLY=(late); }}\n\
             complete -F _slow slow\n\
             bind -x '\"\\C-t\": zz=$(true)'\n",
            pid.display()
        ),
        "(setq inkline-bash-completion-timeout 60000)",
    );
    sh.send("slow ");
    let group = written_group(&pid);
    assert!(group_alive(&group), "the rule is not running");
    sh.send("\x14");
    sh.wait_for("the answer", |s| has_row(s, "c  late"));
    sh.send("\x07\x18\x7finkline status\r");
    let s = sh.wait_for("the status", |s| find(s, "bash completion:").is_some());
    assert!(has_row(&s, "bash completion: on"), "{}", dump(&s));
}

/// bash 5.3's `compopt -o fullquote` quotes a match that is not a file name,
/// as Tab does. Earlier bash has no such option, and the match stays as it
/// is.
#[test]
fn a_rule_s_full_quoting_is_kept() {
    let dir = files();
    let mut sh = shell_in(
        dir.path(),
        "_fq() { compopt -o fullquote 2>/dev/null; COMPREPLY=('foo bar'); }\n\
         complete -F _fq fq\n",
        "",
    );
    let row = if bash_version() >= (5, 3) {
        "c  foo\\ bar"
    } else {
        "c  foo bar"
    };
    typed_then(&mut sh, "fq f", row);
}

/// A macro types many words at once: a copy of the shell is asked once the
/// macro ends, not for each word.
#[test]
fn a_macro_asks_once_it_ends() {
    let dir = files();
    let log = tempfile::tempdir().unwrap();
    let asked = log.path().join("asked");
    let mut sh = shell_in(
        dir.path(),
        &format!(
            "_mm() {{ echo \"$COMP_LINE\" >> '{}'; COMPREPLY=(done); }}\n\
             complete -F _mm mm\n\
             bind '\"\\C-xg\": \"mm a b c d e f g h i j \"'\n",
            asked.display()
        ),
        "",
    );
    sh.send("\x18g");
    sh.wait_for("the answer", |s| has_row(s, "c  done"));
    let lines = std::fs::read_to_string(&asked).unwrap();
    assert_eq!(lines.lines().count(), 1, "asked for:\n{lines}");
}

/// While a macro types, the draw does not wait for a copy of the shell
/// working on the word.
#[test]
fn a_macro_does_not_wait_for_a_copy() {
    let dir = files();
    let pids = tempfile::tempdir().unwrap();
    let pid = pids.path().join("slow");
    let many = "a".repeat(150);
    let mut sh = shell_in(
        dir.path(),
        &format!(
            "_slow() {{ _group_to '{}'; sleep 7.5; }}\n\
             complete -F _slow slow\n\
             bind '\"\\C-xa\": \"{many}\"'\n",
            pid.display()
        ),
        "(setq inkline-bash-completion-timeout 60000)",
    );
    sh.send("slow ");
    let group = written_group(&pid);
    assert!(group_alive(&group), "the rule is not running");
    let began = std::time::Instant::now();
    sh.send("\x18a");
    let line = format!("$ slow {many}");
    sh.wait_for("the macro's text", |s| {
        (0..s.size().0)
            .map(|r| row_text(s, r))
            .collect::<String>()
            .contains(&line)
    });
    assert!(
        began.elapsed() < std::time::Duration::from_secs(1),
        "the draw waited for the copy: {:?}",
        began.elapsed()
    );
}

/// The last word a macro types is asked about as soon as the macro ends,
/// without waiting for the pause in typing.
#[test]
fn a_macro_s_last_word_is_asked_about_at_once() {
    // `PAUSE_MS` in src/hooks.rs.
    const PAUSE: std::time::Duration = std::time::Duration::from_millis(150);
    let dir = files();
    let mut sh = shell_in(
        dir.path(),
        "_mm() { COMPREPLY=(done); }\n\
         complete -F _mm mm\n\
         bind '\"\\C-xg\": \"mm a b \"'\n",
        "",
    );
    sh.send("\x18g");
    sh.wait_for("the macro's text", |s| {
        cursor_row(s).starts_with("$ mm a b")
    });
    let began = std::time::Instant::now();
    sh.wait_for("the answer", |s| has_row(s, "c  done"));
    let took = began.elapsed();
    assert!(
        took < PAUSE * 2 / 3,
        "the answer took {took:?} after the macro's text: the last word \
         waited for the pause"
    );
}
