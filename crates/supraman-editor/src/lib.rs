//! Zed's editor without a Project, read-only: a file at a revision, and the diff of two
//! texts inline or split side by side, highlighted by the grammars Zed bundles and those `languages` compiles in.

mod diff;
mod inline;
mod languages;
mod look;
mod ruler;
mod split;
mod words;

use std::sync::Arc;

use editor::{Editor, EditorMode, MinimapVisibility};
use gpui::{App, AppContext as _, BorrowAppContext as _, Context, Entity, KeyBinding, Window};
use language::{Buffer, Capability};
use multi_buffer::MultiBuffer;
use theme::ActiveTheme as _;

pub use diff::{Diff, Hunk};
pub use editor::Editor as ZedEditor;
pub use inline::{inline_editor, tinted_rows, tinted_words};
pub use language::Language;
pub use languages::language_for;
pub use look::{DiffColors, Tint, diff_colors};
pub use split::SplitDiff;
pub use words::word_ranges;

/// The editor's context: its keys are bound in it.
pub const CONTEXT: &str = "Editor";

/// The settings, theme settings and editor globals the editor reads, its text in `font` at `size` px, and its
/// read-only keys. Call once, before the app applies its own theme: the theme settings' init installs Zed's.
pub fn init(font: &str, size: f32, cx: &mut App) {
    settings::init(cx);
    let user = serde_json::json!({ "buffer_font_family": font, "buffer_font_size": size, "word_diff_enabled": false }).to_string();
    cx.update_global(|store: &mut settings::SettingsStore, cx| {
        let _ = store.set_user_settings(&user, cx);
    });
    theme_settings::init(theme::LoadThemes::JustBase, cx);
    release_channel::init(semver::Version::new(0, 0, 0), cx);
    editor::init(cx);
    bind_keys(cx);
    languages::warm(cx);
    languages::follow_theme(cx);
}

fn bind_keys(cx: &mut App) {
    use editor::actions::*;
    use zed_actions::editor::{MoveDown, MoveUp};
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("ctrl-c", Copy, c),
        KeyBinding::new("ctrl-insert", Copy, c),
        KeyBinding::new("ctrl-a", SelectAll, c),
        KeyBinding::new("up", MoveUp, c),
        KeyBinding::new("down", MoveDown, c),
        KeyBinding::new("left", MoveLeft, c),
        KeyBinding::new("right", MoveRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("pageup", MovePageUp::default(), c),
        KeyBinding::new("pagedown", MovePageDown::default(), c),
        KeyBinding::new("ctrl-home", MoveToBeginning, c),
        KeyBinding::new("ctrl-end", MoveToEnd, c),
        KeyBinding::new("ctrl-shift-home", SelectToBeginning, c),
        KeyBinding::new("ctrl-shift-end", SelectToEnd, c),
    ]);
}

/// A buffer nobody can edit, highlighted as `language` with what it injects (Markdown's inline text and fenced code).
pub fn read_only_buffer(text: String, language: Option<Arc<Language>>, cx: &mut App) -> Entity<Buffer> {
    cx.new(|cx| read_only(Buffer::local(text, cx), language, cx))
}

/// [`read_only_buffer`] of a rope built elsewhere (off the UI thread) with its line ending.
pub(crate) fn read_only_buffer_of(rope: text::Rope, ending: text::LineEnding, language: Option<Arc<Language>>, cx: &mut App) -> Entity<Buffer> {
    cx.new(|cx| read_only(Buffer::local_normalized(rope, ending, cx), language, cx))
}

fn read_only(mut buffer: Buffer, language: Option<Arc<Language>>, cx: &mut Context<Buffer>) -> Buffer {
    buffer.set_capability(Capability::ReadOnly, cx);
    if let Some(language) = language {
        buffer.set_language_registry(languages::registry(cx));
        buffer.set_language(Some(language), cx);
    }
    buffer
}

/// A full editor over `multi` that shows and selects but never edits.
pub(crate) fn read_only_editor(multi: Entity<MultiBuffer>, window: &mut Window, cx: &mut Context<Editor>) -> Editor {
    let mut editor = Editor::new(EditorMode::full(), multi, None, window, cx);
    editor.set_read_only(true);
    editor.set_input_enabled(false);
    editor.set_show_code_actions(false, cx);
    editor.set_show_runnables(false, cx);
    editor.set_show_breakpoints(false, cx);
    editor.set_show_bookmarks(false, cx);
    editor.set_show_edit_predictions(Some(false), window, cx);
    editor.set_show_diff_review_button(false, cx);
    editor.set_show_git_diff_gutter(false, cx);
    editor.set_minimap_visibility(MinimapVisibility::Disabled, window, cx);
    editor.disable_inline_diagnostics();
    editor.disable_code_lens(cx);
    editor.disable_mouse_wheel_zoom();
    editor
}

/// The whole of `buffer` in a read-only editor.
pub fn file_editor(buffer: Entity<Buffer>, window: &mut Window, cx: &mut App) -> Entity<Editor> {
    cx.new(|cx| {
        let multi = cx.new(|cx| MultiBuffer::singleton(buffer, cx));
        read_only_editor(multi, window, cx)
    })
}

/// Selects the start of `row` in `editor`, scrolled into view when `scroll`.
pub fn select_row(editor: &Entity<Editor>, row: u32, scroll: bool, window: &mut Window, cx: &mut App) {
    editor.update(cx, |editor, cx| {
        let point = text::Point::new(row, 0);
        let effects = if scroll { editor::SelectionEffects::scroll(editor::scroll::Autoscroll::top_relative(3.)) } else { editor::SelectionEffects::no_scroll() };
        editor.change_selections(effects, window, cx, |s| s.select_ranges([point..point]));
    });
}

/// The row of `editor`'s newest selection head.
pub fn cursor_row(editor: &Entity<Editor>, cx: &mut App) -> u32 {
    editor.update(cx, |editor, cx| {
        let snapshot = editor.display_snapshot(cx);
        editor.selections.newest::<text::Point>(&snapshot).head().row
    })
}

/// The display row at the top of `editor`'s view.
pub fn scroll_row(editor: &Entity<Editor>, cx: &mut App) -> f64 {
    editor.update(cx, |e, cx| e.scroll_position(cx).y)
}

/// Scrolls `editor` to put display `row` at the top of its view.
pub fn set_scroll_row(editor: &Entity<Editor>, row: f64, window: &mut Window, cx: &mut App) {
    editor.update(cx, |e, cx| {
        let x = e.scroll_position(cx).x;
        e.set_scroll_position(gpui::point(x, row), window, cx);
    });
}

/// The text of `editor`'s buffer, as a test reads it.
pub fn text(editor: &Entity<Editor>, cx: &App) -> String {
    editor.read(cx).buffer().read(cx).snapshot(cx).text()
}

/// The selected text of `editor`'s newest selection, what Copy puts on the clipboard.
pub fn selected_text(editor: &Entity<Editor>, cx: &mut App) -> String {
    editor.update(cx, |editor, cx| {
        let snapshot = editor.display_snapshot(cx);
        let range = editor.selections.newest::<text::Point>(&snapshot).range();
        editor.buffer().read(cx).snapshot(cx).text_for_range(range).collect()
    })
}

/// `editor`'s text runs a syntax colour paints, each with the theme's capture name for it, as a test reads them.
pub fn highlighted(editor: &Entity<Editor>, cx: &App) -> Vec<(String, String)> {
    let snapshot = editor.read(cx).buffer().read(cx).snapshot(cx);
    let syntax = cx.theme().syntax();
    let styling = language::LanguageAwareStyling { tree_sitter: true, diagnostics: false };
    let mut runs: Vec<(String, String)> = Vec::new();
    let mut open = false;
    for chunk in snapshot.chunks(text::Point::zero()..snapshot.max_point(), styling) {
        let Some(name) = chunk.syntax_highlight_id.and_then(|id| syntax.get_capture_name(id)) else {
            open = false;
            continue;
        };
        match runs.last_mut() {
            Some((text, last)) if open && last == name => text.push_str(chunk.text),
            _ => runs.push((chunk.text.to_string(), name.to_string())),
        }
        open = true;
    }
    runs
}
