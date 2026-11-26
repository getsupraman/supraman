//! Copy ref, Open in Workman web and a row's right-click menu.

use gpui::{MouseButton, MouseDownEvent, Pixels, Point};
use supraman_ui::icon::Icon;
use supraman_ui::menu::{ContextMenu, item};
use supraman_core::open;
use supraman_ui::button;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RowAction {
    OpenWeb(ItemRef),
    CopyRef(ItemRef),
    CopyLink(ItemRef),
    CopyTitle(ItemRef),
}

pub(crate) fn web_url(base: &str, item: &ItemRef) -> String {
    let path: Vec<String> = item.full().split('/').map(supraman_workman::pct_encode).collect();
    format!("{}/r/{}", base.trim_end_matches('/'), path.join("/"))
}

pub(crate) const COPY_SIDE: f32 = 20.;

impl WorkView {
    pub(crate) fn copy_ref(&mut self, item: &ItemRef, cx: &mut Context<Self>) {
        supraman_ui::clipboard::write_text(cx, item.full().to_string());
        self.toast.show(format!("Copied {}", item.full()), |this: &mut WorkView| &mut this.toast, cx);
    }

    pub(crate) fn open_web(&mut self, item: &ItemRef, cx: &mut Context<Self>) {
        let Some(base) = self.web_base(cx) else {
            self.toast.show_why("Workman is not connected, so there is no web page to open.".into(), |this: &mut WorkView| &mut this.toast, cx);
            return;
        };
        let url = web_url(&base, item);
        match open::allowed(&url, &open::WEB_SCHEMES) {
            Ok(()) => (self.open_link_with)(&Link::Url(url), cx),
            Err(scheme) => self.toast.show_why(open::refused(&scheme), |this: &mut WorkView| &mut this.toast, cx),
        }
    }

    pub(crate) fn run_action(&mut self, action: RowAction, _: &mut Window, cx: &mut Context<Self>) {
        match action {
            RowAction::OpenWeb(item) => self.open_web(&item, cx),
            RowAction::CopyRef(item) => self.copy_ref(&item, cx),
            RowAction::CopyLink(item) => match self.web_base(cx) {
                Some(base) => {
                    supraman_ui::clipboard::write_text(cx, web_url(&base, &item));
                    self.toast.show(format!("Copied the link to {}", item.full()), |this: &mut WorkView| &mut this.toast, cx);
                }
                None => self.toast.show_why("Workman is not connected, so there is no link.".into(), |this: &mut WorkView| &mut this.toast, cx),
            },
            RowAction::CopyTitle(item) => {
                supraman_ui::clipboard::write_text(cx, item.title.clone());
                self.toast.show(format!("Copied the title of {}", item.full()), |this: &mut WorkView| &mut this.toast, cx);
            }
        }
        cx.notify();
    }

    /// `anchor`: the row's hit key, which paints open while the menu is shown.
    pub(crate) fn show_row_menu(&mut self, target: &ItemRef, anchor: String, position: Point<Pixels>, cx: &mut Context<Self>) {
        let items = vec![
            item("work-web", "Open in Workman web", RowAction::OpenWeb(target.clone())),
            item("work-copy-ref", "Copy ref", RowAction::CopyRef(target.clone())),
            item("work-copy-link", "Copy link", RowAction::CopyLink(target.clone())),
            item("work-copy-title", "Copy title", RowAction::CopyTitle(target.clone())),
        ];
        self.menu = Some(ContextMenu::new(target.qualified_ref.clone(), position, items).anchored(anchor));
        cx.notify();
    }

    /// The Copy ref and Open in web icons of a breadcrumb or pane header, always shown; keys `<prefix>-copy`, `<prefix>-web`.
    pub(super) fn ref_actions(&self, prefix: &str, item: &ItemRef, cx: &mut Context<Self>) -> gpui::Div {
        let muted = cx.theme().colors().text_muted;
        let (copied, opened) = (item.clone(), item.clone());
        div()
            .flex()
            .flex_none()
            .items_center()
            .child(
                button::Button::new(&self.hits, format!("{prefix}-copy")).icon(Icon::Copy.svg(12., muted)).icon_only(px(COPY_SIDE))
                    .tooltip(supraman_ui::tooltip::Tooltip::text("Copy ref"))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| this.copy_ref(&copied, cx))),
            )
            .child(
                button::Button::new(&self.hits, format!("{prefix}-web")).icon(Icon::ArrowUpRight.svg(12., muted)).icon_only(px(COPY_SIDE))
                    .tooltip(supraman_ui::tooltip::Tooltip::text("Open in Workman web"))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| this.open_web(&opened, cx))),
            )
    }

    pub(super) fn on_row_mouse_down(&self, item: &ItemRef, anchor: String, cx: &mut Context<Self>) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        let item = item.clone();
        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            if e.button == MouseButton::Right {
                this.show_row_menu(&item, anchor.clone(), e.position, cx);
                cx.stop_propagation();
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_web_link_is_the_full_ref_on_the_server() {
        let item = ItemRef { qualified_ref: "refactor3/13".into(), full_ref: "supraman/refactor3/13".into(), ..Default::default() };
        assert_eq!(web_url("https://work.example/", &item), "https://work.example/r/supraman/refactor3/13");
        let odd = ItemRef { full_ref: "p/a b/1".into(), ..Default::default() };
        assert_eq!(web_url("https://work.example", &odd), "https://work.example/r/p/a%20b/1", "each part encoded, its slashes kept");
    }
}
