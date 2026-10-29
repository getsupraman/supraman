//! The Work tab's journal (m4/4): the project's `GET /api/v1/journal?scope=` events, newest last, in
//! place of the runs while toggled on. Opening reads the newest page; a stream event (debounced) or a
//! resync reads what came after the last `seq` shown; "load older" pages back. The v1 journal pages
//! only forward (`since` is exclusive, and no `since` is the newest page), so an older page is read
//! forward from a `seq` window below the oldest event shown, widened until it holds a page or reaches
//! the start ([`older`]).

use foreman_workstate::{Journal, JournalEvent, JournalQuery};
use gpui::ScrollHandle;

use super::*;

/// The newest page, and each older one at least (when there are that many).
pub const PAGE: u32 = 50;
/// Forward reads take the server's maximum page.
const FORWARD_LIMIT: u32 = 500;
/// Forward pages read for one answer at most; more is left for the next event.
const FORWARD_PAGES: usize = 20;
/// The first window below the oldest event shown, in `seq`s; ×4 per retry.
pub const OLDER_SPAN: i64 = 200;
/// Events kept in memory at most; the oldest go first.
const KEPT: usize = 2000;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JournalFeed {
    pub events: Vec<JournalEvent>,
    /// The `since` of the next forward read: the newest `seq` this feed reflects.
    pub cursor: i64,
    /// Older events exist (or may).
    pub older: bool,
    pub loading_older: bool,
    /// The last read failed (what is shown stays).
    pub error: Option<String>,
}

impl JournalFeed {
    pub fn from_newest(page: Journal) -> Self {
        // Without `since` the cursor is the newest event's seq, or 0 when there is none: then the
        // snapshot's seq, so the next read is a forward one.
        let cursor = page.events.last().map_or(page.seq, |e| e.seq);
        JournalFeed { older: page.has_more, events: page.events, cursor, ..Default::default() }
    }

    /// Appends what is newer than the newest shown; how many.
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

    /// Prepends what is older than the oldest shown, up to [`KEPT`] events in all: the newest stay.
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

/// Every event after `since`, ascending (at most [`FORWARD_PAGES`] pages), and the new cursor.
pub fn after(client: &Client, scope: &str, since: i64) -> foreman_workstate::Result<(Vec<JournalEvent>, i64)> {
    let mut events = Vec::new();
    let mut cursor = since;
    for _ in 0..FORWARD_PAGES {
        let page = client.journal(&JournalQuery { scope: Some(scope), since: Some(cursor.max(1)), limit: Some(FORWARD_LIMIT), kinds: &[] })?;
        cursor = cursor.max(page.cursor);
        events.extend(page.events);
        if !page.has_more {
            break;
        }
    }
    Ok((events, cursor))
}

/// The events just before `oldest`, ascending, and whether older ones may exist. The window
/// `(oldest − span, oldest)` is read forward; with fewer than [`PAGE`] events in it, it widens ×4 until
/// it reaches `since=1` (`since=0` would be the newest page, so `seq` 1, the workspace's first event,
/// is never shown). At most [`FORWARD_LIMIT`] events come back.
pub fn older(client: &Client, scope: &str, oldest: i64) -> foreman_workstate::Result<(Vec<JournalEvent>, bool)> {
    let mut span = OLDER_SPAN;
    loop {
        let low = (oldest - span).max(1);
        let mut got: Vec<JournalEvent> = Vec::new();
        let mut since = low;
        for _ in 0..FORWARD_PAGES {
            let page = client.journal(&JournalQuery { scope: Some(scope), since: Some(since), limit: Some(FORWARD_LIMIT), kinds: &[] })?;
            got.extend(page.events.into_iter().filter(|e| e.seq < oldest));
            if !page.has_more || page.cursor >= oldest - 1 || page.cursor <= since {
                break;
            }
            since = page.cursor;
        }
        if got.len() >= PAGE as usize || low == 1 {
            let cut = got.len().saturating_sub(FORWARD_LIMIT as usize);
            got.drain(..cut);
            return Ok((got, low > 1 || cut > 0));
        }
        span *= 4;
    }
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

    /// Once the overview names the project (the journal may have been opened before).
    pub(super) fn journal_project_known(&mut self, cx: &mut Context<Self>) {
        if self.journal.open && self.journal.feed.is_none() && !self.journal.reading {
            self.read_journal(cx);
        }
    }

    /// A stream event: the events after the cursor, debounced; nothing before the feed was read.
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

    /// The newest page when there is no feed yet, else what came after its cursor.
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
                            .journal(&JournalQuery { scope: Some(&project), since: None, limit: Some(PAGE), kinds: &[] })
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
                div()
                    .id("journal-older")
                    .relative()
                    .flex_none()
                    .self_start()
                    .my_1()
                    .px_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(colors.border)
                    .text_xs()
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .active(|s| s.bg(colors.ghost_element_active))
                    .child(label)
                    .child(hit_target(&self.hits, "journal-older".into()))
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
                .child(div().flex_none().child(SharedString::from(short_time(&e.at))))
                .child(
                    div()
                        .flex_none()
                        .text_color(if e.item.is_some() { colors.text_accent } else { colors.text_muted })
                        .child(SharedString::from(target.map(|t| t.qualified_ref.clone()).unwrap_or_else(|| "—".into()))),
                )
                .child(div().flex_none().child(SharedString::from(e.kind.clone())))
                .child(crate::clipped::clipped(format!("{key}-who"), who, false).flex_1());
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
                .child(crate::clipped::clipped(format!("{key}-summary"), e.summary.clone().unwrap_or_default(), false).text_sm())
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
    use foreman_workstate::fake::{Answer, FakeWorkState};
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
    fn older_widens_its_window_until_it_holds_a_page_or_the_start() {
        let scope = "p1";
        let target = |since: i64| format!("/api/v1/journal?scope={scope}&since={since}&limit=500");
        // 10 events in (400, 600), then 60 in (1, 600): the window widens from 200 to 800 (→ since=1).
        let near: Vec<i64> = (590..600).collect();
        let far: Vec<i64> = (540..600).collect();
        let fake = FakeWorkState::start(vec![(target(400), page(&near, 599, false)), (target(1), page(&far, 599, false))]);
        let client = Client::new(fake.base(), "fake-test-reader");
        let (events, more) = older(&client, scope, 600).unwrap();
        assert_eq!(events.len(), far.len());
        assert!(!more, "the window reached since=1");
        let asked: Vec<String> = fake.seen().into_iter().map(|s| s.target).collect();
        assert_eq!(asked, [target(400), target(1)]);

        // A page in the first window: no widening, and older ones may exist.
        let full: Vec<i64> = (1000..1060).collect();
        fake.set(target(860), page(&full, 1059, false));
        let (events, more) = older(&client, scope, 1060).unwrap();
        assert_eq!((events.len(), more), (60, true));
    }

    #[test]
    fn after_follows_the_cursor_while_there_is_more() {
        let target = |since: i64| format!("/api/v1/journal?scope=p1&since={since}&limit=500");
        let fake = FakeWorkState::start(vec![(target(10), page(&[11, 12], 12, true)), (target(12), page(&[13], 13, false))]);
        let client = Client::new(fake.base(), "fake-test-reader");
        let (events, cursor) = after(&client, "p1", 10).unwrap();
        assert_eq!((seqs(&events), cursor), (vec![11, 12, 13], 13));
    }
}
