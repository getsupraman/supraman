//! The Activity page, the web's: the newest events of the workspace's projects, grouped by day, newest first.

use gpui::{ClickEvent, FontWeight};
use supraman_core::time::{self, rfc3339_ms};
use supraman_ui::hit::hit_target;
use supraman_ui::clipped::clipped;
use supraman_ui::icon::Icon;
use supraman_ui::{button, fonts};
use supraman_workman::{JournalEvent, JournalQuery};

use super::actions::COPY_SIDE;
use super::*;

pub(crate) const SHOWN: usize = 150;

#[derive(Default)]
pub(crate) struct Activity {
    pub(crate) events: Vec<JournalEvent>,
    pub(crate) error: Option<String>,
    pub(crate) loaded: bool,
    reading: bool,
    again: bool,
    pending: bool,
    pub(crate) reads: u64,
}

fn read(client: &Client, projects: &[String]) -> Result<Vec<JournalEvent>, String> {
    let mut events: Vec<JournalEvent> = Vec::new();
    for project in projects {
        let page = client.journal(&JournalQuery { scope: Some(project), limit: Some(SHOWN as u32), ..Default::default() }).map_err(read_failed)?;
        let fresh: Vec<JournalEvent> = page.events.into_iter().filter(|e| !events.iter().any(|seen| seen.seq == e.seq)).collect();
        events.extend(fresh);
    }
    events.sort_by_key(|e| std::cmp::Reverse(e.seq));
    events.truncate(SHOWN);
    Ok(events)
}

/// Days newest first, each day's events as given (newest first): the day's heading and its events.
pub(crate) fn by_day(events: &[JournalEvent]) -> Vec<(String, Vec<&JournalEvent>)> {
    let mut days: Vec<(String, Vec<&JournalEvent>)> = Vec::new();
    for e in events {
        let day = rfc3339_ms(&e.at).map(time::day).unwrap_or_default();
        match days.last_mut() {
            Some((d, list)) if *d == day => list.push(e),
            _ => days.push((day, vec![e])),
        }
    }
    days
}

pub(crate) fn sentence(e: &JournalEvent) -> String {
    match e.summary.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(summary) => summary.to_string(),
        None => [e.kind.as_str(), e.item.as_ref().map_or("", |i| i.qualified_ref.as_str())].join(" ").trim().to_string(),
    }
}

pub(crate) fn actor(e: &JournalEvent) -> String {
    e.session.as_ref().map(|s| s.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| e.actor.clone())
}

impl WorkView {
    pub(super) fn read_activity(&mut self, cx: &mut Context<Self>) {
        if self.activity.reading {
            self.activity.again = true;
            return;
        }
        let projects: Vec<String> = self.projects.iter().map(|p| p.item.id.clone()).collect();
        let Some(client) = self.client(cx) else { return };
        self.activity.reading = true;
        self.activity.reads += 1;
        cx.spawn(async move |this, cx| {
            let read = cx.background_executor().spawn(async move { read(&client, &projects) }).await;
            this.update(cx, |this, cx| {
                let a = &mut this.activity;
                a.reading = false;
                match read {
                    Ok(events) => {
                        a.events = events;
                        a.error = None;
                        a.loaded = true;
                    }
                    Err(why) => a.error = Some(why),
                }
                if std::mem::take(&mut a.again) {
                    this.schedule_activity(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn schedule_activity(&mut self, cx: &mut Context<Self>) {
        if self.page != Page::Activity || self.activity.pending {
            return;
        }
        self.activity.pending = true;
        after_debounce(cx, |this, cx| {
            this.activity.pending = false;
            this.read_activity(cx);
        });
    }

    pub(super) fn render_activity_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let a = &self.activity;
        let mut root = div()
            .id("work-activity")
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .px_5()
            .pt_4()
            .pb_6()
            .overflow_y_scroll()
            .track_scroll(&self.list_scroll)
            .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child("Activity"))
            .child(div().text_sm().text_color(colors.text_muted).child(SharedString::from(format!(
                "The last {SHOWN} events across this workspace's projects. Older events are on each item's Activity tab."
            ))));
        if let Some(why) = &a.error {
            root = root.child(div().text_sm().text_color(cx.theme().status().error).child(SharedString::from(why.clone())));
        }
        if !a.loaded {
            return root.child(div().text_sm().text_color(colors.text_muted).child("Reading the journal…")).into_any_element();
        }
        if a.events.is_empty() {
            return root.child(div().text_sm().text_color(colors.text_muted).child("No activity yet.")).into_any_element();
        }
        let mut list = div().flex().flex_col().rounded_md().border_1().border_color(colors.border_variant).bg(colors.panel_background);
        for (day, events) in by_day(&a.events) {
            list = list.child(
                div()
                    .relative()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(day.clone()))
                    .child(hit_target(&self.hits, format!("work-activity-day-{day}={}", events.len()))),
            );
            for e in events {
                list = list.child(self.render_event(e, cx));
            }
        }
        root.child(list).into_any_element()
    }

    fn render_event(&self, e: &JournalEvent, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let key = format!("work-activity-{}", e.seq);
        let at = rfc3339_ms(&e.at).map(time::clock).unwrap_or_default();
        let item = e.item.clone();
        let reference = item.as_ref().map(|item| {
            let copied = item.clone();
            div()
                .flex()
                .flex_none()
                .items_center()
                .child(div().font_family(fonts::MONO).text_xs().text_color(colors.text_muted).child(SharedString::from(item.qualified_ref.clone())))
                .child(
                    button::revealed(&self.hits, format!("work-activity-copy-{}", e.seq), Icon::Copy.svg(11., colors.text_muted), px(COPY_SIDE), key.clone(), false, cx)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.copy_ref(&copied, cx))),
                )
        });
        div()
            .id(SharedString::from(key.clone()))
            .group(SharedString::from(key.clone()))
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py(px(3.))
            .border_t_1()
            .border_color(colors.border_variant)
            .text_sm()
            .child(fonts::figures(div().flex_none().w(px(40.)).text_xs().text_color(colors.text_muted)).child(SharedString::from(at)))
            .child(div().flex_none().text_xs().text_color(colors.text_muted).child(SharedString::from(e.kind.clone())))
            .children(reference)
            .child(clipped(format!("work-activity-sentence-{}", e.seq), sentence(e), false).flex_1())
            .child(div().flex_none().text_xs().text_color(colors.text_muted).child(SharedString::from(actor(e))))
            .child(hit_target(&self.hits, key))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use supraman_workman::ItemRef;

    use super::*;

    fn event(seq: i64, at: &str, summary: Option<&str>) -> JournalEvent {
        JournalEvent {
            seq,
            at: at.into(),
            kind: "transition".into(),
            actor: "aw-1".into(),
            item: Some(ItemRef { qualified_ref: "w/3".into(), ..Default::default() }),
            summary: summary.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn events_group_by_their_local_day_in_the_order_given() {
        let day1 = rfc3339_ms("2026-11-06T12:00:00Z").unwrap();
        let day2 = day1 + 24 * 3_600_000;
        let at = |ms: i64| time::rfc3339_utc(ms);
        let events = [event(3, &at(day2), None), event(2, &at(day1 + 60_000), Some("w/3 open → in_progress")), event(1, &at(day1), None)];
        let days: Vec<(String, Vec<i64>)> = by_day(&events).into_iter().map(|(d, es)| (d, es.iter().map(|e| e.seq).collect())).collect();
        assert_eq!(days, [(time::day(day2), vec![3]), (time::day(day1), vec![2, 1])]);
        assert_eq!(sentence(&events[1]), "w/3 open → in_progress");
        assert_eq!(sentence(&events[0]), "transition w/3", "no summary: its kind and ref");
        assert_eq!(actor(&events[0]), "aw-1");
    }
}
