//! The rows of the completion menu under the line.

use super::{expand_tabs, fit};
use crate::colors::Colors;
use crate::menu::{Item, window};

/// A menu to draw: its items, the picked one, and the most rows it takes.
pub struct MenuView<'a> {
    pub items: &'a [Item],
    pub picked: Option<usize>,
    pub max_rows: usize,
}

/// The rows to draw for `view` on a screen `cols` wide with `room` rows free
/// under the line, each with its colours, in order. Empty when nothing fits.
pub fn rows(view: &MenuView, room: usize, cols: usize, colors: &Colors) -> Vec<String> {
    let shown = window(view.items.len(), view.picked, view.max_rows.min(room));
    // The last column stays free, so the terminal never wraps.
    let width = cols.saturating_sub(1);
    // Every item row starts with its letter and two spaces.
    if width < 3 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for i in shown.first..shown.first + shown.count {
        let item = &view.items[i];
        let letter = item.source.letter().to_string();
        let text = item_text(&item.text, width.saturating_sub(3));
        rows.push(if view.picked == Some(i) {
            paint(colors.menu_selected(), &format!("{letter}  {text}"))
        } else {
            format!(
                "{}  {}",
                paint(colors.menu_source(), &letter),
                paint(colors.menu(), &text)
            )
        });
    }
    if shown.more > 0 {
        let note = format!("   … {} more", shown.more);
        rows.push(paint(colors.menu_source(), fit(&note, width)));
    }
    rows
}

/// `text` in the SGR `codes`, or as it is when there are none.
fn paint(codes: &str, text: &str) -> String {
    if codes.is_empty() || text.is_empty() {
        text.to_owned()
    } else {
        format!("\x1b[{codes}m{text}\x1b[0m")
    }
}

/// What a row shows of an item: its first line with tabs expanded (it
/// starts three columns in), ` …` after it when the item has more lines,
/// cut with `…` to `room` columns. A control character stops it, the same
/// as everywhere else a row is cut, so nothing past it reaches the
/// terminal.
fn item_text(text: &str, room: usize) -> String {
    let mut lines = text.split('\n');
    let first = expand_tabs(lines.next().unwrap_or(""), 3);
    let whole = if lines.next().is_some() {
        format!("{first} …")
    } else {
        first
    };
    if room == 0 {
        return String::new();
    }
    let shown = fit(&whole, room);
    if shown.len() == whole.len() {
        whole
    } else {
        format!("{}…", fit(&whole, room - 1))
    }
}
