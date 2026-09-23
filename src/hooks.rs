//! Connects inkline to bash and readline: the `inkline` builtin, the hooks
//! readline calls, and the readline commands inkline adds.

use std::cell::RefCell;
use std::ffi::c_int;

use crate::ffi;

struct State {
    enabled: bool,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State { enabled: false }) };
}

pub fn load() {
    enable();
}

pub fn unload() {
    disable();
}

pub fn builtin(args: &[String]) -> c_int {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] | ["status"] => {
            let on = STATE.with_borrow(|s| s.enabled);
            println!("inkline: {}", if on { "on" } else { "off" });
            ffi::EXECUTION_SUCCESS
        }
        ["on"] => {
            enable();
            ffi::EXECUTION_SUCCESS
        }
        ["off"] => {
            disable();
            ffi::EXECUTION_SUCCESS
        }
        _ => {
            eprintln!("inkline: usage: inkline [on|off|status]");
            ffi::EX_USAGE
        }
    }
}

fn enable() {
    STATE.with_borrow_mut(|s| s.enabled = true);
}

fn disable() {
    STATE.with_borrow_mut(|s| s.enabled = false);
}
