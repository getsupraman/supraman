//! The Inbox's Workman cards: Needs you, then Housekeeping, closed until opened.

use std::rc::Rc;

use gpui::{AnyElement, App, ClickEvent, ClipboardItem, Div, Entity, FontWeight, Hsla, SharedString, Stateful, Window, div, prelude::*};
use supraman_core::time::now_ms;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;
use supraman_ui::inbox_card::{self as shared, InboxCard};
use supraman_ui::tooltip::Tooltip;
use supraman_workman::InboxRow;
use theme::ActiveTheme as _;

use crate::work_view::cards::{self, Card};
use crate::work_view::{self, HOUSEKEEPING_KEY, WorkInbox};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardAction {
    Open,
    Web,
    /// The call is on the clipboard already.
    Copied,
    Dismiss,
}

pub type OnCard = Rc<dyn Fn(CardAction, &mut Window, &mut App)>;

pub type Mark = fn(Hsla) -> AnyElement;

pub fn part_key(part: &str, card: &Card) -> String {
    format!("inbox-card-{part}-{}", card.key)
}

fn action_button(hits: &HitTargets, card: &Card, part: &str, icon: Icon, tip: &'static str, on: &OnCard, does: CardAction, cx: &App) -> Stateful<Div> {
    let key = if does == CardAction::Dismiss { work_view::dismiss_key_of(&card.key) } else { part_key(part, card) };
    acting(shared::action(hits, key, work_view::row_key_of(&card.key), icon.svg(shared::ICON, shared::icon_color(cx)), tip, cx), card, on, does)
}

fn acting(button: Stateful<Div>, card: &Card, on: &OnCard, does: CardAction) -> Stateful<Div> {
    let (on, call) = (on.clone(), card.call.clone());
    button.on_click(move |_: &ClickEvent, window, cx| {
        cx.stop_propagation();
        if does == CardAction::Copied {
            cx.write_to_clipboard(ClipboardItem::new_string(call.clone()));
        }
        on(does, window, cx);
    })
}

/// Paints [`Card::texts`] and nothing else; the call only reaches the clipboard, through Copy.
pub fn render(card: &Card, hits: &HitTargets, now: i64, mark: Mark, on: OnCard, cx: &App) -> AnyElement {
    let item = card.clone();
    let mut shown = InboxCard::new(work_view::row_key_of(&card.key), card.kind.clone(), card.badge.clone(), card.title.clone())
        .parts(move |part| part_key(part, &item))
        .mark(mark, cx)
        .context(card.workspace.clone())
        .when(card.since, now)
        .line("ref", card.ref_.clone(), true)
        .line("detail", card.detail.clone().unwrap_or_default(), false)
        .primary(acting(shared::primary(hits, part_key("open", card), "Open", cx).tooltip(Tooltip::text("Open in the Work tab")), card, &on, CardAction::Open));
    if card.url.is_some() {
        shown = shown.action(action_button(hits, card, "web", Icon::ArrowUpRight, "Open in Workman's web page", &on, CardAction::Web, cx));
    }
    if !card.call.is_empty() {
        shown = shown.action(action_button(hits, card, "copy", Icon::Copy, "Copy the call that resolves it", &on, CardAction::Copied, cx));
    }
    shown
        .action(action_button(hits, card, "dismiss", Icon::Close, "Dismiss here (Workman is not changed)", &on, CardAction::Dismiss, cx))
        .on_click(move |_: &ClickEvent, window, cx| on(CardAction::Open, window, cx))
        .render(hits, cx)
}

fn caption(text: String, cx: &App) -> Div {
    div().pt_1().text_xs().font_weight(FontWeight::BOLD).text_color(cx.theme().colors().text_muted).child(SharedString::from(text))
}

/// Needs you's cards, then the Housekeeping header and, while `inbox` keeps it open, its groups; `live` names the live sessions.
pub fn render_sections(
    rows: &[(String, InboxRow)],
    live: &[String],
    inbox: &Entity<WorkInbox>,
    hits: &HitTargets,
    mark: Mark,
    on_card: &dyn Fn(&Card) -> OnCard,
    cx: &App,
) -> (Vec<AnyElement>, Option<AnyElement>) {
    let refs: Vec<(&str, &InboxRow)> = rows.iter().map(|(w, r)| (w.as_str(), r)).collect();
    let now = now_ms();
    let sections = cards::sections(&refs, live, now);
    let needs = sections.needs_you.iter().map(|card| render(card, hits, now, mark, on_card(card), cx)).collect();
    if sections.housekeeping.is_empty() {
        return (needs, None);
    }
    let open = inbox.read(cx).housekeeping_open;
    let inbox = inbox.clone();
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
        .child(hit_target(hits, format!("{HOUSEKEEPING_KEY}={}", if open { "open" } else { "closed" })))
        .on_click(move |_: &ClickEvent, _, cx| inbox.update(cx, |inbox, cx| inbox.toggle_housekeeping(cx)));
    let mut section = div().flex().flex_col().gap_1().pt_1().child(header);
    if open {
        for group in &sections.housekeeping {
            section = section.child(caption(group.caption.clone(), cx).font_weight(FontWeight::NORMAL));
            section = section.children(group.cards.iter().map(|card| render(card, hits, now, mark, on_card(card), cx)));
        }
    }
    (needs, Some(section.into_any_element()))
}

#[cfg(test)]
mod tests;
