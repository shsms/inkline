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
    sh.send("\x15ls | ");
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
    sh.send("\x15cat s");
    sh.wait_for("the directory", |s| has_row(s, "c  src/"));
    sh.send("\t");
    sh.wait_for("the directory taken and its files", |s| {
        cursor_row(s) == "$ cat src/main.rs"
            && s.cursor_position().1 == 10
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
    sh.send("\x15echo still-$((1 + 1))\r");
    sh.wait_for("the next command", |s| has_row(s, "still-2"));
    assert!(sh.keys_one_by_one());
}
