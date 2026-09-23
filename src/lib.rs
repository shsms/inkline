//! inkline: syntax highlighting, history suggestions and bracket pairing
//! for bash's readline, loaded into bash with `enable -f`.

pub mod colors;
pub mod lexer;

// These reference symbols that only exist inside bash, so they are left out
// of the unit-test binary, which runs without bash.
#[cfg(not(test))]
mod ffi;
#[cfg(not(test))]
mod hooks;
