//! A details page: a title with a status pill, sections of `label  value` rows and paragraphs, and an actions
//! bar under them. The owner builds the [`Section`]s from its data and keeps the [`ScrollHandle`].

use gpui::{AnyElement, ClickEvent, ClipboardItem, Context, FontWeight, Hsla, ScrollHandle, SharedString, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::banner;
use crate::button;
use crate::hit::{HitTargets, hit_target};

const LABEL: f32 = 128.;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// `label  value`.
    Row(String, String),
    Text(String),
    /// Wrapped as written, in the mono font, with a Copy button.
    Long(String),
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

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.lines.push(Line::Text(text.into()));
        self
    }

    pub fn long(mut self, text: impl Into<String>) -> Self {
        self.lines.push(Line::Long(text.into()));
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
                Line::Row(label, value) => out.push_str(&format!("\n{label}  {value}")),
                Line::Text(t) | Line::Long(t) => out.push_str(&format!("\n{t}")),
            }
        }
    }
    out
}

/// The index [`ScrollHandle::scroll_to_item`] takes for the section `key`.
pub fn index_of(sections: &[Section], key: &str) -> Option<usize> {
    sections.iter().position(|s| s.key == key)
}

pub struct Page<'a> {
    pub key: &'a str,
    pub title: String,
    pub pill: Option<(String, Hsla)>,
    pub sections: &'a [Section],
    pub actions: Vec<AnyElement>,
    pub scroll: &'a ScrollHandle,
}

pub fn render<V: 'static>(page: Page, hits: &HitTargets, cx: &mut Context<V>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let pill = page.pill.map(|(label, color)| {
        div().flex_none().px_2().rounded_full().bg(banner::tint(color)).text_xs().text_color(color).whitespace_nowrap().child(SharedString::from(label))
    });
    let title = div()
        .flex()
        .items_center()
        .gap_3()
        .px_6()
        .pt_5()
        .pb_3()
        .child(div().min_w_0().text_lg().font_weight(FontWeight::SEMIBOLD).text_color(colors.text).child(SharedString::from(page.title)))
        .children(pill);
    let mut body = div().id(SharedString::from(format!("{}-body", page.key))).flex().flex_col().flex_1().min_h_0().overflow_y_scroll().track_scroll(page.scroll).px_6().pb_4();
    for section in page.sections {
        body = body.child(render_section(page.key, section, hits, cx));
    }
    let actions = div().flex().flex_none().flex_wrap().items_center().gap_2().px_6().py_3().border_t_1().border_color(colors.border_variant).children(page.actions);
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(colors.editor_background)
        .text_color(colors.text)
        .child(title)
        .child(body)
        .child(actions)
        .into_any_element()
}

fn render_section<V: 'static>(page: &str, section: &Section, hits: &HitTargets, cx: &mut Context<V>) -> AnyElement {
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
            Line::Row(label, value) => div()
                .flex()
                .items_start()
                .gap_3()
                .text_sm()
                .child(div().flex_none().w(px(LABEL)).text_color(colors.text_muted).child(SharedString::from(label.clone())))
                .child(div().flex_1().min_w_0().child(SharedString::from(value.clone())))
                .into_any_element(),
            Line::Text(t) => div().text_sm().child(SharedString::from(t.clone())).into_any_element(),
            Line::Long(t) => {
                let copy_key = format!("{key}-copy-{i}");
                let copied = t.clone();
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .bg(colors.element_background)
                    .child(div().flex_1().min_w_0().font_family(crate::fonts::MONO).text_sm().child(SharedString::from(t.clone())))
                    .child(button::bordered(hits, copy_key, "Copy", cx).on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                    })))
                    .into_any_element()
            }
        });
    }
    out.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_reads_each_section_under_its_caption() {
        let sections = [Section::new("when", "When").row("Runs", "Every Monday at 08:00").text("Missed runs catch up once."), Section::new("what", "What").long("fix\nit")];
        assert_eq!(text("Gate", &sections), "Gate\n\nWhen\nRuns  Every Monday at 08:00\nMissed runs catch up once.\n\nWhat\nfix\nit");
        assert_eq!((index_of(&sections, "what"), index_of(&sections, "may")), (Some(1), None));
    }
}
