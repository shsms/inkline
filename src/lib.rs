//! inkline: syntax highlighting, history suggestions and bracket pairing for
//! bash's readline, loaded into bash with `enable -f`.

pub mod colors;
pub mod commands;
pub mod indent;
pub mod lexer;
pub mod lines;
pub mod pairs;
pub mod render;
pub mod suggest;
pub mod syntax;

// These reference symbols that only exist inside bash, so they are left out of
// the unit-test binary, which runs without bash.
#[cfg(not(test))]
mod ffi;
#[cfg(not(test))]
mod hooks;
