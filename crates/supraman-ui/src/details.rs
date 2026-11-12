//! A details page: a title with a status pill, sections of `label  value` rows and paragraphs, and an actions
//! bar under them. The owner builds the [`Section`]s from its data and keeps the [`ScrollHandle`].

use std::rc::Rc;

use gpui::{AnyElement, ClickEvent, ClipboardItem, Context, Div, FontWeight, Hsla, ScrollHandle, SharedString, Window, div, prelude::*, px};
use theme::{ActiveTheme as _, ThemeColors};

use crate::banner;
use crate::button;
use crate::clipped::path_label;
use crate::hit::{HitTargets, hit_target};

const LABEL: f32 = 128.;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// `label  value`.
    Row(String, String),
    /// `label  path`, the path in `/`s, its folder muted and cut ([`crate::clipped::path_label`]).
    Path(String, String),
    Text(String),
    /// Wrapped as written, in the mono font, with a Copy button.
    Long(String),
    /// `label  value`, the value a link: a click passes `target` to the page's `on_link`.
    Link { label: String, value: String, target: String },
    /// A tick, ticked when `done`, before `text`, with any `note` muted under it.
    Check { done: bool, text: String, note: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Its hit key is `<page key>-section-<key>`.
    pub key: &'static str,
    pub caption: String,
    pub lines: Vec<Line>,
}

impl Section {
    pub fn new(key: &'static str, caption: impl Into<String>) -> Self {
        Self { key, caption: caption.into(), lines: Vec::new() }
    }

    pub fn row(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.lines.push(Line::Row(label.into(), value.into()));
        self
    }

    pub fn path(mut self, label: impl Into<String>, path: &std::path::Path) -> Self {
        self.lines.push(Line::Path(label.into(), supraman_core::paths::normalize(path)));
        self
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.lines.push(Line::Text(text.into()));
        self
    }

    pub fn long(mut self, text: impl Into<String>) -> Self {
        self.lines.push(Line::Long(text.into()));
        self
    }

    pub fn link(mut self, label: impl Into<String>, value: impl Into<String>, target: impl Into<String>) -> Self {
        self.lines.push(Line::Link { label: label.into(), value: value.into(), target: target.into() });
        self
    }

    pub fn check(mut self, done: bool, text: impl Into<String>, note: Option<String>) -> Self {
        self.lines.push(Line::Check { done, text: text.into(), note });
        self
    }
}

/// The sections as plain text: a caption line, then a line per row or paragraph.
pub fn text(title: &str, sections: &[Section]) -> String {
    let mut out = title.to_string();
    for s in sections {
        out.push_str(&format!("\n\n{}", s.caption));
        for line in &s.lines {
            match line {
                Line::Row(label, value) | Line::Path(label, value) | Line::Link { label, value, .. } => out.push_str(&format!("\n{label}  {value}")),
                Line::Text(t) | Line::Long(t) => out.push_str(&format!("\n{t}")),
                Line::Check { done, text, note } => {
                    out.push_str(&format!("\n[{}] {text}", if *done { "x" } else { " " }));
                    if let Some(note) = note {
                        out.push_str(&format!("\n    {note}"));
                    }
                }
            }
        }
    }
    out
}

/// The index [`ScrollHandle::scroll_to_item`] takes for the section `key`.
pub fn index_of(sections: &[Section], key: &str) -> Option<usize> {
    sections.iter().position(|s| s.key == key)
}

pub type OnLink<V> = Rc<dyn Fn(&mut V, &str, &mut Window, &mut Context<V>)>;

pub struct Page<'a, V> {
    pub key: &'a str,
    pub title: String,
    pub pill: Option<(String, Hsla)>,
    pub sections: &'a [Section],
    pub actions: Vec<AnyElement>,
    pub scroll: &'a ScrollHandle,
    pub on_link: Option<OnLink<V>>,
    /// Above the sections, in the scrolled body: a form of the owner's.
    pub top: Option<AnyElement>,
    /// Above the title: the owner's line, such as a ref with its actions.
    pub over: Option<AnyElement>,
    /// Between the title and the scrolled body, held in place: chips, a tab bar.
    pub bar: Option<AnyElement>,
}

pub fn render<V: 'static>(page: Page<V>, hits: &HitTargets, cx: &mut Context<V>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let pill = page.pill.map(|(label, color)| {
        div().flex_none().px_2().rounded_full().bg(banner::tint(color)).text_xs().text_color(color).whitespace_nowrap().child(SharedString::from(label))
    });
    let over = page.over.map(|o| div().px_6().pt_4().child(o));
    let title = div()
        .flex()
        .items_center()
        .gap_3()
        .px_6()
        .when(over.is_none(), |d| d.pt_5())
        .when(over.is_some(), |d| d.pt_1())
        .pb_3()
        .child(div().min_w_0().text_lg().font_weight(FontWeight::SEMIBOLD).text_color(colors.text).child(SharedString::from(page.title)))
        .children(pill);
    let mut body = div().id(SharedString::from(format!("{}-body", page.key))).flex().flex_col().flex_1().min_h_0().overflow_y_scroll().track_scroll(page.scroll).px_6().pb_4();
    if let Some(top) = page.top {
        body = body.child(top);
    }
    for section in page.sections {
        body = body.child(render_section(page.key, section, page.on_link.clone(), hits, cx));
    }
    let actions = (!page.actions.is_empty())
        .then(|| div().flex().flex_none().flex_wrap().items_center().gap_2().px_6().py_3().border_t_1().border_color(colors.border_variant).children(page.actions));
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(colors.editor_background)
        .text_color(colors.text)
        .children(over)
        .child(title)
        .children(page.bar.map(|b| div().flex_none().px_6().pb_2().child(b)))
        .child(body)
        .children(actions)
        .into_any_element()
}

fn render_section<V: 'static>(page: &str, section: &Section, on_link: Option<OnLink<V>>, hits: &HitTargets, cx: &mut Context<V>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let key = format!("{page}-section-{}", section.key);
    let mut out = div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .gap_1()
        .py_3()
        .border_b_1()
        .border_color(colors.border_variant)
        .child(hit_target(hits, key.clone()))
        .child(div().text_xs().font_weight(FontWeight::BOLD).text_color(colors.text_muted).child(SharedString::from(section.caption.to_uppercase())));
    for (i, line) in section.lines.iter().enumerate() {
        out = out.child(match line {
            Line::Row(label, value) => labelled(label, &colors).child(div().flex_1().min_w_0().child(SharedString::from(value.clone()))).into_any_element(),
            Line::Path(label, path) => labelled(label, &colors)
                .child(path_label(format!("{key}-path-{i}"), path, crate::themes::secondary(&colors), colors.text).flex_1())
                .into_any_element(),
            Line::Link { label, value, target } => link_line(label, value, target, format!("{key}-link-{i}"), on_link.clone(), hits, cx),
            Line::Text(t) => div().text_sm().whitespace_normal().child(SharedString::from(t.clone())).into_any_element(),
            Line::Check { done, text, note } => check_line(*done, text, note.as_deref(), format!("{key}-check-{i}"), hits, cx),
            Line::Long(t) => long_line(t, format!("{key}-copy-{i}"), hits, cx),
        });
    }
    out.into_any_element()
}

fn labelled(label: &str, colors: &ThemeColors) -> Div {
    div()
        .flex()
        .items_start()
        .gap_3()
        .text_sm()
        .child(div().flex_none().w(px(LABEL)).text_color(colors.text_muted).child(SharedString::from(label.to_string())))
}

fn link_line<V: 'static>(label: &str, value: &str, target: &str, link_key: String, on_link: Option<OnLink<V>>, hits: &HitTargets, cx: &mut Context<V>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let target = target.to_string();
    labelled(label, &colors)
        .child(
            div()
                .id(SharedString::from(link_key.clone()))
                .relative()
                .min_w_0()
                .text_color(colors.text_accent)
                .underline()
                .cursor_pointer()
                .child(SharedString::from(value.to_string()))
                .child(hit_target(hits, link_key))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    if let Some(f) = &on_link {
                        f(this, &target, window, cx);
                    }
                })),
        )
        .into_any_element()
}

fn check_line<V: 'static>(done: bool, text: &str, note: Option<&str>, key: String, hits: &HitTargets, cx: &mut Context<V>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    div()
        .relative()
        .flex()
        .items_start()
        .gap_2()
        .text_sm()
        .child(div().flex_none().h(px(20.)).flex().items_center().child(button::tick(done, cx.theme().status().success, cx)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(SharedString::from(text.to_string()))
                .children(note.map(|n| div().text_xs().text_color(colors.text_muted).child(SharedString::from(n.to_string())))),
        )
        .child(hit_target(hits, format!("{key}={}", if done { "done" } else { "open" })))
        .into_any_element()
}

fn long_line<V: 'static>(text: &str, copy_key: String, hits: &HitTargets, cx: &mut Context<V>) -> AnyElement {
    let copied = text.to_string();
    div()
        .flex()
        .items_start()
        .gap_2()
        .p_2()
        .rounded_md()
        .bg(cx.theme().colors().element_background)
        .child(div().flex_1().min_w_0().font_family(crate::fonts::MONO).text_sm().child(SharedString::from(text.to_string())))
        .child(button::action(hits, copy_key, "Copy", button::Tone::Neutral, button::Fill::Outline, cx).on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
        })))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_reads_each_section_under_its_caption() {
        let sections = [
            Section::new("when", "When").row("Runs", "Every Monday at 08:00").text("Missed runs catch up once."),
            Section::new("what", "What").long("fix\nit").link("Schedule", "nightly", "demo/nightly"),
        ];
        assert_eq!(text("Gate", &sections), "Gate\n\nWhen\nRuns  Every Monday at 08:00\nMissed runs catch up once.\n\nWhat\nfix\nit\nSchedule  nightly");
        assert_eq!((index_of(&sections, "what"), index_of(&sections, "may")), (Some(1), None));
    }
}
