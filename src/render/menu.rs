//! The rows of the completion menu under the line.

use unicode_width::UnicodeWidthChar;

use super::{expand_tabs, fit};
use crate::colors::Colors;
use crate::menu::{Item, window};

/// A menu to draw: its items, the highlighted one, and the most rows it
/// takes.
pub struct MenuView<'a> {
    pub items: &'a [Item],
    pub highlighted: Option<usize>,
    pub max_rows: usize,
}

/// The rows to draw for `view` on a screen `cols` wide with `room` rows free
/// under the line, each with its colours, in order. Empty when nothing fits.
pub fn rows(view: &MenuView, room: usize, cols: usize, colors: &Colors) -> Vec<String> {
    let shown = window(view.items.len(), view.highlighted, view.max_rows.min(room));
    // The last column stays free, so the terminal never wraps.
    let width = cols.saturating_sub(1);
    // Every item row starts with its letter and two spaces.
    if width < 3 {
        return Vec::new();
    }
    let shown_items = &view.items[shown.first..shown.first + shown.count];
    // An item's text is padded to line up its note only when a shown item
    // has one, and only when there is room left for both the item and the
    // note; otherwise notes are left off every row.
    let notes = shown_items.iter().any(|item| item.note.is_some());
    let text_width = if notes {
        let cap = (width / 2).saturating_sub(3);
        shown_items
            .iter()
            .map(|item| display_width(&item_text(&item.text, cap)))
            .max()
            .unwrap_or(0)
    } else {
        0
    };
    let rest = width
        .saturating_sub(3)
        .saturating_sub(text_width)
        .saturating_sub(2);
    let notes = notes && text_width > 0 && rest > 0;

    let mut rows = Vec::new();
    for i in shown.first..shown.first + shown.count {
        let item = &view.items[i];
        let letter = item.source.letter().to_string();
        let (text, note) = match item.note.as_deref().filter(|_| notes) {
            Some(note) => (
                pad(&item_text(&item.text, text_width), text_width),
                Some(item_text(note, rest)),
            ),
            None => (item_text(&item.text, width.saturating_sub(3)), None),
        };
        rows.push(if view.highlighted == Some(i) {
            let note = note.map(|note| format!("  {note}")).unwrap_or_default();
            paint(colors.menu_selected(), &format!("{letter}  {text}{note}"))
        } else {
            let note = note
                .map(|note| format!("  {}", paint(colors.menu_note(), &note)))
                .unwrap_or_default();
            format!(
                "{}  {}{note}",
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

/// The columns `text` takes on screen.
fn display_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// `text` with spaces added to reach `width` columns.
fn pad(text: &str, width: usize) -> String {
    let mut padded = text.to_owned();
    padded.extend(std::iter::repeat_n(
        ' ',
        width.saturating_sub(display_width(text)),
    ));
    padded
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
