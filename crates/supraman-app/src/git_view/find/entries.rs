//! The Git palette's rows as plain values: the read-only commands and the mode rows of the top list, and the
//! refs, files and loaded commits its modes search, each with how a query fits it and how it paints.

use std::ops::Range;
use std::rc::Rc;

use gpui::{AnyElement, App, FontWeight, SharedString, div, prelude::*, px};
use supraman_gitman::short_sha;
use supraman_ui::fonts;
use supraman_ui::icon::Icon;
use supraman_ui::matcher::highlighted;
use supraman_ui::palette::{Entry, Fit, Marks, Rank, fuzzy, marks_in, path_fuzzy, ranked_by};
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::super::load::GraphRow;
use super::super::ref_panel::{self, Section};

pub const REFS: &str = "refs";
pub const FILES: &str = "files";
pub const HISTORY: &str = "history";
pub const COMMITS: &str = "commits";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Refresh,
    ToggleRefs,
    FilterRefs,
    ShowAllRefs,
    GoToHead,
    GoToUpstream,
    GoToWip,
    FilesAsTree,
    FilesAsPaths,
    CloseViewer,
}

impl Command {
    pub fn id(self) -> &'static str {
        match self {
            Command::Refresh => "refresh",
            Command::ToggleRefs => "toggle-refs",
            Command::FilterRefs => "filter-refs",
            Command::ShowAllRefs => "show-all-refs",
            Command::GoToHead => "go-to-head",
            Command::GoToUpstream => "go-to-upstream",
            Command::GoToWip => "go-to-wip",
            Command::FilesAsTree => "files-as-tree",
            Command::FilesAsPaths => "files-as-paths",
            Command::CloseViewer => "close-viewer",
        }
    }

    pub fn words(self) -> (&'static str, &'static str) {
        match self {
            Command::Refresh => ("View", "Refresh"),
            Command::ToggleRefs => ("View", "Show or hide the ref panel"),
            Command::FilterRefs => ("Refs", "Filter refs"),
            Command::ShowAllRefs => ("Refs", "Show all refs in the graph"),
            Command::GoToHead => ("Graph", "Go to HEAD"),
            Command::GoToUpstream => ("Graph", "Go to the upstream"),
            Command::GoToWip => ("Graph", "Go to the work in progress"),
            Command::FilesAsTree => ("Commit panel", "Show the files as a tree"),
            Command::FilesAsPaths => ("Commit panel", "Show the files as paths"),
            Command::CloseViewer => ("Viewer", "Close the file"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
    Stash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitEntry {
    Command { command: Command, why_not: Option<SharedString> },
    Mode { mode: &'static str, label: &'static str, what: &'static str },
    Ref { name: String, kind: RefKind, sha: String, walked: bool },
    /// `history`: listed in History mode.
    File { path: String, history: bool },
    Commit { sha: SharedString, summary: SharedString, body: SharedString, author: SharedString, time_ms: i64 },
}

impl Entry for GitEntry {
    fn key(&self) -> SharedString {
        match self {
            GitEntry::Command { command, .. } => format!("cmd:{}", command.id()),
            GitEntry::Mode { mode, .. } => format!("mode:{mode}"),
            GitEntry::Ref { name, kind, .. } => format!("ref:{kind:?}:{name}"),
            GitEntry::File { path, history: false } => format!("file:{path}"),
            GitEntry::File { path, history: true } => format!("history:{path}"),
            GitEntry::Commit { sha, .. } => format!("commit:{sha}"),
        }
        .into()
    }

    fn text(&self) -> String {
        match self {
            GitEntry::Command { command, .. } => {
                let (group, label) = command.words();
                format!("{group}: {label}")
            }
            GitEntry::Mode { label, .. } => label.to_string(),
            GitEntry::Ref { name, .. } => name.clone(),
            GitEntry::File { path, .. } => path.clone(),
            GitEntry::Commit { sha, summary, author, .. } => format!("{summary}  {author}  {}", short_sha(sha)),
        }
    }

    fn mode(&self) -> Option<SharedString> {
        match self {
            GitEntry::Mode { mode, .. } => Some((*mode).into()),
            _ => None,
        }
    }

    fn dimmed(&self) -> Option<SharedString> {
        match self {
            GitEntry::Command { why_not, .. } => why_not.clone(),
            _ => None,
        }
    }
}

pub fn modes() -> [GitEntry; 4] {
    [
        GitEntry::Mode { mode: REFS, label: "Refs…", what: "branches, remote branches, tags and stashes  @" },
        GitEntry::Mode { mode: FILES, label: "Files…", what: "the files at HEAD, opened in the viewer  /" },
        GitEntry::Mode { mode: HISTORY, label: "History…", what: "a file at HEAD: GitMan has no file history yet" },
        GitEntry::Mode { mode: COMMITS, label: "Commits…", what: "the commits loaded in the graph  #" },
    ]
}

/// The refs the palette lists: local and remote branches, tags and stashes, from the ref panel's entries.
pub fn refs(entries: &[(String, ref_panel::Entry)], walk: &ref_panel::Walk) -> Vec<GitEntry> {
    entries
        .iter()
        .filter_map(|(path, entry)| {
            let kind = match Section::of(path)? {
                Section::Local => RefKind::Local,
                Section::Remote => RefKind::Remote,
                Section::Tags => RefKind::Tag,
                Section::Stashes => RefKind::Stash,
                Section::Worktrees => return None,
            };
            let name = match entry {
                ref_panel::Entry::Branch { name, .. } | ref_panel::Entry::Remote { name, .. } | ref_panel::Entry::Tag { name, .. } => name.clone(),
                ref_panel::Entry::Detached { .. } => "HEAD".into(),
                ref_panel::Entry::Stash { label, .. } => label.clone(),
                ref_panel::Entry::Worktree { .. } => return None,
            };
            Some(GitEntry::Ref { name, kind, sha: entry.commit()?.to_string(), walked: entry.tip_of(walk) })
        })
        .collect()
}

pub fn files(paths: Vec<String>, history: bool) -> Vec<GitEntry> {
    paths.into_iter().map(|path| GitEntry::File { path, history }).collect()
}

/// The graph's loaded commits, the WIP row and stashes left out.
pub fn commits(rows: &[GraphRow]) -> Vec<GitEntry> {
    rows.iter()
        .filter(|r| !r.wip() && r.stash().is_none())
        .map(|r| GitEntry::Commit { sha: r.sha.clone(), summary: r.summary.clone(), body: r.body.clone(), author: r.author.clone(), time_ms: r.time_ms })
        .collect()
}

/// A query of four or more hex digits that starts the sha ranks first, marked on the short sha; else a fuzzy
/// match of summary and author.
fn commit_fit(entry: &GitEntry, query: &str) -> Option<(Fit, Marks)> {
    let GitEntry::Commit { sha, .. } = entry else { return None };
    let text = entry.text();
    let word = query.trim();
    if word.len() >= 4 && word.chars().all(|c| c.is_ascii_hexdigit()) && sha.starts_with(&word.to_ascii_lowercase()) {
        let short = text.len() - short_sha(sha).len();
        return Some(((0, 0, 0), vec![short..short + word.len().min(short_sha(sha).len())]));
    }
    let searched = text.len() - short_sha(sha).len() - 2;
    fuzzy(&text[..searched], query).map(|((_, runs, first), marks)| ((1, runs, first), marks))
}

pub fn commit_rank() -> Rank<GitEntry> {
    Rc::new(|entries, query| ranked_by(entries, query, commit_fit, |_| 0))
}

pub fn file_rank() -> Rank<GitEntry> {
    Rc::new(|entries, query| ranked_by(entries, query, |e, q| path_fuzzy(&e.text(), q), |_| 0))
}

pub fn ref_rank() -> Rank<GitEntry> {
    Rc::new(|entries, query| ranked_by(entries, query, |e, q| fuzzy(&e.text(), q), |_| 0))
}

/// The top list: a fuzzy fit first, then `boost` (the picks kept) among equal fits.
pub fn top_rank(boost: impl Fn(&str, &str) -> u32 + 'static) -> Rank<GitEntry> {
    let boost = Rc::new(boost);
    Rc::new(move |entries, query| {
        let boost = boost.clone();
        ranked_by(entries, query, |e, q| fuzzy(&e.text(), q), move |e| boost(&e.key(), query))
    })
}

fn kind_mark(kind: RefKind) -> (Icon, &'static str) {
    match kind {
        RefKind::Local => (Section::Local.icon(), "Local branch"),
        RefKind::Remote => (Section::Remote.icon(), "Remote branch"),
        RefKind::Tag => (Section::Tags.icon(), "Tag"),
        RefKind::Stash => (Section::Stashes.icon(), "Stash"),
    }
}

pub fn paint(entry: &GitEntry, marks: &[Range<usize>], on: bool, cx: &mut App) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let key = entry.key();
    let row = || div().flex().items_center().gap_2().min_w_0().text_sm();
    let muted = if on { colors.text } else { colors.text_muted };
    match entry {
        GitEntry::Command { command, .. } => {
            let (group, label) = command.words();
            let at = group.len() + 2;
            row()
                .child(highlighted(format!("{key}-group"), group, &marks_in(marks, 0..group.len()), cx).flex_none().text_color(muted))
                .child(highlighted(format!("{key}-label"), label, &marks_in(marks, at..at + label.len()), cx))
                .into_any_element()
        }
        GitEntry::Mode { label, what, .. } => row()
            .child(highlighted(format!("{key}-label"), *label, marks, cx).flex_none().font_weight(FontWeight::MEDIUM))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_xs().text_color(muted).child(*what))
            .into_any_element(),
        GitEntry::Ref { name, kind, .. } => {
            let (icon, word) = kind_mark(*kind);
            row()
                .child(div().id(SharedString::from(format!("{key}-kind"))).flex_none().child(icon.svg(14., muted)).tooltip(Tooltip::text(word)))
                .child(highlighted(format!("{key}-name"), name.clone(), marks, cx))
                .into_any_element()
        }
        GitEntry::File { path, .. } => paint_path(&key, path, marks, muted, cx),
        GitEntry::Commit { .. } => paint_commit(entry, marks, muted, cx),
    }
}

/// The file's name whole, its folder after it cut at its end: a long path loses its middle.
fn paint_path(key: &str, path: &str, marks: &[Range<usize>], muted: gpui::Hsla, cx: &mut App) -> AnyElement {
    let name = path.rfind('/').map_or(0, |i| i + 1);
    let folder = &path[..name];
    div()
        .flex()
        .items_center()
        .gap_2()
        .min_w_0()
        .text_sm()
        .child(Icon::File.svg(14., muted))
        .child(highlighted(format!("{key}-name"), path[name..].to_string(), &marks_in(marks, name..path.len()), cx).flex_none().max_w(px(300.)))
        .child(highlighted(format!("{key}-folder"), folder.to_string(), &marks_in(marks, 0..folder.len()), cx).text_xs().text_color(muted))
        .into_any_element()
}

fn paint_commit(entry: &GitEntry, marks: &[Range<usize>], muted: gpui::Hsla, cx: &mut App) -> AnyElement {
    let GitEntry::Commit { sha, summary, body, author, time_ms } = entry else { return div().into_any_element() };
    let key = entry.key();
    let author_at = summary.len() + 2;
    let sha_at = author_at + author.len() + 2;
    let short = short_sha(sha);
    let whole = if body.is_empty() { summary.to_string() } else { format!("{summary}\n\n{body}") };
    let figures = |text: SharedString| fonts::figures(div()).flex_none().text_xs().text_color(muted).child(text);
    div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .min_w_0()
        .child(highlighted(format!("{key}-summary"), summary.clone(), &marks_in(marks, 0..summary.len()), cx).text_sm().tooltip(Tooltip::text(whole)))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .text_xs()
                .child(highlighted(format!("{key}-author"), author.clone(), &marks_in(marks, author_at..author_at + author.len()), cx).text_color(muted))
                .child(figures(supraman_core::time::stamp(*time_ms).into()))
                .child(fonts::figures(highlighted(format!("{key}-sha"), short.to_string(), &marks_in(marks, sha_at..sha_at + short.len()), cx)).flex_none().text_color(muted)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str, summary: &str, author: &str) -> GitEntry {
        GitEntry::Commit { sha: sha.to_string().into(), summary: summary.to_string().into(), body: "".into(), author: author.to_string().into(), time_ms: 0 }
    }

    #[test]
    fn a_sha_prefix_of_four_hex_digits_finds_its_commit_first() {
        let entries = [commit("ab12cd34ef", "Fix the beef", "Ada"), commit("beef0123aa", "Other", "Bob")];
        let rank = commit_rank();
        let found: Vec<(String, Marks)> = rank(&entries, "BEEF").into_iter().map(|(e, m)| (e.text(), m)).collect();
        assert_eq!(found[0].0, "Other  Bob  beef012", "the sha prefix first");
        assert_eq!(found[0].1, [12..16]);
        assert_eq!(found[1].0, "Fix the beef  Ada  ab12cd3", "then the summary's match");
        assert!(rank(&entries, "bee").iter().all(|(e, _)| e.text().starts_with("Fix")), "three digits are a word, not a sha");
        assert!(rank(&entries, "ab12").iter().any(|(e, _)| e.text().starts_with("Fix")));
        assert!(rank(&entries, "cd34").is_empty(), "a sha matches from its start only, and never through the short sha's text");
        assert_eq!(rank(&entries, "bob").len(), 1, "the author matches");
    }
}
