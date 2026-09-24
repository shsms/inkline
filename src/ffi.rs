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
    short_doc: c"inkline [on|off|status|load FILE|eval EXPR]".as_ptr(),
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
    if unsafe { rl_readline_state } & RL_STATE_READCMD != 0 {
        READ_ERROR
    } else {
        libc::EOF
    }
}

unsafe extern "C" {
    static mut asynchronous_notification: c_int;
    fn signal_is_trapped(signal: c_int) -> c_int;
}

/// Whether bash may write to the terminal while it handles `signal`, a value
/// from `Wait::Signal`: under `set -b`, a job notice from its `SIGCHLD`
/// handler (0: readline does not catch it), or a trap on a signal readline
/// catches. A `WINCH` trap runs while readline handles the resize.
pub fn signal_may_print(signal: c_int) -> bool {
    unsafe {
        if signal == 0 {
            asynchronous_notification != 0
        } else {
            signal_is_trapped(signal) != 0
        }
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
    unsafe {
        if rl_pending_input != 0 || replaying_macro() {
            return true;
        }
        poll_input(rl_instream, 0) > 0
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
