use chrono::{DateTime, Datelike, Days, Local, LocalResult, NaiveDate, NaiveDateTime, TimeDelta, TimeZone};

use super::Trigger;

/// The first fire strictly after `after`, in `after`'s zone.
pub fn next_fire<Tz: TimeZone>(trigger: &Trigger, after: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let today = after.date_naive();
    window(today, 1, 8).flat_map(|d| fires_on(trigger, &after.timezone(), d)).filter(|t| t > after).min()
}

/// The last fire at or before `at`, in `at`'s zone.
pub fn previous_fire<Tz: TimeZone>(trigger: &Trigger, at: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let today = at.date_naive();
    window(today, 8, 1).flat_map(|d| fires_on(trigger, &at.timezone(), d)).filter(|t| t <= at).max()
}

/// [`next_fire`] in this machine's zone, as epoch ms.
pub fn next_fire_local(trigger: &Trigger, after_ms: i64) -> Option<i64> {
    let after = Local.timestamp_millis_opt(after_ms).single()?;
    next_fire(trigger, &after).map(|t| t.timestamp_millis())
}

fn window(day: NaiveDate, back: u64, ahead: u64) -> impl Iterator<Item = NaiveDate> {
    let first = day.checked_sub_days(Days::new(back)).unwrap_or(day);
    first.iter_days().take((back + ahead + 1) as usize)
}

/// The fires of the cycle that starts on `day`; an interval's cycle runs into the next day.
fn fires_on<Tz: TimeZone>(trigger: &Trigger, tz: &Tz, day: NaiveDate) -> Vec<DateTime<Tz>> {
    let starts: Vec<NaiveDateTime> = match trigger {
        Trigger::Weekly { days, at } => days.iter().filter(|d| d.weekday() == day.weekday()).take(1).map(|_| day.and_time(at.time())).collect(),
        Trigger::Daily { at } => vec![day.and_time(at.time())],
        Trigger::Interval { at, every } => {
            let start = day.and_time(at.time());
            let step = every.0.max(60) as i64;
            (0..).map(|k| k * step).take_while(|s| *s < 24 * 3600).map(|s| start + TimeDelta::seconds(s)).collect()
        }
    };
    let mut fires: Vec<DateTime<Tz>> = starts.into_iter().filter_map(|t| resolve(tz, t)).collect();
    fires.sort();
    fires.dedup();
    fires
}

/// A wall time a DST change skips fires at the first minute after the gap; one it repeats fires once, at
/// the earlier.
fn resolve<Tz: TimeZone>(tz: &Tz, local: NaiveDateTime) -> Option<DateTime<Tz>> {
    (0..=24 * 60).find_map(|m| match tz.from_local_datetime(&(local + TimeDelta::minutes(m))) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t),
        LocalResult::None => None,
    })
}
