//! The region between the mark and the cursor, active as in Emacs's
//! transient mark mode: `C-SPC` makes it active, `C-w`, `M-w`, DEL and
//! `C-d` act on it, and any other change to the line's text ends it.
//! readline's own active mark is not used: readline ends it after every
//! command and draws it itself. While inkline is off nothing would draw
//! or end the region, so it is never active then: turning inkline off
//! ends it, and `C-SPC` and `C-x C-x` run readline's own commands.

use std::cell::{Cell, RefCell};
use std::ffi::c_int;
use std::ops::Range;

use super::guard;
use crate::ffi;

thread_local! {
    /// While the region is active: the line's text when it became active.
    /// A draw that finds other text ends it (`end_if_changed`).
    static ACTIVE: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    /// The mark when an incremental search started with the region active,
    /// put back once the search has ended (see `update`).
    static MARK_BEFORE_SEARCH: Cell<Option<usize>> = const { Cell::new(None) };
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
    let _ = MARK_BEFORE_SEARCH.try_with(|m| m.set(None));
    ACTIVE
        .try_with(|a| a.try_borrow_mut().is_ok_and(|mut a| a.take().is_some()))
        .unwrap_or(false)
}

/// Brings the region up to date before a draw: any change to the line's
/// text but the region commands' own ends it. An incremental search (`C-r`,
/// `C-s`) keeps the mark where it was, as Emacs's isearch does, though
/// readline moves the mark to the end of each match it finds. Readline
/// redraws when a search starts, before its first key, and once more when
/// the search has ended, before the key that ended it runs its command.
/// While the search runs the line shows the lines it finds, so the text is
/// checked once it has ended: a search that ends on the text the region
/// became active on keeps the region, with the mark back where it was.
pub(super) fn update() {
    if ffi::searching_incrementally() {
        if active() && MARK_BEFORE_SEARCH.get().is_none() {
            MARK_BEFORE_SEARCH.set(Some(ffi::mark()));
        }
        return;
    }
    end_if_changed();
    // Ending the region forgets the mark.
    if let Some(mark) = MARK_BEFORE_SEARCH.take() {
        ffi::set_mark(mark.min(ffi::line_bytes().len()));
    }
}

/// Ends the region when the line's text is not the text it became active
/// on: any edit but the region commands' own ends it.
fn end_if_changed() {
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

/// `C-w`: kills the region onto the kill ring, with the cursor at its
/// start, and ends it; a count is ignored, as Emacs's `kill-region` does.
/// With no active region, runs what the key had before inkline bound it
/// (bash's `unix-word-rubout`), with the count.
pub(super) extern "C" fn kill_region_or_word(count: c_int, key: c_int) -> c_int {
    use crate::lisp::keys::{self, Fallback};
    let saved = guard(
        || {
            let Some(r) = range() else {
                return Some(keys::saved_binding_of(kill_region_or_word));
            };
            end();
            if !r.is_empty() {
                ffi::kill_text(r.start, r.end);
            }
            ffi::set_point(r.start);
            ffi::set_mark(r.start);
            None
        },
        || Some(Fallback::Nothing),
    );
    match saved {
        Some(saved) => super::run_fallback(saved, count, key),
        None => 0,
    }
}

/// `M-w`: copies the region to the kill ring, leaves the line, and ends
/// the region. With no active region, rings the bell.
pub(super) extern "C" fn kill_ring_save(_count: c_int, key: c_int) -> c_int {
    let copied = guard(
        || {
            let Some(r) = range() else {
                return false;
            };
            end();
            if !r.is_empty() {
                // readline copies between the cursor and the mark; the mark
                // may have been past the end.
                let point = ffi::point();
                ffi::set_mark(if point == r.start { r.end } else { r.start });
                ffi::run_command(ffi::copy_region_command(), 1, key);
            }
            true
        },
        || false,
    );
    if !copied {
        ffi::ding();
    }
    0
}

/// `C-d`: deletes the region in use, not onto the kill ring (see
/// `delete_in_use`). Otherwise, or with a count other than 1, runs what
/// the key had before inkline bound it (`delete-char`), with the count.
pub(super) extern "C" fn delete_char_or_region(count: c_int, key: c_int) -> c_int {
    use crate::lisp::keys::{self, Fallback};
    let saved = guard(
        || (!delete_in_use(count)).then(|| keys::saved_binding_of(delete_char_or_region)),
        || Some(Fallback::Nothing),
    );
    match saved {
        Some(saved) => super::run_fallback(saved, count, key),
        None => 0,
    }
}

/// For DEL and `C-d` run with `count`: deletes the region in use
/// (`in_use`), not onto the kill ring, puts the cursor and the mark at its
/// start, and ends the region, as one undo step. Returns whether it
/// deleted the region. As in Emacs, an empty active region, or a count
/// other than 1, is left to the key's own command, which deletes
/// characters.
pub(super) fn delete_in_use(count: c_int) -> bool {
    if count != 1 {
        return false;
    }
    let Some(r) = in_use() else {
        return false;
    };
    end();
    ffi::delete_text(r.start, r.end);
    ffi::set_point(r.start);
    ffi::set_mark(r.start);
    true
}
