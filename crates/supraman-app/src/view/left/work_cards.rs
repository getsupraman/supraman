//! The Inbox's Workman cards in the sidebar: `work_view::inbox_card` paints them, the fleet acts on them.

use gpui::{App, Context, Window};
use supraman_workman::InboxRow;

use super::*;
use crate::work_view::cards::Card;
use crate::work_view::inbox_card::{self, CardAction, OnCard};

impl FleetView {
    fn on_card(this: &gpui::WeakEntity<Self>, card: &Card) -> OnCard {
        let (this, card) = (this.clone(), card.clone());
        Rc::new(move |action, window, cx: &mut App| {
            this.update(cx, |this, cx| this.card_action(&card, action, window, cx)).ok();
        })
    }

    fn card_action(&mut self, card: &Card, action: CardAction, _: &mut Window, cx: &mut Context<Self>) {
        match action {
            CardAction::Open => {
                self.open_work_item(&card.workspace, &card.item_id, cx);
            }
            CardAction::Web => match card.url.as_deref().map(|url| supraman_ui::open::allowed(url, &supraman_ui::open::WEB_SCHEMES).map(|()| url)) {
                Some(Ok(url)) => supraman_ui::open::open_off_thread(url.to_string(), cx).detach(),
                Some(Err(scheme)) => self.toast.show_why(supraman_ui::open::refused(&scheme), |this: &mut Self| &mut this.toast, cx),
                None => {}
            },
            CardAction::Copied => self.toast.show("Copied".into(), |this: &mut Self| &mut this.toast, cx),
            CardAction::Dismiss => {
                let (workspace, key) = (card.workspace.clone(), card.key.clone());
                self.work_inbox.update(cx, |inbox, cx| inbox.dismiss(&workspace, &key, cx));
            }
        }
    }

    pub(super) fn render_work_cards(&self, rows: &[(String, InboxRow)], cx: &mut Context<Self>) -> (Vec<AnyElement>, Option<AnyElement>) {
        let live: Vec<String> = crate::view::work_tab::attachable(&self.occupancy);
        let this = cx.weak_entity();
        let mark: inbox_card::Mark = |color| crate::brand::workman(rems(0.875), color).into_any_element();
        inbox_card::render_sections(rows, &live, &self.work_inbox, &self.hits, mark, &|card| Self::on_card(&this, card), cx)
    }
}
