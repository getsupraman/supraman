//! A settings window's pages: each registers by id with a title, an icon and its rows, and the window
//! paints the page list and the open page's rows. The settings themselves are `foreman_core::Settings`.

use gpui::{AnyElement, App, ClickEvent, Context, FontWeight, IntoElement, ParentElement as _, SharedString, Styled as _, div, prelude::*};
use theme::ActiveTheme as _;

pub struct SettingsPage<V: 'static> {
    pub id: &'static str,
    pub title: &'static str,
    /// A glyph of [`crate::fonts::icons`].
    pub icon: &'static str,
    pub render: fn(&V, &mut Context<V>) -> Vec<AnyElement>,
}

/// The pages in the order they were registered; the first is the one a window opens on.
pub struct SettingsPages<V: 'static> {
    pages: Vec<SettingsPage<V>>,
}

impl<V: 'static> Default for SettingsPages<V> {
    fn default() -> Self {
        Self { pages: Vec::new() }
    }
}

impl<V: 'static> SettingsPages<V> {
    /// Panics when `page.id` is registered already.
    pub fn register(mut self, page: SettingsPage<V>) -> Self {
        assert!(self.get(page.id).is_none(), "settings page {} is registered twice", page.id);
        self.pages.push(page);
        self
    }

    pub fn get(&self, id: &str) -> Option<&SettingsPage<V>> {
        self.pages.iter().find(|page| page.id == id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.pages.iter().map(|page| page.id)
    }

    /// `id` when registered, else the first page's.
    pub fn resolve(&self, id: Option<&str>) -> &'static str {
        id.and_then(|id| self.get(id)).or(self.pages.first()).map_or("", |page| page.id)
    }

    /// One entry per page with its icon and title, `active` highlighted; a click calls `select` with its id.
    pub fn nav(&self, active: &str, select: fn(&mut V, &'static str, &mut Context<V>), cx: &mut Context<V>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        self.pages
            .iter()
            .map(|page| {
                let (id, on) = (page.id, page.id == active);
                div()
                    .id(id)
                    .mx_2()
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_sm()
                    .text_sm()
                    .cursor_pointer()
                    .when(on, |d| d.bg(colors.element_selected))
                    .when(!on, |d| d.text_color(colors.text_muted))
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .active(|s| s.bg(colors.ghost_element_active))
                    .child(div().w_4().font_family(crate::fonts::icons()).text_xs().child(page.icon))
                    .child(page.title)
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| select(view, id, cx)))
                    .into_any_element()
            })
            .collect()
    }
}

/// One setting: its title and description on the left with `below` under them, `control` on the right.
pub fn row(title: &'static str, description: impl Into<SharedString>, control: AnyElement, below: Option<AnyElement>, cx: &App) -> AnyElement {
    let colors = cx.theme().colors();
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap_6()
        .py_3()
        .border_b_1()
        .border_color(colors.border_variant)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().text_xs().text_color(colors.text_muted).child(description.into()))
                .children(below),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct View;

    fn page(id: &'static str) -> SettingsPage<View> {
        SettingsPage { id, title: id, icon: "", render: |_, _| Vec::new() }
    }

    #[test]
    fn pages_are_found_by_id_in_their_order_and_an_unknown_id_opens_the_first() {
        let pages = SettingsPages::default().register(page("sessions")).register(page("about"));
        assert_eq!(pages.ids().collect::<Vec<_>>(), ["sessions", "about"]);
        assert_eq!(pages.get("about").map(|p| p.title), Some("about"));
        assert_eq!((pages.resolve(Some("about")), pages.resolve(Some("gitman")), pages.resolve(None)), ("about", "sessions", "sessions"));
        assert_eq!(SettingsPages::<View>::default().resolve(None), "");
    }

    #[test]
    #[should_panic(expected = "settings page about is registered twice")]
    fn an_id_registers_once() {
        let _ = SettingsPages::default().register(page("about")).register(page("about"));
    }
}
