//! Each section's samples.

use std::ops::Range;

use gpui::{AnyElement, App, Context, Div, Entity, Focusable as _, Hsla, IntoElement, Rems, SharedString, Window, div, prelude::*, rems};
use theme::ActiveTheme as _;

use super::{Gallery, Sample};
use crate::button::{self, Button, ButtonSplit, Fill, Size, Tone};
use crate::checkbox::Checkbox;
use crate::label::{Label, LabelMark};
use crate::hit::hit_target;
use crate::text_input::TextInput;
use crate::icon::Icon;
use crate::marks::{Activity, Mark};
use crate::tokens;

pub const LONG: &str = "A label far longer than the room it is given, cut with an ellipsis";
pub const PATH: &str = "crates/sample-kit/src/a/deeply/nested/folder/module_file.rs";
/// What the cut [`PATH`]'s tooltip says: its folder.
pub const PATH_FOLDER: &str = "crates/sample-kit/src/a/deeply/nested/folder/";
pub const TREE: [&str; 6] = ["docs/guide.md", "docs/notes/today.md", "src/main.rs", "src/view/list.rs", "src/view/row.rs", "README.md"];

pub(super) fn group(caption: &str, cx: &App) -> Div {
    let muted = crate::themes::secondary(cx.theme().colors());
    div().flex().flex_col().gap(tokens::space::L).child(div().text_size(tokens::text::SMALL).text_color(muted).child(caption.to_string()))
}

pub(super) fn row() -> Div {
    div().flex().flex_wrap().items_center().gap(tokens::space::XL)
}

fn caption(text: &str, cx: &App) -> Div {
    div().text_size(tokens::text::SMALL).text_color(crate::themes::secondary(cx.theme().colors())).child(text.to_string())
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
    let mut grid = div().flex().flex_col().gap(tokens::space::XL);
    for size in Size::ALL {
        let mut line = row().child(caption(if size == Size::Small { "Small" } else { "Medium" }, cx).w(width(4.)));
        for tone in [Tone::Neutral, Tone::Success, Tone::Danger] {
            for fill in [Fill::Solid, Fill::Outline] {
                let label = format!("{tone:?} {fill:?}");
                line = line.child(Button::new(hits, key(tone, fill, size)).label(label).tone(tone).fill(fill).size(size));
            }
        }
        let dimmed = Button::new(hits, format!("kit-dimmed-{size:?}")).label("Dimmed").size(size).dimmed_if(true);
        grid = grid.child(line.child(dimmed));
    }
    let focused = Button::new(hits, "kit-focusable").label("Focusable").focus(&g.members.handle("kit-focusable", cx), |_, _, _| {});
    let with = row()
        .child(Button::new(hits, "kit-with-icon").icon(Icon::Play).label("With icon"))
        .child(Button::new(hits, "kit-turning").icon(Icon::Refresh).turning(true).label("Turning").tone(Tone::Success))
        .child(Button::new(hits, "kit-with-dimmed").icon(Icon::Trash).label("Dimmed").tone(Tone::Danger).fill(Fill::Outline).dimmed_if(true))
        .child(focused)
        .child(Button::line().child(Button::new(hits, "kit-line-a").label("In a line")).child(Button::new(hits, "kit-line-b").label("GAP apart").fill(Fill::Outline)));
    let fg = cx.theme().colors().text_muted;
    let side = gpui::px(22.);
    let square = |key: &str, svg: gpui::Svg| Button::new(hits, key.to_string()).icon(svg).icon_only(side);
    let icons = row()
        .child(square("kit-icon", Icon::Settings.svg(tokens::icon::M, fg)))
        .child(square("kit-icon-neutral", Icon::Copy.svg(tokens::icon::M, fg)).tone(Tone::Neutral))
        .child(square("kit-icon-success", Icon::Check.svg(tokens::icon::M, cx.theme().status().success)).tone(Tone::Success))
        .child(square("kit-icon-danger", Icon::Trash.svg(tokens::icon::M, cx.theme().status().error)).tone(Tone::Danger))
        .child(square("kit-icon-dimmed", Icon::Close.svg(tokens::icon::M, fg)).dimmed_if(true))
        .child(div().id("kit-revealed-row").group("kit-revealed-row").flex().items_center().gap(tokens::space::S).child("Revealed:").child(square("kit-revealed", Icon::Star.svg(tokens::icon::M, fg)).reveal("kit-revealed-row", true)));
    let bar = |key: &str, svg: gpui::Svg| Button::toolbar(hits, key.to_string()).icon(svg).text_color(fg);
    let toolbars = row()
        .child(bar("kit-toolbar", Icon::Refresh.svg(tokens::icon::M, fg)).child("Toolbar"))
        .child(bar("kit-toolbar-on", Icon::GitGraph.svg(tokens::icon::M, fg)).on(true).child("On"))
        .child(bar("kit-toolbar-turning", Icon::Refresh.svg(tokens::icon::M, fg)).turning(true).child("Turning"))
        .child(ButtonSplit::new(bar("kit-split-main", Icon::Plus.svg(tokens::icon::M, fg)), bar("kit-split-menu", Icon::ChevronDown.svg(tokens::icon::S, fg))))
        .child(button::ghost(hits, "kit-ghost", cx).px(tokens::space::L).py(tokens::space::S).rounded(tokens::radius::S).child("Ghost"));
    grid.child(group(".icon, .turning, .dimmed, .focus (ringed while focused), Button::line", cx).child(with))
        .child(group(".icon_only, toned (Neutral, Success, Danger), .dimmed, .reveal", cx).child(icons))
        .child(group("Button::toolbar (off, .on, .turning), ButtonSplit, ghost", cx).child(toolbars))
        .into_any_element()
}

pub fn toggles(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let hits = &g.hits;
    let checks = row()
        .child(Checkbox::new(hits, "kit-check-off", false).label("Off"))
        .child(Checkbox::new(hits, "kit-check-on", true).label("On"))
        .child(Checkbox::new(hits, "kit-check-focus", true).label("Focusable").focus(&g.members.handle("kit-check-focus", cx)))
        .child(Checkbox::new(hits, "kit-check-disabled", false).label("Disabled").disabled(Some("Why it cannot change now")))
        .child(Label::new("tick, read only").mark(LabelMark::Tick(true)));
    let fg = cx.theme().colors().text_muted;
    let pills = row().child(button::pill(hits, "kit-pill", "pill", fg, cx)).child(button::pill(hits, "kit-pill-accent", "accent", cx.theme().colors().text_accent, cx));
    let segments = [
        crate::segmented::Segment::new("one").label("One"),
        crate::segmented::Segment::new("two").icon(Icon::ListTree).label("Two"),
        crate::segmented::Segment::new("three").icon(Icon::ListFlat),
    ];
    let segmented = crate::segmented::segmented(hits, "kit-seg", &segments, 0, |_: &mut Gallery, _, _, _| {}, cx);
    let stepper = button::stepper(hits, "kit-stepper", "20", &g.members, |_: &mut Gallery, _, _, _| {}, cx);
    let list = div().flex().flex_col().gap(tokens::space::S).w(width(14.)).children((0..3).map(|i| {
        button::choice(hits, format!("kit-choice-{i}"), format!("Choice {}", i + 1), i == 0, cx)
    }));
    let choices = button::choices(list, &g.members.handle("kit-choices", cx), 3, Some(0), |_: &mut Gallery, _, _, _| {}, cx);
    div()
        .flex()
        .flex_col()
        .gap(tokens::space::XXL)
        .child(group("Checkbox (off, on, focusable, disabled), a tick as a Label's mark", cx).child(checks))
        .child(group("pill", cx).child(pills))
        .child(group("segmented, the first chosen", cx).child(row().child(segmented)))
        .child(group("stepper", cx).child(stepper))
        .child(group("choices: a list of choice, the first chosen", cx).child(choices))
        .into_any_element()
}

pub fn inputs(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let i = &g.inputs;
    let field = |caption: &str, input: AnyElement| div().flex().items_center().gap(tokens::space::XL).child(div().w(width(9.)).text_size(tokens::text::BODY).child(caption.to_string())).child(div().w(width(22.)).child(input));
    let quiet = crate::text_input::quiet_frame(false, cx).w_full().child(div().px(tokens::space::S).child("A quiet field's still copy"));
    let form_fields = vec![
        (SharedString::from("Name"), crate::form::input(&g.hits, "kit-form-name", &i.form_name)),
        (SharedString::from("Address"), crate::form::input(&g.hits, "kit-form-url", &i.form_url)),
    ];
    let fields = div().flex().flex_col().gap(tokens::space::L).children(form_fields.into_iter().map(|(label, input)| field(&label, input)));
    let row_sample = crate::settings_page::row("A setting", "What the setting does, in one line under its title.", Button::new(&g.hits, "kit-setting").label("Change").size(Size::Medium).into_any_element(), None, cx);
    div()
        .flex()
        .flex_col()
        .gap(tokens::space::XL)
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
    div().flex().items_center().gap(tokens::space::L).px(tokens::space::XL).py(tokens::space::S).when(selected, |d| d.font_weight(gpui::FontWeight::MEDIUM)).child(Icon::File.svg(tokens::icon::S, muted)).child(text).into_any_element()
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
            div().px(tokens::space::L).child(text).into_any_element()
        });
        group(caption_text, cx).child(div().h(width(13.)).w(width(18.)).border(tokens::stroke::HAIRLINE).border_color(cx.theme().colors().border).child(grid.render(cx)))
    };
    let tree = |n: usize, id: &str, caption_text: &str, cx: &mut Context<Gallery>| {
        let list = crate::tree::TreeList::new(id.to_string(), &g.trees[n], &g.hits, |_: &mut Gallery, row: crate::tree::Row<'_, (), usize>, _, _| div().child(row.name.clone()).into_any_element());
        group(caption_text, cx).child(div().h(width(16.)).w(width(16.)).border(tokens::stroke::HAIRLINE).border_color(cx.theme().colors().border).child(list.render(cx)))
    };
    let selected = Some(crate::themes::selection(cx.theme().colors(), true));
    let rows = div().flex().flex_col().w(width(16.)).children((0..4).map(|i| {
        let fill = if i == 1 { selected } else { None };
        button::row(&g.hits, format!("kit-row-{i}"), fill, cx).px(tokens::space::L).py(tokens::space::S).child(format!("button::row {}", i + 1))
    }));
    let states = [("at rest", false, false), ("selected, the keys in it", true, true), ("selected, without the keys", true, false), ("at rest", false, false)];
    let core = div().flex().flex_col().w(width(16.)).children(states.iter().enumerate().map(|(i, (words, selected, focused))| {
        let chrome = crate::list::Chrome::new().selected(*selected).focused(*focused);
        crate::list::row(&g.hits, button::RowKeys::one(format!("kit-list-{i}").into()), chrome, cx).px(tokens::space::L).py(tokens::space::S).child(*words)
    }));
    let top = row().items_start().child(table(0, "kit-table-a", "table, the keys in it: selection active", cx)).child(table(1, "kit-table-b", "table without the keys: inactive selection", cx));
    let menu = g.row_menu.as_ref().map(|m| m.render(&g.hits, |_: &mut Gallery, _, _, _| {}, |_: &mut Gallery, _| {}, cx));
    let bottom = row().items_start().child(tree(0, "kit-tree-a", "tree (its focus frame gives it the keys)", cx)).child(tree(1, "kit-tree-b", "tree without the keys", cx)).child(group("button::row, the second selected", cx).child(rows)).child(group("list::row: the core's chrome (keyed lists, the sidebar)", cx).child(core));
    div().flex().flex_col().gap(tokens::space::XXL).child(top).child(bottom).children(menu).into_any_element()
}

pub fn tabs(g: &Gallery, window: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let headers = (1..=24u64)
        .map(|id| crate::tabs::TabHeader { id, label: div().child(format!("Sample tab {id:02}")).into_any_element(), title: format!("Sample tab {id:02}").into(), tooltip: None })
        .collect();
    let fg = cx.theme().colors().text_muted;
    let trailing = Button::new(&g.hits, "kit-tab-new").icon(Icon::Plus.svg(tokens::icon::M, fg)).icon_only(gpui::px(26.)).into_any_element();
    let strip = crate::tabs::tab_strip(
        &g.hits,
        headers,
        Some(trailing),
        Some(24),
        || Icon::Close.svg(tokens::icon::S, gpui::white()),
        |_: &mut Gallery, _, _| {},
        |_: &mut Gallery, _, _| {},
        |_: &mut Gallery, _, _, _| {},
        |_: &mut Gallery, _, _, _| {},
        window,
        cx,
    );
    div().flex().flex_col().gap(tokens::space::XXL).child(strip).into_any_element()
}

pub fn splits(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let panel = |words: &str| div().size_full().p(tokens::space::L).bg(colors.panel_background).child(words.to_string());
    let split = crate::split::SplitPane { id: "kit-split".into(), min_first: gpui::px(80.), min_second: gpui::px(80.), default: gpui::px(200.) };
    let split = div().relative().h(width(8.)).w(width(40.)).border(tokens::stroke::HAIRLINE).border_color(colors.border).child(
        split
            .render(div().w(g.split).h_full().child(panel("First pane")), div().flex_1().h_full().child(panel("Second pane")), |g: &mut Gallery, w, cx| {
                g.split = w;
                cx.notify();
            }, |_: &mut Gallery, _| {}, cx)
            .size_full(),
    );
    let bar = crate::split::HeightBar { id: "kit-height-bar".into(), min: gpui::px(40.), max: gpui::px(160.), default: gpui::px(80.) };
    let bar = div().w(width(40.)).border(tokens::stroke::HAIRLINE).border_color(colors.border).child(bar.render(panel("A box whose height the bar under it drags"), g.bar, &g.hits, |g: &mut Gallery, h, cx| {
        g.bar = h;
        cx.notify();
    }, |_: &mut Gallery, _| {}, cx));
    let header = |words: &str| div().px(tokens::space::L).py(tokens::space::S).text_size(tokens::text::SMALL).font_weight(gpui::FontWeight::BOLD).child(words.to_string()).into_any_element();
    let panes = vec![
        crate::split::Pane { header: header("OPEN PANE"), body: Some(panel("Its body").into_any_element()), height: gpui::Pixels::ZERO },
        crate::split::Pane { header: header("CLOSED PANE"), body: None, height: gpui::Pixels::ZERO },
        crate::split::Pane { header: header("ANOTHER OPEN PANE"), body: Some(panel("Its body").into_any_element()), height: gpui::Pixels::ZERO },
    ];
    let stack = crate::split::Stack { id: "kit-stack".into(), min: gpui::px(48.) };
    let stack = div().h(width(16.)).w(width(24.)).border(tokens::stroke::HAIRLINE).border_color(colors.border).child(stack.render(&g.stack, panes, &g.hits, |_: &mut Gallery, _, _| {}, |_: &mut Gallery, _| {}, cx));
    div()
        .flex()
        .flex_col()
        .gap(tokens::space::XXL)
        .child(group("SplitPane", cx).child(split))
        .child(group("HeightBar", cx).child(bar))
        .child(group("Stack: open, closed, open", cx).child(stack))
        .into_any_element()
}

/// The upstream prompt's shape: the remote picked, then the branch on it.
fn upstream(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let hits = &g.hits;
    let segments = [crate::segmented::Segment::new("origin").label("origin"), crate::segmented::Segment::new("fork").label("fork")];
    let remotes = crate::segmented::segmented(hits, "kit-upstream", &segments, 0, |_: &mut Gallery, _, _, _| {}, cx);
    let muted = crate::themes::secondary(cx.theme().colors());
    let line = div().flex().items_center().gap(tokens::space::L).child(remotes).child(div().text_color(muted).child("/")).child(crate::form::input(hits, "kit-upstream-field", &g.inputs.upstream));
    let body = div().flex().flex_col().gap(tokens::space::XL).child("Push main with no upstream to").child(line);
    let ok = crate::dialog::Button::new("kit-upstream-ok", "Push", Tone::Neutral, |_: &mut Gallery, _, _| {});
    let cancel = crate::dialog::Button::new("kit-upstream-cancel", "Cancel", Tone::Neutral, |_: &mut Gallery, _, _| {});
    let panel = crate::dialog::Panel {
        key: "kit-upstream".into(),
        title: Some("Set upstream".into()),
        body: body.into_any_element(),
        fields: vec![g.inputs.upstream.focus_handle(cx)],
        buttons: vec![ok, cancel],
        first: None,
        default: Some("kit-upstream-ok".into()),
        close: None,
        cancel: std::rc::Rc::new(|_: &mut Gallery, _, _| {}),
    };
    g.prompts[3].render(hits, panel, cx)
}

pub fn dialog(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let question = crate::confirm::Confirm::ask("Remove the sample? Its words cannot be brought back.", "Remove", ()).or("Keep a copy", ()).danger().titled("Remove the sample");
    let behind = div().size_full().p(tokens::space::XXL).child(crate::placeholder::placeholder("What lies behind", ["The scrim covers it."], cx));
    let shown = g.dialog.ask(&g.hits, "kit-dialog", &question, |_: &mut Gallery, _, _, _| {}, cx);
    crate::dialog::over(behind, Some(shown)).into_any_element()
}

pub fn prompts(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let hits = &g.hits;
    let status = cx.theme().status().clone();
    let question = crate::confirm::Confirm::ask("Apply the sample change here?", "Apply", ()).or("Apply elsewhere", ());
    let asked = g.prompts[0].ask(hits, "kit-ask", &question, |_: &mut Gallery, _, _, _| {}, cx);
    let danger = crate::confirm::Confirm::ask("Discard the sample change? It cannot be brought back.", "Discard", ()).danger();
    let danger = g.prompts[1].ask(hits, "kit-ask-danger", &danger, |_: &mut Gallery, _, _, _| {}, cx);
    let form = crate::form::Form {
        key: "kit-form".into(),
        title: "Add Sample".into(),
        ok: "Add".into(),
        why: None,
        running: false,
        answers: crate::form::Answers { ok: |_: &mut Gallery, _| {}, close: |_: &mut Gallery, _| {} },
    };
    let fields = vec![(SharedString::from("Name"), crate::form::input(hits, "kit-form-name", &g.inputs.form_name)), (SharedString::from("Address"), crate::form::input(hits, "kit-form-url", &g.inputs.form_url))];
    let members = vec![g.inputs.form_name.focus_handle(cx), g.inputs.form_url.focus_handle(cx)];
    let form = form.render(&g.prompts[2], hits, fields, members, None, cx);
    let upstream = upstream(g, cx);
    let frame = |caption: &str, prompt: AnyElement, cx: &App| {
        let behind = div().size_full().p(tokens::space::XXL).child(crate::placeholder::placeholder("The tab behind", ["Its keys wait."], cx));
        group(caption, cx).child(div().w(width(36.)).h(width(20.)).border(tokens::stroke::HAIRLINE).border_color(cx.theme().colors().border).child(crate::dialog::over(behind, Some(prompt))))
    };
    let dismiss = |key: String, cx: &App| Button::new(hits, key).icon(Icon::Close.svg(tokens::icon::S, cx.theme().colors().text_muted)).icon_only(gpui::px(18.));
    let notices = crate::notices::render(vec![(Kind, "A plain notice with its ×".to_string()), (Kind, "Another notice".to_string())], status.warning, dismiss, |_: &mut Gallery, _, _, _| {}, cx);
    let banners = div()
        .flex()
        .flex_col()
        .gap(tokens::space::L)
        .w(width(30.))
        .child(crate::banner::tinted(status.info).child("tinted: info"))
        .child(crate::banner::tinted(status.success).child("tinted: success"))
        .child(crate::banner::tinted(status.error).child("tinted: error"))
        .child(crate::banner::plain(cx).child("plain"))
        .child(crate::banner::tinted(status.warning).child(crate::banner::marked(Icon::Warning.svg(tokens::icon::M, status.warning), "marked: a mark beside text that wraps in the room left, short of a dismissable ×")))
        .children(notices);
    let toast = div().relative().w(width(30.)).h(width(4.)).border(tokens::stroke::HAIRLINE).border_color(cx.theme().colors().border).children(g.toast.render(hits, "kit-toast", cx));
    div()
        .flex()
        .flex_col()
        .gap(tokens::space::XL)
        .child(row().items_start().child(frame("a question", asked, cx)).child(frame("a danger question: Enter cancels", danger, cx)))
        .child(row().items_start().child(frame("a small form", form, cx)).child(frame("the upstream prompt", upstream, cx)))
        .child(row().items_start().child(group("banner kinds, notices", cx).child(banners)).child(group("toast", cx).child(toast)))
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
    let actions = vec![Button::new(hits, "kit-details-act").label("Act").into_any_element()];
    let page = crate::details::Page { key: "kit-details", title: "A details page".into(), pill: Some(("state".into(), cx.theme().status().success)), sections: &sections, actions, scroll: &g.scroll, on_link: None, top: None, over: None, bar: None };
    let page = div().w(width(24.)).h(width(30.)).border(tokens::stroke::HAIRLINE).border_color(cx.theme().colors().border).child(crate::details::render(page, hits, cx));
    let fg = crate::inbox_card::icon_color(cx);
    let status = cx.theme().status().clone();
    let inbox = |key: &str, color: Hsla, badge: &str| {
        crate::inbox_card::InboxCard::new(key, color, badge.to_string(), "A card's title, long enough to wrap over its two lines and then be cut at the end")
            .context("context")
            .line("line", "A one-line line", false)
            .line("muted", "A muted line", true)
            .action(crate::inbox_card::action(hits, format!("{key}-copy"), key.to_string(), Icon::Copy.svg(tokens::icon::M, fg), "Copy", cx))
            .primary(crate::inbox_card::primary(hits, format!("{key}-open"), "Open", cx))
            .render(hits, cx)
    };
    let cards = div().flex().flex_col().gap(tokens::space::L).w(width(22.)).child(inbox("kit-inbox-a", status.warning, "question")).child(inbox("kit-inbox-b", status.error, "failed")).child(inbox("kit-inbox-c", status.info, "note"));
    let placeholder = div().w(width(20.)).h(width(8.)).border(tokens::stroke::HAIRLINE).border_color(cx.theme().colors().border).flex().child(crate::placeholder::placeholder("Nothing to show", ["A muted detail line.", "Another."], cx).size_full());
    let left = div().flex().flex_col().gap(tokens::space::XL).w(width(20.)).child(group("hover_card", cx).child(card)).child(group("tooltip", cx).child(tooltip)).child(group("placeholder", cx).child(placeholder));
    row().items_start().child(left).child(group("details page", cx).child(page)).child(group("inbox cards: warning, error, info kinds", cx).child(cards)).into_any_element()
}

pub fn marks(cx: &mut Context<Gallery>) -> AnyElement {
    let status = cx.theme().status().clone();
    let colors: [(&str, Hsla); 4] = [("success", status.success), ("warning", status.warning), ("error", status.error), ("info", status.info)];
    let pair = |a: Mark, b: Mark| LabelMark::Element(div().flex().gap(tokens::space::S).child(crate::marks::dot(a)).child(crate::marks::dot(b)).into_any_element());
    let dots = row().children(colors.iter().map(|(n, c)| Label::new(n.to_string()).mark(pair(Mark::Filled(*c), Mark::Ring(*c)))));
    let muted = crate::themes::secondary(cx.theme().colors());
    let activity = row().children([Activity::Working, Activity::Waiting, Activity::Idle].map(|a| Label::new(a.word()).mark(Mark::Filled(a.color(cx)))));
    let in_color = row().child(Label::new("in_color, muted").mark(pair(Mark::Filled(status.success).in_color(muted), Mark::Ring(status.warning).in_color(muted))));
    let labels = row()
        .child(Label::new("an icon").mark(Icon::Folder))
        .child(Label::new("a dot").mark(Mark::Filled(status.success)))
        .child(Label::new("a tick").mark(LabelMark::Tick(false)))
        .child(Label::new("small, muted").mark(Icon::GitBranch).size(tokens::text::SMALL).muted())
        .child(div().w(width(8.)).child(Label::new(LONG).mark(Icon::Star).clipped("kit-label-cut", false)));
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
        .gap(tokens::space::XXL)
        .child(group("Label: a mark on the letters' middle, sized and muted with its text, cut", cx).child(labels))
        .child(group("dot: filled and ring per status colour", cx).child(dots))
        .child(group("Activity", cx).child(activity))
        .child(group("Mark::in_color", cx).child(in_color))
        .child(group("squares: a few, then past SQUARES_MAX (a plain bar)", cx).child(div().w(width(24.)).child(crate::marks::squares(&few))).child(div().w(width(24.)).child(crate::marks::squares(&many))))
        .child(group("progress_line: shares, then one part of a larger total", cx).child(line(&[(3, status.success), (1, status.warning), (2, status.error)], 6)).child(line(&[(2, status.info)], 9)))
        .into_any_element()
}

pub const SELECTED: &str = "Read-only text selects with the mouse, Shift+arrows and Ctrl+A, and Ctrl+C copies the selection.";
pub const SELECTION: Range<usize> = 15..43;
pub const SUMMARY: &str = "fix(dolomiti): skipass error codes are camelCase on the wire and in the widget";

pub fn text(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let hits = &g.hits;
    let colors = cx.theme().colors().clone();
    let muted = crate::themes::secondary(&colors);
    let boxed = |key: &str, w: f32, label: AnyElement| div().relative().w(width(w)).px(tokens::space::S).border(tokens::stroke::HAIRLINE).border_color(colors.border_variant).child(label).child(hit_target(hits, key.to_string()));
    let summary = |input: &Entity<TextInput>| div().text_size(tokens::text::TITLE).font_weight(gpui::FontWeight::MEDIUM).p(tokens::space::L).child(input.clone()).into_any_element();
    div()
        .flex()
        .flex_col()
        .gap(tokens::space::XL)
        .child(group("TextView: a selection, inactive here and active under the keys", cx).child(boxed("kit-text-view", 30., g.inputs.view.clone().into_any_element())))
        .child(group("TextBox: read only, then editable, the same box", cx)
            .child(boxed("kit-text-read", 22., summary(&g.inputs.pair[0])))
            .child(boxed("kit-text-edit", 22., summary(&g.inputs.pair[1]))))
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

/// The scroll section's scrollers.
#[derive(Default)]
pub struct Scrolls {
    list: crate::keyed_list::KeyedList,
    grid: gpui::UniformListScrollHandle,
}

const SCROLL_ROWS: usize = 40;

pub fn scroll(g: &mut Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    g.scrolls.list.set_rows((0..SCROLL_ROWS).map(|i| (SharedString::from(format!("row-{i}")), Vec::new())).collect());
    let line = |text: String| div().flex_none().px(tokens::space::L).py(tokens::space::XS).child(text);
    let boxed = |caption_text: &str, key: &str, body: AnyElement, cx: &App| {
        let frame = div().relative().h(width(20.)).w(width(13.)).border(tokens::stroke::HAIRLINE).border_color(cx.theme().colors().border).child(body).child(hit_target(&g.hits, key.to_string()));
        group(caption_text, cx).child(frame)
    };
    let list = gpui::list(g.scrolls.list.state(), |ix, _, _| div().px(tokens::space::L).py(tokens::space::XS).child(format!("List row {}", ix + 1)).into_any_element()).size_full();
    let list = div().relative().size_full().child(list).child(g.scrolls.list.scrollbar("kit-scroll-list-bar").hit(&g.hits, "kit-scroll-list-bar"));
    let rows = |n: usize, word: &str| (0..n).map(|i| line(format!("{word} {}", i + 1))).collect::<Vec<_>>();
    let long = crate::scroll::area("kit-scroll-div-area").hit(&g.hits, "kit-scroll-div-bar").size_full().flex().flex_col().children(rows(SCROLL_ROWS, "Box line"));
    let inner = crate::scroll::area("kit-scroll-inner").hit(&g.hits, "kit-scroll-inner-bar").flex_none().h(width(8.)).mx(tokens::space::L).border(tokens::stroke::HAIRLINE).border_color(colors.border).flex().flex_col().children(rows(16, "Inner line"));
    let nested = crate::scroll::area("kit-scroll-outer").hit(&g.hits, "kit-scroll-outer-bar").size_full().flex().flex_col().children(rows(3, "Outer line")).child(inner).children(rows(SCROLL_ROWS, "Outer line"));
    let grid = gpui::uniform_list("kit-scroll-grid-rows", 200, |range, _, _| range.map(|i| div().px(tokens::space::L).child(format!("{i:>4}  0x{:08x}  ........", i * 0x1f3)).into_any_element()).collect())
        .track_scroll(&g.scrolls.grid)
        .size_full()
        .font_family(crate::fonts::MONO)
        .text_size(tokens::text::SMALL);
    let grid = div().relative().size_full().child(grid).child(crate::scroll::Scrollbar::new("kit-scroll-grid-bar", &g.scrolls.grid).hit(&g.hits, "kit-scroll-grid-bar"));
    let always = crate::scroll::area("kit-scroll-always").hit(&g.hits, "kit-scroll-always-bar").mode(crate::scroll::Scrollbars::Always).size_full().flex().flex_col().children(rows(SCROLL_ROWS, "Always line"));
    row()
        .items_start()
        .child(boxed("gpui::list", "kit-scroll-list", list.into_any_element(), cx))
        .child(boxed("scroll::area", "kit-scroll-div", long.into_any_element(), cx))
        .child(boxed("nested areas", "kit-scroll-nested", nested.into_any_element(), cx))
        .child(boxed("uniform_list grid", "kit-scroll-grid", grid.into_any_element(), cx))
        .child(boxed("always", "kit-scroll-always-box", always.into_any_element(), cx))
        .into_any_element()
}
