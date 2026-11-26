//! Each section's samples.

use std::ops::Range;

use gpui::{AnyElement, App, Context, Div, Focusable as _, Hsla, IntoElement, Rems, SharedString, Window, div, prelude::*, rems};
use theme::ActiveTheme as _;

use super::{Gallery, Sample};
use crate::button::{self, Fill, Size, Tone};
use crate::hit::hit_target;
use crate::icon::Icon;
use crate::marks::{Activity, Mark};

pub const LONG: &str = "A label far longer than the room it is given, cut with an ellipsis";
pub const PATH: &str = "crates/sample-kit/src/a/deeply/nested/folder/module_file.rs";
/// What the cut [`PATH`]'s tooltip says: its folder.
pub const PATH_FOLDER: &str = "crates/sample-kit/src/a/deeply/nested/folder/";
pub const TREE: [&str; 6] = ["docs/guide.md", "docs/notes/today.md", "src/main.rs", "src/view/list.rs", "src/view/row.rs", "README.md"];

fn group(caption: &str, cx: &App) -> Div {
    let muted = crate::themes::secondary(cx.theme().colors());
    div().flex().flex_col().gap_2().child(div().text_xs().text_color(muted).child(caption.to_string()))
}

fn row() -> Div {
    div().flex().flex_wrap().items_center().gap_3()
}

fn caption(text: &str, cx: &App) -> Div {
    div().text_xs().text_color(crate::themes::secondary(cx.theme().colors())).child(text.to_string())
}

fn width(r: f32) -> Rems {
    rems(r)
}

fn key(tone: Tone, fill: Fill, size: Size) -> String {
    let tone = match tone {
        Tone::Neutral => "neutral",
        Tone::Success => "success",
        Tone::Danger => "danger",
    };
    let fill = if fill == Fill::Solid { "solid" } else { "outline" };
    let size = if size == Size::Small { "small" } else { "medium" };
    format!("kit-{tone}-{fill}-{size}")
}

pub fn buttons(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let hits = &g.hits;
    let mut grid = div().flex().flex_col().gap_3();
    for size in Size::ALL {
        let mut line = row().child(caption(if size == Size::Small { "Small" } else { "Medium" }, cx).w(width(4.)));
        for tone in [Tone::Neutral, Tone::Success, Tone::Danger] {
            for fill in [Fill::Solid, Fill::Outline] {
                let label = format!("{tone:?} {fill:?}");
                line = line.child(button::sized(button::action(hits, key(tone, fill, size), label, tone, fill, cx), size));
            }
        }
        let dimmed = button::dimmed(button::sized(button::action(hits, format!("kit-dimmed-{size:?}"), "Dimmed", Tone::Neutral, Fill::Solid, cx), size));
        grid = grid.child(line.child(dimmed));
    }
    let focused = button::focusable(button::action(hits, "kit-focusable", "Focusable", Tone::Neutral, Fill::Solid, cx), &g.members.handle("kit-focusable", cx), |_, _, _| {}, cx);
    let with = row()
        .child(button::action_with(hits, "kit-with-icon", Icon::Play, "With icon", Tone::Neutral, Fill::Solid, cx))
        .child(button::action_turning(hits, "kit-turning", Icon::Refresh, "Turning", Tone::Success, Fill::Solid, cx))
        .child(button::dimmed(button::action_with(hits, "kit-with-dimmed", Icon::Trash, "Dimmed", Tone::Danger, Fill::Outline, cx)))
        .child(focused)
        .child(button::line().child(button::action(hits, "kit-line-a", "In a line", Tone::Neutral, Fill::Solid, cx)).child(button::action(hits, "kit-line-b", "GAP apart", Tone::Neutral, Fill::Outline, cx)));
    let fg = cx.theme().colors().text_muted;
    let side = gpui::px(22.);
    let icons = row()
        .child(button::icon(hits, "kit-icon", Icon::Settings.svg(14., fg), side, cx))
        .child(button::icon_toned(hits, "kit-icon-neutral", Icon::Copy.svg(14., fg), side, Tone::Neutral, cx))
        .child(button::icon_toned(hits, "kit-icon-success", Icon::Check.svg(14., cx.theme().status().success), side, Tone::Success, cx))
        .child(button::icon_toned(hits, "kit-icon-danger", Icon::Trash.svg(14., cx.theme().status().error), side, Tone::Danger, cx))
        .child(button::dimmed(button::icon(hits, "kit-icon-dimmed", Icon::Close.svg(14., fg), side, cx)))
        .child(div().id("kit-revealed-row").group("kit-revealed-row").flex().items_center().gap_1().child("Revealed:").child(button::revealed(hits, "kit-revealed", Icon::Star.svg(14., fg), side, "kit-revealed-row", true, cx)));
    let toolbars = row()
        .child(button::toolbar(hits, "kit-toolbar", Icon::Refresh.svg(14., fg), false, fg, cx).child("Toolbar"))
        .child(button::toolbar(hits, "kit-toolbar-on", Icon::GitGraph.svg(14., fg), true, fg, cx).child("On"))
        .child(button::toolbar_turning(hits, "kit-toolbar-turning", Icon::Refresh.svg(14., fg), false, true, fg, cx).child("Turning"))
        .child(button::split(button::toolbar(hits, "kit-split-main", Icon::Plus.svg(14., fg), false, fg, cx), button::toolbar(hits, "kit-split-menu", Icon::ChevronDown.svg(12., fg), false, fg, cx), cx))
        .child(button::ghost(hits, "kit-ghost", cx).px_2().py_1().rounded_sm().child("Ghost"));
    grid.child(group("action_with, action_turning, dimmed, focusable (ringed while focused), line", cx).child(with))
        .child(group("icon, icon_toned (Neutral, Success, Danger), dimmed, revealed", cx).child(icons))
        .child(group("toolbar (off, on, turning), split, ghost", cx).child(toolbars))
        .into_any_element()
}

pub fn toggles(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let hits = &g.hits;
    let labelled = |check: gpui::Stateful<Div>, words: &str| div().flex().items_center().gap_2().child(check).child(words.to_string());
    let checks = row()
        .child(labelled(button::checkbox(hits, "kit-check-off", false, cx), "Off"))
        .child(labelled(button::checkbox(hits, "kit-check-on", true, cx), "On"))
        .child(labelled(button::focusable_checkbox(hits, "kit-check-focus", true, &g.members.handle("kit-check-focus", cx), |_, _, _| {}, cx), "Focusable"))
        .child(div().flex().items_center().gap_2().child(button::tick(false, cx.theme().colors().text_accent, cx)).child(button::tick(true, cx.theme().colors().text_accent, cx)).child("tick"));
    let fg = cx.theme().colors().text_muted;
    let pills = row().child(button::pill(hits, "kit-pill", "pill", fg, cx)).child(button::pill(hits, "kit-pill-accent", "accent", cx.theme().colors().text_accent, cx));
    let segments = [
        crate::segmented::Segment::new("one").label("One"),
        crate::segmented::Segment::new("two").icon(Icon::ListTree).label("Two"),
        crate::segmented::Segment::new("three").icon(Icon::ListFlat),
    ];
    let segmented = crate::segmented::segmented(hits, "kit-seg", &segments, 0, |_: &mut Gallery, _, _, _| {}, cx);
    let stepper = button::stepper(hits, "kit-stepper", "20", &g.members, |_: &mut Gallery, _, _, _| {}, cx);
    let list = div().flex().flex_col().gap_1().w(width(14.)).children((0..3).map(|i| {
        let key = format!("kit-choice-{i}");
        button::choice(SharedString::from(key.clone()), format!("Choice {}", i + 1), i == 0, cx).relative().child(hit_target(hits, key))
    }));
    let choices = button::choices(list, &g.members.handle("kit-choices", cx), 3, Some(0), |_: &mut Gallery, _, _, _| {}, cx);
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(group("checkbox (off, on), focusable_checkbox, tick", cx).child(checks))
        .child(group("pill", cx).child(pills))
        .child(group("segmented, the first chosen", cx).child(row().child(segmented)))
        .child(group("stepper", cx).child(stepper))
        .child(group("choices: a list of choice, the first chosen", cx).child(choices))
        .into_any_element()
}

pub fn inputs(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let i = &g.inputs;
    let field = |caption: &str, input: AnyElement| div().flex().items_center().gap_3().child(div().w(width(9.)).text_sm().child(caption.to_string())).child(div().w(width(22.)).child(input));
    let quiet = crate::text_input::quiet_frame(false, cx).w_full().child(div().px_1().child("A quiet field's still copy"));
    let form_fields = vec![
        (SharedString::from("Name"), crate::form::input(&g.hits, "kit-form-name", &i.form_name)),
        (SharedString::from("Address"), crate::form::input(&g.hits, "kit-form-url", &i.form_url)),
    ];
    let fields = div().flex().flex_col().gap_2().children(form_fields.into_iter().map(|(label, input)| field(&label, input)));
    let row_sample = crate::settings_page::row("A setting", "What the setting does, in one line under its title.", button::sized(button::action(&g.hits, "kit-setting", "Change", Tone::Neutral, Fill::Solid, cx), Size::Medium).into_any_element(), None, cx);
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(field("One-line, empty", i.empty.clone().into_any_element()))
        .child(field("One-line", i.line.clone().into_any_element()))
        .child(field("quiet", i.quiet.clone().into_any_element()))
        .child(field("quiet_frame (still)", quiet.into_any_element()))
        .child(field("inline", i.inline.clone().into_any_element()))
        .child(field("inline, invalid", i.invalid.clone().into_any_element()))
        .child(field("masked", i.masked.clone().into_any_element()))
        .child(field("multi-line", div().h(width(5.)).child(i.multi.clone()).into_any_element()))
        .child(group("form::input fields", cx).child(fields))
        .child(group("settings_page::row", cx).child(div().w(width(40.)).child(row_sample)))
        .into_any_element()
}

pub fn menu(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    g.menu.render(&g.hits, |_: &mut Gallery, _, _, _| {}, |_: &mut Gallery, _| {}, cx)
}

pub fn palette_row(s: &Sample, marks: &[Range<usize>], selected: bool, cx: &mut App) -> AnyElement {
    let muted = cx.theme().colors().text_muted;
    let text = crate::matcher::highlighted(format!("kit-palette-text-{}", s.key), s.text, marks, cx);
    div().flex().items_center().gap_2().px_3().py_1().when(selected, |d| d.font_weight(gpui::FontWeight::MEDIUM)).child(Icon::File.svg(12., muted)).child(text).into_any_element()
}

pub fn palette(g: &Gallery) -> AnyElement {
    div().flex().justify_center().child(g.palette.clone()).into_any_element()
}

pub fn lists(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let table = |n: usize, id: &str, caption_text: &str, cx: &mut Context<Gallery>| {
        let columns = vec![crate::table::Column::new("name", "Name", gpui::px(60.), gpui::px(180.)), crate::table::Column::new("size", "Size", gpui::px(40.), gpui::px(80.))];
        let widths = [gpui::px(180.), gpui::px(80.)];
        let grid = crate::table::Table::new(id.to_string(), &g.tables[n], &g.hits, columns, &widths, 6, |_: &mut Gallery, ix, col, _, _| {
            let text = if col == 0 { format!("Row {}", ix + 1) } else { format!("{} KB", (ix + 1) * 3) };
            div().px_2().child(text).into_any_element()
        });
        group(caption_text, cx).child(div().h(width(13.)).w(width(18.)).border_1().border_color(cx.theme().colors().border).child(grid.render(cx)))
    };
    let tree = |n: usize, id: &str, caption_text: &str, cx: &mut Context<Gallery>| {
        let list = crate::tree::TreeList::new(id.to_string(), &g.trees[n], &g.hits, |_: &mut Gallery, row: crate::tree::Row<'_, (), usize>, _, _| div().child(row.name.clone()).into_any_element());
        group(caption_text, cx).child(div().h(width(16.)).w(width(16.)).border_1().border_color(cx.theme().colors().border).child(list.render(cx)))
    };
    let selected = Some(crate::themes::selection(cx.theme().colors(), true));
    let rows = div().flex().flex_col().w(width(16.)).children((0..4).map(|i| {
        let fill = if i == 1 { selected } else { None };
        button::row(&g.hits, format!("kit-row-{i}"), fill, cx).px_2().py_1().child(format!("button::row {}", i + 1))
    }));
    let top = row().items_start().child(table(0, "kit-table-a", "table, the keys in it: selection active", cx)).child(table(1, "kit-table-b", "table without the keys: inactive selection", cx));
    let bottom = row().items_start().child(tree(0, "kit-tree-a", "tree (its focus frame gives it the keys)", cx)).child(tree(1, "kit-tree-b", "tree without the keys", cx)).child(group("button::row, the second selected", cx).child(rows));
    div().flex().flex_col().gap_4().child(top).child(bottom).into_any_element()
}

pub fn tabs(g: &Gallery, window: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let headers = (1..=24u64)
        .map(|id| crate::tabs::TabHeader { id, label: div().child(format!("Sample tab {id:02}")).into_any_element(), title: format!("Sample tab {id:02}").into(), tooltip: None })
        .collect();
    let fg = cx.theme().colors().text_muted;
    let trailing = button::icon(&g.hits, "kit-tab-new", Icon::Plus.svg(14., fg), gpui::px(26.), cx).into_any_element();
    let strip = crate::tabs::tab_strip(
        &g.hits,
        headers,
        Some(trailing),
        Some(24),
        || Icon::Close.svg(12., gpui::white()),
        |_: &mut Gallery, _, _| {},
        |_: &mut Gallery, _, _| {},
        |_: &mut Gallery, _, _, _| {},
        |_: &mut Gallery, _, _, _| {},
        window,
        cx,
    );
    let chip = crate::tabs::chip("A dragged tab's chip".into(), cx);
    div().flex().flex_col().gap_4().child(strip).child(group("tabs::chip", cx).child(row().child(chip))).into_any_element()
}

pub fn splits(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let panel = |words: &str| div().size_full().p_2().bg(colors.panel_background).child(words.to_string());
    let split = crate::split::SplitPane { id: "kit-split".into(), min_first: gpui::px(80.), min_second: gpui::px(80.), default: gpui::px(200.) };
    let split = div().relative().h(width(8.)).w(width(40.)).border_1().border_color(colors.border).child(
        split
            .render(div().w(g.split).h_full().child(panel("First pane")), div().flex_1().h_full().child(panel("Second pane")), |g: &mut Gallery, w, cx| {
                g.split = w;
                cx.notify();
            }, |_: &mut Gallery, _| {}, cx)
            .size_full(),
    );
    let bar = crate::split::HeightBar { id: "kit-height-bar".into(), min: gpui::px(40.), max: gpui::px(160.), default: gpui::px(80.) };
    let bar = div().w(width(40.)).border_1().border_color(colors.border).child(bar.render(panel("A box whose height the bar under it drags"), g.bar, &g.hits, |g: &mut Gallery, h, cx| {
        g.bar = h;
        cx.notify();
    }, |_: &mut Gallery, _| {}, cx));
    let header = |words: &str| div().px_2().py_1().text_xs().font_weight(gpui::FontWeight::BOLD).child(words.to_string()).into_any_element();
    let panes = vec![
        crate::split::Pane { header: header("OPEN PANE"), body: Some(panel("Its body").into_any_element()), height: gpui::px(0.) },
        crate::split::Pane { header: header("CLOSED PANE"), body: None, height: gpui::px(0.) },
        crate::split::Pane { header: header("ANOTHER OPEN PANE"), body: Some(panel("Its body").into_any_element()), height: gpui::px(0.) },
    ];
    let stack = crate::split::Stack { id: "kit-stack".into(), min: gpui::px(48.) };
    let stack = div().h(width(16.)).w(width(24.)).border_1().border_color(colors.border).child(stack.render(&g.stack, panes, &g.hits, |_: &mut Gallery, _, _| {}, |_: &mut Gallery, _| {}, cx));
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(group("SplitPane", cx).child(split))
        .child(group("HeightBar", cx).child(bar))
        .child(group("Stack: open, closed, open", cx).child(stack))
        .into_any_element()
}

pub fn dialog(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let question = crate::confirm::Confirm::ask("Remove the sample? Its words cannot be brought back.", "Remove", ()).or("Keep a copy", ()).danger().titled("Remove the sample");
    let behind = div().size_full().p_4().child(crate::placeholder::placeholder("What lies behind", ["The scrim covers it."], cx));
    let shown = g.dialog.ask(&g.hits, "kit-dialog", &question, |_: &mut Gallery, _, _, _| {}, cx);
    crate::dialog::over(behind, Some(shown)).into_any_element()
}

pub fn prompts(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let hits = &g.hits;
    let status = cx.theme().status().clone();
    let question = crate::confirm::Confirm::ask("Apply the sample change here?", "Apply", ()).or("Apply elsewhere", ());
    let band = question.render_band(hits, "kit-band", |_: &mut Gallery, _, _, _| {}, cx);
    let danger = crate::confirm::Confirm::ask("Discard the sample change?", "Discard", ()).danger();
    let danger_band = danger.render_band(hits, "kit-band-danger", |_: &mut Gallery, _, _, _| {}, cx);
    let form = crate::form::Form {
        key: "kit-form".into(),
        icon: Icon::FolderAdd,
        title: "A side form".into(),
        ok: "Add".into(),
        why: None,
        running: false,
        answers: crate::form::Answers { ok: |_: &mut Gallery, _| {}, close: |_: &mut Gallery, _| {} },
    };
    let fields = vec![(SharedString::from("Name"), crate::form::input(hits, "kit-form-name", &g.inputs.form_name)), (SharedString::from("Address"), crate::form::input(hits, "kit-form-url", &g.inputs.form_url))];
    let order = crate::focus_order::FocusOrder::new([g.inputs.form_name.focus_handle(cx), g.inputs.form_url.focus_handle(cx)]);
    let form = div().w(width(20.)).h(width(16.)).relative().child(form.render(hits, fields, order, None, cx));
    let dismiss = |key: String, cx: &App| button::icon(hits, key, Icon::Close.svg(12., cx.theme().colors().text_muted), gpui::px(18.), cx);
    let notices = crate::notices::render(vec![(Kind, "A plain notice with its ×".to_string()), (Kind, "Another notice".to_string())], status.warning, dismiss, |_: &mut Gallery, _, _, _| {}, cx);
    let banners = div()
        .flex()
        .flex_col()
        .gap_2()
        .w(width(30.))
        .child(crate::banner::tinted(status.info).child("tinted: info"))
        .child(crate::banner::tinted(status.success).child("tinted: success"))
        .child(crate::banner::tinted(status.error).child("tinted: error"))
        .child(crate::banner::plain(cx).child("plain"))
        .child(crate::banner::tinted(status.warning).child(crate::banner::marked(Icon::Warning.svg(14., status.warning), "marked: a mark beside text that wraps in the room left, short of a dismissable ×")))
        .children(notices);
    let toast = div().relative().w(width(30.)).h(width(4.)).border_1().border_color(cx.theme().colors().border).children(g.toast.render(hits, "kit-toast", cx));
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group("confirm band: a question, then one with a danger answer", cx).child(band).child(danger_band))
        .child(row().items_start().child(group("form", cx).child(form)).child(group("banner kinds, notices", cx).child(banners)).child(group("toast", cx).child(toast)))
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Kind;

impl crate::notices::NoticeKind for Kind {
    fn tag(self) -> &'static str {
        "kit"
    }
}

pub fn cards(g: &Gallery, window: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let hits = &g.hits;
    let card = crate::hover_card::tooltip(|| crate::hover_card::HoverCard { title: "A hover card".into(), rows: vec![("label", "value".into()), ("another", "longer value".into())] })(window, cx);
    let tooltip = (crate::tooltip::Tooltip::text("A plain tooltip"))(window, cx);
    let sections = [
        crate::details::Section::new("facts", "Facts").row("Label", "Value").path("Path", std::path::Path::new("C:/samples/folder/file.txt")).link("Link", "a sample link", "sample"),
        crate::details::Section::new("checks", "Checks").check(true, "A ticked line", Some("Its note".into())).check(false, "An open line", None),
        crate::details::Section::new("text", "Text").text("A paragraph of plain words.").long("long text\n  in the mono font"),
    ];
    let actions = vec![button::action(hits, "kit-details-act", "Act", Tone::Neutral, Fill::Solid, cx).into_any_element()];
    let page = crate::details::Page { key: "kit-details", title: "A details page".into(), pill: Some(("state".into(), cx.theme().status().success)), sections: &sections, actions, scroll: &g.scroll, on_link: None, top: None, over: None, bar: None };
    let page = div().w(width(24.)).h(width(30.)).border_1().border_color(cx.theme().colors().border).child(crate::details::render(page, hits, cx));
    let fg = crate::inbox_card::icon_color(cx);
    let inbox = |key: &str, kind: &str, badge: &str| {
        crate::inbox_card::InboxCard::new(key, kind, badge.to_string(), "A card's title, long enough to wrap over its two lines and then be cut at the end")
            .context("context")
            .line("line", "A one-line line", false)
            .line("muted", "A muted line", true)
            .action(crate::inbox_card::action(hits, format!("{key}-copy"), key.to_string(), Icon::Copy.svg(14., fg), "Copy", cx))
            .primary(crate::inbox_card::primary(hits, format!("{key}-open"), "Open", cx))
            .render(hits, cx)
    };
    let cards = div().flex().flex_col().gap_2().w(width(22.)).child(inbox("kit-inbox-a", "question", "question")).child(inbox("kit-inbox-b", "run failed", "failed")).child(inbox("kit-inbox-c", "note", "note"));
    let placeholder = div().w(width(20.)).h(width(8.)).border_1().border_color(cx.theme().colors().border).flex().child(crate::placeholder::placeholder("Nothing to show", ["A muted detail line.", "Another."], cx).size_full());
    let left = div().flex().flex_col().gap_3().w(width(20.)).child(group("hover_card", cx).child(card)).child(group("tooltip", cx).child(tooltip)).child(group("placeholder", cx).child(placeholder));
    row().items_start().child(left).child(group("details page", cx).child(page)).child(group("inbox cards: warning, error, info kinds", cx).child(cards)).into_any_element()
}

pub fn marks(cx: &mut Context<Gallery>) -> AnyElement {
    let status = cx.theme().status().clone();
    let colors: [(&str, Hsla); 4] = [("success", status.success), ("warning", status.warning), ("error", status.error), ("info", status.info)];
    let dots = row().children(colors.iter().map(|(n, c)| div().flex().items_center().gap_1().child(crate::marks::dot(Mark::Filled(*c))).child(crate::marks::dot(Mark::Ring(*c))).child(n.to_string())));
    let muted = crate::themes::secondary(cx.theme().colors());
    let activity = row().children([Activity::Working, Activity::Waiting, Activity::Idle].map(|a| div().flex().items_center().gap_1().child(crate::marks::dot(Mark::Filled(a.color(cx)))).child(a.word())));
    let in_color = row().child(crate::marks::dot(Mark::Filled(status.success).in_color(muted))).child(crate::marks::dot(Mark::Ring(status.warning).in_color(muted))).child("in_color, muted");
    let few: Vec<Mark> = (0..12).map(|i| match i % 4 {
        0 => Mark::Filled(status.success),
        1 => Mark::Filled(status.warning),
        2 => Mark::Ring(status.info),
        _ => Mark::Filled(status.error),
    }).collect();
    let many: Vec<Mark> = (0..crate::marks::SQUARES_MAX + 20).map(|i| if i % 3 == 0 { Mark::Filled(status.success) } else { Mark::Ring(status.info) }).collect();
    let track = cx.theme().colors().border_variant;
    let line = |parts: &[(u32, Hsla)], total: u32| div().w(width(24.)).child(crate::marks::progress_line(parts, total, track));
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(group("dot: filled and ring per status colour", cx).child(dots))
        .child(group("Activity", cx).child(activity))
        .child(group("Mark::in_color", cx).child(in_color))
        .child(group("squares: a few, then past SQUARES_MAX (a plain bar)", cx).child(div().w(width(24.)).child(crate::marks::squares(&few))).child(div().w(width(24.)).child(crate::marks::squares(&many))))
        .child(group("progress_line: shares, then one part of a larger total", cx).child(line(&[(3, status.success), (1, status.warning), (2, status.error)], 6)).child(line(&[(2, status.info)], 9)))
        .into_any_element()
}

pub fn text(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let hits = &g.hits;
    let colors = cx.theme().colors().clone();
    let muted = crate::themes::secondary(&colors);
    let boxed = |key: &str, w: f32, label: AnyElement| div().relative().w(width(w)).px_1().border_1().border_color(colors.border_variant).child(label).child(hit_target(hits, key.to_string()));
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group("clipped: whole, cut at the end, cut in the middle", cx)
            .child(boxed("kit-clip-whole", 30., crate::clipped::clipped("kit-clip-whole-label", "A label that fits", false).into_any_element()))
            .child(boxed("kit-clip-end", 14., crate::clipped::clipped("kit-clip-end-label", LONG, false).into_any_element()))
            .child(boxed("kit-clip-middle", 14., crate::clipped::clipped("kit-clip-middle-label", LONG, true).into_any_element())))
        .child(group("path_label: whole, then its folder cut", cx)
            .child(boxed("kit-path-whole", 40., crate::clipped::path_label("kit-path-whole-dir", PATH, muted, colors.text).into_any_element()))
            .child(boxed("kit-path-long", 16., crate::clipped::path_label("kit-path-long-dir", PATH, muted, colors.text).into_any_element())))
        .child(group("pair: bright then muted, the muted cut first", cx)
            .child(boxed("kit-pair-whole", 30., crate::clipped::pair("kit-pair-whole-label", "bright", div().child("bright"), "muted words after it", muted).into_any_element()))
            .child(boxed("kit-pair-cut", 10., crate::clipped::pair("kit-pair-cut-label", "bright", div().child("bright"), "muted words after it", muted).into_any_element())))
        .child(group("clamped: two lines, the last cut", cx).child(boxed("kit-clamped", 14., crate::clipped::clamped("kit-clamped-label", LONG, 2).into_any_element())))
        .into_any_element()
}
