//! Shared helpers for the end-to-end tests.
#![allow(
    dead_code,
    reason = "each test binary uses a different subset of the helpers"
)]

use std::path::PathBuf;
use std::process::Command;

/// The bash to test: `$INKLINE_TEST_BASH`, or `bash` from `PATH`.
pub fn bash_path() -> PathBuf {
    std::env::var_os("INKLINE_TEST_BASH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bash"))
}

/// The library `cargo test` built for this run, next to the test binary in
/// `target/<profile>/deps`.
pub fn so_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().join("libinkline.so")
}

/// A non-interactive bash, for tests that need no terminal.
pub fn bash_command() -> Command {
    let mut cmd = Command::new(bash_path());
    cmd.env("INPUTRC", "/dev/null");
    cmd
}
