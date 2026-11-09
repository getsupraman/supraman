use super::GitSizes;

pub fn sizes() -> GitSizes {
    GitSizes { widths: vec![200, 90, 300, 130, 70], panel: Some(360), message: Some(150), refs: Some(210), refs_shut: true, hidden: vec!["refs/heads/feature".into(), "stash/0123".into()], collapsed: Vec::new(), solo: Vec::new(), sections: Vec::new() }
}
