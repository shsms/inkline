//! What the bash under test does with each command in
//! `tests/data/syntax-cases.txt` still matches what was recorded, so a bash
//! version that parses differently shows up here rather than as a wrong
//! answer from inkline.

#[path = "support/common.rs"]
mod common;

use std::io::Write;
use std::process::Stdio;

use common::*;

struct Case {
    text: String,
    /// Recorded answers: bash 5.2, bash 5.2 with extglob, bash 5.0.
    bash: [String; 3],
}

fn cases() -> Vec<Case> {
    include_str!("data/syntax-cases.txt")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            let (columns, text) = line.split_once('\t').unwrap();
            let c: Vec<&str> = columns.split(' ').collect();
            Case {
                text: unescape(text),
                bash: [c[0].to_owned(), c[1].to_owned(), c[2].to_owned()],
            }
        })
        .collect()
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// What `bash -n` says about `text`: U when bash runs out of input, W for a
/// syntax error, F otherwise.
fn bash_says(text: &str, extglob: bool) -> &'static str {
    let mut cmd = bash_command();
    cmd.env("LC_ALL", "C");
    if extglob {
        cmd.args(["-O", "extglob"]);
    }
    let mut child = cmd
        .arg("-n")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{text}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("here-document at line")
        || err.contains("unexpected end of file")
        || err.contains("unexpected EOF")
    {
        "U"
    } else if err.contains("syntax error") {
        "W"
    } else {
        "F"
    }
}

/// A trailing backslash continues the line: interactive bash asks for more,
/// but `bash -n` reads to the end of its input and accepts it.
fn ends_in_continuation(text: &str) -> bool {
    (text.len() - text.trim_end_matches('\\').len()) % 2 == 1
}

#[test]
fn recorded_answers_hold() {
    let version = bash_version();
    // Versions without a recorded column are held to bash 5.2's.
    let columns: &[(usize, bool)] = if version == (5, 0) {
        &[(2, false)]
    } else {
        &[(0, false), (1, true)]
    };
    let cases = cases();
    let differ: Vec<String> = std::thread::scope(|scope| {
        let workers: Vec<_> = cases
            .chunks(cases.len().div_ceil(8))
            .map(|chunk| {
                scope.spawn(move || {
                    let mut differ = Vec::new();
                    for case in chunk.iter().filter(|c| !ends_in_continuation(&c.text)) {
                        for &(column, extglob) in columns {
                            let want = case.bash[column].as_str();
                            let got = bash_says(&case.text, extglob);
                            if want != "-" && got != want {
                                differ.push(format!(
                                    "{:?} (extglob {extglob}): recorded {want}, bash says {got}",
                                    case.text
                                ));
                            }
                        }
                    }
                    differ
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    });
    assert!(
        differ.is_empty(),
        "bash {}.{} differs on {} commands:\n{}",
        version.0,
        version.1,
        differ.len(),
        differ.join("\n")
    );
}
