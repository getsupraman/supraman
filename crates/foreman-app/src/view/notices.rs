use foreman_ui::notices::{NoticeKind, remove_one};
use gpui::{App, Context};

use super::FleetView;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Notice {
    Setup,
    Supervise,
    Hand,
    Inbox,
    Action,
}

impl NoticeKind for Notice {
    fn tag(self) -> &'static str {
        match self {
            Notice::Setup => "setup",
            Notice::Supervise => "supervise",
            Notice::Hand => "hand",
            Notice::Inbox => "inbox",
            Notice::Action => "action",
        }
    }
}

impl FleetView {
    pub(super) fn plain_notices(&self, cx: &App) -> Vec<(Notice, String)> {
        let setup = self.setup_notices.iter().map(|t| (Notice::Setup, t.clone()));
        let supervise =
            self.supervise_errors.iter().filter(|t| !self.supervise_dismissed.contains(t)).map(|t| (Notice::Supervise, t.clone()));
        let hand = self.hand_notices.iter().map(|t| (Notice::Hand, t.clone()));
        let inbox = self.work_inbox.read(cx).notice.clone().map(|t| (Notice::Inbox, t));
        let action = self.action_error.iter().map(|t| (Notice::Action, t.clone()));
        setup.chain(supervise).chain(hand).chain(inbox).chain(action).collect()
    }

    pub fn dismiss_notice(&mut self, kind: Notice, text: &str, cx: &mut Context<Self>) {
        match kind {
            Notice::Setup => remove_one(&mut self.setup_notices, text),
            Notice::Supervise => self.supervise_dismissed.push(text.to_string()),
            Notice::Hand => remove_one(&mut self.hand_notices, text),
            Notice::Inbox => self.work_inbox.update(cx, |inbox, _| {
                if inbox.notice.as_deref() == Some(text) {
                    inbox.notice = None;
                }
            }),
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
    use foreman_ui::notices::dismiss_key;

    use super::*;

    #[test]
    fn each_kind_has_its_own_dismiss_key() {
        let keys = [Notice::Setup, Notice::Supervise, Notice::Hand, Notice::Inbox, Notice::Action].map(|k| dismiss_key(k, 0));
        assert_eq!(
            keys,
            ["notice-dismiss-setup-0", "notice-dismiss-supervise-0", "notice-dismiss-hand-0", "notice-dismiss-inbox-0", "notice-dismiss-action-0"]
        );
    }
}
