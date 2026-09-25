//! Lisp commands: Lisp functions bound to keys. Each gets one of 256
//! readline functions, its slot, or past those, shares one that finds the
//! command by its key. A command runs with readline's line as the buffer,
//! as one undo step.

use std::cell::{Cell, RefCell};
use std::ffi::{c_int, c_void};
use std::io::Write;

use tulisp::{Error, ErrorKind, Form, Rest, TulispContext, TulispObject};

use super::buffer::{self, Buffer};
use super::errors;
use crate::ffi;

use super::errors::QUIT;

/// How many Lisp commands have a readline function of their own.
pub const SLOT_COUNT: usize = 256;

extern "C" fn slot<const N: usize>(count: c_int, key: c_int) -> c_int {
    crate::hooks::run_lisp_key(Some(N), count, key, || {
        run(slot_command(N).as_ref(), count, key)
    })
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
/// readline passes a command no name. A `static`, so each function has one
/// address that keymaps and `rl_last_func` can be compared with.
static SLOTS: [[ffi::CommandFn; 16]; 16] = [
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
fn slot_index(f: ffi::CommandFn) -> Option<usize> {
    (0..SLOT_COUNT).find(|&n| std::ptr::fn_addr_eq(slot_function(n), f))
}

extern "C" fn lisp_key(count: c_int, key: c_int) -> c_int {
    crate::hooks::run_lisp_key(None, count, key, || run_shared(count, key))
}

/// `inkline-lisp-key`: the readline function of the keys of Lisp commands
/// that got no slot. It finds the command by the key readline runs it for.
pub static SHARED: ffi::CommandFn = lisp_key;

/// Whether `f` is a slot's function or `SHARED`.
pub fn is_lisp_key_function(f: ffi::CommandFn) -> bool {
    std::ptr::fn_addr_eq(f, SHARED) || slot_index(f).is_some()
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
    /// While a Lisp step runs (`one_step`): its undo group.
    static UNDO_GROUP: Cell<Option<Group>> = const { Cell::new(None) };
    /// While a Lisp step runs: its key, as readline gave it.
    static KEY: Cell<c_int> = const { Cell::new(0) };
    /// The name of the Lisp command `SHARED` last ran, `None` for a lambda.
    static SHARED_LAST: RefCell<Option<String>> = const { RefCell::new(None) };
    /// While hook functions run (`in_hook`): the hook's name.
    static IN_HOOK: Cell<Option<&'static str>> = const { Cell::new(None) };
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

/// The slot for `command`: the one it already has, or a free one. None when
/// all are taken. A named command that gets its first slot is also added to
/// readline's commands under its name, so `bind` finds it.
pub fn assign_slot(command: &LispCommand) -> Option<usize> {
    let same = |used: &LispCommand| match (used, command) {
        (LispCommand::Named(a), LispCommand::Named(b)) => a == b,
        (LispCommand::Lambda(a), LispCommand::Lambda(b)) => a.eq(b),
        _ => false,
    };
    let (n, new) = SLOT_USE.with_borrow_mut(|slots| {
        if let Some(n) = slots.iter().position(|s| s.as_ref().is_some_and(same)) {
            return Some((n, false));
        }
        let n = match slots.iter().position(Option::is_none) {
            Some(n) => n,
            None if slots.len() < SLOT_COUNT => {
                slots.push(None);
                slots.len() - 1
            }
            None => return None,
        };
        slots[n] = Some(command.clone());
        Some((n, true))
    })?;
    if new
        && let LispCommand::Named(name) = command
        && !ffi::add_named_command(name, slot_function(n))
    {
        notice(&format!(
            "{name}: readline already has a command of this name; bind finds readline's"
        ));
    }
    Some(n)
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
    message(&format!("inkline: {text}"));
}

/// Shows `text` under the line while a line is being edited, else prints it
/// on stderr.
fn message(text: &str) {
    if buffer::editing() {
        crate::hooks::show_message(text);
    } else {
        let _ = writeln!(std::io::stderr(), "{text}");
    }
}

/// Shows `text` under the line while a line is being edited, else writes it
/// to stdout at once.
fn print(text: &str) -> Result<(), Error> {
    if buffer::editing() {
        crate::hooks::show_message(text);
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| Error::os_error(format!("print: {e}")))
}

/// The running step's undo group.
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

/// Opens the running step's undo group before a change, unless one is
/// already open on the current history line. After the step moved to
/// another history line, the old group stays open on the line left
/// behind. Does nothing outside a step.
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

/// Closes the running step's undo group if it is open, and drops it
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

/// How Lisp run from a key or a hook ended, other than normally.
pub(crate) enum Failure {
    /// A `quit`, or a `C-c`.
    Quit,
    /// A `user-error`, with its text.
    Refused(String),
    /// Any other error, as one line.
    Error(String),
}

/// What the Lisp error `e` means for the command or hook that raised it.
pub(crate) fn failure_of(ctx: &TulispContext, e: &Error) -> Failure {
    if let ErrorKind::Throw(thrown) = e.kind()
        && thrown.car().is_ok_and(|tag| tag.to_string() == QUIT)
    {
        return Failure::Quit;
    }
    match errors::user_error_text(e) {
        Some(text) => Failure::Refused(text),
        None => Failure::Error(errors::describe(e, ctx, None)),
    }
}

/// Tells `before_change` a Lisp step runs, and `call-interactively` its
/// key. What they held before, for a step further up the stack, comes
/// back afterwards. On drop, closes a group still open, so readline's
/// groups stay balanced even after a panic; after a panic, as after an
/// error, the step's change on the line is also taken back, and point
/// and mark go back, unless the step moved to another history line.
struct Running {
    outer: Option<Group>,
    outer_key: c_int,
    /// Point, mark and history line from before the step.
    point: usize,
    mark: usize,
    history: c_int,
}

impl Running {
    fn start(key: c_int) -> Running {
        Running {
            outer: UNDO_GROUP.replace(Some(Group::Closed)),
            outer_key: KEY.replace(key),
            point: ffi::point(),
            mark: ffi::mark(),
            history: ffi::history_position(),
        }
    }

    /// The step's undo group.
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
        let panicking = std::thread::panicking();
        match UNDO_GROUP.replace(self.outer) {
            Some(Group::Open { history, .. }) if ffi::history_position() == history => {
                ffi::end_undo_group();
                if panicking && history == self.history {
                    ffi::do_undo();
                }
            }
            Some(Group::Open { .. }) => ffi::forget_undo_group(),
            _ => {}
        }
        if panicking && ffi::history_position() == self.history {
            let end = ffi::line_bytes().len();
            ffi::set_point(self.point.min(end));
            ffi::set_mark(self.mark.min(end));
        }
    }
}

/// The Lisp command of `slot`.
fn slot_command(slot: usize) -> Option<LispCommand> {
    SLOT_USE.with_borrow(|s| s.get(slot).cloned().flatten())
}

/// Runs the Lisp command of the key `SHARED` runs for, and notes it for
/// `last-command`.
fn run_shared(count: c_int, key: c_int) -> c_int {
    let command = ffi::running_key().and_then(super::keys::shared_command);
    let result = run(command.as_ref(), count, key);
    SHARED_LAST.set(command.as_ref().and_then(|c| c.name().map(str::to_owned)));
    result
}

/// The name of the command `f` runs, for `last-command`: a Lisp command's
/// name (`None` for a lambda), or the name readline knows `f` by.
pub(crate) fn command_symbol_of(f: ffi::CommandFn) -> Option<String> {
    if std::ptr::fn_addr_eq(f, SHARED) {
        return SHARED_LAST.with_borrow(Clone::clone);
    }
    match slot_index(f) {
        Some(n) => slot_command(n)?.name().map(str::to_owned),
        None => ffi::command_name(f),
    }
}

/// Runs `command` for its key, as one undo step (`one_step`); rings the bell
/// for none. Its error, and the bad settings it left, show under the line.
fn run(command: Option<&LispCommand>, count: c_int, key: c_int) -> c_int {
    let Some(command) = command else {
        ffi::ding();
        return 0;
    };
    let prefix = ffi::explicit_count().then_some(count);
    let last = ffi::last_command().and_then(command_symbol_of);
    let result = crate::lisp::with_lisp_marking_panics(|ctx| {
        let function = match command {
            LispCommand::Named(name) => ctx.intern(name),
            LispCommand::Lambda(f) => f.clone(),
        };
        with_command_variables(ctx, prefix, command.name(), last.as_deref(), |ctx| {
            let _line = install_line(true);
            one_step(key, false, || {
                ctx.funcall(&function, ())
                    .map(drop)
                    .map_err(|e| failure_of(ctx, &e))
            })
        })
    });
    let Ok(result) = result else {
        ffi::ding();
        return 0;
    };
    // The command's error and the bad settings it left go in one message,
    // the error first.
    let mut notices = Vec::new();
    if let Err(Failure::Error(text) | Failure::Refused(text)) = result {
        let name = command.name().unwrap_or("lambda");
        notices.push(format!("inkline: {name}: {text}"));
    }
    for problem in crate::lisp::settings::problems() {
        notices.push(format!("inkline: {problem}"));
    }
    if !notices.is_empty() {
        crate::hooks::show_message(&notices.join("; "));
    }
    0
}

/// Runs `f` as one undo step, with `KEY` set to `key`: the changes it
/// makes to the line go in one undo group of their own, opened at the
/// first change and dropped if nothing changed. When `f` fails, the group
/// is undone and point and mark go back to where they were, unless `f`
/// moved to another history line: the group then stays open on the line
/// left behind, and nothing is undone. A readline `undo` that `f` called
/// stays done, since readline cannot redo. Afterwards point and mark are
/// inside the line. `KEY` and the undo group of a step further up the
/// stack come back afterwards, also after a panic, which takes the step
/// back as an error does.
///
/// With `open_now`, the group opens before `f` runs instead of at the first
/// change. A step `f` runs with `one_step` then opens its group inside this
/// one, as readline's groups nest: undoing it takes back only its own changes,
/// and undoing this one takes back everything, the inner steps' changes too, as
/// one step. It is still dropped if nothing changed, as an inner step that
/// changed nothing added no group and one that failed took its group off again.
///
/// Must run inside `with_lisp`, with the line already installed: it installs
/// none, as installing again would drop the outer buffer and `save-excursion`'s
/// places in it.
pub(crate) fn one_step<T>(
    key: c_int,
    open_now: bool,
    f: impl FnOnce() -> Result<T, Failure>,
) -> Result<T, Failure> {
    let running = Running::start(key);
    let (point, mark, history) = (running.point, running.mark, running.history);
    if open_now {
        before_change();
    }
    let result = f();
    let group = running.finish();
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
    result
}

/// Runs `f`, which runs the functions of the hook named `hook`. Meanwhile
/// `call-interactively` refuses readline's undo commands: a hook's changes go
/// in an undo group opened before its functions run, and readline's undo could
/// take that group's start away or close it early. `y-or-n-p` refuses to ask in
/// any hook but the accept hook. What was set before comes back afterwards,
/// also after a panic.
pub(crate) fn in_hook<R>(hook: &'static str, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<&'static str>);
    impl Drop for Restore {
        fn drop(&mut self) {
            IN_HOOK.set(self.0);
        }
    }
    let _restore = Restore(IN_HOOK.replace(Some(hook)));
    f()
}

/// Installs readline's line as the buffer, read-only unless `writable`.
pub(crate) fn install_line(writable: bool) -> buffer::Installed {
    let installed = buffer::install(Box::new(ReadlineBuffer));
    buffer::set_writable(writable);
    installed
}

/// Runs `f` with `current-prefix-arg` set to `prefix` (`nil` for none), and
/// `this-command` and `last-command` to the symbols `this` and `last` (`nil`
/// for none). The bindings come off again on every path.
pub(crate) fn with_command_variables<R>(
    ctx: &mut TulispContext,
    prefix: Option<c_int>,
    this: Option<&str>,
    last: Option<&str>,
    f: impl FnOnce(&mut TulispContext) -> Result<R, Failure>,
) -> Result<R, Failure> {
    let mut symbol = |name: Option<&str>| name.map_or_else(TulispObject::nil, |n| ctx.intern(n));
    let values = [
        (
            "current-prefix-arg",
            prefix.map_or_else(TulispObject::nil, |n| TulispObject::from(i64::from(n))),
        ),
        ("this-command", symbol(this)),
        ("last-command", symbol(last)),
    ];
    /// Takes the variables' bindings off again on every path.
    struct Unbind(Vec<TulispObject>);
    impl Drop for Unbind {
        fn drop(&mut self) {
            for variable in &self.0 {
                let _ = variable.unset();
            }
        }
    }
    let mut unbind = Unbind(Vec::new());
    for (variable, value) in values {
        let variable = ctx.intern(variable);
        variable
            .set_scope(value)
            .map_err(|e| Failure::Error(errors::describe(&e, ctx, None)))?;
        unbind.0.push(variable);
    }
    f(ctx)
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
        Command::Readline(f, name) => {
            // After a `C-c` or a jump to bash's top level, no more readline
            // commands run.
            if crate::hooks::lisp_must_stop() {
                return Err(quit_error(ctx));
            }
            if IN_HOOK.get().is_some() && ffi::undo_command(f).is_some() {
                return Err(Error::lisp_error(format!("{name} cannot run in a hook")));
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
            // A `C-c` while the command read a key, or a jump to bash's top
            // level, stops the Lisp command too.
            match result {
                Ok(_) if !crate::hooks::lisp_must_stop() => Ok(TulispObject::nil()),
                _ => Err(quit_error(ctx)),
            }
        }
    }
}

/// Runs the readline command `f` with `ffi::call_command`. A jump to
/// bash's top level that it stopped is noted for `run_lisp_key` to make.
fn call_command(f: ffi::CommandFn, count: c_int, key: c_int) -> Result<c_int, ffi::Jumped> {
    let result = crate::hooks::in_readline_command(|| ffi::call_command(f, count, key));
    if let Err(ffi::Jumped::Shell(value)) = result {
        crate::hooks::note_shell_jump(value);
    }
    result
}

/// `(y-or-n-p PROMPT)`: shows `PROMPT(y or n) ` under the line and reads a
/// key, until it is `y` or `n` in either case. Any other key rings the bell.
/// `C-g`, `C-c` and a key typed ahead before it asks quit: a key typed
/// before the question showed is not an answer, though macro text is.
/// After a `C-c` or a jump to bash's top level, it quits without asking.
/// It asks only in a command or in `inkline-accept-functions`.
fn y_or_n_p(ctx: &mut TulispContext, prompt: &str) -> Result<TulispObject, Error> {
    // `UNDO_GROUP` is set while a Lisp step runs (`one_step`).
    if UNDO_GROUP.get().is_none()
        || IN_HOOK
            .get()
            .is_some_and(|hook| hook != super::hooks::ACCEPT)
    {
        return Err(Error::lisp_error(format!(
            "y-or-n-p works only in a command or in {}",
            super::hooks::ACCEPT
        )));
    }
    let question = format!("{prompt}(y or n) ");
    loop {
        if ffi::key_waiting() || crate::hooks::lisp_must_stop() {
            return Err(quit_error(ctx));
        }
        crate::hooks::show_message(&question);
        crate::hooks::show_message_now();
        // A jump readline or bash made while it waited quits too.
        let Ok(key) = call_command(ffi::read_key_command(), 1, 0) else {
            return Err(quit_error(ctx));
        };
        // Below 0: no key could be read.
        if key < 0 || key == crate::hooks::CTRL_G || key == crate::hooks::CTRL_C {
            return Err(quit_error(ctx));
        }
        match u8::try_from(key) {
            Ok(b'y' | b'Y') => return Ok(TulispObject::t()),
            Ok(b'n' | b'N') => return Ok(TulispObject::nil()),
            _ => ffi::ding(),
        }
    }
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
    ctx.defun("y-or-n-p", |ctx: &mut TulispContext, prompt: String| {
        y_or_n_p(ctx, &prompt)
    });
    ctx.defun("ding", |_arg: Option<TulispObject>| {
        ffi::ding();
        TulispObject::nil()
    });
    ctx.defun(
        "message",
        |ctx: &mut TulispContext,
         format: TulispObject,
         args: Rest<TulispObject>|
         -> Result<TulispObject, Error> {
            if format.null() {
                if buffer::editing() {
                    crate::hooks::clear_message();
                }
                return Ok(TulispObject::nil());
            }
            let text = errors::format_args(ctx, std::iter::once(format).chain(args))?;
            message(&text);
            Ok(TulispObject::from(text))
        },
    );
    ctx.defun(
        "print",
        |value: TulispObject| -> Result<TulispObject, Error> {
            print(&format!("{}\n", value.fmt_string()))?;
            Ok(value)
        },
    );
    ctx.defun(
        "princ",
        |value: TulispObject| -> Result<TulispObject, Error> {
            print(&value.fmt_string())?;
            Ok(value)
        },
    );
    ctx.defun(
        "prin1",
        |value: TulispObject| -> Result<TulispObject, Error> {
            print(&value.to_string())?;
            Ok(value)
        },
    );
    ctx.eval_prelude(
        "<inkline-commands>",
        "(defvar current-prefix-arg nil)\n(defvar this-command nil)\n(defvar last-command nil)",
    )
    .expect("inkline's own Lisp compiles");
    let own = READLINE_NAMES
        .iter()
        .filter_map(|&name| ctx.intern(name).get().ok().map(|f| (name, f)))
        .collect();
    OWN.set(own);
}
