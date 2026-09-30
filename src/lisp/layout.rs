//! inkline's default key layout, bound when inkline loads: each key is taken
//! only while it still has readline's default binding.

/// The default layout's groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Suggestions,
    MultiLine,
    Pairing,
    Menu,
}

impl Group {
    pub const ALL: [Group; 4] = [
        Group::Suggestions,
        Group::MultiLine,
        Group::Pairing,
        Group::Menu,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Group::Suggestions => "suggestions",
            Group::MultiLine => "multi-line",
            Group::Pairing => "pairing",
            Group::Menu => "menu",
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
    }
}

use Group::{Menu, MultiLine, Pairing, Suggestions};

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
    e(Menu, "C-n", "menu-next", &["next-history"]),
    e(Menu, "C-p", "menu-previous", &["previous-history"]),
    e(
        Menu,
        "<down>",
        "menu-next",
        &[
            "next-history",
            "",
            "history-search-forward",
            "history-substring-search-forward",
        ],
    ),
    e(
        Menu,
        "<up>",
        "menu-previous",
        &[
            "previous-history",
            "",
            "history-search-backward",
            "history-substring-search-backward",
        ],
    ),
    e(Menu, "TAB", "menu-take", &["complete"]),
    e(Menu, "<backtab>", "menu-take-previous", &[""]),
    e(Menu, "C-g", "menu-hide", &["abort"]),
];

/// `menu_key_fallback` for a key that had the readline command named `had`
/// (None when it had nothing).
fn named_fallback(down: bool, had: Option<&str>) -> Option<&'static str> {
    match (down, had) {
        (true, None | Some("next-history" | "history-substring-search-forward")) => {
            Some("next-line-or-substring-search")
        }
        (false, None | Some("previous-history" | "history-substring-search-backward")) => {
            Some("previous-line-or-substring-search")
        }
        (true, Some("history-search-forward")) => Some("next-line-or-search"),
        (false, Some("history-search-backward")) => Some("previous-line-or-search"),
        _ => None,
    }
}

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
                let command = name
                    .as_ref()
                    .filter(|n| entry.defaults.contains(&n.as_str()))
                    .map(|_| entry.command);
                match command.and_then(|c| ffi::named_command(c).map(|f| (c, f))) {
                    Some((c, f)) => {
                        let _ = keys::bind_seq(&seq, entry.key, c, f, None, Some(entry.group));
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

    /// The Up or Down command that `menu-next` (`down`) or `menu-previous`
    /// runs with no menu, from what its key had before inkline bound it
    /// (`saved`). None when what the key had runs as it is, as for macro
    /// text or a command readline has no name for.
    pub fn menu_key_fallback(down: bool, saved: &ffi::Binding) -> Option<&'static str> {
        let had = match saved {
            ffi::Binding::Unbound => None,
            ffi::Binding::Command(f) => Some(ffi::command_name(*f)?),
            ffi::Binding::Macro(_) => return None,
        };
        named_fallback(down, had.as_deref())
    }
}

#[cfg(not(test))]
pub use bash::{bind_defaults, menu_key_fallback, prepare_readline};

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

    #[test]
    fn a_menu_keys_fallback_follows_what_the_key_had() {
        use super::named_fallback as f;
        assert_eq!(f(false, None), Some("previous-line-or-substring-search"));
        assert_eq!(
            f(false, Some("previous-history")),
            Some("previous-line-or-substring-search")
        );
        assert_eq!(
            f(false, Some("history-substring-search-backward")),
            Some("previous-line-or-substring-search")
        );
        assert_eq!(
            f(false, Some("history-search-backward")),
            Some("previous-line-or-search")
        );
        assert_eq!(f(true, None), Some("next-line-or-substring-search"));
        assert_eq!(
            f(true, Some("next-history")),
            Some("next-line-or-substring-search")
        );
        assert_eq!(
            f(true, Some("history-search-forward")),
            Some("next-line-or-search")
        );
        assert_eq!(f(true, Some("backward-char")), None);
        assert_eq!(
            f(false, Some("non-incremental-reverse-search-history")),
            None
        );
    }
}
