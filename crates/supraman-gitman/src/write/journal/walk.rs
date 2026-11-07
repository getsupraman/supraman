//! HEAD's reflog and the journal merged newest first, and the walk from the newest down to what Undo and Redo act on.

use std::collections::HashMap;

/// The journal ref's line that parks it on the rest commit after each entry.
pub(super) const REST: &str = "rest";
pub(super) const UNDO: &str = "undo: ";

/// One reflog line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub old: String,
    pub new: String,
    pub time: i64,
    pub message: String,
}

impl Line {
    /// Old id, new id, time and a short hash of the message: the HEAD line a journal entry follows.
    pub fn fingerprint(&self) -> String {
        format!("{}.{}.{}.{:08x}", self.old, self.new, self.time, fnv(&self.message))
    }

    fn noop_reset(&self) -> bool {
        self.old == self.new && self.message.starts_with("reset: moving to ")
    }
}

fn fnv(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

pub fn is_zero(sha: &str) -> bool {
    sha.bytes().all(|b| b == b'0')
}

/// A reflog file's lines, oldest first; a line it cannot read is left out.
pub fn parse_log(text: &str) -> Vec<Line> {
    text.lines()
        .filter_map(|line| {
            let (head, message) = line.split_once('\t').unwrap_or((line, ""));
            let fields: Vec<&str> = head.split(' ').collect();
            let [old, new, ..] = fields.as_slice() else { return None };
            let time = fields.iter().rev().nth(1)?.parse().ok()?;
            Some(Line { old: old.to_string(), new: new.to_string(), time, message: message.to_string() })
        })
        .collect()
}

/// What GitMan can undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Commit,
    Amend,
    Checkout,
    Reset,
    Rebase,
    Discard,
    DeleteBranch,
}

impl Kind {
    /// The kind a reflog message or a journal label names; `rebase: from` is GitMan's own label, git's rebase
    /// lines are none.
    pub fn of(label: &str) -> Option<Kind> {
        const PREFIXES: [(&str, Kind); 7] = [
            ("commit: ", Kind::Commit),
            ("commit (amend): ", Kind::Amend),
            ("checkout: moving from ", Kind::Checkout),
            ("reset: moving to ", Kind::Reset),
            ("rebase: from ", Kind::Rebase),
            ("discard: ", Kind::Discard),
            ("delete branch: ", Kind::DeleteBranch),
        ];
        PREFIXES.iter().find(|(p, _)| label.starts_with(p)).map(|(_, k)| *k)
    }

    pub fn moves_head(self) -> bool {
        !matches!(self, Kind::Discard | Kind::DeleteBranch)
    }
}

/// A line of the journal: `<label> head:<HEAD> after:<fingerprint>`, its new id the marker commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub marker: String,
    /// `undo: ` and all.
    pub label: String,
    pub anchor: String,
}

impl Entry {
    pub fn parse(line: &Line) -> Option<Entry> {
        if line.message == REST || is_zero(&line.new) {
            return None;
        }
        let (rest, anchor) = line.message.rsplit_once(" after:")?;
        let (label, _head) = rest.rsplit_once(" head:")?;
        Some(Entry { marker: line.new.clone(), label: label.to_string(), anchor: anchor.to_string() })
    }

    /// The reflog message of an entry labelled `label`, written while HEAD is `head` and its newest line `after`.
    pub fn message(label: &str, head: &str, after: Option<&Line>) -> String {
        let anchor = after.map_or_else(|| "none".to_string(), Line::fingerprint);
        format!("{label} head:{head} after:{anchor}")
    }
}

/// One step back in the repository's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The action's words, without `undo: `.
    pub label: String,
    pub undo: bool,
    /// The HEAD line it wrote, the newest for a rebase.
    pub head: Option<Line>,
    /// Where HEAD was before it, for an action that moved HEAD.
    pub from: Option<String>,
    pub entry: Option<Entry>,
}

impl Item {
    fn of_head(line: &Line, entry: Option<Entry>) -> Item {
        let (undo, label) = split_undo(entry.as_ref().map_or(&line.message, |e| &e.label));
        Item { label: label.to_string(), undo, head: Some(line.clone()), from: Some(line.old.clone()), entry }
    }

    fn of_entry(entry: &Entry) -> Item {
        let (undo, label) = split_undo(&entry.label);
        Item { label: label.to_string(), undo, head: None, from: None, entry: Some(entry.clone()) }
    }

    /// The kind GitMan can undo, none for an entry it cannot.
    pub fn kind(&self) -> Option<Kind> {
        let kind = Kind::of(&self.label)?;
        match (&self.head, &self.entry) {
            (Some(_), None) if kind == Kind::Rebase => None,
            (Some(_), _) if kind == Kind::Commit && self.from.as_deref().is_some_and(is_zero) => None,
            (Some(_), _) if !kind.moves_head() => None,
            (None, _) if kind.moves_head() => None,
            _ => Some(kind),
        }
    }

    /// What the reflog says, for an entry GitMan cannot undo.
    pub fn message(&self) -> &str {
        self.head.as_ref().map_or(&self.label, |l| &l.message)
    }
}

fn split_undo(label: &str) -> (bool, &str) {
    match label.strip_prefix(UNDO) {
        Some(rest) => (true, rest),
        None => (false, label),
    }
}

/// HEAD's lines and the journal's entries (both oldest first) as items, newest first: an entry follows the HEAD
/// line it names; one of an action that moved HEAD is that line's (a rebase's also takes the lines down to its
/// start); a reset that moved nothing (a stash's, or the one an undo of a hard reset runs) is left out.
pub fn items(head: &[Line], journal: &[Entry]) -> Vec<Item> {
    let mut anchored: HashMap<&str, Vec<&Entry>> = HashMap::new();
    for entry in journal.iter().rev() {
        anchored.entry(entry.anchor.as_str()).or_default().push(entry);
    }
    let mut items = Vec::new();
    let mut lines = head.iter().rev().peekable();
    while let Some(line) = lines.next() {
        let mut own = None;
        for entry in anchored.remove(line.fingerprint().as_str()).unwrap_or_default() {
            let (undo, label) = split_undo(&entry.label);
            match Kind::of(label) {
                Some(kind) if kind.moves_head() => {
                    if !undo && own.is_none() {
                        own = Some(entry.clone());
                    }
                }
                _ => items.push(Item::of_entry(entry)),
            }
        }
        if own.is_none() && line.noop_reset() {
            continue;
        }
        let mut item = Item::of_head(line, own);
        if let Some(from) = item.label.strip_prefix("rebase: from ").map(str::to_string).filter(|_| item.entry.is_some()) {
            let mut at = line;
            while at.old != from && lines.peek().is_some_and(|l| l.message.starts_with("rebase")) {
                at = lines.next().expect("peeked");
            }
            item.from = Some(from);
        }
        items.push(item);
    }
    let unanchored = journal.iter().rev().filter(|e| anchored.contains_key(e.anchor.as_str()));
    items.extend(unanchored.filter(|e| Kind::of(split_undo(&e.label).1).is_some_and(|k| !k.moves_head())).map(Item::of_entry));
    items
}

/// What Undo would act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    Undo(Item, Kind),
    /// The newest entry not undone is one GitMan cannot undo; its reflog message.
    Cannot(String),
}

/// Redo of the newest undo: its own item and the action it undid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Again {
    pub undo: Item,
    pub original: Item,
    pub kind: Kind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Walk {
    pub undo: Option<Next>,
    pub redo: Option<Again>,
}

/// From the newest item down: an `undo: X` pairs with the next X, so Undo acts on the newest action not undone
/// and stops at the first it cannot undo; Redo acts on the newest item while it is an undo.
pub fn walk(items: &[Item]) -> Walk {
    let mut pending = 0usize;
    let mut walk = Walk::default();
    for item in items {
        if item.undo {
            pending += 1;
            continue;
        }
        let Some(kind) = item.kind() else {
            walk.undo = Some(Next::Cannot(item.message().to_string()));
            return walk;
        };
        if pending == 0 {
            walk.undo = Some(Next::Undo(item.clone(), kind));
            return walk;
        }
        pending -= 1;
        if pending == 0 && items[0].undo && walk.redo.is_none() {
            walk.redo = Some(Again { undo: items[0].clone(), original: item.clone(), kind });
        }
    }
    walk
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(old: &str, new: &str, time: i64, message: &str) -> Line {
        Line { old: old.into(), new: new.into(), time, message: message.into() }
    }

    fn entry(label: &str, after: &Line) -> Entry {
        Entry { marker: format!("m{}", label.len()), label: label.into(), anchor: after.fingerprint() }
    }

    #[test]
    fn a_reflog_line_reads_with_its_ids_time_and_message() {
        let text = "0000 1111 Ada <a@x> 1700000000 +0100\tcommit (initial): Root\n1111 2222 Ada Lovelace <a@x> 1700000060 -0500\tcheckout: moving from main to feature\n";
        let lines = parse_log(text);
        assert_eq!(lines, [line("0000", "1111", 1_700_000_000, "commit (initial): Root"), line("1111", "2222", 1_700_000_060, "checkout: moving from main to feature")]);
        assert_ne!(lines[0].fingerprint(), line("0000", "1111", 1_700_000_000, "commit (initial): Roo").fingerprint());
    }

    #[test]
    fn undo_pairs_with_the_action_before_it_and_redo_takes_the_newest_undo() {
        let head = [line("0", "a", 1, "commit (initial): Root"), line("a", "b", 2, "commit: One"), line("b", "c", 3, "commit: Two")];
        let items_now = items(&head, &[]);
        assert!(matches!(walk(&items_now).undo, Some(Next::Undo(ref i, Kind::Commit)) if i.label == "commit: Two"));
        assert_eq!(walk(&items_now).redo, None);

        let undone = [&head[..], &[line("c", "b", 4, "undo: commit: Two"), line("b", "a", 5, "undo: commit: One")]].concat();
        let w = walk(&items(&undone, &[]));
        assert_eq!(w.undo, Some(Next::Cannot("commit (initial): Root".into())), "the initial commit stops the walk");
        let again = w.redo.expect("a redo");
        assert_eq!((again.undo.label.as_str(), again.original.label.as_str(), again.kind), ("commit: One", "commit: One", Kind::Commit));

        let redone = [&undone[..], &[line("a", "b", 6, "commit: One")]].concat();
        let w = walk(&items(&redone, &[]));
        assert!(matches!(w.undo, Some(Next::Undo(ref i, _)) if i.label == "commit: One" && i.head.as_ref().unwrap().time == 6));
        assert_eq!(w.redo, None, "a new action clears redo");
    }

    #[test]
    fn the_first_entry_gitman_cannot_undo_stops_the_walk() {
        let head = [line("0", "a", 1, "commit (initial): Root"), line("a", "b", 2, "merge feature: Merge made by the 'ort' strategy."), line("b", "c", 3, "commit: After")];
        let undone = [&head[..], &[line("c", "b", 4, "undo: commit: After")]].concat();
        let w = walk(&items(&undone, &[]));
        assert_eq!(w.undo, Some(Next::Cannot("merge feature: Merge made by the 'ort' strategy.".into())));
        assert!(w.redo.is_some());
        let rebase = [line("0", "a", 1, "commit: A"), line("a", "x", 2, "rebase (start): checkout main"), line("x", "y", 3, "rebase (finish): returning to refs/heads/b")];
        assert_eq!(walk(&items(&rebase, &[])).undo, Some(Next::Cannot("rebase (finish): returning to refs/heads/b".into())), "git's own rebase");
    }

    #[test]
    fn a_journal_entry_sits_after_its_head_line_and_one_of_a_head_move_is_that_line() {
        let head = [line("0", "a", 1, "commit (initial): Root"), line("a", "b", 2, "reset: moving to HEAD~1"), line("b", "b", 3, "reset: moving to HEAD"), line("b", "c", 4, "commit: Next")];
        let journal = [entry("reset: moving to HEAD~1", &head[1]), entry("discard: a.txt", &head[1]), entry("delete branch: old", &head[3])];
        let all = items(&head, &journal);
        let labels: Vec<&str> = all.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["delete branch: old", "commit: Next", "discard: a.txt", "reset: moving to HEAD~1", "commit (initial): Root"], "the stash's reset is left out");
        assert!(all[3].entry.is_some() && all[3].head.is_some(), "the reset's entry is its line's");
        assert_eq!(all[0].kind(), Some(Kind::DeleteBranch));
    }

    #[test]
    fn gitmans_rebase_takes_its_lines_down_to_its_start() {
        let head = [line("0", "a", 1, "commit: A"), line("a", "x", 2, "rebase (start): checkout main"), line("x", "y", 3, "rebase (pick): B"), line("y", "z", 4, "rebase (finish): returning to refs/heads/b")];
        let journal = [entry("rebase: from a", &head[3])];
        let all = items(&head, &journal);
        assert_eq!(all.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["rebase: from a", "commit: A"]);
        assert_eq!(all[0].from.as_deref(), Some("a"));
        assert!(matches!(walk(&all).undo, Some(Next::Undo(_, Kind::Rebase))));
    }

    #[test]
    fn a_journal_entry_parses_back_and_rest_lines_are_none() {
        let after = line("a", "b", 2, "commit: One");
        let written = line("0", "m1", 3, &Entry::message("discard: a.txt (and 2 other files)", "b", Some(&after)));
        assert_eq!(Entry::parse(&written), Some(Entry { marker: "m1".into(), label: "discard: a.txt (and 2 other files)".into(), anchor: after.fingerprint() }));
        assert_eq!(Entry::parse(&line("m1", "r", 3, REST)), None);
    }
}
