//! A settings window's pages: each registers by id with a title, an icon and its rows, and the window
//! paints the page list and the open page's rows. The settings themselves are `supraman_core::Settings`.

use gpui::{AnyElement, App, ClickEvent, Context, FontWeight, Hsla, IntoElement, ParentElement as _, SharedString, Styled as _, div, prelude::*};
use theme::ActiveTheme as _;

use crate::button;
use crate::hit::HitTargets;

pub struct SettingsPage<V: 'static> {
    pub id: &'static str,
    pub title: &'static str,
    /// A glyph of [`crate::fonts::icons`].
    pub icon: &'static str,
    /// Painted in place of `icon`: a product's own mark, in the colour the glyph would have.
    pub mark: Option<fn(Hsla) -> AnyElement>,
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

    /// One [`button::row`] per page with its icon and title under `settings-page-<id>`, `active` selected; a click calls `select` with its id.
    pub fn nav(&self, hits: &HitTargets, active: &str, select: fn(&mut V, &'static str, &mut Context<V>), cx: &mut Context<V>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        self.pages
            .iter()
            .map(|page| {
                let (id, on) = (page.id, page.id == active);
                button::row(hits, format!("settings-page-{id}"), on.then_some(colors.element_selected), cx)
                    .mx_2()
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_sm()
                    .text_sm()
                    .when(!on, |d| d.text_color(colors.text_muted))
                    .child(match page.mark {
                        Some(mark) => div().w_4().flex().items_center().child(mark(if on { colors.text } else { colors.text_muted })),
                        None => div().w_4().font_family(crate::fonts::icons()).text_xs().child(page.icon),
                    })
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
        SettingsPage { id, title: id, icon: "", mark: None, render: |_, _| Vec::new() }
    }

    #[test]
    fn pages_are_found_by_id_in_their_order_and_an_unknown_id_opens_the_first() {
        let pages = SettingsPages::default().register(page("sessions")).register(page("about"));
        assert_eq!(pages.ids().collect::<Vec<_>>(), ["sessions", "about"]);
        assert_eq!(pages.get("about").map(|p| p.title), Some("about"));
        assert_eq!((pages.resolve(Some("about")), pages.resolve(Some("gitman")), pages.resolve(None)), ("about", "sessions", "sessions"));
        assert_eq!(SettingsPages::<View>::default().resolve(None), "");
    }

    struct Nav {
        pages: SettingsPages<Nav>,
        hits: HitTargets,
    }

    impl Render for Nav {
        fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().w(gpui::px(180.)).children(self.pages.nav(&self.hits, "sessions", |_, _, _| {}, cx))
        }
    }

    fn listed(id: &'static str) -> SettingsPage<Nav> {
        SettingsPage { id, title: id, icon: "", mark: None, render: |_, _| Vec::new() }
    }

    fn hovered_fills(view: &gpui::Entity<Nav>, id: &str, cx: &mut gpui::VisualTestContext) -> Vec<gpui::Background> {
        let hits = view.read_with(cx, |v, _| v.hits.clone());
        let row = hits.borrow().get(&format!("settings-page-{id}")).map(|h| h.bounds).unwrap_or_else(|| panic!("page {id} paints"));
        cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::none());
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        cx.update(|window, _| {
            let middle = row.center().scale(window.scale_factor());
            window.painted_quads().into_iter().filter(|q| q.bounds.contains(&middle) && q.content_mask.bounds.contains(&middle)).map(|q| q.background).collect()
        })
    }

    #[gpui::test]
    fn a_hovered_open_page_keeps_its_selected_fill_and_another_page_takes_the_hover(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (view, cx) = cx.add_window_view(|_, _| Nav {
            pages: SettingsPages::default().register(listed("sessions")).register(listed("about")),
            hits: HitTargets::default(),
        });
        cx.run_until_parked();
        let colors = cx.update(|_, cx| cx.theme().colors().clone());
        let (selected, hover) = (gpui::Background::from(colors.element_selected), gpui::Background::from(colors.ghost_element_hover));
        let open = hovered_fills(&view, "sessions", cx);
        assert!(open.contains(&selected) && !open.contains(&hover), "the open page under the pointer keeps its selected fill: {open:?}");
        let other = hovered_fills(&view, "about", cx);
        assert!(other.contains(&hover) && !other.contains(&selected), "another page under the pointer takes the hover: {other:?}");
    }

    #[test]
    #[should_panic(expected = "settings page about is registered twice")]
    fn an_id_registers_once() {
        let _ = SettingsPages::default().register(page("about")).register(page("about"));
    }
}
