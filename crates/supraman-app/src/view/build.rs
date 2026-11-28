//! The window's state as it opens: the app's, which reads its home and starts every poll, or a test's, which starts none.

use super::*;

impl FleetView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let claude_dir = claude_dir_or(CLAUDE_DIR_OVERRIDE.get().map(PathBuf::as_path));
        Self::build(supraman_core::home_from_env(), claude_dir, true, window, cx)
    }

    /// A window with no home and no claude dir that polls, detects and asks Workman nothing: a test's.
    #[cfg(test)]
    pub(crate) fn inert(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(None, None, false, window, cx)
    }

    fn build(home: Option<PathBuf>, claude_dir: Option<PathBuf>, live: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self::ask_before_close(window, cx);
        let defaults = Settings::default();
        let hands = home.as_deref().map(|home| Self::open_hands(home, cx));
        let (polls, transcripts_nudged) = polls::Polls::start(home.clone(), hands.clone(), live, cx);
        Self::load_settings(home.clone(), cx);
        Self::load_stores(home.clone(), cx);
        let term_look = term_look::TermLook::new(&defaults.terminal, cx);
        let (workman_recheck, recheck) = nudge::nudge();
        if live {
            Self::watch_workman(home.clone(), recheck, cx);
            Self::detect_shells(cx);
        }
        let work_inbox = WorkInbox::shared(home.clone(), cx);
        Self::observe_inbox(&work_inbox, cx);
        let scheduling = schedule_state::ScheduleState::new();
        Self::watch_transcripts(claude_dir.clone(), home.as_deref().map(|h| h.join(TRANSCRIPTS_CACHE)), transcripts_nudged, cx);
        let updates = crate::updates::Updates::start(home.clone(), |v: &mut Self| &mut v.updates, cx);
        let tabs_writes = Self::tabs_writer(home.clone(), cx);
        cx.on_focus_lost(window, |this, window, cx| if !supraman_ui::focus::restore(window, cx) { this.focus.focus(window, cx) }).detach();
        let search = cx.new(|cx| TextInput::new("Search sessions, groups, folders (Ctrl+Shift+E)", None, window, cx));
        Self::watch_search(&search, window, cx);
        Self {
            cli: Cli::Checking,
            listed: listed::Listed::default(),
            sessions: Vec::new(),
            show_stopped: false,
            own_prefix: None,
            tabs: Vec::new(),
            active: None,
            next_tab_id: 1,
            shell_args: vec!["-NoLogo".into()],
            focus,
            probe: Default::default(),
            hits: HitTargets::default(),
            window: window.window_handle(),
            home,
            plugin_dir: None,
            setup_notices: Vec::new(),
            settings_window: None,
            writes: writes::Writes::default(),
            occupancy: occupancy::Live::default(),
            work_inbox,
            open_with: |path| open::open(path.as_os_str()),
            open_link_with: startup::open_link,
            workman: WorkmanStatus::Checking,
            hand_notices: Vec::new(),
            transcripts: TranscriptsLoad::new(claude_dir.is_some()),
            loaded: false,
            action_error: None,
            stars: Stars::default(),
            claude_command: "claude".into(),
            search,
            toast: ToastSlot::default(),
            pane_toast: ToastSlot::default(),
            updates,
            claude_dir,
            painted_sessions: false,
            saving: tab_saving::TabSaving::new(tabs_writes),
            sidebar: sidebar::Sidebar::new(cx),
            term_look,
            workman_state: workman_state::WorkmanState::new(workman_recheck),
            lifecycle: lifecycle::Lifecycle::new(hands),
            polls,
            polled: polled::Polled::new(),
            listing: listing::Listing::default(),
            dialogs: dialogs::Dialogs::default(),
            scheduling,
            tab_keys: cx.new(|_| tab_keys::TabKeyState::new(defaults.window.tab_keys())),
            docks: dock::Docks::default(),
        }
    }
}
