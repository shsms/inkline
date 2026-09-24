//! Key bindings made from Lisp and by the default layout, with what each
//! sequence had before inkline first bound it.

use std::cell::RefCell;

use tulisp::{Error, TulispContext, TulispObject};

use super::keydesc;
use super::layout::Group;
use crate::ffi;

/// A sequence inkline bound.
struct Bound {
    seq: Vec<u8>,
    /// The key description it was bound through.
    key: String,
    command: String,
    function: ffi::CommandFn,
    /// The layout group, for keys the default layout bound.
    group: Option<Group>,
    /// What the sequence had before inkline first bound it.
    saved: ffi::Found,
}

/// A layout key left alone because it no longer had readline's default.
pub struct LeftAlone {
    pub key: String,
    pub seq: Vec<u8>,
    pub wanted: &'static str,
}

thread_local! {
    static TABLE: RefCell<Vec<Bound>> = const { RefCell::new(Vec::new()) };
    static LEFT_ALONE: RefCell<Vec<LeftAlone>> = const { RefCell::new(Vec::new()) };
}

/// Binds every sequence of `desc` to the readline or inkline command
/// `command`.
pub fn bind(desc: &str, command: &str, group: Option<Group>) -> Result<(), String> {
    let seqs = keydesc::parse(desc)?;
    let function = ffi::named_command(command)
        .ok_or_else(|| format!("{command}: not a readline or inkline command"))?;
    // readline would turn such a key into a prefix, and an unset could not
    // turn it back.
    let runs_a_command = |start: &[u8]| {
        ffi::lookup(start).is_some_and(|f| !f.prefix && f.binding != ffi::Binding::Unbound)
    };
    if seqs
        .iter()
        .any(|seq| (1..seq.len()).any(|n| runs_a_command(&seq[..n])))
    {
        return Err(format!("{desc}: starts with a key that runs a command"));
    }
    for seq in seqs {
        bind_seq(&seq, desc, command, function, group)?;
    }
    Ok(())
}

/// Binds one sequence of `desc`. A sequence bound again while it still runs
/// inkline's binding keeps what it had before inkline first bound it.
pub fn bind_seq(
    seq: &[u8],
    desc: &str,
    command: &str,
    function: ffi::CommandFn,
    group: Option<Group>,
) -> Result<(), String> {
    let saved = TABLE.with_borrow(|t| {
        t.iter()
            .find(|b| b.seq == seq && still_ours(b))
            .map(|b| b.saved.clone())
    });
    let saved = saved.unwrap_or_else(|| {
        ffi::lookup(seq).unwrap_or(ffi::Found {
            prefix: false,
            binding: ffi::Binding::Unbound,
        })
    });
    if !ffi::bind_command(&keydesc::readline_text(seq), function) {
        return Err(format!("{desc}: readline would not bind it"));
    }
    if TTY_KEYS.contains(&seq) {
        ffi::set_readline_variable("bind-tty-special-chars", "off");
    }
    TABLE.with_borrow_mut(|t| {
        t.retain(|b| b.seq != seq);
        t.push(Bound {
            seq: seq.to_vec(),
            key: desc.to_owned(),
            command: command.to_owned(),
            function,
            group,
            saved,
        });
    });
    Ok(())
}

/// Whether `b`'s sequence still runs what inkline bound it to.
fn still_ours(b: &Bound) -> bool {
    ffi::lookup(&b.seq).is_some_and(
        |f| matches!(f.binding, ffi::Binding::Command(g) if std::ptr::fn_addr_eq(g, b.function)),
    )
}

/// Forgets the sequences `pick` chooses, putting back what they had before
/// wherever inkline still owns them.
fn unset_where(pick: impl Fn(&Bound) -> bool) {
    TABLE.with_borrow_mut(|t| {
        t.retain(|b| {
            if !pick(b) {
                return true;
            }
            if still_ours(b) {
                ffi::restore(&b.seq, &b.saved);
            }
            false
        })
    });
}

/// Puts back what every sequence of `desc` had before inkline bound it.
pub fn unset(desc: &str) -> Result<(), String> {
    let seqs = keydesc::parse(desc)?;
    unset_where(|b| seqs.contains(&b.seq));
    Ok(())
}

/// Puts back the sequences the default layout bound for `groups`.
pub fn unset_groups(groups: &[Group]) {
    unset_where(|b| b.group.is_some_and(|g| groups.contains(&g)));
}

/// Puts back every sequence inkline still owns (for `reload`).
pub fn restore_all() {
    unset_where(|_| true);
    LEFT_ALONE.with_borrow_mut(Vec::clear);
}

/// The keys readline binds to its own commands at each new line while
/// `bind-tty-special-chars` is on: the usual erase (`DEL` or `C-h`), kill
/// (`C-u`), literal-next (`C-v`) and word-erase (`C-w`) characters.
pub const TTY_KEYS: [&[u8]; 5] = [b"\x7f", b"\x08", b"\x15", b"\x16", b"\x17"];

/// Whether inkline has any of `seqs` bound.
pub fn bound_any(seqs: &[&[u8]]) -> bool {
    TABLE.with_borrow(|t| t.iter().any(|b| seqs.contains(&b.seq.as_slice())))
}

pub fn note_left_alone(entry: LeftAlone) {
    LEFT_ALONE.with_borrow_mut(|l| l.push(entry));
}

/// `inkline keys`: one line per sequence inkline bound, then one per layout
/// key left alone.
pub fn lines() -> Vec<String> {
    let mut out: Vec<String> = TABLE.with_borrow(|t| {
        let mut seen: Vec<(&str, &str)> = Vec::new();
        t.iter()
            .filter(|b| still_ours(b))
            .filter(|b| {
                let key = (b.key.as_str(), b.command.as_str());
                let new = !seen.contains(&key);
                seen.push(key);
                new
            })
            .map(|b| format!("{}\t{}", b.key, b.command))
            .collect()
    });
    LEFT_ALONE.with_borrow(|l| {
        for a in l {
            let has = ffi::lookup(&a.seq)
                .map_or_else(|| "unreachable".to_owned(), |f| describe(&f.binding));
            out.push(format!(
                "{} ({})\tleft alone for {}: bound to {has}",
                a.key,
                keydesc::readline_text(&a.seq),
                a.wanted
            ));
        }
    });
    out
}

fn describe(b: &ffi::Binding) -> String {
    match b {
        ffi::Binding::Unbound => "nothing".into(),
        ffi::Binding::Command(f) => {
            ffi::command_name(*f).unwrap_or_else(|| "an unnamed function".into())
        }
        ffi::Binding::Macro(text) => format!("the text {:?}", String::from_utf8_lossy(text)),
    }
}

pub fn register(ctx: &mut TulispContext) {
    ctx.defun(
        "keymap-global-set",
        |key: String, command: TulispObject| -> Result<TulispObject, Error> {
            if !command.symbolp() {
                return Err(Error::invalid_argument(format!(
                    "{key}: the command must be a symbol naming a readline or inkline command"
                )));
            }
            bind(&key, &command.to_string(), None).map_err(Error::invalid_argument)?;
            Ok(command)
        },
    );
    ctx.defun(
        "keymap-global-unset",
        |key: String| -> Result<TulispObject, Error> {
            unset(&key).map_err(Error::invalid_argument)?;
            Ok(TulispObject::nil())
        },
    );
    ctx.defun(
        "inkline-unbind-defaults",
        |groups: Option<TulispObject>| -> Result<TulispObject, Error> {
            let group = |g: TulispObject| {
                g.symbolp()
                    .then(|| Group::named(&g.to_string()))
                    .flatten()
                    .ok_or_else(|| {
                        Error::invalid_argument(format!(
                            "{g}: not a layout group (suggestions, multi-line, pairing)"
                        ))
                    })
            };
            let groups: Vec<Group> = match groups.filter(|g| !g.null()) {
                None => Group::ALL.to_vec(),
                Some(list) if list.consp() => {
                    // Stops at the end of a dotted list, and once a circular
                    // list comes back to a cell it has been through.
                    let mut items = list.base_iter();
                    let groups = items.by_ref().map(group).collect::<Result<_, _>>()?;
                    items.take_error().map_err(|_| {
                        Error::invalid_argument(format!("{list}: not a list of layout groups"))
                    })?;
                    groups
                }
                Some(one) => vec![group(one)?],
            };
            unset_groups(&groups);
            if !bound_any(&TTY_KEYS) {
                ffi::set_readline_variable("bind-tty-special-chars", "on");
            }
            Ok(TulispObject::nil())
        },
    );
    ctx.defun(
        "inkline-set-readline-variable",
        |name: String, value: TulispObject| -> Result<TulispObject, Error> {
            let text = if value.stringp() {
                value.as_string()?
            } else if value.null() {
                "off".into()
            } else if value.symbolp() && value.to_string() == "t" {
                "on".into()
            } else {
                value.to_string()
            };
            if !ffi::set_readline_variable(&name, &text) {
                return Err(Error::invalid_argument(format!(
                    "{name}: no such readline variable"
                )));
            }
            Ok(TulispObject::nil())
        },
    );
}
