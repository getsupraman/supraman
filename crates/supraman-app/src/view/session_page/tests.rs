use supraman_claude::{Health, parse};
use supraman_ui::details::Section;

use super::*;
use crate::view::menu::session_items;

fn row(json: &str, health: Health) -> SessionRow {
    SessionRow::new(parse(&format!("[{json}]")).unwrap().remove(0), health)
}

const LIVE: &str = r#"{"id":"aaaa0001","sessionId":"aaaa0001-x","name":"mine","cwd":"C:/demo","kind":"background","startedAt":1,"state":"working","status":"busy","pid":1}"#;

fn sections() -> Vec<Section> {
    vec![Section::new("session", "Session").row("State", "working"), Section::new("transcript", "Transcript").row("File", "x.jsonl")]
}

/// The Details tab's page and the Git agent panel's, of one session placed alike.
fn pages(row: &SessionRow, placed: Placed) -> (SessionView, SessionView) {
    let items = || session_items("demo", "alpha", row, placed.resumable, None, None, false, None);
    let details = build(row, true, placed, sections(), items(), true, gpui::red());
    let git = build(row, true, placed, sections(), items(), false, gpui::red());
    (details, git)
}

fn keys(sections: &[Section]) -> Vec<&str> {
    sections.iter().map(|s| s.key.as_ref()).collect()
}

#[test]
fn both_pages_open_a_session_one_way_its_tab_first_then_an_attach() {
    let live = row(LIVE, Health::Running);
    let (details, git) = pages(&live, Placed { tab: Some(3), active: Some(1), resumable: false });
    assert_eq!((details.open, git.open), (Open::Tab(3), Open::Tab(3)), "a session with a tab: both show its tab, never a second attach");
    let (details, git) = pages(&live, Placed { tab: None, active: Some(1), resumable: false });
    assert_eq!((details.open, git.open), (Open::Attach, Open::Attach), "an attachable one without a tab attaches");
    let (details, _) = pages(&live, Placed { tab: Some(1), active: Some(1), resumable: false });
    assert_eq!(details.open, Open::Shown);

    let gone = build(&live, false, Placed { tab: None, active: None, resumable: false }, sections(), Vec::new(), true, gpui::red());
    assert_eq!(gone.open, Open::Unlisted, "a session the tree dropped opens nothing");
}

#[test]
fn both_pages_list_the_same_sections_but_the_transcript_and_open_in_place_of_attach() {
    let live = row(LIVE, Health::Running);
    let (details, git) = pages(&live, Placed { tab: None, active: None, resumable: false });
    assert_eq!(keys(&details.sections), ["session", "transcript"]);
    assert_eq!(keys(&git.sections), ["session"], "the Git panel leaves out the transcript");
    assert_eq!((details.title.as_str(), &details.pill.0), (git.title.as_str(), &git.pill.0));
    let actions = |v: &SessionView| v.actions.iter().map(|i| i.key.to_string()).collect::<Vec<_>>();
    assert_eq!(actions(&details), actions(&git), "one list of actions");
    assert!(!actions(&details).iter().any(|k| k == "attach" || k == "resume"), "Open stands for the menu's Attach and Resume: {:?}", actions(&details));
    assert!(actions(&details).iter().any(|k| k == "stop-session"), "the menu's other items stay");
}
