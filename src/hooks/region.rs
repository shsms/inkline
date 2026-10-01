//! The region between the mark and the cursor, active as in Emacs's
//! transient mark mode: `C-SPC` makes it active, `C-w`, `M-w`, DEL and
//! `C-d` act on it, and any other change to the line's text ends it.
//! readline's own active mark is not used: readline ends it after every
//! command and draws it itself. While inkline is off nothing would draw
//! or end the region, so it is never active then: turning inkline off
//! ends it, and `C-SPC` and `C-x C-x` run readline's own commands.

use std::cell::RefCell;
use std::ffi::c_int;
use std::ops::Range;

use super::guard;
use crate::ffi;

thread_local! {
    /// While the region is active: the line's text when it became active.
    /// A draw that finds other text ends it (`end_if_changed`).
    static ACTIVE: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

/// Whether the region is active.
pub(super) fn active() -> bool {
    ACTIVE.with_borrow(Option::is_some)
}

fn activate() {
    ACTIVE.with_borrow_mut(|a| *a = Some(ffi::line_bytes()));
}

/// Ends the region; returns whether it was active. Never panics, so
/// `disable` can call it whatever state inkline is in.
pub(super) fn end() -> bool {
    ACTIVE
        .try_with(|a| a.try_borrow_mut().is_ok_and(|mut a| a.take().is_some()))
        .unwrap_or(false)
}

/// Ends the region when the line's text is not the text it became active
/// on: any edit but the region commands' own ends it.
pub(super) fn end_if_changed() {
    let changed = ACTIVE.with_borrow(|a| a.as_ref().is_some_and(|text| *text != ffi::line_bytes()));
    if changed {
        end();
    }
}

/// The bytes between the mark and the cursor while the region is active.
/// A mark past the end of the line counts as at the end.
pub(super) fn range() -> Option<Range<usize>> {
    if !active() {
        return None;
    }
    let len = ffi::line_bytes().len();
    let (point, mark) = (ffi::point().min(len), ffi::mark().min(len));
    Some(point.min(mark)..point.max(mark))
}

/// The region to act on: active and not empty, as Emacs's `use-region-p`.
#[expect(dead_code, reason = "used by the region commands in the next commit")]
pub(super) fn in_use() -> Option<Range<usize>> {
    range().filter(|r| !r.is_empty())
}

/// `C-SPC`: sets the mark at the cursor and makes the region active; with
/// the region active and the mark already at the cursor, ends it. While
/// inkline is off, runs what the key had before inkline bound it
/// (readline's `set-mark`), with the count.
pub(super) extern "C" fn set_mark_command(count: c_int, key: c_int) -> c_int {
    use crate::lisp::keys;
    let saved = guard(
        || {
            if !super::is_on() {
                return Some(keys::saved_binding_of(set_mark_command));
            }
            if active() && ffi::mark() == ffi::point() {
                end();
            } else {
                ffi::set_mark(ffi::point());
                activate();
            }
            None
        },
        || None,
    );
    match saved {
        Some(saved) => super::run_fallback(saved, count, key),
        None => 0,
    }
}

/// `C-x C-x`: swaps the cursor and the mark and makes the region active. A
/// mark past the end of the line counts as at the end. While inkline is
/// off, runs what the key had before inkline bound it (readline's
/// `exchange-point-and-mark`), with the count.
pub(super) extern "C" fn swap_point_and_mark(count: c_int, key: c_int) -> c_int {
    use crate::lisp::keys;
    let saved = guard(
        || {
            if !super::is_on() {
                return Some(keys::saved_binding_of(swap_point_and_mark));
            }
            let len = ffi::line_bytes().len();
            let (point, mark) = (ffi::point(), ffi::mark().min(len));
            ffi::set_point(mark);
            ffi::set_mark(point);
            if !active() {
                activate();
            }
            None
        },
        || None,
    );
    match saved {
        Some(saved) => super::run_fallback(saved, count, key),
        None => 0,
    }
}
