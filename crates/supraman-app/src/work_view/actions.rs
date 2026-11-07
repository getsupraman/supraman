//! Copy ref, Open in Workman web and a row's right-click menu.

use gpui::{ClipboardItem, MouseButton, MouseDownEvent, Pixels, Point};
use supraman_ui::icon::Icon;
use supraman_ui::menu::{ContextMenu, item};
use supraman_ui::{button, open};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowAction {
    OpenWeb(ItemRef),
    CopyRef(ItemRef),
    CopyLink(ItemRef),
    CopyTitle(ItemRef),
}

pub fn web_url(base: &str, item: &ItemRef) -> String {
    format!("{}/r/{}", base.trim_end_matches('/'), item.qualified_ref)
}

pub const COPY_SIDE: f32 = 20.;

impl WorkView {
    pub fn copy_ref(&mut self, item: &ItemRef, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(item.qualified_ref.clone()));
        self.toast.show(format!("Copied {}", item.qualified_ref), |this: &mut WorkView| &mut this.toast, cx);
    }

    pub fn open_web(&mut self, item: &ItemRef, cx: &mut Context<Self>) {
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

    pub fn run_action(&mut self, action: RowAction, _: &mut Window, cx: &mut Context<Self>) {
        match action {
            RowAction::OpenWeb(item) => self.open_web(&item, cx),
            RowAction::CopyRef(item) => self.copy_ref(&item, cx),
            RowAction::CopyLink(item) => match self.web_base(cx) {
                Some(base) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(web_url(&base, &item)));
                    self.toast.show(format!("Copied the link to {}", item.qualified_ref), |this: &mut WorkView| &mut this.toast, cx);
                }
                None => self.toast.show_why("Workman is not connected, so there is no link.".into(), |this: &mut WorkView| &mut this.toast, cx),
            },
            RowAction::CopyTitle(item) => {
                cx.write_to_clipboard(ClipboardItem::new_string(item.title.clone()));
                self.toast.show(format!("Copied the title of {}", item.qualified_ref), |this: &mut WorkView| &mut this.toast, cx);
            }
        }
        cx.notify();
    }

    pub fn show_row_menu(&mut self, target: &ItemRef, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.menu = Some(ContextMenu {
            title: target.qualified_ref.clone(),
            position,
            items: vec![
                item("work-web", "Open in Workman web", RowAction::OpenWeb(target.clone())),
                item("work-copy-ref", "Copy ref", RowAction::CopyRef(target.clone())),
                item("work-copy-link", "Copy link", RowAction::CopyLink(target.clone())),
                item("work-copy-title", "Copy title", RowAction::CopyTitle(target.clone())),
            ],
        });
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
                button::icon(&self.hits, format!("{prefix}-copy"), Icon::Copy.svg(12., muted), px(COPY_SIDE), cx)
                    .tooltip(supraman_ui::tooltip::Tooltip::text("Copy ref"))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| this.copy_ref(&copied, cx))),
            )
            .child(
                button::icon(&self.hits, format!("{prefix}-web"), Icon::ArrowUpRight.svg(12., muted), px(COPY_SIDE), cx)
                    .tooltip(supraman_ui::tooltip::Tooltip::text("Open in Workman web"))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| this.open_web(&opened, cx))),
            )
    }

    pub(super) fn on_row_mouse_down(&self, item: &ItemRef, cx: &mut Context<Self>) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        let item = item.clone();
        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            if e.button == MouseButton::Right {
                this.show_row_menu(&item, e.position, cx);
                cx.stop_propagation();
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_web_link_is_the_qualified_ref_on_the_server() {
        let item = ItemRef { qualified_ref: "supraman/refactor3/13".into(), ..Default::default() };
        assert_eq!(web_url("https://work.example/", &item), "https://work.example/r/supraman/refactor3/13");
    }
}
