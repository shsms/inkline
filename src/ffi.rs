//! Declarations for the bash and readline symbols inkline uses, with safe
//! wrappers. All of inkline's unsafe code is in this file.
//!
//! The symbols are left undefined in the library and resolved against the bash
//! binary when `enable -f` loads it.

use std::ffi::{CStr, c_char, c_int};

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
