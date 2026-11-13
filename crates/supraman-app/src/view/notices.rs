use supraman_ui::notices::{NoticeKind, remove_one};
use gpui::{App, Context};

use super::FleetView;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Notice {
    Setup,
    Supervise,
    Schedule,
    Hand,
    Tabs,
    Inbox,
    Action,
    Renewal,
}

impl NoticeKind for Notice {
    fn tag(self) -> &'static str {
        match self {
            Notice::Setup => "setup",
            Notice::Supervise => "supervise",
            Notice::Schedule => "schedule",
            Notice::Hand => "hand",
            Notice::Tabs => "tabs",
            Notice::Inbox => "inbox",
            Notice::Action => "action",
            Notice::Renewal => "renewal",
        }
    }
}

impl FleetView {
    pub(super) fn plain_notices(&self, cx: &App) -> Vec<(Notice, String)> {
        let setup = self.setup_notices.iter().map(|t| (Notice::Setup, t.clone()));
        let supervise = undismissed(&self.supervising.supervise_errors, &self.supervising.supervise_dismissed).map(|t| (Notice::Supervise, t));
        let schedule = undismissed(&self.scheduling.schedule_errors, &self.scheduling.schedule_errors_dismissed).map(|t| (Notice::Schedule, t));
        let hand = self.hand_notices.iter().map(|t| (Notice::Hand, t.clone()));
        let tabs = self.saving.tab_notices.iter().map(|t| (Notice::Tabs, t.clone()));
        let inbox = self.work_inbox.read(cx).notice.clone().map(|t| (Notice::Inbox, t));
        let action = self.action_error.iter().map(|t| (Notice::Action, t.clone()));
        let renewal = crate::keeping::notices(cx).into_iter().map(|t| (Notice::Renewal, t));
        setup.chain(supervise).chain(schedule).chain(hand).chain(tabs).chain(inbox).chain(action).chain(renewal).collect()
    }

    pub fn dismiss_notice(&mut self, kind: Notice, text: &str, cx: &mut Context<Self>) {
        match kind {
            Notice::Setup => remove_one(&mut self.setup_notices, text),
            Notice::Supervise => self.supervising.supervise_dismissed.push(text.to_string()),
            Notice::Schedule => self.scheduling.schedule_errors_dismissed.push(text.to_string()),
            Notice::Hand => remove_one(&mut self.hand_notices, text),
            Notice::Tabs => remove_one(&mut self.saving.tab_notices, text),
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
            Notice::Renewal => crate::keeping::dismiss(text, cx),
        }
        cx.notify();
    }
}

fn undismissed<'a>(errors: &'a [String], dismissed: &'a [String]) -> impl Iterator<Item = String> + 'a {
    errors.iter().filter(|t| !dismissed.contains(t)).cloned()
}

#[cfg(test)]
mod tests {
    use supraman_ui::notices::dismiss_key;

    use super::*;

    #[test]
    fn each_kind_has_its_own_dismiss_key() {
        let keys = [Notice::Setup, Notice::Supervise, Notice::Schedule, Notice::Hand, Notice::Tabs, Notice::Inbox, Notice::Action, Notice::Renewal].map(|k| dismiss_key(k, 0));
        assert_eq!(
            keys,
            ["notice-dismiss-setup-0", "notice-dismiss-supervise-0", "notice-dismiss-schedule-0", "notice-dismiss-hand-0",
                "notice-dismiss-tabs-0",
                "notice-dismiss-inbox-0", "notice-dismiss-action-0", "notice-dismiss-renewal-0"]
        );
    }

    #[test]
    fn an_error_shows_until_its_text_is_dismissed() {
        let errors = vec!["Cannot read the schedules of a".to_string(), "Cannot read the run records of b".to_string()];
        assert_eq!(undismissed(&errors, &[]).collect::<Vec<_>>(), errors);
        let dismissed = vec![errors[0].clone()];
        assert_eq!(undismissed(&errors, &dismissed).collect::<Vec<_>>(), [errors[1].clone()]);
    }
}
