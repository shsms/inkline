//! Declarations for the bash and readline symbols inkline uses, with safe
//! wrappers. All of inkline's unsafe code is in this file.
//!
//! The symbols are left undefined in the library and resolved against the bash
//! binary when `enable -f` loads it.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

pub const EXECUTION_SUCCESS: c_int = 0;
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
    short_doc: c"inkline [on|off|status]".as_ptr(),
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

pub fn horizontal_scroll_mode() -> bool {
    unsafe {
        let value = rl_variable_value(c"horizontal-scroll-mode".as_ptr());
        !value.is_null() && CStr::from_ptr(value).to_bytes() == b"on"
    }
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

/// Writes to readline's output stream, so the bytes stay in order with
/// what readline writes.
pub fn write_out(bytes: &[u8]) {
    unsafe {
        let out = rl_outstream;
        if out.is_null() {
            return;
        }
        libc::fwrite(bytes.as_ptr().cast(), 1, bytes.len(), out);
        libc::fflush(out);
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
