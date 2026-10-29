//! What a terminal tab's hover card says (m5/29): its whole title, then what is known of the tab, one
//! line per field ([`foreman_ui::hover_card`]). The title is the one its program set with OSC 0/2
//! ([`foreman_ui::tabs::sanitize`]), else the tab's own label.

use std::path::Path;

use foreman_core::Workspace;
use foreman_ui::hover_card::HoverCard;

/// What a tab is, as its card says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `claude` started from `+` or resumed in the tab: a human drives it.
    Hand,
    /// `claude attach` to a background session.
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

/// What the card shows; `None` fields are left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    pub title: String,
    pub workspace: Option<String>,
    pub repo: Option<String>,
    pub folder: Option<String>,
    pub branch: Option<String>,
    pub session: Option<String>,
    pub kind: Option<Kind>,
    /// A hand tab's WorkState credential: Foreman's token overrides the user's `workstate` entry
    /// ([`crate::hand`]).
    pub workstate: Option<String>,
    /// Epoch milliseconds.
    pub started: Option<i64>,
}

impl Card {
    /// The lines under the title, as `(label, value)`.
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
        put("WorkState", self.workstate.clone());
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

/// `name · id`, or the id alone for an unnamed session.
pub fn session_label(name: &str, id: &str) -> String {
    if name.is_empty() { id.to_string() } else { format!("{name} · {id}") }
}

/// The workspace holding `cwd` (the innermost when roots nest) and the repo of it holding `cwd`, by
/// name. Case- and `/`-insensitive, as [`crate::fleet::shell_cwd`].
pub fn place(cwd: &Path, workspaces: &[Workspace]) -> (Option<String>, Option<String>) {
    let key = |p: &Path| -> Vec<String> { p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect() };
    let at = key(cwd);
    let inner = |p: &Path| at.starts_with(&key(p)).then(|| p.components().count());
    let Some(ws) = workspaces.iter().filter(|w| inner(&w.root).is_some()).max_by_key(|w| w.root.components().count()) else {
        return (None, None);
    };
    let repo = ws.repos.iter().filter_map(|r| inner(&r.path).map(|depth| (depth, r))).max_by_key(|(depth, _)| *depth);
    (Some(ws.name.clone()), repo.map(|(_, r)| r.name.clone()))
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
            workstate: Some("Foreman token · orchestrator · mw-workflow".into()),
            ..Card::default()
        };
        assert_eq!(hand.hover_card().text(), "✳ mw-workflow\nkind  hand\nWorkState  Foreman token · orchestrator · mw-workflow");
    }

    #[test]
    fn the_place_is_the_innermost_workspace_and_its_repo() {
        let repo = |name: &str, path: &str| foreman_core::Repo {
            name: name.into(),
            path: path.into(),
            has_claude_md: false,
            has_agents_md: false,
            branch: None,
            enabled: true,
        };
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
