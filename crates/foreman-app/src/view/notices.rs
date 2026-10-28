//! The plain notices above the tree (m5/37): setup, supervise errors, hand tokens and the last action's
//! error, each with its own ×. Dismissing removes that one notice; raising it again shows it again.

use gpui::Context;

use super::FleetView;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Notice {
    Setup,
    Supervise,
    Hand,
    Action,
}

impl Notice {
    fn tag(self) -> &'static str {
        match self {
            Notice::Setup => "setup",
            Notice::Supervise => "supervise",
            Notice::Hand => "hand",
            Notice::Action => "action",
        }
    }
}

/// The × of the `index`th shown notice of `kind`.
pub fn dismiss_key(kind: Notice, index: usize) -> String {
    format!("notice-dismiss-{}-{index}", kind.tag())
}

/// A poll replaces the supervise errors, so a dismissed one is remembered by its text, and only while the
/// poll keeps reporting it: once it is gone, the same error coming back shows again.
pub fn keep_dismissed(dismissed: &mut Vec<String>, errors: &[String]) {
    dismissed.retain(|d| errors.contains(d));
}

impl FleetView {
    /// Every notice of the box with its kind, in the order drawn.
    pub(super) fn plain_notices(&self) -> Vec<(Notice, String)> {
        let setup = self.setup_notices.iter().map(|t| (Notice::Setup, t.clone()));
        let supervise =
            self.supervise_errors.iter().filter(|t| !self.supervise_dismissed.contains(t)).map(|t| (Notice::Supervise, t.clone()));
        let hand = self.hand_notices.iter().map(|t| (Notice::Hand, t.clone()));
        let action = self.action_error.iter().map(|t| (Notice::Action, t.clone()));
        setup.chain(supervise).chain(hand).chain(action).collect()
    }

    /// Removes one notice of `kind` reading `text`.
    pub fn dismiss_notice(&mut self, kind: Notice, text: &str, cx: &mut Context<Self>) {
        let remove_one = |list: &mut Vec<String>| {
            if let Some(i) = list.iter().position(|t| t == text) {
                list.remove(i);
            }
        };
        match kind {
            Notice::Setup => remove_one(&mut self.setup_notices),
            Notice::Supervise => self.supervise_dismissed.push(text.to_string()),
            Notice::Hand => remove_one(&mut self.hand_notices),
            Notice::Action => {
                if self.action_error.as_deref() == Some(text) {
                    self.action_error = None;
                }
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dismissed_supervise_error_shows_again_once_it_has_cleared() {
        let mut dismissed = vec!["a".to_string(), "b".to_string()];
        keep_dismissed(&mut dismissed, &["a".into(), "c".into()]);
        assert_eq!(dismissed, ["a"], "still reported: still hidden; gone: forgotten");
        keep_dismissed(&mut dismissed, &[]);
        assert!(dismissed.is_empty());
        keep_dismissed(&mut dismissed, &["a".into()]);
        assert!(dismissed.is_empty(), "the same error raised again is not hidden");
    }

    #[test]
    fn each_kind_has_its_own_dismiss_key() {
        let keys = [Notice::Setup, Notice::Supervise, Notice::Hand, Notice::Action].map(|k| dismiss_key(k, 0));
        assert_eq!(keys, ["notice-dismiss-setup-0", "notice-dismiss-supervise-0", "notice-dismiss-hand-0", "notice-dismiss-action-0"]);
    }
}
