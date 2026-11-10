//! The palette's picks, `<home>/gitman/palette.json`, which break ties of an equal fuzzy score; its one reader and writer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{App, Global};
use serde::{Deserialize, Serialize};
use supraman_core::store::{JsonStore, Save};

const LOST: &str = "The Git palette ranks without your recent picks until the file is fixed or removed";
/// The picks kept per item.
const KEPT: usize = 10;
/// The queries kept; the one picked from longest ago goes first.
const QUERIES: usize = 200;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Picks {
    pub(crate) count: u32,
    /// Epoch milliseconds, newest last.
    pub(crate) times: Vec<i64>,
}

impl Picks {
    fn pick(&mut self, now: i64) {
        self.count = self.count.saturating_add(1);
        self.times.push(now);
        let over = self.times.len().saturating_sub(KEPT);
        self.times.drain(..over);
    }

    /// The count times the mean weight of the picks kept: frequent and recent picks score high.
    pub(crate) fn score(&self, now: i64) -> u32 {
        if self.times.is_empty() {
            return 0;
        }
        let sum: u32 = self.times.iter().map(|t| weight(now - t)).sum();
        self.count.saturating_mul(sum) / self.times.len() as u32
    }
}

/// A pick's weight by its age.
pub(crate) fn weight(age_ms: i64) -> u32 {
    match age_ms {
        a if a <= 4 * HOUR => 100,
        a if a <= DAY => 80,
        a if a <= 3 * DAY => 60,
        a if a <= 7 * DAY => 45,
        a if a <= 15 * DAY => 30,
        a if a <= 30 * DAY => 10,
        _ => 0,
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct File {
    #[serde(default)]
    pub(crate) items: BTreeMap<String, Picks>,
    #[serde(default)]
    pub(crate) queries: BTreeMap<String, BTreeMap<String, Picks>>,
}

fn normal(query: &str) -> String {
    query.trim().to_lowercase()
}

impl File {
    pub(crate) fn pick(&mut self, key: &str, query: &str, now: i64) {
        self.items.entry(key.to_string()).or_default().pick(now);
        let query = normal(query);
        if query.is_empty() {
            return;
        }
        self.queries.entry(query).or_default().entry(key.to_string()).or_default().pick(now);
        while self.queries.len() > QUERIES {
            let last = |picks: &BTreeMap<String, Picks>| picks.values().filter_map(|p| p.times.last().copied()).max().unwrap_or(0);
            let Some(oldest) = self.queries.iter().min_by_key(|(_, picks)| last(picks)).map(|(q, _)| q.clone()) else { break };
            self.queries.remove(&oldest);
        }
    }

    /// `key`'s boost for `query`: its picks for this very query count twice its picks at all.
    pub(crate) fn boost(&self, key: &str, query: &str, now: i64) -> u32 {
        let own = self.queries.get(&normal(query)).and_then(|q| q.get(key)).map_or(0, |p| p.score(now));
        self.items.get(key).map_or(0, |p| p.score(now)).saturating_add(own.saturating_mul(2))
    }
}

pub(crate) fn path(home: &Path) -> PathBuf {
    home.join("gitman").join("palette.json")
}

struct Frecency {
    store: Option<JsonStore<File>>,
    notice: Option<String>,
}

impl Global for Frecency {}

/// Reads the file on the background executor, once.
pub(crate) fn start(home: Option<PathBuf>, cx: &mut App) {
    if cx.has_global::<Frecency>() {
        return;
    }
    cx.set_global(Frecency { store: None, notice: None });
    let read = cx.background_executor().spawn(async move {
        JsonStore::load_kept(home.as_deref().map(path), LOST)
    });
    cx.spawn(async move |cx| {
        let (store, notice) = read.await;
        cx.update(|cx| *cx.global_mut::<Frecency>() = Frecency { store: Some(store), notice });
    })
    .detach();
}

/// The picks as they stand; none until the file is read.
pub(crate) fn snapshot(cx: &App) -> Rc<File> {
    Rc::new(cx.try_global::<Frecency>().and_then(|f| f.store.as_ref()).map(|s| s.get().clone()).unwrap_or_default())
}

/// Why the file was not read, once.
pub(crate) fn take_notice(cx: &mut App) -> Option<String> {
    if cx.has_global::<Frecency>() { cx.global_mut::<Frecency>().notice.take() } else { None }
}

/// Keeps a pick; the save of the change, unless the file is not read yet.
pub(crate) fn remember(key: &str, query: &str, now: i64, cx: &mut App) -> Option<Save> {
    if !cx.has_global::<Frecency>() {
        return None;
    }
    cx.global_mut::<Frecency>().store.as_mut()?.edit(|file| {
        file.pick(key, query, now);
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pick_weighs_less_as_it_ages_and_more_as_it_repeats() {
        assert_eq!([0, 5 * HOUR, 2 * DAY, 5 * DAY, 10 * DAY, 20 * DAY, 40 * DAY].map(weight), [100, 80, 60, 45, 30, 10, 0]);
        let now = 100 * DAY;
        let mut once = Picks::default();
        once.pick(now);
        let mut twice = once.clone();
        twice.pick(now);
        assert!(twice.score(now) > once.score(now));
        assert!(once.score(now + 2 * DAY) < once.score(now));
        let mut many = Picks::default();
        (0..15).for_each(|i| many.pick(i));
        assert_eq!((many.count, many.times.len(), many.times[0]), (15, KEPT, 5), "the last ten kept, all counted");
    }

    #[test]
    fn a_pick_for_this_query_counts_above_one_for_another() {
        let now = 10 * DAY;
        let mut file = File::default();
        file.pick("a", "ref", now);
        file.pick("b", "zz", now);
        assert!(file.boost("a", " REF", now) > file.boost("b", "ref", now));
        assert_eq!(file.boost("c", "ref", now), 0);
        file.pick("d", "", now);
        assert!(!file.queries.contains_key(""), "an empty query keeps no picks of its own");
    }

    #[test]
    fn the_queries_kept_are_capped_by_their_last_pick() {
        let mut file = File::default();
        (0..QUERIES as i64 + 3).for_each(|i| file.pick("k", &format!("q{i}"), i));
        assert_eq!(file.queries.len(), QUERIES);
        assert!(!file.queries.contains_key("q0") && file.queries.contains_key(&format!("q{}", QUERIES + 2)));
    }
}
