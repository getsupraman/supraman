//! Create Worktree in the commit panel's place: the reference picked in the shared palette, an optional new branch and the folder;
//! hit keys `git-add-worktree{,-close,-ok,-ref,-branch,-folder,-browse}`, the palette's rows `git-add-worktree-refs-row-<key>`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, SharedString, Subscription, Window, deferred, div, prelude::*, px};
use supraman_gitman::write::LockSet;
use supraman_gitman::write::branch::{self, name_problem};
use supraman_ui::button;
use supraman_ui::focus_order::{Dismiss, FocusOrder, NextField, Press, PreviousField};
use supraman_ui::folder_picker::pick_folder;
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use supraman_ui::palette::{Palette, PaletteEvent};
use supraman_ui::text_input::{InputEvent, TextInput};
use supraman_ui::themes::secondary;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::{BranchActions, Doing};
use crate::git_view::find::entries::{self, GitEntry, RefKind};

pub(crate) const KEY: &str = "git-add-worktree";
pub(crate) const REFS: &str = "git-add-worktree-refs";
pub(crate) const NEEDS_REF: &str = "Pick the reference to check out";
pub(crate) const NEEDS_FOLDER: &str = "Type the working directory";

#[cfg(test)]
mod tests;

/// The branch a reference names: `feature` for `origin/feature`.
fn branch_of(entry: &GitEntry) -> Option<&str> {
    match entry {
        GitEntry::Ref { name, kind: RefKind::Remote, .. } => Some(name.split_once('/').map_or(name.as_str(), |(_, b)| b)),
        GitEntry::Ref { name, .. } => Some(name),
        _ => None,
    }
}

struct Fields {
    branch: Entity<TextInput>,
    folder: Entity<TextInput>,
    reference: FocusHandle,
    browse: FocusHandle,
    ok: FocusHandle,
    _events: [Subscription; 2],
}

pub(crate) struct AddWorktree {
    refs: Vec<GitEntry>,
    /// The repository's main checkout: new folders go beside it.
    main: PathBuf,
    locals: HashSet<String>,
    picked: Option<GitEntry>,
    fields: Option<Fields>,
    picking: Option<(Entity<Palette<GitEntry>>, Subscription)>,
    wants_picker: bool,
    /// What the form last put in each field; a field holding something else was typed.
    suggested: (String, String),
}

impl AddWorktree {
    fn text(field: Option<&Entity<TextInput>>, cx: &gpui::App) -> String {
        field.map(|f| f.read(cx).text().trim().to_string()).unwrap_or_default()
    }

    pub(crate) fn new_branch(&self, cx: &gpui::App) -> String {
        Self::text(self.fields.as_ref().map(|f| &f.branch), cx)
    }

    pub(crate) fn folder(&self, cx: &gpui::App) -> String {
        Self::text(self.fields.as_ref().map(|f| &f.folder), cx)
    }

    pub(crate) fn picked(&self) -> Option<&str> {
        match &self.picked {
            Some(GitEntry::Ref { name, .. }) => Some(name),
            _ => None,
        }
    }

    /// The reference (its open picker with it), the new branch, the folder, Browse, then Create Worktree.
    fn order(&self, cx: &gpui::App) -> FocusOrder {
        let Some(f) = &self.fields else { return FocusOrder::default() };
        let picker = self.picking.as_ref().map(|(p, _)| p.focus_handle(cx));
        let fields = [f.branch.focus_handle(cx), f.folder.focus_handle(cx), f.browse.clone(), f.ok.clone()];
        FocusOrder::new(std::iter::once(f.reference.clone()).chain(picker).chain(fields))
    }

    #[cfg(test)]
    pub(crate) fn focus_at(&self, window: &Window, cx: &gpui::App) -> Option<usize> {
        self.order(cx).focused(window, cx)
    }

    /// Why Create Worktree cannot act on what is set.
    pub(crate) fn why_not(&self, cx: &gpui::App) -> Option<&'static str> {
        let new = self.new_branch(cx);
        if self.picked.is_none() {
            Some(NEEDS_REF)
        } else if !new.is_empty()
            && let Some(problem) = name_problem(&new)
        {
            Some(problem)
        } else if self.folder(cx).is_empty() {
            Some(NEEDS_FOLDER)
        } else {
            None
        }
    }

    /// `<repo's parent>/<repo>.worktrees/<branch>`.
    fn default_folder(&self, branch: &str) -> String {
        let main = self.main.to_string_lossy();
        let sep = if main.contains('\\') { '\\' } else { '/' };
        let (parent, name) = main.trim_end_matches(['/', '\\']).rsplit_once(['/', '\\']).unwrap_or(("", &main));
        let branch = if sep == '/' { branch.to_string() } else { branch.replace('/', "\\") };
        format!("{parent}{sep}{name}.worktrees{sep}{branch}")
    }

    /// The new branch a remote branch with no local one suggests, and the folder, in fields not typed in.
    fn suggest(&mut self, cx: &mut gpui::App) {
        let Some(fields) = &self.fields else { return };
        let (branch, folder) = (fields.branch.clone(), fields.folder.clone());
        let suggested_branch = match &self.picked {
            Some(e @ GitEntry::Ref { kind: RefKind::Remote, .. }) => branch_of(e).filter(|b| !self.locals.contains(*b)).unwrap_or_default().to_string(),
            _ => String::new(),
        };
        let typed = branch.read(cx).text().trim().to_string();
        if typed.is_empty() || typed == self.suggested.0 {
            branch.update(cx, |i, cx| i.set_text(&suggested_branch, cx));
            self.suggested.0 = suggested_branch;
        }
        let new = branch.read(cx).text().trim().to_string();
        let name = if new.is_empty() { self.picked.as_ref().and_then(branch_of).unwrap_or_default().to_string() } else { new };
        let typed = folder.read(cx).text().trim().to_string();
        if !name.is_empty() && (typed.is_empty() || typed == self.suggested.1) {
            let at = self.default_folder(&name);
            folder.update(cx, |i, cx| i.set_text(&at, cx));
            self.suggested.1 = at;
        }
    }

    #[cfg(test)]
    pub(crate) fn picker(&self) -> Option<&Entity<Palette<GitEntry>>> {
        self.picking.as_ref().map(|(p, _)| p)
    }

    #[cfg(test)]
    pub(crate) fn type_in(&mut self, branch: Option<&str>, folder: Option<&str>, cx: &mut gpui::App) {
        if let Some(fields) = &self.fields {
            let (b, f) = (fields.branch.clone(), fields.folder.clone());
            if let Some(text) = branch {
                b.update(cx, |i, cx| i.set_text(text, cx));
            }
            if let Some(text) = folder {
                f.update(cx, |i, cx| i.set_text(text, cx));
            }
            self.suggest(cx);
        }
    }

    #[cfg(test)]
    pub(crate) fn focused(&self, window: &Window, cx: &gpui::App) -> Option<&'static str> {
        let fields = self.fields.as_ref()?;
        if fields.branch.read(cx).is_focused(window) {
            Some("branch")
        } else if fields.folder.read(cx).is_focused(window) {
            Some("folder")
        } else {
            None
        }
    }
}

impl BranchActions {
    /// Opens Create Worktree over `refs` (local and remote branches and tags), its folders beside `main`.
    pub(crate) fn open_add_worktree(&mut self, refs: Vec<GitEntry>, main: PathBuf, cx: &mut Context<Self>) {
        if self.adding.is_none() {
            let refs: Vec<GitEntry> = refs.into_iter().filter(|e| matches!(e, GitEntry::Ref { kind: RefKind::Local | RefKind::Remote | RefKind::Tag, .. })).collect();
            let locals = refs.iter().filter_map(|e| match e {
                GitEntry::Ref { name, kind: RefKind::Local, .. } => Some(name.clone()),
                _ => None,
            });
            let locals = locals.collect();
            self.adding = Some(AddWorktree { refs, main, locals, picked: None, fields: None, picking: None, wants_picker: true, suggested: Default::default() });
        }
        cx.notify();
    }

    pub(crate) fn close_add_worktree(&mut self, cx: &mut Context<Self>) {
        self.adding = None;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn adding(&self) -> Option<&AddWorktree> {
        self.adding.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn adding_mut(&mut self) -> Option<&mut AddWorktree> {
        self.adding.as_mut()
    }

    /// The form's fields and its picker, made once a window is at hand.
    pub(super) fn ensure_add_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(adding) = &self.adding else { return };
        if adding.fields.is_none() {
            let field = |placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
                let input = cx.new(|cx| TextInput::new(placeholder.to_string(), None, window, cx));
                let sub = cx.subscribe(&input, |this: &mut BranchActions, _, e: &InputEvent, cx| match e {
                    InputEvent::Confirm => this.submit_add_worktree(cx),
                    InputEvent::Cancel => this.close_add_worktree(cx),
                    InputEvent::Edited => this.suggest_worktree(cx),
                    _ => cx.notify(),
                });
                (input, sub)
            };
            let (branch, b) = field("Optional: a new branch", window, cx);
            let (folder, f) = field("Where the worktree goes", window, cx);
            if let Some(a) = self.adding.as_mut() {
                a.fields = Some(Fields { branch, folder, reference: cx.focus_handle(), browse: cx.focus_handle(), ok: cx.focus_handle(), _events: [b, f] });
            }
        }
        if self.adding.as_ref().is_some_and(|a| a.wants_picker && a.picking.is_none()) {
            self.open_picker(window, cx);
        }
    }

    fn suggest_worktree(&mut self, cx: &mut Context<Self>) {
        if let Some(a) = self.adding.as_mut() {
            a.suggest(cx);
        }
        cx.notify();
    }

    fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(adding) = self.adding.as_mut() else { return };
        adding.wants_picker = false;
        let (refs, hits) = (adding.refs.clone(), self.hits.clone());
        let palette = cx.new(|cx| {
            let mut p = Palette::new(REFS, hits, refs, Rc::new(entries::paint), window, cx);
            p.set_rank(entries::ref_rank(), cx);
            p.set_top("Search local and remote branches and tags", "REFERENCE TO CHECK OUT", cx);
            p
        });
        palette.update(cx, |p, cx| p.focus(window, cx));
        let sub = cx.subscribe_in(&palette, window, |this: &mut BranchActions, _, e: &PaletteEvent<GitEntry>, window, cx| match e {
            PaletteEvent::Query(_) => {}
            PaletteEvent::Close => this.close_picker(window, cx),
            PaletteEvent::Open(entry) => {
                if let Some(a) = this.adding.as_mut() {
                    a.picked = Some(entry.clone());
                    a.suggest(cx);
                }
                this.close_picker(window, cx);
            }
        });
        if let Some(a) = self.adding.as_mut() {
            a.picking = Some((palette, sub));
        }
        cx.notify();
    }

    /// The picker closed, the new branch's field taking the keyboard.
    fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(adding) = self.adding.as_mut() else { return };
        adding.picking = None;
        if let Some(fields) = &adding.fields {
            fields.branch.update(cx, |i, cx| i.focus(window, cx));
        }
        cx.notify();
    }

    /// Tab or Shift+Tab in the open picker: the form's next (previous) member focused, the picker shut.
    fn tab_out_of_picker(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(adding) = self.adding.as_mut() else { return };
        adding.order(cx).step(back, window, cx);
        adding.picking = None;
        cx.notify();
    }

    /// Adds the worktree; the form closes once it is made.
    pub(crate) fn submit_add_worktree(&mut self, cx: &mut Context<Self>) {
        let Some(adding) = &self.adding else { return };
        if let Some(why) = adding.why_not(cx).or_else(|| self.busy(cx)) {
            return cx.emit(super::BranchEvent::Why(why.into()));
        }
        let (Some(reference), folder) = (adding.picked().map(str::to_string), PathBuf::from(adding.folder(cx))) else { return };
        let new = Some(adding.new_branch(cx)).filter(|b| !b.is_empty());
        let job_folder = folder.clone();
        self.request(Doing::Worktree(folder), LockSet::branch(), move |s| branch::add_worktree(s, &job_folder, &reference, new.as_deref()), cx);
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pick_folder("Working directory", window, cx, |this: &mut BranchActions, path: PathBuf, window, cx| {
            let Some(fields) = this.adding.as_ref().and_then(|a| a.fields.as_ref()) else { return };
            let folder = fields.folder.clone();
            folder.update(cx, |i, cx| {
                i.set_text(&path.to_string_lossy(), cx);
                i.focus(window, cx);
            });
            cx.notify();
        });
    }

    fn ref_field(&self, adding: &AddWorktree, focus: &FocusHandle, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let muted = secondary(colors);
        let shown = match &adding.picked {
            Some(GitEntry::Ref { name, kind, .. }) => div().flex().items_center().gap_2().min_w_0().child(entries::kind_mark(*kind).0.svg(14., muted)).child(SharedString::from(name.clone())),
            _ => div().text_color(muted).child(NEEDS_REF),
        };
        let field = button::ghost(&self.hits, format!("{KEY}-ref"), cx)
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_sm()
            .border_1()
            .border_color(colors.border)
            .bg(colors.editor_background)
            .child(shown)
            .child(Icon::ChevronDown.svg(12., muted))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_picker(window, cx)));
        let field = button::focusable(field, focus, cx.listener(|this, _: &Press, window, cx| this.open_picker(window, cx)), cx);
        let picker = adding.picking.as_ref().map(|(p, _)| deferred(div().absolute().top_full().left_0().right_0().child(p.clone())).with_priority(supraman_ui::menu::FLOAT));
        div()
            .relative()
            .when(picker.is_some(), |d| {
                d.on_action(cx.listener(|this, _: &NextField, window, cx| this.tab_out_of_picker(false, window, cx)))
                    .on_action(cx.listener(|this, _: &PreviousField, window, cx| this.tab_out_of_picker(true, window, cx)))
            })
            .child(field)
            .children(picker)
            .into_any_element()
    }

    /// The form as the commit panel's place shows it, while it is open.
    pub(crate) fn render_add_worktree(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let adding = self.adding.as_ref()?;
        let fields = adding.fields.as_ref()?;
        let reference = self.ref_field(adding, &fields.reference, cx);
        let colors = cx.theme().colors();
        let (muted, border) = (secondary(colors), colors.border);
        let close = button::icon(&self.hits, format!("{KEY}-close"), Icon::Close.svg(14., muted), px(22.), cx)
            .tooltip(Tooltip::text("Close (Esc)"))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_add_worktree(cx)));
        let title = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .child(Icon::Worktree.svg(16., muted))
            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child("Create Worktree"))
            .child(close);
        let labelled = |label: &'static str, field: AnyElement| div().flex().flex_col().gap_1().child(div().text_xs().text_color(muted).child(label)).child(field);
        let input = |key: &str, input: &Entity<TextInput>| div().relative().flex_1().min_w_0().child(input.clone()).child(hit_target(&self.hits, format!("{KEY}-{key}"))).into_any_element();
        let browse = button::action(&self.hits, format!("{KEY}-browse"), "Browse", button::Tone::Neutral, button::Fill::Outline, cx).on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.browse(window, cx)));
        let browse = button::focusable(browse, &fields.browse, cx.listener(|this, _: &Press, window, cx| this.browse(window, cx)), cx);
        let folder = div().flex().items_center().gap_2().child(input("folder", &fields.folder)).child(browse).into_any_element();
        let why = adding.why_not(cx).or_else(|| self.busy(cx));
        let ok = button::action(&self.hits, format!("{KEY}-ok"), "Create Worktree", button::Tone::Success, button::Fill::Solid, cx);
        let ok = match why {
            Some(why) => button::dimmed(ok).tooltip(Tooltip::text(why)),
            None => ok.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit_add_worktree(cx))),
        };
        let ok = button::focusable(ok, &fields.ok, cx.listener(|this, _: &Press, _, cx| this.submit_add_worktree(cx)), cx);
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child(labelled("Reference to checkout", reference))
            .child(labelled("Worktree branch to create (optional)", input("branch", &fields.branch)))
            .child(labelled("Working directory", folder))
            .child(div().flex().child(ok));
        let form = adding.order(cx).on(div()).on_action(cx.listener(|this, _: &Dismiss, _, cx| this.close_add_worktree(cx))).relative().size_full().flex().flex_col().bg(colors.panel_background).child(title).child(body).child(hit_target(&self.hits, KEY.to_string()));
        Some(form.into_any_element())
    }

    /// The worktree is made: the form closed.
    pub(super) fn added_worktree(&mut self, folder: &Path) -> String {
        self.adding = None;
        format!("Created worktree {}", folder.display())
    }
}
