//! Readline's own completion on the lines after the first of a multi-line
//! command. Tab runs it with the menu off.

#[path = "support/common.rs"]
mod common;

use common::*;

/// A shell with `rc` and the menu off.
fn menu_off(rc: String) -> Options {
    Options {
        rc,
        init_el: Some("(setq inkline-show-menu nil)".to_owned()),
        ..Options::default()
    }
}

#[test]
fn command_names_complete_on_a_later_line() {
    let mut sh = Shell::start(menu_off(String::new()));
    sh.send("true");
    sh.send(CTRL_J);
    sh.send("histor\t");
    sh.wait_for("the completed name", |s| row_text(s, 1) == "history");
}

#[test]
fn programmable_completion_on_a_later_line() {
    let mut sh = Shell::start(menu_off("complete -W 'alpha beta' mycmd\n".into()));
    sh.send("true");
    sh.send(CTRL_J);
    sh.send("mycmd al\t");
    sh.wait_for("the completed word", |s| row_text(s, 1) == "mycmd alpha");
}
