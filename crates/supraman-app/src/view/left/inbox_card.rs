//! The Inbox's Workman cards (workman-ui/inbox-cards): Needs you, then Housekeeping, closed until opened.

use gpui::{App, ClipboardItem, Window};
use supraman_ui::clipped::clamped;
use supraman_workman::InboxRow;

use super::*;
use crate::work_view::HOUSEKEEPING_KEY;
use crate::work_view::cards::{self, Card};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardAction {
    Open,
    Web,
    /// The call is on the clipboard already.
    Copied,
    Dismiss,
}

pub type OnCard = Rc<dyn Fn(CardAction, &mut Window, &mut App)>;

pub fn kind_color(kind: &str, cx: &App) -> Hsla {
    let status = cx.theme().status();
    match kind {
        "blocked" | "blocker_finding" => status.error,
        "question" | "assumption_due" | "done_unverified" => status.warning,
        _ => status.info,
    }
}

pub fn part_key(part: &str, card: &Card) -> String {
    format!("inbox-card-{part}-{}", card.key)
}

fn action_button(hits: &HitTargets, card: &Card, part: &str, icon: Icon, tip: &'static str, on: &OnCard, does: CardAction, cx: &App) -> Stateful<Div> {
    let key = if does == CardAction::Dismiss { work_view::dismiss_key_of(&card.key) } else { part_key(part, card) };
    let group = work_view::row_key_of(&card.key);
    let (on, call) = (on.clone(), card.call.clone());
    button::revealed(hits, key, icon.svg(14., cx.theme().colors().text_muted), px(20.), group, false, cx)
        .tooltip(Tooltip::text(tip))
        .on_click(move |_: &ClickEvent, window, cx| {
            cx.stop_propagation();
            if does == CardAction::Copied {
                cx.write_to_clipboard(ClipboardItem::new_string(call.clone()));
            }
            on(does, window, cx);
        })
}

/// Paints [`Card::texts`] and nothing else; the call only reaches the clipboard, through Copy.
pub fn render(card: &Card, hits: &HitTargets, now: i64, on: OnCard, cx: &App) -> AnyElement {
    let colors = cx.theme().colors();
    let color = kind_color(&card.kind, cx);
    let muted = supraman_ui::themes::secondary(colors);
    let key = work_view::row_key_of(&card.key);
    let when = card.when(now);
    let mut buttons = div().flex().flex_none().items_center().child(action_button(hits, card, "open", Icon::Work, "Open in the Work tab", &on, CardAction::Open, cx));
    if card.url.is_some() {
        buttons = buttons.child(action_button(hits, card, "web", Icon::ArrowUpRight, "Open in Workman's web page", &on, CardAction::Web, cx));
    }
    if !card.call.is_empty() {
        buttons = buttons.child(action_button(hits, card, "copy", Icon::Copy, "Copy the call that resolves it", &on, CardAction::Copied, cx));
    }
    buttons = buttons.child(action_button(hits, card, "dismiss", Icon::Close, "Dismiss here (Workman is not changed)", &on, CardAction::Dismiss, cx));
    let head = div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .child(crate::brand::workman(rems(0.875), color))
        .child(clipped(part_key("workspace", card), card.workspace.clone(), false).flex_shrink(1.).text_color(muted))
        .when(!when.is_empty(), |d| {
            let full = card.since.map(supraman_core::time::date_words).unwrap_or_default();
            d.child(div().id(SharedString::from(part_key("when", card))).flex_none().text_color(muted).child(SharedString::from(when)).tooltip(Tooltip::text(full)))
        })
        .child(div().flex_none().px_1().rounded_sm().bg(color.opacity(0.16)).text_color(color).font_weight(FontWeight::BOLD).child(SharedString::from(card.badge.clone())))
        .child(div().flex_1())
        .child(buttons)
        .child(hit_target(hits, part_key("head", card)));
    let title = clamped(part_key("title", card), card.title.clone(), 2).text_sm().text_color(colors.text).child(hit_target(hits, part_key("title", card)));
    let ref_ = div().relative().text_xs().text_color(muted).child(SharedString::from(card.ref_.clone())).child(hit_target(hits, part_key("ref", card)));
    let detail = card.detail.clone().map(|text| clipped(part_key("detail", card), text, false).text_xs().text_color(colors.text).child(hit_target(hits, part_key("detail", card))));
    let open = on.clone();
    div()
        .id(SharedString::from(key.clone()))
        .group(SharedString::from(key.clone()))
        .relative()
        .flex()
        .flex_col()
        .gap(px(2.))
        .p_2()
        .rounded_md()
        .border_l_2()
        .border_color(color)
        .bg(color.opacity(0.06))
        .cursor_pointer()
        .hover(|s| s.bg(color.opacity(0.14)))
        .child(head)
        .child(title)
        .child(ref_)
        .children(detail)
        .child(hit_target(hits, key))
        .on_click(move |_: &ClickEvent, window, cx| open(CardAction::Open, window, cx))
        .into_any_element()
}

fn caption(text: String, cx: &App) -> Div {
    div().pt_1().text_xs().font_weight(FontWeight::BOLD).text_color(cx.theme().colors().text_muted).child(SharedString::from(text))
}

impl FleetView {
    fn on_card(&self, card: &Card, cx: &mut Context<Self>) -> OnCard {
        let (this, card) = (cx.entity().downgrade(), card.clone());
        Rc::new(move |action, window, cx| {
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

    /// Needs you, then the Housekeeping header and, while open, its groups.
    pub(super) fn render_work_cards(&self, rows: &[(String, InboxRow)], cx: &mut Context<Self>) -> (Vec<AnyElement>, Option<AnyElement>) {
        let live: Vec<String> = self.work_live.borrow().iter().map(|s| s.name.clone()).collect();
        let refs: Vec<(&str, &InboxRow)> = rows.iter().map(|(w, r)| (w.as_str(), r)).collect();
        let now = now_ms();
        let sections = cards::sections(&refs, &live, now);
        let needs = sections.needs_you.iter().map(|card| render(card, &self.hits, now, self.on_card(card, cx), cx)).collect();
        if sections.housekeeping.is_empty() {
            return (needs, None);
        }
        let open = self.work_inbox.read(cx).housekeeping_open;
        let inbox = self.work_inbox.clone();
        let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
        let header = div()
            .id(HOUSEKEEPING_KEY)
            .relative()
            .flex()
            .items_center()
            .gap_1()
            .cursor_pointer()
            .child(chevron.svg(12., cx.theme().colors().text_muted))
            .child(caption(format!("HOUSEKEEPING · {}", sections.housekeeping_count()), cx).pt_0())
            .child(hit_target(&self.hits, format!("{HOUSEKEEPING_KEY}={}", if open { "open" } else { "closed" })))
            .on_click(move |_: &ClickEvent, _, cx| inbox.update(cx, |inbox, cx| inbox.toggle_housekeeping(cx)));
        let mut section = div().flex().flex_col().gap_1().pt_1().child(header);
        if open {
            for group in &sections.housekeeping {
                section = section.child(caption(group.caption.clone(), cx).font_weight(FontWeight::NORMAL));
                section = section.children(group.cards.iter().map(|card| render(card, &self.hits, now, self.on_card(card, cx), cx)));
            }
        }
        (needs, Some(section.into_any_element()))
    }
}

#[cfg(test)]
mod tests;
