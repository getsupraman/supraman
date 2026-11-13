use std::path::Path;

use supraman_core::folders::{Entries, Entry};
use supraman_core::paths::{folder_name, path_key};
use supraman_ui::hover_card::HoverCard;

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
    pub group: Option<String>,
    pub repo: Option<String>,
    pub folder: Option<String>,
    pub branch: Option<String>,
    pub session: Option<String>,
    pub kind: Option<Kind>,
    /// Supraman's token overrides the user's `workman` entry ([`crate::hand`]).
    pub workman: Option<String>,
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
        put("group", self.group.clone());
        put("repo", self.repo.clone());
        put("folder", self.folder.clone());
        put("branch", self.branch.clone());
        put("session", self.session.clone());
        put("kind", self.kind.map(|k| k.label().to_string()));
        put("Workman", self.workman.clone());
        put("started", self.started.and_then(|ms| supraman_core::time::short(ms, supraman_core::time::now_ms())));
        rows
    }

    pub fn hover_card(&self) -> HoverCard {
        HoverCard { title: self.title.clone(), rows: self.rows() }
    }
}

pub fn session_label(name: &str, id: &str) -> String {
    if name.is_empty() { id.to_string() } else { format!("{name} · {id}") }
}

/// The group and the listed folder a session in `cwd` shows under ([`supraman_claude::holding`]), without a disk walk.
pub fn place(cwd: &Path, entries: &Entries) -> (Option<String>, Option<String>) {
    let Some(folder) = supraman_claude::holding(cwd, entries, |_| None) else {
        return (None, None);
    };
    let key = path_key(&folder.path);
    let group = entries.list.iter().find(|e| e.folders().iter().any(|f| path_key(&f.path) == key)).and_then(Entry::group_name);
    (group.map(str::to_string), Some(folder_name(&folder.path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_lists_only_known_fields_in_order() {
        let card = Card {
            title: "✳ fix the tray".into(),
            group: Some("autowork".into()),
            folder: Some(r"P:\x".into()),
            branch: Some(String::new()),
            kind: Some(Kind::Attach),
            ..Card::default()
        };
        assert_eq!(card.hover_card().text(), "✳ fix the tray\ngroup  autowork\nfolder  P:\\x\nkind  bg attach");
        let hand = Card {
            title: "✳ mw-workflow".into(),
            kind: Some(Kind::Hand),
            workman: Some("Supraman token · orchestrator · mw-workflow".into()),
            ..Card::default()
        };
        assert_eq!(hand.hover_card().text(), "✳ mw-workflow\nkind  hand\nWorkman  Supraman token · orchestrator · mw-workflow");
    }

    #[test]
    fn the_place_is_the_deepest_listed_folder_and_its_group() {
        use supraman_core::folders::FolderEntry;
        let entries = Entries::new(vec![
            Entry::Folder(FolderEntry::new("P:/Projects")),
            Entry::Group { name: "autowork".into(), folders: vec![FolderEntry::new(r"P:\Projects\AutoWork"), FolderEntry::new(r"P:\Projects\AutoWork\.repos\zed")] },
        ]);
        let at = |p: &str| place(Path::new(p), &entries);
        assert_eq!(at(r"p:\projects\autowork\.repos\zed\crates"), (Some("autowork".into()), Some("zed".into())));
        assert_eq!(at(r"P:\Projects\AutoWork\docs"), (Some("autowork".into()), Some("AutoWork".into())));
        assert_eq!(at(r"P:\Projects\Other"), (None, Some("Projects".into())));
        assert_eq!(at(r"D:\x"), (None, None));
        assert_eq!(session_label("", "abc"), "abc");
        assert_eq!(session_label("fix-tray", "abc"), "fix-tray · abc");
    }
}
