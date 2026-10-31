use std::path::Path;

use foreman_core::Workspace;
use foreman_core::paths::{path_key, within};
use foreman_ui::hover_card::HoverCard;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `claude` started from `+` or resumed in the tab: a human drives it.
    Hand,
    Attach,
    Shell,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Hand => "hand",
            Kind::Attach => "bg attach",
            Kind::Shell => "shell",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    pub title: String,
    pub workspace: Option<String>,
    pub repo: Option<String>,
    pub folder: Option<String>,
    pub branch: Option<String>,
    pub session: Option<String>,
    pub kind: Option<Kind>,
    /// Foreman's token overrides the user's `workstate` entry ([`crate::hand`]).
    pub workstate: Option<String>,
    /// Epoch milliseconds.
    pub started: Option<i64>,
}

impl Card {
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = Vec::new();
        let mut put = |label, value: Option<String>| {
            if let Some(v) = value.filter(|v| !v.is_empty()) {
                rows.push((label, v));
            }
        };
        put("workspace", self.workspace.clone());
        put("repo", self.repo.clone());
        put("folder", self.folder.clone());
        put("branch", self.branch.clone());
        put("session", self.session.clone());
        put("kind", self.kind.map(|k| k.label().to_string()));
        put("Workman", self.workstate.clone());
        put("started", self.started.and_then(local_time));
        rows
    }

    pub fn hover_card(&self) -> HoverCard {
        HoverCard { title: self.title.clone(), rows: self.rows() }
    }
}

fn local_time(ms: i64) -> Option<String> {
    use chrono::TimeZone as _;
    let at = chrono::Local.timestamp_millis_opt(ms).single()?;
    let today = chrono::Local::now().date_naive() == at.date_naive();
    Some(if today { at.format("%H:%M") } else { at.format("%Y-%m-%d %H:%M") }.to_string())
}

pub fn session_label(name: &str, id: &str) -> String {
    if name.is_empty() { id.to_string() } else { format!("{name} · {id}") }
}

/// The workspace [`crate::fleet::holder_of`] names and its deepest folder holding `cwd`.
pub fn place(cwd: &Path, workspaces: &[Workspace]) -> (Option<String>, Option<String>) {
    let Some(ws) = crate::fleet::holder_of(cwd, workspaces) else {
        return (None, None);
    };
    let repo = ws.folders.iter().filter(|r| within(cwd, &r.path)).max_by_key(|r| path_key(&r.path).len());
    (Some(ws.name.clone()), repo.map(|r| r.name.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_lists_only_known_fields_in_order() {
        let card = Card {
            title: "✳ fix the tray".into(),
            workspace: Some("autowork".into()),
            folder: Some(r"P:\x".into()),
            branch: Some(String::new()),
            kind: Some(Kind::Attach),
            ..Card::default()
        };
        assert_eq!(card.hover_card().text(), "✳ fix the tray\nworkspace  autowork\nfolder  P:\\x\nkind  bg attach");
        let hand = Card {
            title: "✳ mw-workflow".into(),
            kind: Some(Kind::Hand),
            workstate: Some("Supraman token · orchestrator · mw-workflow".into()),
            ..Card::default()
        };
        assert_eq!(hand.hover_card().text(), "✳ mw-workflow\nkind  hand\nWorkman  Supraman token · orchestrator · mw-workflow");
    }

    #[test]
    fn the_place_is_the_innermost_workspace_and_its_repo() {
        let repo = foreman_core::Folder::new;
        let outer = Workspace::new("outer", "P:/Projects", vec![]);
        let aw = Workspace::new("autowork", r"P:\Projects\AutoWork", vec![repo("AutoWork", r"P:\Projects\AutoWork"), repo("zed", r"P:\Projects\AutoWork\.repos\zed")]);
        let all = [outer, aw];
        let at = |p: &str| place(Path::new(p), &all);
        assert_eq!(at(r"p:\projects\autowork\.repos\zed\crates"), (Some("autowork".into()), Some("zed".into())));
        assert_eq!(at(r"P:\Projects\AutoWork\docs"), (Some("autowork".into()), Some("AutoWork".into())));
        assert_eq!(at(r"P:\Projects\Other"), (Some("outer".into()), None));
        assert_eq!(at(r"D:\x"), (None, None));
        assert_eq!(session_label("", "abc"), "abc");
        assert_eq!(session_label("fix-tray", "abc"), "fix-tray · abc");
    }
}
