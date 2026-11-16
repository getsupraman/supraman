//! The one reader and writer of the clipboard: gpui's platform clipboard, or this process's own once `use_memory` ran (every selftest mode).

use std::cell::RefCell;

use gpui::{App, ClipboardItem, Global};
use supraman_term::terminal_settings::ClipboardRoute;

#[derive(Default)]
struct Memory {
    item: RefCell<Option<ClipboardItem>>,
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    primary: RefCell<Option<ClipboardItem>>,
    writes: std::cell::Cell<usize>,
}

impl Global for Memory {}

/// Every later read and write stays in this process; called before any window opens.
pub fn use_memory(cx: &mut App) {
    cx.set_global(Memory::default());
}

pub fn write(cx: &App, item: ClipboardItem) {
    match cx.try_global::<Memory>() {
        Some(memory) => {
            *memory.item.borrow_mut() = Some(item);
            memory.writes.set(memory.writes.get() + 1);
        }
        None => cx.write_to_clipboard(item),
    }
}

pub fn write_text(cx: &App, text: impl Into<String>) {
    write(cx, ClipboardItem::new_string(text.into()));
}

pub fn read(cx: &App) -> Option<ClipboardItem> {
    match cx.try_global::<Memory>() {
        Some(memory) => memory.item.borrow().clone(),
        None => cx.read_from_clipboard(),
    }
}

pub fn text(cx: &App) -> Option<String> {
    read(cx).and_then(|item| item.text())
}

/// How many writes the memory clipboard took; `None` on the platform's.
pub fn writes(cx: &App) -> Option<usize> {
    cx.try_global::<Memory>().map(|memory| memory.writes.get())
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn read_primary(cx: &App) -> Option<ClipboardItem> {
    match cx.try_global::<Memory>() {
        Some(memory) => memory.primary.borrow().clone(),
        None => cx.read_from_primary(),
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn write_primary(cx: &App, item: ClipboardItem) {
    match cx.try_global::<Memory>() {
        Some(memory) => *memory.primary.borrow_mut() = Some(item),
        None => cx.write_to_primary(item),
    }
}

/// What a terminal copies and pastes through: this module.
pub fn terminal() -> ClipboardRoute {
    ClipboardRoute {
        read,
        write,
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        read_primary,
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        write_primary,
    }
}

#[cfg(test)]
mod tests;
