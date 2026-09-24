//! inkline's default key layout, bound when inkline loads: each key is taken
//! only while it still has readline's default binding.

/// The default layout's groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Suggestions,
    MultiLine,
    Pairing,
}

impl Group {
    pub const ALL: [Group; 3] = [Group::Suggestions, Group::MultiLine, Group::Pairing];

    pub fn name(self) -> &'static str {
        match self {
            Group::Suggestions => "suggestions",
            Group::MultiLine => "multi-line",
            Group::Pairing => "pairing",
        }
    }

    pub fn named(name: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|g| g.name() == name)
    }
}

pub struct Entry {
    pub group: Group,
    pub key: &'static str,
    pub command: &'static str,
    /// The readline commands that count as the key's default; "" is unbound.
    pub defaults: &'static [&'static str],
    /// A binding to replace with another command instead: (was, becomes).
    pub instead: Option<(&'static str, &'static str)>,
}

const fn e(
    group: Group,
    key: &'static str,
    command: &'static str,
    defaults: &'static [&'static str],
) -> Entry {
    Entry {
        group,
        key,
        command,
        defaults,
        instead: None,
    }
}

use Group::{MultiLine, Pairing, Suggestions};

pub const LAYOUT: &[Entry] = &[
    e(
        Suggestions,
        "C-f",
        "accept-suggestion-char",
        &["forward-char"],
    ),
    e(
        Suggestions,
        "<right>",
        "accept-suggestion-char",
        &["forward-char", ""],
    ),
    e(
        Suggestions,
        "M-f",
        "accept-suggestion-word",
        &["forward-word"],
    ),
    e(Suggestions, "C-e", "accept-suggestion", &["end-of-line"]),
    e(
        Suggestions,
        "<end>",
        "accept-suggestion",
        &["end-of-line", ""],
    ),
    e(MultiLine, "RET", "accept-or-newline", &["accept-line"]),
    e(MultiLine, "C-j", "insert-newline", &["accept-line"]),
    e(MultiLine, "M-RET", "accept-as-is", &["", "vi-editing-mode"]),
    Entry {
        instead: Some(("history-search-backward", "previous-line-or-search")),
        ..e(
            MultiLine,
            "<up>",
            "previous-line-or-history",
            &["previous-history", ""],
        )
    },
    e(
        MultiLine,
        "C-p",
        "previous-line-or-history",
        &["previous-history"],
    ),
    Entry {
        instead: Some(("history-search-forward", "next-line-or-search")),
        ..e(
            MultiLine,
            "<down>",
            "next-line-or-history",
            &["next-history", ""],
        )
    },
    e(MultiLine, "C-n", "next-line-or-history", &["next-history"]),
    e(MultiLine, "C-a", "line-start", &["beginning-of-line"]),
    e(
        MultiLine,
        "<home>",
        "line-start",
        &["beginning-of-line", ""],
    ),
    e(MultiLine, "C-k", "kill-to-line-end", &["kill-line"]),
    e(
        MultiLine,
        "C-u",
        "kill-to-line-start",
        &["unix-line-discard"],
    ),
    e(MultiLine, "M-#", "comment-lines", &["insert-comment"]),
    e(Pairing, "(", "insert-pair", &["self-insert"]),
    e(Pairing, "[", "insert-pair", &["self-insert"]),
    e(Pairing, "{", "insert-pair", &["self-insert"]),
    e(Pairing, "\"", "insert-pair", &["self-insert"]),
    e(Pairing, "'", "insert-pair", &["self-insert"]),
    e(Pairing, "`", "insert-pair", &["self-insert"]),
    e(Pairing, ")", "insert-close", &["self-insert"]),
    e(Pairing, "]", "insert-close", &["self-insert"]),
    e(Pairing, "}", "insert-close", &["self-insert"]),
    e(Pairing, "DEL", "delete-pair", &["backward-delete-char"]),
];

#[cfg(not(test))]
mod bash {
    use super::*;
    use crate::ffi;
    use crate::lisp::keydesc;
    use crate::lisp::keys::{self, LeftAlone};

    /// Readline set-up before binding: sets the variables the layout needs,
    /// then runs readline's own start-up if bash has not run it yet, which
    /// reads `inputrc`. When inkline is first to set readline up, `inputrc`
    /// can still change the variables. When a `bind` earlier in `.bashrc` set
    /// readline up first, `inputrc` was already read and the variables are
    /// set anyway: the layout's `DEL` and `C-u` need them.
    pub fn prepare_readline() {
        ffi::set_readline_variable("bind-tty-special-chars", "off");
        if ffi::readline_version() < 0x0801 {
            ffi::set_readline_variable("enable-bracketed-paste", "on");
        }
        ffi::initialize_readline_once();
    }

    /// Binds each layout sequence that still has one of its defaults.
    pub fn bind_defaults() {
        for entry in LAYOUT {
            for seq in keydesc::parse(entry.key).expect("layout keys parse") {
                let current = ffi::lookup(&seq);
                let name = match current.as_ref().map(|f| (&f.binding, f.prefix)) {
                    Some((ffi::Binding::Unbound, false)) => Some(String::new()),
                    Some((ffi::Binding::Command(f), false)) => ffi::command_name(*f),
                    _ => None,
                };
                let command = match (&name, entry.instead) {
                    (Some(n), Some((was, becomes))) if n == was => Some(becomes),
                    (Some(n), _) if entry.defaults.contains(&n.as_str()) => Some(entry.command),
                    _ => None,
                };
                match command.and_then(|c| ffi::named_command(c).map(|f| (c, f))) {
                    Some((c, f)) => {
                        let _ = keys::bind_seq(&seq, entry.key, c, f, Some(entry.group));
                    }
                    None => keys::note_left_alone(LeftAlone {
                        key: entry.key.to_owned(),
                        seq,
                        wanted: entry.command,
                    }),
                }
            }
        }
    }
}

#[cfg(not(test))]
pub use bash::{bind_defaults, prepare_readline};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_are_found_by_name() {
        for g in Group::ALL {
            assert_eq!(Group::named(g.name()), Some(g));
        }
        assert_eq!(Group::MultiLine.name(), "multi-line");
        assert_eq!(Group::named("multiline"), None);
    }

    #[test]
    fn every_key_parses_and_every_group_is_used() {
        for e in LAYOUT {
            assert!(super::super::keydesc::parse(e.key).is_ok(), "{}", e.key);
        }
        for g in Group::ALL {
            assert!(LAYOUT.iter().any(|e| e.group == g));
        }
    }
}
