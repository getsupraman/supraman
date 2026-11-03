//! The v1 journal pages forward from `since` and back from `before`, both exclusive; neither is the newest page.

use supraman_workman::{Journal, JournalEvent, JournalQuery};
use gpui::ScrollHandle;

use super::*;

pub const PAGE: u32 = 50;
/// Forward reads take the server's maximum page.
const FORWARD_LIMIT: u32 = 500;
const FORWARD_PAGES: usize = 20;
const OLDER_LIMIT: u32 = 200;
const KEPT: usize = 2000;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JournalFeed {
    pub events: Vec<JournalEvent>,
    /// The `since` of the next forward read: the newest `seq` this feed reflects.
    pub cursor: i64,
    pub older: bool,
    pub loading_older: bool,
    pub error: Option<String>,
}

impl JournalFeed {
    pub fn from_newest(page: Journal) -> Self {
        // Without `since`, an empty page's cursor is the snapshot's seq, so the next read is a forward one.
        let cursor = page.events.last().map_or(page.seq, |e| e.seq);
        JournalFeed { older: page.has_more, events: page.events, cursor, ..Default::default() }
    }

    pub fn append(&mut self, events: Vec<JournalEvent>, cursor: i64) -> usize {
        let last = self.events.last().map_or(i64::MIN, |e| e.seq);
        let before = self.events.len();
        self.events.extend(events.into_iter().filter(|e| e.seq > last));
        self.cursor = self.cursor.max(cursor);
        if self.events.len() > KEPT {
            self.events.drain(..self.events.len() - KEPT);
            self.older = true;
        }
        self.events.len() - before
    }

    pub fn prepend(&mut self, events: Vec<JournalEvent>, more: bool) {
        let first = self.events.first().map_or(i64::MAX, |e| e.seq);
        let mut older: Vec<JournalEvent> = events.into_iter().filter(|e| e.seq < first).collect();
        let cut = (older.len() + self.events.len()).saturating_sub(KEPT).min(older.len());
        older.drain(..cut);
        older.append(&mut self.events);
        self.events = older;
        self.older = more || cut > 0;
    }
}

pub fn after(client: &Client, scope: &str, since: i64) -> supraman_workman::Result<(Vec<JournalEvent>, i64)> {
    let mut events = Vec::new();
    let mut cursor = since;
    for _ in 0..FORWARD_PAGES {
        let page = client.journal(&JournalQuery { scope: Some(scope), since: Some(cursor.max(1)), limit: Some(FORWARD_LIMIT), ..Default::default() })?;
        cursor = cursor.max(page.cursor);
        events.extend(page.events);
        if !page.has_more {
            break;
        }
    }
    Ok((events, cursor))
}

/// The page just below `oldest`, ascending, and whether older events exist below it.
pub fn older(client: &Client, scope: &str, oldest: i64) -> supraman_workman::Result<(Vec<JournalEvent>, bool)> {
    let page = client.journal(&JournalQuery { scope: Some(scope), before: Some(oldest), limit: Some(OLDER_LIMIT), ..Default::default() })?;
    Ok((page.events, page.has_more))
}

pub struct JournalView {
    pub open: bool,
    pub feed: Option<JournalFeed>,
    reading: bool,
    pending: bool,
    pub reads: u64,
    scroll: ScrollHandle,
}

impl Default for JournalView {
    fn default() -> Self {
        JournalView { open: false, feed: None, reading: false, pending: false, reads: 0, scroll: ScrollHandle::new() }
    }
}

impl WorkView {
    pub fn toggle_journal(&mut self, cx: &mut Context<Self>) {
        self.journal.open = !self.journal.open;
        if self.journal.open {
            match self.journal.feed {
                None => self.read_journal(cx),
                Some(_) => self.journal.scroll.scroll_to_bottom(),
            }
        }
        cx.notify();
    }

    /// The journal may have been opened before the overview named the project.
    pub(super) fn journal_project_known(&mut self, cx: &mut Context<Self>) {
        if self.journal.open && self.journal.feed.is_none() && !self.journal.reading {
            self.read_journal(cx);
        }
    }

    pub(super) fn schedule_journal(&mut self, cx: &mut Context<Self>) {
        if self.journal.feed.is_none() || self.journal.pending {
            return;
        }
        self.journal.pending = true;
        after_debounce(cx, |this, cx| {
            this.journal.pending = false;
            this.read_journal(cx);
        });
    }

    pub(super) fn read_journal(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.as_ref().map(|p| p.id.clone()) else { return };
        if self.journal.reading {
            // One read at a time; the one running is followed by another.
            self.journal.pending = true;
            return;
        }
        let Some(client) = self.client(cx) else { return };
        let since = self.journal.feed.as_ref().map(|f| f.cursor);
        self.journal.reading = true;
        self.journal.reads += 1;
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move {
                    match since {
                        None => client
                            .journal(&JournalQuery { scope: Some(&project), limit: Some(PAGE), ..Default::default() })
                            .map(|page| (Some(page), Vec::new(), 0)),
                        Some(since) => after(&client, &project, since).map(|(events, cursor)| (None, events, cursor)),
                    }
                    .map_err(|e| format!("Cannot read the journal: {e}"))
                })
                .await;
            this.update(cx, |this, cx| {
                this.journal.reading = false;
                let mut grew = false;
                match (read, this.journal.feed.as_mut()) {
                    (Ok((Some(page), _, _)), _) => {
                        this.journal.feed = Some(JournalFeed::from_newest(page));
                        grew = true;
                    }
                    (Ok((None, events, cursor)), Some(feed)) => {
                        feed.error = None;
                        grew = feed.append(events, cursor) > 0;
                    }
                    (Ok((None, _, _)), None) => {}
                    (Err(message), Some(feed)) => feed.error = Some(message),
                    (Err(message), None) => this.journal.feed = Some(JournalFeed { error: Some(message), ..Default::default() }),
                }
                if grew {
                    this.journal.scroll.scroll_to_bottom();
                }
                if std::mem::take(&mut this.journal.pending) {
                    this.schedule_journal(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn load_older(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.as_ref().map(|p| p.id.clone()) else { return };
        let Some(feed) = self.journal.feed.as_mut().filter(|f| f.older && !f.loading_older) else { return };
        let Some(oldest) = feed.events.first().map(|e| e.seq) else { return };
        let Some(client) = self.client(cx) else { return };
        if let Some(feed) = self.journal.feed.as_mut() {
            feed.loading_older = true;
        }
        self.journal.reads += 1;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move { older(&client, &project, oldest).map_err(|e| format!("Cannot read older events: {e}")) })
                .await;
            this.update(cx, |this, cx| {
                let Some(feed) = this.journal.feed.as_mut() else { return };
                feed.loading_older = false;
                match read {
                    Ok((events, more)) => {
                        feed.error = None;
                        feed.prepend(events, more);
                    }
                    Err(message) => feed.error = Some(message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_event(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected = Some(id.clone());
        self.on_select_item(&id, cx);
        cx.notify();
    }

    pub(super) fn render_journal(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let mut list = div()
            .id(SharedString::from(format!("work-journal-{}", self.workspace)))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.journal.scroll)
            .px_3()
            .pb_2();
        let Some(feed) = &self.journal.feed else {
            return list.child(div().py_2().text_color(colors.text_muted).child("Reading the journal…")).into_any_element();
        };
        if feed.older {
            let label = if feed.loading_older { "loading older…" } else { "load older" };
            list = list.child(
                button::outline(&self.hits, "journal-older", label, colors.border, cx)
                    .self_start()
                    .my_1()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.load_older(cx))),
            );
        }
        if feed.events.is_empty() && feed.error.is_none() {
            list = list.child(div().py_2().text_color(colors.text_muted).child("No events yet."));
        }
        for e in &feed.events {
            let who = e.session.as_ref().map(|s| s.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| e.actor.clone());
            let target = e.item.as_ref().or(e.run.as_ref());
            let key = format!("journal-event-{}", e.seq);
            // Two lines, so the summary keeps the full width of a pane narrowed by the item page.
            let meta = div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(colors.text_muted)
                .child(div().flex_none().child(SharedString::from(rfc3339_local(&e.at))))
                .child(
                    div()
                        .flex_none()
                        .text_color(if e.item.is_some() { colors.text_accent } else { colors.text_muted })
                        .child(SharedString::from(target.map(|t| t.qualified_ref.clone()).unwrap_or_else(|| "—".into()))),
                )
                .child(div().flex_none().child(SharedString::from(e.kind.clone())))
                .child(supraman_ui::clipped::clipped(format!("{key}-who"), who, false).flex_1());
            let row = div()
                .id(SharedString::from(key.clone()))
                .relative()
                .flex_none()
                .flex()
                .flex_col()
                .py(px(3.))
                .border_b_1()
                .border_color(colors.border_variant)
                .child(meta)
                .child(supraman_ui::clipped::clipped(format!("{key}-summary"), e.summary.clone().unwrap_or_default(), false).text_sm())
                .child(hit_target(&self.hits, key));
            list = list.child(match e.item.as_ref() {
                Some(item) => {
                    let id = item.id.clone();
                    row.cursor_pointer()
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .active(|s| s.bg(colors.ghost_element_active))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_event(id.clone(), cx)))
                        .into_any_element()
                }
                None => row.into_any_element(),
            });
        }
        if let Some(error) = &feed.error {
            list = list.child(div().py_1().text_xs().text_color(status.error).child(SharedString::from(error.clone())));
        }
        list.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use supraman_workman::fake::{Answer, FakeWorkman};
    use serde_json::json;

    use super::*;

    fn event(seq: i64) -> JournalEvent {
        JournalEvent { seq, ..Default::default() }
    }

    fn seqs(events: &[JournalEvent]) -> Vec<i64> {
        events.iter().map(|e| e.seq).collect()
    }

    fn page(events: &[i64], cursor: i64, more: bool) -> Answer {
        let events: Vec<_> = events.iter().map(|s| json!({ "seq": s, "at": "2026-10-26T12:00:00+00:00", "kind": "transition" })).collect();
        Answer::json(json!({ "seq": 999, "cursor": cursor, "hasMore": more, "events": events }).to_string())
    }

    #[test]
    fn a_feed_appends_only_newer_and_prepends_only_older() {
        let mut feed = JournalFeed::from_newest(Journal { seq: 30, has_more: true, events: vec![event(20), event(30)], ..Default::default() });
        assert_eq!((feed.cursor, feed.older), (30, true));
        assert_eq!(feed.append(vec![event(30), event(31), event(35)], 35), 2);
        assert_eq!(seqs(&feed.events), [20, 30, 31, 35]);
        assert_eq!(feed.cursor, 35);
        feed.prepend(vec![event(5), event(20), event(25)], false);
        assert_eq!(seqs(&feed.events), [5, 20, 30, 31, 35], "25 is not older than the oldest shown");
        assert!(!feed.older);
        let empty = JournalFeed::from_newest(Journal { seq: 77, ..Default::default() });
        assert_eq!(empty.cursor, 77, "an empty journal reads forward from the snapshot's seq");
    }

    #[test]
    fn older_pages_stop_at_the_cap_and_keep_the_newest() {
        let newest = KEPT as i64 * 2;
        let mut feed = JournalFeed::from_newest(Journal { seq: newest, events: vec![event(newest)], ..Default::default() });
        feed.prepend((1..newest).map(event).collect(), false);
        assert_eq!(feed.events.len(), KEPT);
        assert_eq!(feed.events.last().map(|e| e.seq), Some(newest));
        assert_eq!(feed.events.first().map(|e| e.seq), Some(newest - KEPT as i64 + 1));
        assert!(feed.older, "the events cut are older ones still to read");
    }

    #[test]
    fn older_pages_back_by_the_cursor_down_to_the_first_event() {
        let fake = FakeWorkman::start(Vec::new());
        let journal = fake.enable_journal();
        journal.add("p1", 1..=300);
        journal.add("other", (301..=700).step_by(2));
        journal.add("p1", (302..=700).step_by(2));
        let client = Client::new(fake.base(), "fake-test-reader");

        let (events, more) = older(&client, "p1", 650).unwrap();
        assert_eq!(events.len(), OLDER_LIMIT as usize);
        assert_eq!((events.first().map(|e| e.seq), events.last().map(|e| e.seq), more), (Some(275), Some(648), true), "ascending, gaps of the other scope skipped");
        let (events, more) = older(&client, "p1", 275).unwrap();
        assert_eq!((seqs(&events), more), ((75..275).collect::<Vec<_>>(), true));
        let (events, more) = older(&client, "p1", 75).unwrap();
        assert_eq!((seqs(&events), more), ((1..75).collect::<Vec<_>>(), false), "seq 1 is shown");
        let (events, more) = older(&client, "p1", 1).unwrap();
        assert_eq!((events.len(), more), (0, false));
        let asked: Vec<String> = fake.seen().into_iter().map(|s| s.target).collect();
        assert_eq!(asked[0], "/api/v1/journal?scope=p1&before=650&limit=200");
    }

    #[test]
    fn after_follows_the_cursor_while_there_is_more() {
        let target = |since: i64| format!("/api/v1/journal?scope=p1&since={since}&limit=500");
        let fake = FakeWorkman::start(vec![(target(10), page(&[11, 12], 12, true)), (target(12), page(&[13], 13, false))]);
        let client = Client::new(fake.base(), "fake-test-reader");
        let (events, cursor) = after(&client, "p1", 10).unwrap();
        assert_eq!((seqs(&events), cursor), (vec![11, 12, 13], 13));
    }
}
