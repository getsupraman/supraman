use super::*;

fn picks(keys: &[&str]) -> Picks {
    let mut picks = Picks { shown: true, ..Picks::default() };
    for key in keys {
        picks.add(key, Pick::Group(key.to_string()));
    }
    picks
}

fn picked(picks: &Picks) -> Option<String> {
    match picks.picked() {
        Some(Pick::Group(name)) => Some(name),
        _ => None,
    }
}

#[test]
fn down_from_none_picks_the_first_and_up_and_down_stop_at_the_ends() {
    let mut p = picks(&["result-group-a", "result-repo-a/x", "result-session-1"]);
    assert_eq!(picked(&p), None);
    assert!(p.step(1));
    assert_eq!(picked(&p).as_deref(), Some("result-group-a"), "Down from none: the first");
    assert!(p.step(1) && p.step(1));
    assert!(!p.step(1), "Down on the last stays");
    assert_eq!(picked(&p).as_deref(), Some("result-session-1"));
    assert!(p.step(-1));
    assert_eq!(picked(&p).as_deref(), Some("result-repo-a/x"));
    assert!(p.is_selected("result-repo-a/x"));
}
