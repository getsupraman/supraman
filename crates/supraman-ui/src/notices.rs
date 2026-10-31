//! Plain notices, each with its own ×. The owner keeps the kinds and their sources; a notice is
//! dismissed by its text, so raising it again shows it again.

use std::collections::BTreeMap;

use gpui::{App, ClickEvent, Context, Div, Hsla, SharedString, Stateful, prelude::*};

use crate::banner;

pub trait NoticeKind: Copy + Ord + 'static {
    /// The kind's part of its × key ([`dismiss_key`]).
    fn tag(self) -> &'static str;
}

/// The × of the `index`th shown notice of `kind`.
pub fn dismiss_key<K: NoticeKind>(kind: K, index: usize) -> String {
    format!("notice-dismiss-{}-{index}", kind.tag())
}

/// A source that replaces its list on every read remembers a dismissed notice by its text, and only while
/// the source keeps reporting it: once it is gone, the same text coming back shows again.
pub fn keep_dismissed(dismissed: &mut Vec<String>, current: &[String]) {
    dismissed.retain(|d| current.contains(d));
}

/// Removes the first notice reading `text`.
pub fn remove_one(list: &mut Vec<String>, text: &str) {
    if let Some(i) = list.iter().position(|t| t == text) {
        list.remove(i);
    }
}

/// One `color` banner per notice, in order. `dismiss` builds the × for a key; `on_dismiss` gets the kind
/// and text of the one clicked.
pub fn render<V: 'static, K: NoticeKind>(
    notices: Vec<(K, String)>,
    color: Hsla,
    dismiss: impl Fn(String, &App) -> Stateful<Div>,
    on_dismiss: impl Fn(&mut V, K, &str, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Vec<Div> {
    let mut shown: BTreeMap<K, usize> = BTreeMap::new();
    notices
        .into_iter()
        .map(|(kind, text)| {
            let index = shown.entry(kind).or_default();
            let key = dismiss_key(kind, *index);
            *index += 1;
            let label = SharedString::from(text.clone());
            let on_dismiss = on_dismiss.clone();
            let dismiss = dismiss(key, cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                on_dismiss(this, kind, &text, cx);
            }));
            banner::tinted(color).child(banner::dismissable(label, dismiss))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Kind {
        A,
        B,
    }

    impl NoticeKind for Kind {
        fn tag(self) -> &'static str {
            match self {
                Kind::A => "a",
                Kind::B => "b",
            }
        }
    }

    #[test]
    fn a_dismissed_notice_shows_again_once_it_has_cleared() {
        let mut dismissed = vec!["a".to_string(), "b".to_string()];
        keep_dismissed(&mut dismissed, &["a".into(), "c".into()]);
        assert_eq!(dismissed, ["a"], "still reported: still hidden; gone: forgotten");
        keep_dismissed(&mut dismissed, &[]);
        assert!(dismissed.is_empty());
        keep_dismissed(&mut dismissed, &["a".into()]);
        assert!(dismissed.is_empty(), "the same text raised again is not hidden");
    }

    #[test]
    fn remove_one_takes_only_the_first_match() {
        let mut list = vec!["x".to_string(), "y".into(), "x".into()];
        remove_one(&mut list, "x");
        assert_eq!(list, ["y", "x"]);
        remove_one(&mut list, "z");
        assert_eq!(list, ["y", "x"]);
    }

    #[test]
    fn the_key_names_the_kind_and_its_index() {
        assert_eq!(dismiss_key(Kind::A, 0), "notice-dismiss-a-0");
        assert_eq!(dismiss_key(Kind::B, 2), "notice-dismiss-b-2");
    }
}
