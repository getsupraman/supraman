//! The viewer's header (change mark, path, encoding, ×), its toolbar and the editor or placeholder below.

use gpui::{AnyElement, ClickEvent, Context, Div, SharedString, Window, div, prelude::*, px};
use supraman_gitman::ChangeKind;
use supraman_ui::button;
use supraman_ui::clipped::clipped;
use supraman_ui::icon::Icon;
use supraman_ui::segmented::{Segment, segmented};
use supraman_ui::stall;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::{CloseDiff, Content, DiffView, KEY_CONTEXT, Layout, Mode, NextChange, PreviousChange};
use crate::git_view::details::{kind_word, mark_icon};

/// `dir/` muted and the name bright, cut from the left side when narrow.
fn path(key: &str, path: &str, cx: &Context<DiffView>) -> Div {
    let colors = cx.theme().colors();
    let (dir, name) = match path.rsplit_once('/') {
        Some((dir, name)) => (format!("{dir}/"), name.to_string()),
        None => (String::new(), path.to_string()),
    };
    div()
        .flex()
        .min_w_0()
        .whitespace_nowrap()
        .when(!dir.is_empty(), |d| d.child(clipped(format!("{key}-dir"), dir, true).flex_shrink(1.).text_color(colors.text_muted)))
        .child(div().flex_none().text_color(colors.text).child(SharedString::from(name)))
}

impl DiffView {
    fn header(&self, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let mark = self.target.kind().map(|kind| {
            let (icon, colour) = mark_icon(kind, cx);
            div().flex().flex_none().items_center().child(icon.svg(13., colour)).id("diff-mark").tooltip(Tooltip::text(kind_word(kind)))
        });
        let renamed = matches!(self.target.kind(), Some(ChangeKind::Renamed | ChangeKind::Copied));
        let encoding = match &self.content {
            Content::Text(t) => SharedString::from(t.encoding.label()),
            Content::Binary => "Binary".into(),
            _ => SharedString::default(),
        };
        let close = button::icon(&self.hits, "diff-close", Icon::Close.svg(14., colors.text_muted), px(22.), cx)
            .tooltip(Tooltip::text("Close (Esc): back to the graph"))
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(super::Close)));
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(32.))
            .px_2()
            .border_b_1()
            .border_color(colors.border_variant)
            .children(mark)
            .when(renamed, |d| d.child(path("diff-old-path", self.target.old_path(), cx)).child(Icon::Renamed.svg(12., colors.text_muted)))
            .child(path("diff-path", &self.target.path, cx).flex_1())
            .child(div().flex_none().text_xs().text_color(colors.text_muted).child(encoding))
            .child(close)
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let modes = [Segment::new("file").icon(Icon::File).label("File View"), Segment::new("diff").icon(Icon::FileDiff).label("Diff View")];
        let chosen = usize::from(self.mode == Mode::Diff);
        let mode = segmented(&self.hits, "diff-mode", &modes, chosen, |this: &mut DiffView, i, _, cx| this.set_mode(if i == 0 { Mode::File } else { Mode::Diff }, cx), cx);
        let bar = div().flex().flex_none().items_center().gap_2().h(px(34.)).px_2().border_b_1().border_color(colors.border_variant).child(mode);
        if self.mode == Mode::File || !matches!(self.content, Content::Text(_)) {
            return bar;
        }
        let layouts = [Segment::new("split").icon(Icon::DiffSplit).label("Split"), Segment::new("inline").icon(Icon::DiffInline).label("Inline")];
        let chosen = usize::from(self.layout == Layout::Inline);
        let layout = segmented(&self.hits, "diff-layout", &layouts, chosen, |this: &mut DiffView, i, _, cx| this.set_layout(if i == 0 { Layout::Split } else { Layout::Inline }, cx), cx);
        let count = self.hunk_count();
        let place = match (self.current, count) {
            (_, 0) => "no changes".to_string(),
            (Some(c), n) => format!("change {} of {n}", c + 1),
            (None, 1) => "1 change".to_string(),
            (None, n) => format!("{n} changes"),
        };
        let previous = button::icon(&self.hits, "diff-previous", Icon::ChevronUp.svg(14., colors.text_muted), px(24.), cx)
            .tooltip(Tooltip::text("Previous change (Shift+F7)"))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.previous_change(window, cx)));
        let next = button::icon(&self.hits, "diff-next", Icon::ChevronDown.svg(14., colors.text_muted), px(24.), cx)
            .tooltip(Tooltip::text("Next change (F7)"))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.next_change(window, cx)));
        let (previous, next) = if count == 0 { (button::dimmed(previous), button::dimmed(next)) } else { (previous, next) };
        bar.child(layout).child(div().flex_1()).child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(place))).child(previous).child(next)
    }

    fn placeholder(title: &str, detail: Option<String>, cx: &Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .child(div().text_color(colors.text).child(SharedString::from(title.to_string())))
            .when_some(detail, |d, detail| d.child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(detail))))
            .into_any_element()
    }

    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match (&self.content, self.mode, self.layout) {
            (Content::Loading, ..) => Self::placeholder("Reading the file…", None, cx),
            (Content::Failed(why), ..) => Self::placeholder("Cannot read this file", Some(why.clone()), cx),
            (Content::Binary, ..) => Self::placeholder("Binary file", Some("Its contents are not shown.".into()), cx),
            (Content::Text(_), Mode::File, _) => self.file_editor(window, cx).map_or_else(|| div().into_any_element(), |e| e.into_any_element()),
            (Content::Text(_), Mode::Diff, Layout::Inline) => self.inline_editor(window, cx).into_any_element(),
            (Content::Text(_), Mode::Diff, Layout::Split) => self.split(window, cx).into_any_element(),
        }
    }
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _stall = stall::section("render: diff view");
        let colors = cx.theme().colors().clone();
        let body = self.body(window, cx);
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NextChange, window, cx| this.next_change(window, cx)))
            .on_action(cx.listener(|this, _: &PreviousChange, window, cx| this.previous_change(window, cx)))
            .on_action(cx.listener(|_, _: &CloseDiff, _, cx| cx.emit(super::Close)))
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.editor_background)
            .child(self.header(cx))
            .child(self.toolbar(cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}
