//! Declarations for the bash and readline symbols inkline uses, with safe
//! wrappers. All of inkline's unsafe code is in this file.
//!
//! The symbols are left undefined in the library and resolved against the bash
//! binary when `enable -f` loads it.

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_ulong, c_void};

pub const EXECUTION_SUCCESS: c_int = 0;
pub const EXECUTION_FAILURE: c_int = 1;
/// bash reports this as exit status 2, a usage error.
pub const EX_USAGE: c_int = 258;

const BUILTIN_ENABLED: c_int = 0x01;

#[repr(C)]
pub struct WordDesc {
    word: *mut c_char,
    flags: c_int,
}

#[repr(C)]
pub struct WordList {
    next: *mut WordList,
    word: *mut WordDesc,
}

/// bash's `struct builtin`.
#[repr(C)]
pub struct Builtin {
    name: *const c_char,
    function: Option<unsafe extern "C" fn(*mut WordList) -> c_int>,
    flags: c_int,
    long_doc: *const *const c_char,
    short_doc: *const c_char,
    handle: *mut c_char,
}

struct LongDoc([*const c_char; 3]);
// Only ever read, from bash's single thread.
unsafe impl Sync for LongDoc {}

static LONG_DOC: LongDoc = LongDoc([
    c"Syntax highlighting and history suggestions for readline.".as_ptr(),
    c"`inkline on' and `inkline off' switch them; `inkline status' shows which.".as_ptr(),
    std::ptr::null(),
]);

/// The entry bash looks up as `<name>_struct` when `enable -f` loads the
/// library. bash writes to `flags`, so it has to be mutable.
#[unsafe(no_mangle)]
pub static mut inkline_struct: Builtin = Builtin {
    name: c"inkline".as_ptr(),
    function: Some(inkline_builtin),
    flags: BUILTIN_ENABLED,
    long_doc: (&raw const LONG_DOC.0) as *const *const c_char,
    short_doc: c"inkline [on|off|status|keys|load FILE|eval EXPR|reload]".as_ptr(),
    handle: std::ptr::null_mut(),
};

/// The string at `p`, or None for a NULL pointer.
///
/// # Safety
///
/// `p` is NULL or points to a NUL-terminated string that stays valid for `'a`.
unsafe fn c_str<'a>(p: *const c_char) -> Option<&'a CStr> {
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) })
}

unsafe extern "C" fn inkline_builtin(list: *mut WordList) -> c_int {
    let mut args = Vec::new();
    let mut node = list;
    // SAFETY: bash passes a NULL-terminated list of valid words.
    unsafe {
        while !node.is_null() {
            let word = (*node).word;
            if !word.is_null()
                && let Some(text) = c_str((*word).word)
            {
                args.push(text.to_string_lossy().into_owned());
            }
            node = (*node).next;
        }
    }
    crate::hooks::builtin(&args)
}

/// Called by `enable -f` after loading. Returning 0 makes bash refuse the
/// builtin.
#[unsafe(no_mangle)]
pub extern "C" fn inkline_builtin_load(_name: *mut c_char) -> c_int {
    crate::hooks::load();
    1
}

/// Called by `enable -d` before it removes the builtin.
#[unsafe(no_mangle)]
pub extern "C" fn inkline_builtin_unload(_name: *mut c_char) {
    crate::hooks::unload();
}

// ---- Drawing ----

/// A readline hook that takes and returns nothing, such as
/// `rl_redisplay_function`.
pub type VoidFn = unsafe extern "C" fn();

unsafe extern "C" {
    static mut rl_redisplay_function: Option<VoidFn>;
    static mut rl_line_buffer: *mut c_char;
    static mut rl_point: c_int;
    static mut rl_end: c_int;
    static mut rl_display_prompt: *mut c_char;
    static mut rl_outstream: *mut libc::FILE;
    fn rl_redisplay();
    fn rl_get_screen_size(rows: *mut c_int, cols: *mut c_int);
    fn rl_variable_value(name: *const c_char) -> *const c_char;
    fn get_string_value(name: *const c_char) -> *const c_char;
    fn find_reserved_word(word: *const c_char) -> c_int;
    fn find_alias(name: *const c_char) -> *mut c_void;
    fn find_function(name: *const c_char) -> *mut c_void;
    fn find_shell_builtin(name: *const c_char) -> *mut c_void;
}

pub fn redisplay_function() -> Option<VoidFn> {
    unsafe { rl_redisplay_function }
}

pub fn set_redisplay_function(f: Option<VoidFn>) {
    unsafe { rl_redisplay_function = f }
}

/// Calls `f`, or readline's own `rl_redisplay` if there is none.
pub fn call_redisplay(f: Option<VoidFn>) {
    unsafe { f.unwrap_or(rl_redisplay as VoidFn)() }
}

/// The line being edited, or None if it is not valid UTF-8.
pub fn line() -> Option<String> {
    // SAFETY: readline keeps rl_end bytes of rl_line_buffer valid.
    unsafe {
        if rl_line_buffer.is_null() || rl_end < 0 {
            return None;
        }
        let bytes = std::slice::from_raw_parts(rl_line_buffer as *const u8, rl_end as usize);
        String::from_utf8(bytes.to_vec()).ok()
    }
}

/// The cursor position in the line, in bytes.
pub fn point() -> usize {
    unsafe { rl_point.max(0) as usize }
}

/// The prompt readline is showing, with its `\001`/`\002` markers.
pub fn display_prompt() -> Vec<u8> {
    unsafe { c_str(rl_display_prompt) }.map_or_else(Vec::new, |p| p.to_bytes().to_vec())
}

/// Rows and columns of the terminal.
pub fn screen_size() -> (usize, usize) {
    let (mut rows, mut cols) = (0, 0);
    unsafe { rl_get_screen_size(&mut rows, &mut cols) };
    (rows.max(0) as usize, cols.max(0) as usize)
}

/// Whether the readline boolean variable `name` is on.
pub fn variable_on(name: &CStr) -> bool {
    unsafe { c_str(rl_variable_value(name.as_ptr())) }.is_some_and(|v| v.to_bytes() == b"on")
}

/// The value of the readline variable `name`, such as `comment-begin`.
pub fn variable(name: &CStr) -> Option<String> {
    unsafe { c_str(rl_variable_value(name.as_ptr())) }.map(|v| v.to_string_lossy().into_owned())
}

unsafe extern "C" {
    static mut bash_readline_initialized: c_int;
    static rl_readline_version: c_int;
    fn initialize_readline();
    fn rl_variable_bind(name: *const c_char, value: *const c_char) -> c_int;
}

/// Runs bash's readline set-up, as `bind` does, unless it already ran; it
/// reads `inputrc`.
pub fn initialize_readline_once() {
    unsafe {
        if bash_readline_initialized == 0 {
            initialize_readline();
        }
    }
}

/// Readline's version, such as 0x0800 for 8.0.
pub fn readline_version() -> c_int {
    unsafe { rl_readline_version }
}

/// Sets a readline variable; false when readline has no such variable.
pub fn set_readline_variable(name: &str, value: &str) -> bool {
    let (Ok(n), Ok(v)) = (CString::new(name), CString::new(value)) else {
        return false;
    };
    if variable(&n).is_none() {
        return false;
    }
    unsafe { rl_variable_bind(n.as_ptr(), v.as_ptr()) };
    true
}

unsafe extern "C" {
    fn rl_get_termcap(cap: *const c_char) -> *mut c_char;
    static mut _rl_echoing_p: c_int;
}

/// Whether readline draws the line being edited. It does not if the terminal's
/// echo was off when readline set up the terminal. `read -e -s` turns echo off
/// before readline starts, unless `-n`, `-N` or `-d` is given.
pub fn echoing() -> bool {
    unsafe { _rl_echoing_p != 0 }
}

/// Whether readline knows how to move the cursor up. Without it, readline
/// scrolls long lines sideways instead of wrapping them.
pub fn terminal_can_move_up() -> bool {
    unsafe {
        let up = rl_get_termcap(c"up".as_ptr());
        !up.is_null() && *up != 0
    }
}

/// Whether the locale's character set is UTF-8, as bash set it.
pub fn utf8_locale() -> bool {
    unsafe { c_str(libc::nl_langinfo(libc::CODESET)) }.is_some_and(|c| c.to_bytes() == b"UTF-8")
}

/// Whether readline is highlighting an active region (a search match or pasted
/// text). `rl_mark_active_p` only exists from readline 8.1, so it is looked up
/// at run time.
pub fn region_active() -> bool {
    type MarkActiveFn = unsafe extern "C" fn() -> c_int;
    static MARK_ACTIVE: std::sync::OnceLock<Option<MarkActiveFn>> = std::sync::OnceLock::new();
    let mark_active = *MARK_ACTIVE.get_or_init(|| {
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"rl_mark_active_p".as_ptr()) };
        // SAFETY: readline defines rl_mark_active_p as `int (void)`.
        (!symbol.is_null())
            .then(|| unsafe { std::mem::transmute::<*mut c_void, MarkActiveFn>(symbol) })
    });
    mark_active.is_some_and(|f| unsafe { f() != 0 })
}

/// The value of a shell variable, exported or not.
pub fn shell_variable(name: &str) -> Option<String> {
    let name = CString::new(name).ok()?;
    unsafe { c_str(get_string_value(name.as_ptr())) }.map(|v| v.to_string_lossy().into_owned())
}

/// Whether `word` is a keyword, alias, function or builtin.
pub fn known_to_bash(word: &str) -> bool {
    let Ok(word) = CString::new(word) else {
        return false;
    };
    let word = word.as_ptr();
    unsafe {
        find_reserved_word(word) >= 0
            || !find_alias(word).is_null()
            || !find_function(word).is_null()
            || !find_shell_builtin(word).is_null()
    }
}

/// Adds to readline's output stream without flushing, so the bytes stay in
/// order with what readline writes and reach the terminal with its next flush.
pub fn write_queued(bytes: &[u8]) {
    unsafe {
        let out = rl_outstream;
        if !out.is_null() {
            libc::fwrite(bytes.as_ptr().cast(), 1, bytes.len(), out);
        }
    }
}

pub fn flush_out() {
    unsafe {
        let out = rl_outstream;
        if !out.is_null() {
            libc::fflush(out);
        }
    }
}

// ---- Hooks around reading keys ----

pub type GetcFn = unsafe extern "C" fn(*mut libc::FILE) -> c_int;

unsafe extern "C" {
    static mut rl_getc_function: Option<GetcFn>;
    static mut rl_deprep_term_function: Option<VoidFn>;
    fn rl_getc(stream: *mut libc::FILE) -> c_int;
}

pub fn getc_function() -> Option<GetcFn> {
    unsafe { rl_getc_function }
}

pub fn set_getc_function(f: Option<GetcFn>) {
    unsafe { rl_getc_function = f }
}

/// Calls `f`, or readline's own `rl_getc` if there is none.
pub fn call_getc(f: Option<GetcFn>, stream: *mut libc::FILE) -> c_int {
    unsafe { f.unwrap_or(rl_getc as GetcFn)(stream) }
}

pub fn deprep_function() -> Option<VoidFn> {
    unsafe { rl_deprep_term_function }
}

pub fn set_deprep_function(f: Option<VoidFn>) {
    unsafe { rl_deprep_term_function = f }
}

/// Calls `f` if there is one; readline skips the call when it is NULL.
pub fn call_deprep(f: Option<VoidFn>) {
    if let Some(f) = f {
        unsafe { f() }
    }
}

/// A readline hook that returns an int, such as `rl_pre_input_hook`.
pub type HookFn = unsafe extern "C" fn() -> c_int;

unsafe extern "C" {
    static mut rl_pre_input_hook: Option<HookFn>;
}

pub fn pre_input_hook() -> Option<HookFn> {
    unsafe { rl_pre_input_hook }
}

pub fn set_pre_input_hook(f: Option<HookFn>) {
    unsafe { rl_pre_input_hook = f }
}

/// Calls `f` if there is one, returning its result or 0.
pub fn call_hook(f: Option<HookFn>) -> c_int {
    f.map_or(0, |f| unsafe { f() })
}

// ---- Suggestions ----

// rl_readline_state flags; the same values in readline 8.0 and 8.2.
const RL_STATE_READCMD: c_ulong = 0x8;
const RL_STATE_DISPATCHING: c_ulong = 0x20;
const RL_STATE_MOREINPUT: c_ulong = 0x40;
const RL_STATE_ISEARCH: c_ulong = 0x80;
const RL_STATE_NSEARCH: c_ulong = 0x100;
const RL_STATE_SEARCH: c_ulong = 0x200;
const RL_STATE_NUMERICARG: c_ulong = 0x400;
const RL_STATE_MACROINPUT: c_ulong = 0x800;
const RL_STATE_DONE: c_ulong = 0x2000000;

#[repr(C)]
struct HistEntry {
    line: *mut c_char,
    timestamp: *mut c_char,
    data: *mut c_void,
}

unsafe extern "C" {
    static mut rl_readline_state: c_ulong;
    static mut rl_done: c_int;
    static mut history_base: c_int;
    static mut history_length: c_int;
    fn history_get(offset: c_int) -> *mut HistEntry;
}

/// Whether readline is doing plain editing: not searching, reading a count,
/// reading a quoted character, or finishing the line.
pub fn normal_editing() -> bool {
    let busy = RL_STATE_MOREINPUT
        | RL_STATE_ISEARCH
        | RL_STATE_NSEARCH
        | RL_STATE_SEARCH
        | RL_STATE_NUMERICARG
        | RL_STATE_DONE;
    unsafe { rl_readline_state & busy == 0 && rl_done == 0 }
}

/// Whether readline is reading the key for the next command of a line.
/// A key that a running command reads (a question, the key after `C-q`,
/// the keys of a search) is not one, so while a Lisp command runs, this
/// holds only inside a line of its own, such as `read -e` in shell code
/// that a readline command runs.
pub fn reading_command_key() -> bool {
    unsafe { rl_readline_state & RL_STATE_READCMD != 0 }
}

/// Whether readline is running a key's command.
pub fn dispatching() -> bool {
    unsafe { rl_readline_state & RL_STATE_DISPATCHING != 0 }
}

/// Whether readline has accepted the line.
pub fn line_done() -> bool {
    unsafe { rl_done != 0 }
}

/// Calls `f` on the history entries from newest to oldest, until it returns
/// Some. Entries that are not valid UTF-8 are skipped.
pub fn history_find_map<T>(mut f: impl FnMut(&str) -> Option<T>) -> Option<T> {
    let (base, length) = unsafe { (history_base, history_length) };
    for offset in (base..base + length).rev() {
        // SAFETY: offsets history_base .. history_base + history_length - 1 are
        // valid, and history does not change while `f` runs.
        let text = unsafe {
            let entry = history_get(offset);
            if entry.is_null() {
                continue;
            }
            c_str((*entry).line)
        };
        let Some(Ok(text)) = text.map(CStr::to_str) else {
            continue;
        };
        if let Some(found) = f(text) {
            return Some(found);
        }
    }
    None
}

unsafe extern "C" {
    static mut rl_signal_event_hook: Option<unsafe extern "C" fn() -> c_int>;
    fn rl_check_signals();
    fn rl_pending_signal() -> c_int;
}

/// A signal that came before the wait for a key started (such as while
/// readline redrew the line), as `Wait::Signal` gives it: one readline
/// caught and has not handled yet, or 0 for a `C-c` readline has handled
/// and bash's signal hook has yet to act on. Readline's own reader would
/// wait for a key before either is acted on.
pub fn signal_before_wait() -> Option<Wait> {
    // SAFETY: these read plain values readline and bash keep.
    unsafe {
        let signal = rl_pending_signal();
        let hook = rl_signal_event_hook;
        if signal != 0 {
            Some(Wait::Signal(signal))
        } else if interrupted() && hook.is_some() {
            Some(Wait::Signal(0))
        } else {
            None
        }
    }
}

pub enum Wait {
    /// Input is ready to read, or readline's timeout (`read -t`) is up.
    Ready,
    /// The pause asked for passed without input.
    Paused,
    /// A signal interrupted the wait. The value is the signal readline caught
    /// and has not handled yet, or 0 for one readline does not catch, such as
    /// the `SIGCHLD` of a background job ending.
    Signal(c_int),
    /// Waiting failed; readline's own reader will report it.
    Error,
}

/// Milliseconds left until readline's timeout (`read -t`), rounded up, or -1
/// when there is none. Readline 8.2 keeps the timeout itself and checks it in
/// its own reader; older versions leave it to bash's `SIGALRM`, and have no
/// `rl_timeout_remaining`, so it is looked up at run time.
fn timeout_remaining() -> c_int {
    type TimeoutFn = unsafe extern "C" fn(*mut c_uint, *mut c_uint) -> c_int;
    static TIMEOUT: std::sync::OnceLock<Option<TimeoutFn>> = std::sync::OnceLock::new();
    let timeout = *TIMEOUT.get_or_init(|| {
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"rl_timeout_remaining".as_ptr()) };
        // SAFETY: readline defines rl_timeout_remaining as
        // `int (unsigned int *, unsigned int *)`.
        (!symbol.is_null())
            .then(|| unsafe { std::mem::transmute::<*mut c_void, TimeoutFn>(symbol) })
    });
    let Some(timeout) = timeout else { return -1 };
    let (mut secs, mut usecs) = (0, 0);
    match unsafe { timeout(&mut secs, &mut usecs) } {
        1 => c_int::try_from(u64::from(secs) * 1000 + u64::from(usecs).div_ceil(1000))
            .unwrap_or(c_int::MAX),
        // No timeout, or reading the clock failed.
        -1 => -1,
        // Expired, or a value this code does not know: readline's reader
        // decides.
        _ => 0,
    }
}

/// Blocks until `stream` has input, readline's timeout is up, `pause`
/// milliseconds pass, or a signal arrives.
pub fn wait_for_input(stream: *mut libc::FILE, pause: Option<c_int>) -> Wait {
    let remaining = timeout_remaining();
    // The pause only counts if it ends before readline's timeout.
    let pause = pause.filter(|&p| remaining < 0 || p < remaining);
    match poll_input(stream, pause.unwrap_or(remaining)) {
        0 if pause.is_some() => Wait::Paused,
        n if n >= 0 => Wait::Ready,
        _ if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) => {
            Wait::Signal(unsafe { rl_pending_signal() })
        }
        _ => Wait::Error,
    }
}

/// What readline's reader returns when it cannot read a key.
pub fn read_error() -> c_int {
    // READERR in readline.h.
    const READ_ERROR: c_int = -2;
    if reading_command_key() {
        READ_ERROR
    } else {
        libc::EOF
    }
}

unsafe extern "C" {
    static mut asynchronous_notification: c_int;
    fn signal_is_trapped(signal: c_int) -> c_int;
    fn first_pending_trap() -> c_int;
}

/// Whether bash may write to the terminal while it handles `signal`, a value
/// from `Wait::Signal`: under `set -b`, a job notice from its `SIGCHLD`
/// handler (0: readline does not catch it), a trap on a signal readline
/// catches, or a trap waiting to run on any other signal, where bash's
/// signal hook runs it now (as in `read -e`). A `WINCH` trap runs while
/// readline handles the resize.
pub fn signal_may_print(signal: c_int) -> bool {
    // SAFETY: these read plain values bash and readline keep.
    unsafe {
        let trapped = if signal == 0 {
            asynchronous_notification != 0
        } else {
            signal_is_trapped(signal) != 0
        };
        let hook = rl_signal_event_hook;
        trapped || (hook.is_some() && first_pending_trap() > 0)
    }
}

/// Does what `rl_getc` does after a signal interrupts its read: readline
/// handles the signal (for `C-c`: echoes `^C` and passes the signal on to
/// bash), then the application's event hook runs (bash's jumps back to a new
/// prompt). Only needed when the signal interrupted inkline's wait rather than
/// readline's read.
pub fn handle_interrupted_wait() {
    unsafe {
        rl_check_signals();
        if let Some(hook) = rl_signal_event_hook {
            hook();
        }
    }
}

unsafe extern "C" {
    /// The signal readline's handler caught and readline has not handled
    /// yet; `rl_pending_signal` reads it.
    static mut _rl_caught_signal: c_int;
}

/// Takes from readline a `SIGINT` it caught and has not handled yet, and
/// returns whether there was one. Readline's key reader then does not
/// handle it under the command reading the key (handling it frees the
/// line's undo list, ends a search and passes the signal on to bash).
/// `release_interrupt` gives it back.
pub fn hold_interrupt() -> bool {
    // SAFETY: readline's signal handler writes this int at any time, so it
    // is read and written as volatile. A second SIGINT arriving between the
    // two is lost, as it would be to readline.
    unsafe {
        let caught = &raw mut _rl_caught_signal;
        let held = caught.read_volatile() == libc::SIGINT;
        if held {
            caught.write_volatile(0);
        }
        held
    }
}

/// Gives readline back the `SIGINT` `hold_interrupt` took, for
/// `handle_interrupted_wait` to handle.
pub fn release_interrupt() {
    // SAFETY: as in `hold_interrupt`.
    unsafe { (&raw mut _rl_caught_signal).write_volatile(libc::SIGINT) };
}

// ---- Reading a command ----

/// bash's completion function, `rl_attempted_completion_function`.
pub type CompletionFn = unsafe extern "C" fn(*const c_char, c_int, c_int) -> *mut *mut c_char;

unsafe extern "C" {
    static mut current_command_line_count: c_int;
    static mut executing: c_int;
    static mut extended_glob: c_int;
    static mut rl_attempted_completion_function: Option<CompletionFn>;
    static mut interrupt_state: c_int;
}

/// Whether bash has a `SIGINT` it has not acted on yet. Readline passes a
/// `C-c` on to bash, but bash only jumps to a new prompt from a read the
/// signal interrupted, so with more keys already waiting it acts on the
/// signal once the line is accepted.
pub fn interrupted() -> bool {
    unsafe { interrupt_state != 0 }
}

/// Whether readline is reading the first line of a command for bash. Not so on
/// a continuation line after `PS2`, where readline holds only part of the
/// command, nor for `read -e`, which runs while bash executes a command and
/// clears bash's completion function while it reads.
pub fn reading_command() -> bool {
    unsafe {
        let completion = rl_attempted_completion_function;
        current_command_line_count == 0 && executing == 0 && completion.is_some()
    }
}

/// Whether bash's `extglob` option is on.
pub fn extglob() -> bool {
    unsafe { extended_glob != 0 }
}

pub fn completion_function() -> Option<CompletionFn> {
    unsafe { rl_attempted_completion_function }
}

pub fn set_completion_function(f: Option<CompletionFn>) {
    unsafe { rl_attempted_completion_function = f }
}

pub fn call_completion(
    f: CompletionFn,
    text: *const c_char,
    start: c_int,
    end: c_int,
) -> *mut *mut c_char {
    unsafe { f(text, start, end) }
}

/// Overwrites byte `index` of the line, if the line has one there.
pub fn set_line_byte(index: usize, byte: u8) {
    unsafe {
        if !rl_line_buffer.is_null() && (index as c_int) < rl_end {
            *rl_line_buffer.add(index) = byte as c_char;
        }
    }
}

// ---- Commands ----

pub type CommandFn = unsafe extern "C" fn(c_int, c_int) -> c_int;

unsafe extern "C" {
    fn rl_add_defun(name: *const c_char, function: Option<CommandFn>, key: c_int) -> c_int;
    fn rl_insert_text(text: *const c_char) -> c_int;
    fn rl_forward_char(count: c_int, key: c_int) -> c_int;
    fn rl_forward_word(count: c_int, key: c_int) -> c_int;
    fn rl_end_of_line(count: c_int, key: c_int) -> c_int;
    static mut rl_last_func: Option<CommandFn>;
    fn rl_get_previous_history(count: c_int, key: c_int) -> c_int;
    fn rl_get_next_history(count: c_int, key: c_int) -> c_int;
    fn rl_history_search_backward(count: c_int, key: c_int) -> c_int;
    fn rl_history_search_forward(count: c_int, key: c_int) -> c_int;
}

/// The command readline ran for the previous key.
pub fn last_command() -> Option<CommandFn> {
    unsafe { rl_last_func }
}

/// readline's `previous-history`.
pub fn previous_history(count: c_int, key: c_int) -> c_int {
    unsafe { rl_get_previous_history(count, key) }
}

/// readline's `next-history`.
pub fn next_history(count: c_int, key: c_int) -> c_int {
    unsafe { rl_get_next_history(count, key) }
}

/// readline's `history-search-backward`.
pub fn history_search_backward(count: c_int, key: c_int) -> c_int {
    unsafe { rl_history_search_backward(count, key) }
}

/// readline's `history-search-forward`.
pub fn history_search_forward(count: c_int, key: c_int) -> c_int {
    unsafe { rl_history_search_forward(count, key) }
}

/// Marks the history search about to run as a continuation of the last one:
/// readline tells the two apart by checking whether `rl_last_func` is one of
/// its own search functions, which it is not once a call reaches it through
/// one of this crate's own commands.
pub fn continue_history_search() {
    unsafe { rl_last_func = Some(rl_history_search_backward) };
}

/// Registers a readline command under `name` without binding a key.  Readline
/// keeps the name pointer, so it must be `'static`.
pub fn add_command(name: &'static CStr, f: CommandFn) {
    unsafe { rl_add_defun(name.as_ptr(), Some(f), -1) };
}

pub fn insert_text(text: &str) {
    if let Ok(text) = CString::new(text) {
        unsafe { rl_insert_text(text.as_ptr()) };
    }
}

pub fn forward_char(count: c_int, key: c_int) -> c_int {
    unsafe { rl_forward_char(count, key) }
}

pub fn forward_word(count: c_int, key: c_int) -> c_int {
    unsafe { rl_forward_word(count, key) }
}

pub fn end_of_line(count: c_int, key: c_int) -> c_int {
    unsafe { rl_end_of_line(count, key) }
}

unsafe extern "C" {
    fn rl_newline(count: c_int, key: c_int) -> c_int;
    static mut rl_pending_input: c_int;
    static mut rl_instream: *mut libc::FILE;
    /// Keys readline read ahead and put back, as while it matched a longer
    /// key sequence.
    fn _rl_pushed_input_available() -> c_int;
}

/// readline's `accept-line`.
pub fn accept_line(count: c_int, key: c_int) -> c_int {
    unsafe { rl_newline(count, key) }
}

/// Whether readline is replaying a macro: the text bound to a key, or a
/// keyboard macro.
pub fn replaying_macro() -> bool {
    unsafe { rl_readline_state & RL_STATE_MACROINPUT != 0 }
}

/// Whether more input is already waiting: typed ahead, pasted without
/// bracketed paste, or coming from a macro.
pub fn input_waiting() -> bool {
    replaying_macro() || key_waiting()
}

/// Whether a key is already waiting: typed ahead, pasted without bracketed
/// paste, or read by readline and put back. Macro text does not count.
pub fn key_waiting() -> bool {
    unsafe {
        rl_pending_input != 0 || _rl_pushed_input_available() != 0 || poll_input(rl_instream, 0) > 0
    }
}

unsafe extern "C" {
    fn rl_read_key() -> c_int;
}

/// Reads one more key for the command that is running, as readline's own
/// commands do: from a macro or `rl_pending_input` first, else through the
/// key reader. A readline command to run with `call_command`, so that a
/// jump readline or bash makes while it waits stops there; it returns the
/// key, below 0 when no key can be read.
pub fn read_key_command() -> CommandFn {
    read_key
}

unsafe extern "C" fn read_key(_count: c_int, _key: c_int) -> c_int {
    // SAFETY: rl_read_key reads through the key reader and then handles
    // the signals readline caught. `RL_STATE_MOREINPUT` is only a flag,
    // set around the read as readline's own commands do.
    unsafe {
        rl_readline_state |= RL_STATE_MOREINPUT;
        let key = rl_read_key();
        rl_readline_state &= !RL_STATE_MOREINPUT;
        key
    }
}

/// `poll` on `stream`, or on standard input when it is null, for up to
/// `timeout` milliseconds (-1 waits for ever): above 0 when input is waiting,
/// 0 when the time ran out, -1 on an error.
fn poll_input(stream: *mut libc::FILE, timeout: c_int) -> c_int {
    let fd = if stream.is_null() {
        0
    } else {
        unsafe { libc::fileno(stream) }
    };
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut poll, 1, timeout) }
}

// ---- Pairing ----

unsafe extern "C" {
    static mut rl_explicit_arg: c_int;
    fn rl_insert(count: c_int, key: c_int) -> c_int;
    fn rl_rubout(count: c_int, key: c_int) -> c_int;
    fn rl_delete_text(start: c_int, end: c_int) -> c_int;
    fn rl_begin_undo_group() -> c_int;
    fn rl_end_undo_group() -> c_int;
}

/// readline's `self-insert`.
pub fn self_insert(count: c_int, key: c_int) -> c_int {
    unsafe { rl_insert(count, key) }
}

/// readline's `backward-delete-char`.
pub fn rubout(count: c_int, key: c_int) -> c_int {
    unsafe { rl_rubout(count, key) }
}

/// Deletes bytes `start..end` of the line.
pub fn delete_text(start: usize, end: usize) {
    unsafe { rl_delete_text(start as c_int, end as c_int) };
}

pub fn begin_undo_group() {
    unsafe { rl_begin_undo_group() };
}

pub fn end_undo_group() {
    unsafe { rl_end_undo_group() };
}

pub fn set_point(point: usize) {
    unsafe { rl_point = point as c_int }
}

/// Whether the user typed a count prefix for this command.
pub fn explicit_count() -> bool {
    unsafe { rl_explicit_arg != 0 }
}

unsafe extern "C" {
    fn rl_beg_of_line(count: c_int, key: c_int) -> c_int;
    fn rl_kill_line(count: c_int, key: c_int) -> c_int;
    fn rl_unix_line_discard(count: c_int, key: c_int) -> c_int;
    fn rl_kill_text(from: c_int, to: c_int) -> c_int;
}

/// readline's `beginning-of-line`.
pub fn beginning_of_line(count: c_int, key: c_int) -> c_int {
    unsafe { rl_beg_of_line(count, key) }
}

/// readline's `kill-line`.
pub fn kill_line(count: c_int, key: c_int) -> c_int {
    unsafe { rl_kill_line(count, key) }
}

/// readline's `unix-line-discard`.
pub fn unix_line_discard(count: c_int, key: c_int) -> c_int {
    unsafe { rl_unix_line_discard(count, key) }
}

/// Moves bytes `from..to` of the line to the kill ring, after the last kill
/// when `from < to` and before it otherwise. The cursor does not move.
pub fn kill_text(from: usize, to: usize) {
    unsafe { rl_kill_text(from as c_int, to as c_int) };
}

unsafe extern "C" {
    fn rl_insert_comment(count: c_int, key: c_int) -> c_int;
}

/// readline's `insert-comment`.
pub fn insert_comment(count: c_int, key: c_int) -> c_int {
    unsafe { rl_insert_comment(count, key) }
}

// ---- Lisp commands ----

// readline's `enum undo_code` values (readline.h, the same in readline 8.0
// and 8.3).
const UNDO_BEGIN: c_int = 2;
const UNDO_END: c_int = 3;

/// readline's `UNDO_LIST` entry (readline.h, the same in readline 8.0 and
/// 8.3). The list is newest first.
#[repr(C)]
struct UndoList {
    next: *mut UndoList,
    start: c_int,
    end: c_int,
    text: *mut c_char,
    /// An `enum undo_code`, which C stores as an int.
    what: c_int,
}

unsafe extern "C" {
    static mut rl_mark: c_int;
    static mut rl_undo_list: *mut UndoList;
    fn rl_do_undo() -> c_int;
    fn where_history() -> c_int;
    fn rl_ding() -> c_int;
    fn rl_crlf() -> c_int;
    fn rl_on_new_line() -> c_int;
}

/// The mark, in bytes.
pub fn mark() -> usize {
    // SAFETY: rl_mark is a plain int readline keeps.
    unsafe { rl_mark.max(0) as usize }
}

pub fn set_mark(mark: usize) {
    // SAFETY: rl_mark is a plain int readline keeps.
    unsafe { rl_mark = c_int::try_from(mark).unwrap_or(c_int::MAX) }
}

/// The bytes of the line being edited.
pub fn line_bytes() -> Vec<u8> {
    // SAFETY: readline keeps rl_end bytes of rl_line_buffer valid.
    unsafe {
        if rl_line_buffer.is_null() || rl_end <= 0 {
            return Vec::new();
        }
        std::slice::from_raw_parts(rl_line_buffer as *const u8, rl_end as usize).to_vec()
    }
}

/// The newest entry of the line's undo list, only to compare with a later
/// one.
pub fn undo_list_head() -> *const c_void {
    // SAFETY: reads a pointer readline keeps; nothing is dereferenced.
    unsafe { rl_undo_list.cast_const().cast() }
}

/// Removes the undo group just closed when nothing was changed inside it:
/// an `UNDO_END` right after an `UNDO_BEGIN`, with `before` the entry that
/// was newest when the group opened. `rl_do_undo` pops and frees the pair,
/// and also points history entries that held the pair at what is left.
/// Returns whether it removed them.
pub fn drop_empty_undo_group(before: *const c_void) -> bool {
    // SAFETY: rl_undo_list is NULL or a valid list readline keeps. Undoing
    // an empty group changes no text.
    unsafe {
        let end = rl_undo_list;
        if end.is_null() || (*end).what != UNDO_END {
            return false;
        }
        let begin = (*end).next;
        if begin.is_null()
            || (*begin).what != UNDO_BEGIN
            || (*begin).next.cast_const().cast() != before
        {
            return false;
        }
        rl_do_undo();
    }
    true
}

/// Undoes the newest change or group on the line.
pub fn do_undo() {
    // SAFETY: rl_do_undo works on readline's own list.
    unsafe { rl_do_undo() };
}

/// The history entry being edited, as `where_history` gives it.
pub fn history_position() -> c_int {
    // SAFETY: where_history only reads history's state.
    unsafe { where_history() }
}

/// Rings the bell, as readline's `bell-style` says.
pub fn ding() {
    // SAFETY: rl_ding only writes to the terminal.
    unsafe { rl_ding() };
}

/// Adds `f` to readline's commands as `name`, so `bind` can find it. False,
/// and nothing added, when readline already has a command of that name
/// (readline ignores case). The name lives for the life of the process:
/// readline keeps the pointer.
pub fn add_named_command(name: &str, f: CommandFn) -> bool {
    if named_command(name).is_some() {
        return false;
    }
    let Ok(name) = CString::new(name) else {
        return false;
    };
    let name: &'static CStr = Box::leak(name.into_boxed_c_str());
    add_command(name, f);
    true
}

unsafe extern "C" {
    static mut _rl_undo_group_level: c_int;
    fn rl_copy_region_to_kill(count: c_int, key: c_int) -> c_int;
    fn rl_undo_command(count: c_int, key: c_int) -> c_int;
    fn rl_revert_line(count: c_int, key: c_int) -> c_int;
    /// Named `vi-undo` from readline 8.1; exported, without a name, in 8.0.
    fn rl_vi_undo(count: c_int, key: c_int) -> c_int;
    /// In src/rlcall.c.
    fn inkline_call_command(f: CommandFn, count: c_int, key: c_int, jumped: *mut c_int) -> c_int;
    fn jump_to_top_level(value: c_int) -> !;
}

/// Tells readline one open undo group is no longer open, without adding an
/// `UNDO_END` to the line's undo list. For a group left open on a line the
/// command has moved away from, so readline's count of open groups (which
/// vi mode reads) stays right.
pub fn forget_undo_group() {
    // SAFETY: _rl_undo_group_level is a plain int readline keeps.
    unsafe {
        if _rl_undo_group_level > 0 {
            _rl_undo_group_level -= 1;
        }
    }
}

/// A jump back to a top level that stopped a readline command.
#[derive(Clone, Copy, Debug)]
pub enum Jumped {
    /// To readline's: its abort, as for C-g or a yank with an empty kill
    /// ring.
    Readline,
    /// To bash's, with this value: shell code the command ran failed, as
    /// `${x:?}` does. `jump_to_shell_top_level` makes the jump later.
    Shell(c_int),
}

/// Runs the readline command `f`. A jump back to readline's or bash's top
/// level stops at this call and gives `Jumped`, so it never skips the Rust
/// and Lisp frames above.
pub fn call_command(f: CommandFn, count: c_int, key: c_int) -> Result<c_int, Jumped> {
    let mut jumped: c_int = 0;
    let reading_flags = RL_STATE_READCMD | RL_STATE_MOREINPUT;
    // SAFETY: rl_readline_state is a plain set of flags readline keeps.
    let reading = unsafe { rl_readline_state } & reading_flags;
    // SAFETY: inkline_call_command calls `f` between saving and putting
    // back readline's and bash's jump points. A jump from inside `f` lands
    // in its own C frame, so it never crosses this function or its callers.
    let result = unsafe { inkline_call_command(f, count, key, &raw mut jumped) };
    // A key read that `f` left by a jump, as a timed-out `read -e -t`
    // makes, can leave readline's flags for reading a key set.
    // SAFETY: rl_readline_state is a plain set of flags readline keeps.
    unsafe { rl_readline_state = rl_readline_state & !reading_flags | reading };
    match jumped {
        0 => Ok(result),
        value if value > 0 => Err(Jumped::Shell(value)),
        _ => Err(Jumped::Readline),
    }
}

/// Jumps to bash's top level with `value`, from `Jumped::Shell`: the jump
/// `call_command` stopped. The caller's frames must hold nothing to drop.
pub fn jump_to_shell_top_level(value: c_int) -> ! {
    // SAFETY: bash's jump_to_top_level longjmps to bash's command loop, as
    // the shell code that jumped would have; the frames it skips hold
    // nothing to drop.
    unsafe { jump_to_top_level(value) }
}

/// Runs the readline command `f` as readline runs a key's command. A jump
/// back to readline's top level from `f` is not stopped: the caller's frames
/// must hold nothing to drop.
pub fn run_command(f: CommandFn, count: c_int, key: c_int) -> c_int {
    // SAFETY: `f` is a readline command; readline calls such commands with
    // any count and key.
    unsafe { f(count, key) }
}

/// readline's `copy-region-as-kill`, to run with `call_command`.
pub fn copy_region_command() -> CommandFn {
    rl_copy_region_to_kill
}

/// What one of readline's undo commands takes back. They work on whole
/// undo groups.
pub enum Undo {
    /// As many steps as the count (`undo`, `vi-undo`); none when the count
    /// is 0 or less.
    Steps,
    /// Every step of the line (`revert-line`), whatever the count.
    All,
}

/// Which undo command `f` is, if it is one.
pub fn undo_command(f: CommandFn) -> Option<Undo> {
    if std::ptr::fn_addr_eq(f, rl_undo_command as CommandFn)
        || std::ptr::fn_addr_eq(f, rl_vi_undo as CommandFn)
    {
        Some(Undo::Steps)
    } else if std::ptr::fn_addr_eq(f, rl_revert_line as CommandFn) {
        Some(Undo::All)
    } else {
        None
    }
}

/// Sets whether the running command counts as given a count by the user,
/// and returns the old setting.
pub fn replace_explicit_count(explicit: bool) -> bool {
    // SAFETY: rl_explicit_arg is a plain int readline keeps.
    unsafe {
        let old = rl_explicit_arg;
        rl_explicit_arg = c_int::from(explicit);
        old != 0
    }
}

/// Tells readline the cursor is at the start of a new row, below a
/// prompt and line that are no longer to be drawn over, so its next
/// redisplay draws them in full there.
pub fn on_new_line() {
    // SAFETY: rl_on_new_line only resets readline's idea of what is on
    // screen.
    unsafe { rl_on_new_line() };
}

unsafe extern "C" {
    /// The row, counted from the prompt's first, of the line's last row on
    /// screen.
    static mut _rl_vis_botlin: c_int;
    fn _rl_move_vert(to: c_int);
}

/// Moves to the start of a new row below the whole line, as readline does
/// before listing completions, and tells readline the prompt and line are
/// no longer on screen, so its next redisplay draws them in full there.
pub fn new_line_for_message() {
    // SAFETY: these only write to readline's output stream and reset its
    // idea of what is on screen; _rl_vis_botlin is a plain int readline
    // keeps, 0 once rl_on_new_line has run.
    unsafe {
        _rl_move_vert(_rl_vis_botlin);
        rl_crlf();
        rl_on_new_line();
    }
}

// ---- The shell ----

unsafe extern "C" {
    static mut interactive_shell: c_int;
    /// Changed by `set -o emacs`/`vi` and `set +o emacs`/`vi`.
    static mut no_line_editing: c_int;
}

/// Whether bash is interactive with line editing on: the shells inkline sets
/// up readline and reads `init.el` for.
pub fn line_editing_shell() -> bool {
    unsafe { interactive_shell != 0 && no_line_editing == 0 }
}

// ---- Keymaps ----

// From readline's keymaps.h; the same in readline 8.0 and 8.3.
const KEYMAP_SIZE: usize = 257;
const ANYOTHERKEY: usize = KEYMAP_SIZE - 1;
const ISFUNC: c_char = 0;
const ISKMAP: c_char = 1;
const ISMACR: c_char = 2;

/// readline's `KEYMAP_ENTRY`. `function` is a command for ISFUNC, a keymap
/// for ISKMAP and macro text for ISMACR.
#[repr(C)]
#[derive(Clone, Copy)]
struct KeymapEntry {
    kind: c_char,
    function: *mut c_void,
}

/// readline's `FUNMAP`: a command and its name.
#[repr(C)]
struct FunmapEntry {
    name: *const c_char,
    function: Option<CommandFn>,
}

unsafe extern "C" {
    static mut emacs_standard_keymap: [KeymapEntry; KEYMAP_SIZE];
    /// A NULL-terminated array, or NULL before readline fills it.
    static mut funmap: *mut *mut FunmapEntry;
    fn rl_generic_bind(
        kind: c_int,
        keyseq: *const c_char,
        data: *mut c_char,
        map: *mut KeymapEntry,
    ) -> c_int;
    fn rl_named_function(name: *const c_char) -> Option<CommandFn>;
}

/// What a key sequence runs in the emacs keymap.
#[derive(Clone, Debug)]
pub enum Binding {
    Unbound,
    Command(CommandFn),
    /// Text readline types in for the key.
    Macro(Vec<u8>),
}

impl PartialEq for Binding {
    fn eq(&self, other: &Binding) -> bool {
        match (self, other) {
            (Binding::Unbound, Binding::Unbound) => true,
            (Binding::Command(f), Binding::Command(g)) => std::ptr::fn_addr_eq(*f, *g),
            (Binding::Macro(a), Binding::Macro(b)) => a == b,
            _ => false,
        }
    }
}

/// A sequence's binding, and whether the sequence is also the start of
/// longer ones (its binding is then the prefix keymap's "any other key").
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub prefix: bool,
    pub binding: Binding,
}

/// The keymap holding `seq`'s last key, and that key. None when a key
/// before the last is not a prefix.
fn keymap_of(seq: &[u8]) -> Option<(*mut KeymapEntry, u8)> {
    let (&last, before) = seq.split_last()?;
    let mut map = (&raw mut emacs_standard_keymap).cast::<KeymapEntry>();
    for &b in before {
        // SAFETY: `map` points to a readline keymap of KEYMAP_SIZE entries,
        // and a byte is below KEYMAP_SIZE.
        let entry = unsafe { *map.add(usize::from(b)) };
        if entry.kind != ISKMAP || entry.function.is_null() {
            return None;
        }
        map = entry.function.cast();
    }
    Some((map, last))
}

/// The prefix keymap `entry` holds, if it holds one.
///
/// # Safety
///
/// `entry` must point to an entry of a readline keymap.
unsafe fn sub_keymap(entry: *const KeymapEntry) -> Option<*mut KeymapEntry> {
    // SAFETY: the caller's promise; an ISKMAP entry points to a keymap of
    // KEYMAP_SIZE entries or is NULL.
    unsafe {
        ((*entry).kind == ISKMAP && !(*entry).function.is_null()).then(|| (*entry).function.cast())
    }
}

/// The entry holding `seq`'s binding, and whether `seq` also starts longer
/// sequences (the entry is then the prefix keymap's "any other key"). None
/// when a key before the last is not a prefix.
fn slot_of(seq: &[u8]) -> Option<(*mut KeymapEntry, bool)> {
    let (map, last) = keymap_of(seq)?;
    // SAFETY: `map` points to a readline keymap of KEYMAP_SIZE entries, a
    // byte is below KEYMAP_SIZE, and a prefix keymap has KEYMAP_SIZE
    // entries too.
    unsafe {
        let entry = map.add(usize::from(last));
        match sub_keymap(entry) {
            Some(sub) => Some((sub.add(ANYOTHERKEY), true)),
            None => Some((entry, false)),
        }
    }
}

/// The keymap address and key readline gives in `rl_executing_keymap` and
/// `rl_executing_key` while `seq`'s command runs. For a sequence that also
/// starts longer ones, readline runs the prefix keymap's "any other key" as
/// if it were bound to the last key in that prefix keymap, so it is that
/// keymap and the last key. None when a key before the last is not a
/// prefix.
pub fn entry_position(seq: &[u8]) -> Option<(usize, u8)> {
    let (map, last) = keymap_of(seq)?;
    // SAFETY: `map` points to a readline keymap of KEYMAP_SIZE entries, and
    // a byte is below KEYMAP_SIZE.
    let sub = unsafe { sub_keymap(map.add(usize::from(last))) };
    Some((sub.unwrap_or(map) as usize, last))
}

unsafe extern "C" {
    static mut rl_executing_keymap: *mut KeymapEntry;
    static mut rl_executing_key: c_int;
    /// While a command runs: the keymap its key is in, or for a prefix
    /// keymap's "any other key", the keymap the prefix is in.
    static mut _rl_dispatching_keymap: *mut KeymapEntry;
    fn rl_push_macro_input(text: *mut c_char);
}

/// The key whose command readline is running.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RunningKey {
    /// As `entry_position` gives it.
    pub position: (usize, u8),
    /// Whether it runs as a prefix keymap's "any other key".
    pub prefix: bool,
}

/// The key whose command readline is running, as readline tells it. None
/// when readline has run no key.
pub fn running_key() -> Option<RunningKey> {
    // SAFETY: these are plain values readline sets before it runs a key's
    // command; they are only read here.
    unsafe {
        let map = rl_executing_keymap;
        let key = u8::try_from(rl_executing_key).ok()?;
        if map.is_null() {
            return None;
        }
        Some(RunningKey {
            position: (map as usize, key),
            prefix: _rl_dispatching_keymap != map,
        })
    }
}

/// The key readline ran a command for last.
pub fn executing_key() -> c_int {
    // SAFETY: a plain value readline sets before it runs a key's command.
    unsafe { rl_executing_key }
}

/// Macro text in memory from `malloc`, which readline frees once it is done
/// with it: for `push_macro_input`, or a keymap entry. Neither `Copy` nor
/// `Clone`, so it is handed over at most once; one dropped without being
/// handed over is never freed.
pub struct MacroText(*mut c_char);

/// A copy of `text` to hand to readline. None when `text` holds a NUL byte
/// or memory runs out.
pub fn macro_text(text: &[u8]) -> Option<MacroText> {
    let text = CString::new(text).ok()?;
    // SAFETY: `text` is a NUL-terminated string; strdup's copy comes from
    // malloc.
    let copy = unsafe { libc::strdup(text.as_ptr()) };
    (!copy.is_null()).then_some(MacroText(copy))
}

/// Has readline read `text` as typed keys next, as for a key bound to macro
/// text. readline frees `text` once it has read it all.
pub fn push_macro_input(text: MacroText) {
    // SAFETY: `text` comes from malloc, and readline takes it over.
    unsafe { rl_push_macro_input(text.0) };
}

fn binding_of(entry: KeymapEntry) -> Binding {
    match entry.kind {
        ISFUNC if entry.function.is_null() => Binding::Unbound,
        // SAFETY: a non-NULL ISFUNC entry holds a readline command.
        ISFUNC => Binding::Command(unsafe {
            std::mem::transmute::<*mut c_void, CommandFn>(entry.function)
        }),
        // SAFETY: a non-NULL ISMACR entry holds a NUL-terminated string.
        ISMACR => unsafe { c_str(entry.function.cast()) }.map_or(Binding::Unbound, |text| {
            Binding::Macro(text.to_bytes().to_vec())
        }),
        _ => Binding::Unbound,
    }
}

/// What `seq` is bound to, or None when it cannot be reached.
pub fn lookup(seq: &[u8]) -> Option<Found> {
    let (slot, prefix) = slot_of(seq)?;
    // SAFETY: `slot_of` returns an entry of a readline keymap.
    let binding = binding_of(unsafe { *slot });
    Some(Found { prefix, binding })
}

/// Binds the sequence (in readline's text form) to `f`. For a sequence that
/// starts longer ones, readline binds the prefix keymap's "any other key".
pub fn bind_command(seq_text: &str, f: CommandFn) -> bool {
    let Ok(text) = CString::new(seq_text) else {
        return false;
    };
    let map = (&raw mut emacs_standard_keymap).cast::<KeymapEntry>();
    // SAFETY: `text` is a NUL-terminated string, `map` is readline's emacs
    // keymap, and readline keeps an ISFUNC entry's data as a command.
    unsafe { rl_generic_bind(c_int::from(ISFUNC), text.as_ptr(), f as *mut c_char, map) == 0 }
}

/// Puts `saved` back as `seq`'s binding, writing the keymap entry directly:
/// readline's binding calls fill an empty "any other key" with a function
/// that does nothing. The entry's current value must be a command, not macro
/// text: it is overwritten, not freed.
pub fn restore(seq: &[u8], saved: &Found) {
    let Some((slot, _)) = slot_of(seq) else {
        return;
    };
    let entry = match &saved.binding {
        Binding::Unbound => KeymapEntry {
            kind: ISFUNC,
            function: std::ptr::null_mut(),
        },
        Binding::Command(f) => KeymapEntry {
            kind: ISFUNC,
            function: *f as *mut c_void,
        },
        Binding::Macro(text) => {
            let Some(MacroText(copy)) = macro_text(text) else {
                return;
            };
            KeymapEntry {
                kind: ISMACR,
                function: copy.cast(),
            }
        }
    };
    // SAFETY: `slot_of` returns an entry of a readline keymap.
    unsafe { *slot = entry };
}

/// The readline or inkline command called `name`. readline ignores case.
pub fn named_command(name: &str) -> Option<CommandFn> {
    let name = CString::new(name).ok()?;
    // SAFETY: `name` is a NUL-terminated string.
    unsafe { rl_named_function(name.as_ptr()) }
}

/// The name readline knows `f` by.
pub fn command_name(f: CommandFn) -> Option<String> {
    // SAFETY: `funmap` is NULL or a NULL-terminated array of valid entries,
    // each with a NUL-terminated name that readline keeps.
    unsafe {
        let mut entry = funmap;
        if entry.is_null() {
            return None;
        }
        while !(*entry).is_null() {
            let e = &**entry;
            if e.function.is_some_and(|g| std::ptr::fn_addr_eq(g, f)) {
                return c_str(e.name).map(|n| n.to_string_lossy().into_owned());
            }
            entry = entry.add(1);
        }
    }
    None
}
