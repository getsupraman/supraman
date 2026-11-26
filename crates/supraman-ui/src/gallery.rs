//! The kit gallery (UI-KIT §10): every control of this crate in each state it holds without the pointer, one section at a time;
//! `--selftest-shots` captures each section and the hover, pressed and focus frames its [`Probe`]s name.

use std::rc::Rc;

use gpui::{AnyElement, App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, ScrollHandle, SharedString, Subscription, Window, div, prelude::*};
use theme::ActiveTheme as _;

use crate::hit::HitTargets;
use crate::icon::Icon;
use crate::menu::{ContextMenu, MenuItem, item};
use crate::palette::{Entry, Palette};
use crate::split::StackState;
use crate::table::TableState;
use crate::text_input::TextInput;
use crate::toast::ToastSlot;
use crate::tree::TreeState;
use crate::tokens;

mod dnd;
mod sections;

/// The window title the shots step captures by.
pub const TITLE: &str = "Supraman Kit Gallery";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Buttons,
    Toggles,
    Inputs,
    Menu,
    Palette,
    Lists,
    Tabs,
    Splits,
    Dialog,
    Prompts,
    Cards,
    Marks,
    Text,
    Scroll,
    Dnd,
}

impl Section {
    pub const ALL: [Section; 15] = [
        Section::Buttons,
        Section::Toggles,
        Section::Inputs,
        Section::Menu,
        Section::Palette,
        Section::Lists,
        Section::Tabs,
        Section::Splits,
        Section::Dialog,
        Section::Prompts,
        Section::Cards,
        Section::Marks,
        Section::Text,
        Section::Scroll,
        Section::Dnd,
    ];

    /// Its part of the shot's name, `kit-<key>-dark.png`.
    pub fn key(self) -> &'static str {
        match self {
            Section::Buttons => "buttons",
            Section::Toggles => "toggles",
            Section::Inputs => "inputs",
            Section::Menu => "menu",
            Section::Palette => "palette",
            Section::Lists => "lists",
            Section::Tabs => "tabs",
            Section::Splits => "splits",
            Section::Dialog => "dialog",
            Section::Prompts => "prompts",
            Section::Cards => "cards",
            Section::Marks => "marks",
            Section::Text => "text",
            Section::Scroll => "scroll",
            Section::Dnd => "dnd",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Section::Buttons => "Buttons: action × tone × fill × size, icon, toolbar, split, revealed",
            Section::Toggles => "Toggles: checkbox, tick, pill, segmented, stepper, choices",
            Section::Inputs => "Text input: one-line, quiet, inline, invalid, masked, multi-line, form fields",
            Section::Menu => "Context menu: sections, a rule, a dimmed and a ticked item, icons and shortcuts, the highlight, a submenu",
            Section::Palette => "Palette: query, ranked rows, a dimmed row",
            Section::Lists => "Lists on the core: table, tree, rows and list rows, the selected row with and without the keys, hovered, open under its menu",
            Section::Tabs => "Tab strip: more tabs than its room, the last active",
            Section::Splits => "Split pane, height bar, stack",
            Section::Dialog => "Dialog: a question over the scrim",
            Section::Prompts => "Prompts over a tab: a question, a danger question, a form, the upstream prompt; banners, notices, toast",
            Section::Cards => "Hover card, tooltip, details page, inbox card, placeholder",
            Section::Marks => "Marks: dots, rings, activity, squares, progress line",
            Section::Text => "Text: a read-only text's selection, the read-only and editable box; clipped labels, path labels, pairs, clamped",
            Section::Scroll => "Scroll areas: a list, a box, a nested pair, a grid, and a box set to always",
            Section::Dnd => "Drag and drop: a reorder's insertion line, a drop zone under a payload it takes, the dimmed source, the chip",
        }
    }
}

/// What a shot does before an extra frame of its section, named `kit-<section>-<name>-<theme>.png`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// The pointer over the hit key's middle.
    Hover(String),
    /// The pointer over it, its left button held.
    Press(String),
    /// The pointer over it until its tooltip says the text.
    Tooltip(String, String),
    /// The keys given to the gallery's focus sample of that name ([`Gallery::focus_sample`]).
    Focus(&'static str),
    /// The first hit key pressed and dragged over the second, held there; Esc ends it after the frame.
    Drag(String, String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub name: &'static str,
    pub act: Act,
}

fn hover(name: &'static str, key: &str) -> Probe {
    Probe { name, act: Act::Hover(key.into()) }
}

fn press(name: &'static str, key: &str) -> Probe {
    Probe { name, act: Act::Press(key.into()) }
}

fn focus(name: &'static str) -> Probe {
    Probe { name, act: Act::Focus(name) }
}

fn drag(name: &'static str, from: &str, to: &str) -> Probe {
    Probe { name, act: Act::Drag(from.into(), to.into()) }
}

fn tip(name: &'static str, key: &str, text: &str) -> Probe {
    Probe { name, act: Act::Tooltip(key.into(), text.into()) }
}

impl Section {
    /// The section's extra frames, one per control kind it shows.
    pub fn probes(self) -> Vec<Probe> {
        match self {
            Section::Buttons => vec![
                hover("hover-action", "kit-neutral-solid-small"),
                press("pressed-action", "kit-neutral-solid-small"),
                hover("hover-danger", "kit-danger-outline-small"),
                hover("hover-icon", "kit-icon"),
                hover("hover-icon-toned", "kit-icon-danger"),
                hover("hover-toolbar", "kit-toolbar"),
                press("pressed-toolbar", "kit-toolbar"),
                hover("hover-ghost", "kit-ghost"),
                focus("focus-action"),
            ],
            Section::Toggles => vec![
                hover("hover-checkbox", "kit-check-off"),
                hover("hover-pill", "kit-pill"),
                hover("hover-segment", "kit-seg-two"),
                hover("hover-choice", "kit-choice-1"),
                focus("focus-checkbox"),
                focus("focus-choices"),
                focus("focus-stepper"),
            ],
            Section::Inputs => vec![focus("focus-line"), focus("focus-quiet"), focus("focus-inline"), focus("focus-invalid"), focus("focus-multi")],
            Section::Menu => vec![hover("hover-item", "menu-kit-copy"), tip("hover-dimmed", "menu-kit-move", "Nothing to move it to"), focus("open-submenu")],
            Section::Palette => vec![hover("hover-row", "kit-palette-row-beta")],
            Section::Lists => vec![hover("hover-table-row", "kit-table-b-row-3"), hover("hover-tree-row", "kit-tree-b-row-docs/guide.md"), hover("hover-row", "kit-row-2"), hover("hover-list-row", "kit-list-3"), focus("focus-table-b"), focus("focus-tree-a"), focus("open-row")],
            Section::Tabs => vec![hover("hover-tab", "tab-3")],
            Section::Splits => vec![hover("hover-bar", "kit-height-bar")],
            Section::Dialog => vec![hover("hover-answer", "kit-dialog-alt")],
            Section::Prompts => vec![hover("hover-answer", "kit-ask-alt"), hover("hover-form-ok", "kit-form-ok")],
            Section::Cards => vec![hover("hover-card", "kit-inbox-a")],
            Section::Marks => Vec::new(),
            Section::Scroll => vec![hover("hover-area", "kit-scroll-div"), hover("hover-thumb", "kit-scroll-div-bar-thumb"), press("scrolling", "kit-scroll-div-bar-track")],
            Section::Dnd => vec![drag("drag-reorder", "kit-dnd-0", "kit-dnd-2"), drag("drag-over", "kit-dnd-0", "kit-dnd-zone")],
            Section::Text => vec![focus("focus-text-view"), focus("focus-text-edit"), tip("tooltip-clipped", "kit-clip-end", sections::LONG), tip("tooltip-path", "kit-path-long", sections::PATH_FOLDER)],
        }
    }
}

/// A palette row of sample words.
#[derive(Clone, Debug)]
pub struct Sample {
    pub key: &'static str,
    pub text: &'static str,
    pub dimmed: Option<&'static str>,
}

impl Entry for Sample {
    fn key(&self) -> SharedString {
        self.key.into()
    }

    fn text(&self) -> String {
        self.text.into()
    }

    fn dimmed(&self) -> Option<SharedString> {
        self.dimmed.map(SharedString::from)
    }
}

/// The text inputs, one per mode.
pub(crate) struct Inputs {
    pub empty: Entity<TextInput>,
    pub line: Entity<TextInput>,
    pub quiet: Entity<TextInput>,
    pub inline: Entity<TextInput>,
    pub invalid: Entity<TextInput>,
    pub masked: Entity<TextInput>,
    pub multi: Entity<TextInput>,
    pub form_name: Entity<TextInput>,
    pub form_url: Entity<TextInput>,
    pub upstream: Entity<TextInput>,
    /// A read-only text with a selection.
    pub view: Entity<TextInput>,
    /// The same message read only, then editable.
    pub pair: [Entity<TextInput>; 2],
}

pub struct Gallery {
    section: Section,
    pub(crate) hits: HitTargets,
    pub(crate) root: FocusHandle,
    pub(crate) members: crate::focus_order::Members,
    pub(crate) inputs: Inputs,
    pub(crate) palette: Entity<Palette<Sample>>,
    pub(crate) tables: [TableState; 2],
    pub(crate) trees: [TreeState<(), usize>; 2],
    pub(crate) toast: ToastSlot,
    pub(crate) menu: ContextMenu<()>,
    /// The lists' table row menu, open in its frame.
    pub(crate) row_menu: Option<ContextMenu<()>>,
    pub(crate) dialog: crate::dialog::Dialog,
    /// The prompts section's: a question, a danger question, a form, the upstream prompt.
    pub(crate) prompts: [crate::dialog::Dialog; 4],
    pub(crate) stack: StackState,
    pub(crate) scroll: ScrollHandle,
    pub(crate) scrolls: sections::Scrolls,
    pub(crate) split: gpui::Pixels,
    pub(crate) bar: gpui::Pixels,
    /// The drag and drop section's cards in their order.
    pub(crate) cards: Vec<usize>,
    _subscriptions: Vec<Subscription>,
}

impl Gallery {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hits = HitTargets::default();
        let input = |placeholder: &str, text: &str, mode: fn(TextInput) -> TextInput, window: &mut Window, cx: &mut Context<Self>| {
            let input = cx.new(|cx| mode(TextInput::new(placeholder.to_string(), None, window, cx)));
            input.update(cx, |i, cx| i.set_text(text, cx));
            input
        };
        let inputs = Inputs {
            empty: input("Placeholder words", "", |i| i, window, cx),
            line: input("Name", "A typed value", |i| i, window, cx),
            quiet: input("Summary", "A quiet field's words", TextInput::quiet, window, cx),
            inline: input("Name", "inline-name", TextInput::inline, window, cx),
            invalid: input("Name", "a name refused", TextInput::inline, window, cx),
            masked: input("Passphrase", "secret", TextInput::masked, window, cx),
            multi: input("Description", "A longer text over several lines, which wraps where the field ends and keeps going on the next line.\nA second paragraph.", TextInput::multi_line, window, cx),
            form_name: input("Name", "sample", |i| i, window, cx),
            form_url: input("Address", "", |i| i, window, cx),
            upstream: input("Branch on the remote", "main", |i| i, window, cx),
            view: input("", sections::SELECTED, |i| i.multi_line().plain().read_only(), window, cx),
            pair: [
                input("Commit summary", sections::SUMMARY, |i| i.wrapped().plain().read_only(), window, cx),
                input("Commit summary", sections::SUMMARY, |i| i.wrapped().plain(), window, cx),
            ],
        };
        inputs.view.update(cx, |i, cx| {
            i.line.select(sections::SELECTION, false);
            cx.notify();
        });
        inputs.invalid.update(cx, |i, cx| i.set_invalid(true, cx));
        let rows = [
            Sample { key: "alpha", text: "Alpha sample row", dimmed: None },
            Sample { key: "beta", text: "Beta sample row", dimmed: None },
            Sample { key: "gamma", text: "Gamma sample row", dimmed: Some("Not available here") },
            Sample { key: "delta", text: "Delta sample row", dimmed: None },
        ];
        let paint: crate::palette::Paint<Sample> = Rc::new(sections::palette_row);
        let palette_hits = hits.clone();
        let palette = cx.new(|cx| {
            let mut palette = Palette::new("kit-palette", palette_hits, rows.to_vec(), paint, window, cx);
            palette.set_top("Search the samples", "Samples", cx);
            palette
        });
        let tables = [TableState::new(cx), TableState::new(cx)];
        tables.iter().for_each(|t| t.select(Some(2)));
        let trees = [TreeState::new(cx), TreeState::new(cx)];
        for tree in &trees {
            tree.set_entries(sections::TREE.iter().map(|p| (p.to_string(), ())), |n: &mut usize, _| *n += 1);
            tree.expand_all();
            tree.select(Some("docs/guide.md"));
        }
        let menu = ContextMenu::new("Sample", gpui::point(gpui::px(40.), gpui::px(80.)), menu_items());
        Self {
            section: Section::Buttons,
            hits,
            root: cx.focus_handle(),
            members: Default::default(),
            inputs,
            palette,
            tables,
            trees,
            toast: ToastSlot::default(),
            menu,
            row_menu: None,
            dialog: Default::default(),
            prompts: std::array::from_fn(|_| crate::dialog::Dialog::in_tab(cx.focus_handle())),
            stack: StackState::default(),
            scroll: ScrollHandle::new(),
            scrolls: sections::Scrolls::default(),
            split: gpui::px(256.),
            bar: gpui::px(80.),
            cards: (0..dnd::CARDS).collect(),
            _subscriptions: Vec::new(),
        }
    }

    pub fn section(&self) -> Section {
        self.section
    }

    pub fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        self.hits.borrow_mut().clear();
        self.menu = ContextMenu::new("Sample", self.menu.position, menu_items());
        self.menu.show_highlighted(1);
        self.row_menu = None;
        cx.notify();
    }

    pub fn hits(&self) -> HitTargets {
        self.hits.clone()
    }

    /// The keys to the gallery's own root: no sample holds them.
    pub fn rest(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.root, cx);
    }

    /// The keys to the sample `name` of [`Act::Focus`]; false when the shown section has none by that name.
    pub fn focus_sample(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match name {
            "open-submenu" if self.section == Section::Menu => {
                let share = self.menu.items.iter().position(|i| !i.submenu.is_empty());
                share.into_iter().for_each(|ix| self.menu.show_highlighted(ix));
                cx.notify();
                return share.is_some();
            }
            "open-row" if self.section == Section::Lists => {
                self.row_menu = Some(row_menu(&self.hits));
                cx.notify();
                return true;
            }
            _ => {}
        }
        let handle = match name {
            "focus-line" => Some(self.inputs.line.focus_handle(cx)),
            "focus-quiet" => Some(self.inputs.quiet.focus_handle(cx)),
            "focus-inline" => Some(self.inputs.inline.focus_handle(cx)),
            "focus-invalid" => Some(self.inputs.invalid.focus_handle(cx)),
            "focus-multi" => Some(self.inputs.multi.focus_handle(cx)),
            "focus-text-view" => Some(self.inputs.view.focus_handle(cx)),
            "focus-text-edit" => Some(self.inputs.pair[1].focus_handle(cx)),
            "focus-table-a" => Some(self.tables[0].focus_handle().clone()),
            "focus-table-b" => Some(self.tables[1].focus_handle().clone()),
            "focus-tree-a" => Some(self.trees[0].focus_handle().clone()),
            "focus-palette" => Some(self.palette.focus_handle(cx)),
            "focus-action" => self.members.get("kit-focusable"),
            "focus-checkbox" => self.members.get("kit-check-focus"),
            "focus-choices" => self.members.get("kit-choices"),
            "focus-stepper" => self.members.get("kit-stepper-inc"),
            _ => None,
        };
        let Some(handle) = handle else { return false };
        crate::focus::keyed(window, cx);
        window.focus(&handle, cx);
        cx.notify();
        true
    }

    /// The section's rest frame: what holds the keys while it is captured without a probe.
    pub fn rest_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.section {
            Section::Lists => {
                self.focus_sample("focus-table-a", window, cx);
            }
            Section::Palette => {
                self.focus_sample("focus-palette", window, cx);
            }
            _ => self.rest(window, cx),
        }
    }

    /// A toast, shown now for its usual time.
    pub fn show_toast(&mut self, cx: &mut Context<Self>) {
        self.toast.show("Copied the sample words".into(), |g: &mut Gallery| &mut g.toast, cx);
    }
}

fn menu_items() -> Vec<MenuItem<()>> {
    let share = MenuItem::submenu("kit-share", "Share", vec![item("kit-share-link", "Copy link", ()).icon(Icon::Copy), item("kit-share-web", "Open in browser", ()).icon(Icon::ArrowUpRight)]);
    vec![
        item("kit-open", "Open", ()).icon(Icon::Folder).shortcut("Enter"),
        item("kit-copy", "Copy words", ()).icon(Icon::Copy).shortcut("Ctrl+C"),
        item("kit-move", "Move to…", ()).dimmed("Nothing to move it to"),
        item("kit-show-a", "Show column A", ()).under("Columns").ticked(true),
        item("kit-show-b", "Show column B", ()).ticked(false),
        item("kit-remove", "Remove", ()).rule(),
    ]
    .into_iter()
    .chain(share)
    .collect()
}

/// The menu a lists frame shows open at the middle of its table's second row, as a right-click there opens it.
fn row_menu(hits: &HitTargets) -> ContextMenu<()> {
    let anchor = crate::table::row_hit("kit-table-b", "1");
    let at = hits.borrow().get(&anchor).map(|h| h.bounds.center()).unwrap_or_default();
    let items = vec![item("kit-row-open", "Open", ()), item("kit-row-copy", "Copy name", ()).icon(Icon::Copy), item("kit-row-remove", "Remove", ()).rule()];
    ContextMenu::new("Row 2", at, items).anchored(anchor)
}

impl Focusable for Gallery {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.members.begin();
        let colors = cx.theme().colors().clone();
        let body: AnyElement = match self.section {
            Section::Buttons => sections::buttons(self, cx),
            Section::Toggles => sections::toggles(self, cx),
            Section::Inputs => sections::inputs(self, cx),
            Section::Menu => sections::menu(self, cx),
            Section::Palette => sections::palette(self),
            Section::Lists => sections::lists(self, cx),
            Section::Tabs => sections::tabs(self, window, cx),
            Section::Splits => sections::splits(self, cx),
            Section::Dialog => sections::dialog(self, cx),
            Section::Prompts => sections::prompts(self, cx),
            Section::Cards => sections::cards(self, window, cx),
            Section::Marks => sections::marks(cx),
            Section::Text => sections::text(self, cx),
            Section::Scroll => sections::scroll(self, cx),
            Section::Dnd => dnd::section(self, cx),
        };
        let title = div().flex_none().px(tokens::space::XXL).py(tokens::space::L).border_b(tokens::stroke::HAIRLINE).border_color(colors.border).text_size(tokens::text::BODY).text_color(crate::themes::secondary(&colors)).child(self.section.title());
        div()
            .id("kit-gallery")
            .track_focus(&self.root)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .font_family(crate::fonts::UI)
            .bg(colors.editor_background)
            .text_color(colors.text)
            .child(title)
            .child(div().relative().flex_1().min_h_0().p(tokens::space::XXL).child(body))
    }
}

#[cfg(test)]
mod tests;
