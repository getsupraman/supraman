//! What two non-empty listings in a row without a minted record's claude session mean: deleted, or unlisted (`claude rm`).

use supraman_claude::Session;

use super::{Absent, Keeper, RENEW_MARGIN, keeping, still_has_transcript};
use crate::Millis;

/// How long an unlisted record whose `expiresAt` is unknown is kept: Workman's 72 h idle window after the last use its listing showed, and [`RENEW_MARGIN`] for the renewal that may have run just before.
pub const UNLISTED_LAPSE: Millis = 72 * 3_600 * 1_000 + RENEW_MARGIN;

/// What [`Keeper::gone`] found among the records it was given.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Gone {
    /// (name, claude session id) of each record whose claude session and transcript are deleted: its owner [`super::revoke`]s it.
    pub deleted: Vec<(String, String)>,
    /// (name, claude session id) of each unlisted record whose token has lapsed: its owner drops it and [`Keeper::forget`]s it, no revoke.
    pub lapsed: Vec<(String, String)>,
    pub notices: Vec<String>,
}

impl Keeper {
    /// The one rule for `bound` (name, claude session id) records, by `absent` over `observed` (read without error): a session
    /// missing twice in a row is deleted, or, while its transcript remains, unlisted ([`keeping::Kept::unlisted_since`]), which
    /// [`super::Keeping`] does not renew; a listed one is unlisted no more.
    pub fn gone(&self, absent: &mut Absent, bound: &[(String, String)], observed: &[Session], now: Millis) -> Gone {
        let missing = absent.deleted(bound, observed);
        let mut gone = Gone::default();
        if observed.is_empty() || bound.is_empty() {
            return gone;
        }
        let mut unlisted = Vec::new();
        for (name, sid) in bound.iter().filter(|(_, sid)| missing.contains(sid)) {
            if still_has_transcript(self.claude_dir.as_deref(), sid) {
                unlisted.push(name.clone());
            } else {
                gone.deleted.push((name.clone(), sid.clone()));
            }
        }
        let listed: Vec<&String> = bound.iter().filter(|(_, sid)| observed.iter().any(|s| s.session_id == *sid)).map(|(n, _)| n).collect();
        let marked = |t: &keeping::Table| {
            unlisted.iter().any(|n| t.get(n).is_none_or(|k| k.unlisted_since.is_none())) || listed.iter().any(|n| t.get(*n).is_some_and(|k| k.unlisted_since.is_some()))
        };
        let table = match keeping::read(&self.home) {
            Ok(t) if !marked(&t) => Ok(t),
            _ => mark(&self.home, &unlisted, &listed, now),
        };
        match table {
            Ok(t) => gone.lapsed = bound.iter().filter(|(n, _)| t.get(n).is_some_and(|k| lapsed(k, now))).cloned().collect(),
            Err(e) => gone.notices.push(format!("{}{}; a session deleted with `claude rm` keeps its Workman token renewed", e[..1].to_uppercase(), &e[1..])),
        }
        gone
    }
}

/// `unlisted` marked from `now` (an earlier mark kept), `listed` unmarked; the table as saved.
fn mark(home: &std::path::Path, unlisted: &[String], listed: &[&String], now: Millis) -> Result<keeping::Table, String> {
    let mut saved = keeping::Table::new();
    keeping::edit(home, |t| {
        for name in unlisted {
            let kept = t.entry(name.clone()).or_default();
            if kept.unlisted_since.is_none() {
                log::info!("{name}: its claude session is no longer listed but its transcript remains, so its Workman token is left to lapse");
                kept.unlisted_since = Some(now);
            }
        }
        for name in listed {
            if let Some(kept) = t.get_mut(*name)
                && kept.unlisted_since.take().is_some()
            {
                log::info!("{name}: its claude session is listed again, so its Workman token is renewed again");
            }
        }
        saved = t.clone();
    })?;
    Ok(saved)
}

/// An unlisted record's token is past its kept `expiresAt`, or [`UNLISTED_LAPSE`] past its mark when that is unknown.
fn lapsed(kept: &keeping::Kept, now: Millis) -> bool {
    kept.unlisted_since.is_some_and(|since| now >= kept.expires_at.unwrap_or(since + UNLISTED_LAPSE))
}

#[cfg(test)]
mod tests;
