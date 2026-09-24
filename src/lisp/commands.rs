//! Lisp commands: Lisp functions bound to keys. Each gets one of 256
//! readline functions, its slot, and runs with readline's line as the
//! buffer, as one undo step.

use std::cell::{Cell, RefCell};
use std::ffi::{c_int, c_void};
use std::io::Write;

use tulisp::{Error, ErrorKind, Form, Rest, TulispContext, TulispObject};

use super::buffer::{self, Buffer};
use super::errors;
use crate::ffi;

use super::errors::QUIT;

/// How many Lisp commands can have keys at once.
const SLOT_COUNT: usize = 256;

extern "C" fn slot<const N: usize>(count: c_int, key: c_int) -> c_int {
    crate::hooks::run_lisp_command(N, count, key)
}

macro_rules! row {
    ($r:literal) => {
        [
            slot::<{ $r * 16 }>,
            slot::<{ $r * 16 + 1 }>,
            slot::<{ $r * 16 + 2 }>,
            slot::<{ $r * 16 + 3 }>,
            slot::<{ $r * 16 + 4 }>,
            slot::<{ $r * 16 + 5 }>,
            slot::<{ $r * 16 + 6 }>,
            slot::<{ $r * 16 + 7 }>,
            slot::<{ $r * 16 + 8 }>,
            slot::<{ $r * 16 + 9 }>,
            slot::<{ $r * 16 + 10 }>,
            slot::<{ $r * 16 + 11 }>,
            slot::<{ $r * 16 + 12 }>,
            slot::<{ $r * 16 + 13 }>,
            slot::<{ $r * 16 + 14 }>,
            slot::<{ $r * 16 + 15 }>,
        ]
    };
}

/// One readline function per Lisp command, so each knows which it is:
/// readline passes a command no name.
pub const SLOTS: [[ffi::CommandFn; 16]; 16] = [
    row!(0),
    row!(1),
    row!(2),
    row!(3),
    row!(4),
    row!(5),
    row!(6),
    row!(7),
    row!(8),
    row!(9),
    row!(10),
    row!(11),
    row!(12),
    row!(13),
    row!(14),
    row!(15),
];

/// The readline function of slot `n`.
pub fn slot_function(n: usize) -> ffi::CommandFn {
    SLOTS[n / 16][n % 16]
}

/// The slot `f` is the readline function of, if it is one.
pub fn slot_index(f: ffi::CommandFn) -> Option<usize> {
    (0..SLOT_COUNT).find(|&n| std::ptr::fn_addr_eq(slot_function(n), f))
}

/// A Lisp function bound to a key.
#[derive(Clone)]
pub enum LispCommand {
    /// Found by name in the interpreter each time its key is pressed.
    Named(String),
    Lambda(TulispObject),
}

impl LispCommand {
    /// The command's name; None for a lambda.
    pub fn name(&self) -> Option<&str> {
        match self {
            LispCommand::Named(name) => Some(name),
            LispCommand::Lambda(_) => None,
        }
    }
}

/// What `keymap-global-set` binds a key to.
pub enum Command {
    /// A readline or inkline command, and its name.
    Readline(ffi::CommandFn, String),
    Lisp(LispCommand),
}

/// inkline's own Lisp functions that share a name with a readline command.
/// While a symbol still has inkline's function, a key bound to it runs
/// readline's command.
const READLINE_NAMES: [&str; 8] = [
    "kill-region",
    "delete-char",
    "forward-char",
    "backward-char",
    "beginning-of-line",
    "end-of-line",
    "copy-region-as-kill",
    "set-mark",
];

thread_local! {
    /// The Lisp command of each slot. A named command keeps its slot for
    /// the life of the process, since readline may know it by name.
    static SLOT_USE: RefCell<Vec<Option<LispCommand>>> = const { RefCell::new(Vec::new()) };
    /// The function objects inkline gave `READLINE_NAMES` in the current
    /// interpreter.
    static OWN: RefCell<Vec<(&'static str, TulispObject)>> = const { RefCell::new(Vec::new()) };
    /// While a Lisp command runs from a key: its undo group.
    static UNDO_GROUP: Cell<Option<Group>> = const { Cell::new(None) };
    /// While a Lisp command runs from a key: that key, as readline gave it.
    static KEY: Cell<c_int> = const { Cell::new(0) };
}

/// What `object`, given to `keymap-global-set`, binds a key to.
pub fn resolve(ctx: &TulispContext, object: &TulispObject) -> Result<Command, String> {
    if !object.symbolp() {
        if object.functionp(ctx) {
            return Ok(Command::Lisp(LispCommand::Lambda(object.clone())));
        }
        return Err(format!(
            "{object}: the command must be a symbol or a lambda"
        ));
    }
    let name = object.to_string();
    let value = object.get().ok();
    let own = OWN.with_borrow(|own| {
        own.iter()
            .any(|(n, f)| *n == name && value.as_ref().is_some_and(|v| v.eq(f)))
    });
    if !own && object.functionp(ctx) {
        return Ok(Command::Lisp(LispCommand::Named(name)));
    }
    match ffi::named_command(&name) {
        Some(f) => Ok(Command::Readline(f, name)),
        None => Err(format!(
            "{name}: not a readline or inkline command, nor a Lisp function"
        )),
    }
}

/// The slot for `command`: the one it already has, or a free one. A named
/// command that gets its first slot is also added to readline's commands
/// under its name, so `bind` finds it.
pub fn assign_slot(command: &LispCommand) -> Result<usize, String> {
    let same = |used: &LispCommand| match (used, command) {
        (LispCommand::Named(a), LispCommand::Named(b)) => a == b,
        (LispCommand::Lambda(a), LispCommand::Lambda(b)) => a.eq(b),
        _ => false,
    };
    let (n, new) = SLOT_USE.with_borrow_mut(|slots| {
        if let Some(n) = slots.iter().position(|s| s.as_ref().is_some_and(same)) {
            return Ok((n, false));
        }
        let n = match slots.iter().position(Option::is_none) {
            Some(n) => n,
            None if slots.len() < SLOT_COUNT => {
                slots.push(None);
                slots.len() - 1
            }
            None => {
                return Err(format!(
                    "no more than {SLOT_COUNT} Lisp commands can have keys"
                ));
            }
        };
        slots[n] = Some(command.clone());
        Ok((n, true))
    })?;
    if new
        && let LispCommand::Named(name) = command
        && !ffi::add_named_command(name, slot_function(n))
    {
        notice(&format!(
            "{name}: readline already has a command of this name; bind finds readline's"
        ));
    }
    Ok(n)
}

/// Frees the slots of lambdas no key in `used` runs.
pub fn free_unused_lambda_slots(used: &[usize]) {
    SLOT_USE.with_borrow_mut(|slots| {
        for (n, s) in slots.iter_mut().enumerate() {
            if matches!(s, Some(LispCommand::Lambda(_))) && !used.contains(&n) {
                *s = None;
            }
        }
    });
}

/// Shows `text` after `inkline: `: on stderr, or while a line is being
/// edited, with the line.
pub fn notice(text: &str) {
    let text = format!("inkline: {text}");
    if buffer::editing() {
        crate::hooks::show_message(&text);
    } else {
        let _ = writeln!(std::io::stderr(), "{text}");
    }
}

/// The running command's undo group.
#[derive(Clone, Copy)]
enum Group {
    Closed,
    /// Open on history line `history`, with `before` the newest undo entry
    /// from before it opened.
    Open {
        before: *const c_void,
        history: c_int,
    },
}

/// Opens the running command's undo group before a change, unless one is
/// already open on the current history line. After the command moved to
/// another history line, the old group stays open on the line left
/// behind. Does nothing outside a command.
fn before_change() {
    if let Some(Group::Open { history, .. }) = UNDO_GROUP.get()
        && history != ffi::history_position()
    {
        close_group();
    }
    if let Some(Group::Closed) = UNDO_GROUP.get() {
        let before = ffi::undo_list_head();
        ffi::begin_undo_group();
        UNDO_GROUP.set(Some(Group::Open {
            before,
            history: ffi::history_position(),
        }));
    }
}

/// Closes the running command's undo group if it is open, and drops it
/// when nothing changed inside it. The next change opens a new one. A
/// group opened on another history line stays open there: the current
/// line gets no `UNDO_END`.
fn close_group() {
    if let Some(Group::Open { before, history }) = UNDO_GROUP.get() {
        if ffi::history_position() == history {
            ffi::end_undo_group();
            ffi::drop_empty_undo_group(before);
        } else {
            ffi::forget_undo_group();
        }
        UNDO_GROUP.set(Some(Group::Closed));
    }
}

/// How a Lisp command ended, other than normally.
enum Failure {
    Quit,
    Error(String),
}

/// Tells `before_change` a Lisp command runs, and `call-interactively` its
/// key. What they held before, for a command further up the stack, comes
/// back afterwards. On drop, closes a group still open, so readline's
/// groups stay balanced even after a panic.
struct Running {
    outer: Option<Group>,
    outer_key: c_int,
}

impl Running {
    fn start(key: c_int) -> Running {
        Running {
            outer: UNDO_GROUP.replace(Some(Group::Closed)),
            outer_key: KEY.replace(key),
        }
    }

    /// The command's undo group.
    fn finish(self) -> Option<Group> {
        let group = UNDO_GROUP.replace(self.outer);
        KEY.set(self.outer_key);
        std::mem::forget(self);
        group
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        KEY.set(self.outer_key);
        if let Some(Group::Open { .. }) = UNDO_GROUP.replace(self.outer) {
            ffi::end_undo_group();
        }
    }
}

/// Runs the Lisp command of `slot` for its key, as one undo step. On an
/// error or `quit`, the line, point and mark go back to what they were,
/// unless the command moved to another history line. A readline `undo` the
/// command called stays done, since readline cannot redo.
pub fn run(slot: usize, count: c_int, key: c_int) -> c_int {
    let Some(command) = SLOT_USE.with_borrow(|s| s.get(slot).cloned().flatten()) else {
        ffi::ding();
        return 0;
    };
    let point = ffi::point();
    let mark = ffi::mark();
    let history = ffi::history_position();
    let prefix = ffi::explicit_count().then_some(count);
    let running = Running::start(key);
    let result = crate::lisp::with_lisp(|ctx| call(ctx, &command, prefix));
    let group = running.finish();
    let Ok(result) = result else {
        ffi::ding();
        return 0;
    };
    let here = ffi::history_position();
    let undo = result.is_err() && here == history;
    match group {
        Some(Group::Open { before, history }) if history == here => {
            ffi::end_undo_group();
            if undo {
                ffi::do_undo();
            } else {
                ffi::drop_empty_undo_group(before);
            }
        }
        // The group stays open on the line left behind; readline's count of
        // open groups must not stay raised.
        Some(Group::Open { .. }) => ffi::forget_undo_group(),
        Some(Group::Closed) | None => {}
    }
    if undo {
        ffi::set_point(point);
        ffi::set_mark(mark);
    }
    let end = ffi::line_bytes().len();
    ffi::set_point(ffi::point().min(end));
    ffi::set_mark(ffi::mark().min(end));
    if let Err(Failure::Error(text)) = result {
        let name = command.name().unwrap_or("lambda");
        crate::hooks::show_message(&format!("inkline: {name}: {text}"));
    }
    0
}

/// Calls `command` with the line installed as the buffer and
/// `current-prefix-arg` set to `prefix`.
fn call(
    ctx: &mut TulispContext,
    command: &LispCommand,
    prefix: Option<c_int>,
) -> Result<(), Failure> {
    let function = match command {
        LispCommand::Named(name) => ctx.intern(name),
        LispCommand::Lambda(f) => f.clone(),
    };
    let arg = ctx.intern("current-prefix-arg");
    let value = prefix.map_or_else(TulispObject::nil, |n| TulispObject::from(i64::from(n)));
    arg.set_scope(value)
        .map_err(|e| Failure::Error(errors::describe(&e, ctx, None)))?;
    /// Takes `current-prefix-arg`'s binding off again on every path.
    struct Unbind(TulispObject);
    impl Drop for Unbind {
        fn drop(&mut self) {
            let _ = self.0.unset();
        }
    }
    let _unbind = Unbind(arg);
    let _installed = buffer::install(Box::new(ReadlineBuffer));
    ctx.funcall(&function, ()).map(drop).map_err(|e| {
        if let ErrorKind::Throw(thrown) = e.kind()
            && thrown.car().is_ok_and(|tag| tag.to_string() == QUIT)
        {
            Failure::Quit
        } else {
            Failure::Error(errors::describe(&e, ctx, None))
        }
    })
}

/// The error `quit` raises: a throw to `QUIT`, which `condition-case`
/// with `error` does not catch.
fn quit_error(ctx: &mut TulispContext) -> Error {
    Error::throw(ctx.intern(QUIT), TulispObject::nil())
}

/// `(call-interactively COMMAND)`: runs a readline, inkline or Lisp command
/// with `current-prefix-arg` as its count. A Lisp command is called here,
/// with the running interpreter; a readline command gets the key that ran
/// the Lisp command, and gives `nil`.
fn call_interactively(
    ctx: &mut TulispContext,
    command: &TulispObject,
) -> Result<TulispObject, Error> {
    if !buffer::editing() {
        return Err(Error::lisp_error("no line is being edited"));
    }
    let prefix = ctx.intern("current-prefix-arg").get()?;
    let count = if prefix.null() {
        None
    } else {
        let n = buffer::position(&prefix)?;
        Some(
            c_int::try_from(n)
                .map_err(|_| Error::out_of_range(format!("Args out of range: {n}")))?,
        )
    };
    match resolve(ctx, command).map_err(Error::lisp_error)? {
        Command::Lisp(LispCommand::Named(name)) => {
            let function = ctx.intern(&name);
            ctx.funcall(&function, ())
        }
        Command::Lisp(LispCommand::Lambda(function)) => ctx.funcall(&function, ()),
        Command::Readline(f, _) => {
            // After a jump to bash's top level, no more readline commands
            // run.
            if crate::hooks::lisp_must_stop() {
                return Err(quit_error(ctx));
            }
            // Undo works on whole steps: the command's changes so far become
            // one, and a later change opens a new one.
            match ffi::undo_command(f) {
                None => before_change(),
                Some(ffi::Undo::All) => close_group(),
                Some(ffi::Undo::Steps) if count.unwrap_or(1) > 0 => close_group(),
                Some(ffi::Undo::Steps) => {}
            }
            let outer = ffi::replace_explicit_count(count.is_some());
            let result = call_command(f, count.unwrap_or(1), KEY.get());
            ffi::replace_explicit_count(outer);
            match result {
                Ok(_) => Ok(TulispObject::nil()),
                Err(_) => Err(quit_error(ctx)),
            }
        }
    }
}

/// Runs the readline command `f` with `ffi::call_command`. A jump to
/// bash's top level that it stopped is noted for `run_lisp_command` to make.
fn call_command(f: ffi::CommandFn, count: c_int, key: c_int) -> Result<c_int, ffi::Jumped> {
    let result = ffi::call_command(f, count, key);
    if let Err(ffi::Jumped::Shell(value)) = result {
        crate::hooks::note_shell_jump(value);
    }
    result
}

/// Readline's line as a `Buffer`. Each change first opens the running
/// command's undo group.
struct ReadlineBuffer;

impl ReadlineBuffer {
    /// Runs `f`, keeping point and mark where they were: readline moves them
    /// back inside a line that got shorter, and `Buffer` leaves that to the
    /// caller.
    fn keeping_point_and_mark<R>(f: impl FnOnce() -> R) -> R {
        let (point, mark) = (ffi::point(), ffi::mark());
        let result = f();
        ffi::set_point(point);
        ffi::set_mark(mark);
        result
    }
}

impl Buffer for ReadlineBuffer {
    fn text(&self) -> Result<String, String> {
        String::from_utf8(ffi::line_bytes()).map_err(|_| "the line is not UTF-8".to_owned())
    }

    fn point(&self) -> usize {
        ffi::point()
    }

    fn mark(&self) -> usize {
        ffi::mark()
    }

    fn set_point(&mut self, byte: usize) {
        ffi::set_point(buffer::char_start_within(&ffi::line_bytes(), byte));
    }

    fn set_mark(&mut self, byte: usize) {
        ffi::set_mark(buffer::char_start_within(&ffi::line_bytes(), byte));
    }

    fn insert(&mut self, text: &str) -> Result<(), String> {
        if text.contains('\0') {
            return Err("the line cannot contain a NUL character".into());
        }
        before_change();
        ffi::insert_text(text);
        Ok(())
    }

    fn delete(&mut self, start: usize, end: usize) -> Result<(), String> {
        before_change();
        Self::keeping_point_and_mark(|| ffi::delete_text(start, end));
        Ok(())
    }

    fn kill(&mut self, start: usize, end: usize, backward: bool) -> Result<(), String> {
        before_change();
        // readline's rl_kill_text puts the text in front of the previous
        // kill when FROM is after TO.
        let (from, to) = if backward { (end, start) } else { (start, end) };
        Self::keeping_point_and_mark(|| ffi::kill_text(from, to));
        Ok(())
    }

    fn copy(&mut self, start: usize, end: usize, backward: bool) -> Result<(), String> {
        let (point, mark) = if backward { (end, start) } else { (start, end) };
        Self::keeping_point_and_mark(|| {
            ffi::set_point(point);
            ffi::set_mark(mark);
            call_command(ffi::copy_region_command(), 1, 0)
        })
        .map(drop)
        .map_err(|_| "copying to the kill ring was stopped".to_owned())
    }

    fn region_active(&self) -> bool {
        ffi::region_active()
    }
}

pub fn register(ctx: &mut TulispContext) {
    ctx.defspecial("interactive", |_args: Rest<Form>| TulispObject::nil());
    ctx.defun(
        "call-interactively",
        |ctx: &mut TulispContext, command: TulispObject| call_interactively(ctx, &command),
    );
    ctx.defun("ding", |_arg: Option<TulispObject>| {
        ffi::ding();
        TulispObject::nil()
    });
    ctx.eval_prelude("<inkline-commands>", "(defvar current-prefix-arg nil)")
        .expect("inkline's own Lisp compiles");
    let own = READLINE_NAMES
        .iter()
        .filter_map(|&name| ctx.intern(name).get().ok().map(|f| (name, f)))
        .collect();
    OWN.set(own);
}
