//! Moving through the completion menu: each move writes the row into the
//! line, and the menu keeps the rows it had at the first move until another
//! key, `C-g` or Enter ends the moving.

use std::ffi::c_void;

use super::{STATE, menu_next, menu_previous, menu_take};
use crate::ffi;
use crate::menu::{Item, Menu};

/// The state kept while moving through the menu, from the first move until
/// another key, `C-g` or Enter ends it.
pub(super) struct Moving {
    /// The menu as it was at the first move; its pick is the row written.
    pub(super) menu: Menu,
    /// The line and cursor the last move left.
    written: (String, usize),
    /// Readline's newest undo entry once the last move's group closed.
    undo_head: *const c_void,
}

impl Moving {
    /// The moving state for `menu`, before the first move.
    fn start(menu: Menu) -> Moving {
        Moving {
            menu,
            written: (String::new(), 0),
            undo_head: std::ptr::null(),
        }
    }

    /// Whether the moving state is live: inkline is on (`on`), readline's
    /// last command was a move, the last draw in plain editing showed the
    /// menu, and the line, cursor and undo list are as the last move left
    /// them.
    fn live(&self, on: bool) -> bool {
        on && is_move(ffi::last_command())
            && self.menu.shown
            && ffi::undo_list_head() == self.undo_head
            && ffi::point() == self.written.1
            && ffi::line().is_some_and(|l| l == self.written.0)
    }
}

/// Whether `f` is one of the commands that move through the menu.
pub(super) fn is_move(f: Option<ffi::CommandFn>) -> bool {
    let moves: [ffi::CommandFn; 3] = [menu_next, menu_previous, menu_take];
    f.is_some_and(|f| moves.iter().any(|&m| std::ptr::fn_addr_eq(m, f)))
}

/// The moving state, taken out of the hook state, when it is live; one that
/// is not live is dropped.
fn take_live() -> Option<Moving> {
    STATE.with_borrow_mut(|s| {
        let moving = s.moving.take()?;
        moving.live(s.enabled && !s.unloaded).then_some(moving)
    })
}

/// Whether the moving state is live, left where it is.
fn is_live() -> bool {
    STATE.with_borrow(|s| {
        s.moving
            .as_ref()
            .is_some_and(|m| m.live(s.enabled && !s.unloaded))
    })
}

/// The moving state when it is live (see `take_live`), with the row it
/// wrote undone for the next move.
fn resume() -> Option<Moving> {
    take_live().inspect(|_| ffi::do_undo())
}

/// Writes `item` into the line as one undo group: it replaces its part of
/// the line, and the cursor goes to its end.
fn write(item: &Item) {
    ffi::begin_undo_group();
    ffi::delete_text(item.start, item.end);
    ffi::set_point(item.start);
    ffi::insert_text(&item.text);
    ffi::end_undo_group();
}

/// Moves the pick `count` rows, down or up, and writes the row into the
/// line: from the moving state when it is live (the row it wrote is undone
/// first), else from the menu on screen for the line and cursor as they
/// are. A count of 0 changes nothing. Whether there were rows to move
/// through.
pub(super) fn step(down: bool, count: i64) -> bool {
    if count == 0 {
        return is_live() || super::showing_menu();
    }
    let Some(moving) = resume().or_else(|| super::shown_menu().map(Moving::start)) else {
        return false;
    };
    advance(moving, down, count)
}

/// Moves the pick of `moving`'s menu `count` rows, down or up, writes the
/// row into the line and keeps the moving state. Whether there was a row.
fn advance(mut moving: Moving, down: bool, count: i64) -> bool {
    moving.menu.step(down, count);
    let Some(item) = moving.menu.picked_item().cloned() else {
        return false;
    };
    write(&item);
    moving.written = (ffi::line().unwrap_or_default(), ffi::point());
    moving.undo_head = ffi::undo_list_head();
    STATE.with_borrow_mut(|s| {
        s.end_vertical_run();
        s.moving = Some(moving);
    });
    true
}

/// Tab (`down`) or Shift-Tab: moves while moving. Otherwise, with a menu on
/// screen, writes its only row when no more items are coming, or moves.
/// Whether it did either.
pub(super) fn tab(down: bool) -> bool {
    if let Some(moving) = resume() {
        return advance(moving, down, 1);
    }
    let Some(menu) = super::shown_menu() else {
        return false;
    };
    if let [only] = menu.items.as_slice()
        && !menu.mode_waiting
        && !menu.bash_waiting
    {
        write(only);
        return true;
    }
    advance(Moving::start(menu), down, 1)
}

/// `C-g` while moving: takes back the row written, puts back the cursor as
/// typed, and hides the menu on the typed text. Whether the moving state
/// was live.
pub(super) fn cancel() -> bool {
    let Some(moving) = take_live() else {
        return false;
    };
    ffi::do_undo();
    ffi::set_point(moving.menu.point);
    STATE.with_borrow_mut(|s| {
        s.hidden_on = Some(moving.menu.line);
        s.menu = None;
    });
    true
}

/// Enter while moving: keeps the row and ends the moving. Whether the
/// moving state was live.
pub(super) fn stop() -> bool {
    take_live().is_some()
}

/// For a draw in plain editing: the menu of the moving state when it is
/// live, to draw instead of gathering one. It counts as shown only once the
/// draw puts it on screen; a moving state that is not live is dropped.
pub(super) fn menu_to_draw() -> Option<Menu> {
    let mut moving = take_live()?;
    let menu = moving.menu.clone();
    moving.menu.shown = false;
    STATE.with_borrow_mut(|s| s.moving = Some(moving));
    Some(menu)
}
