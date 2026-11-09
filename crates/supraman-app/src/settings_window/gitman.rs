//! The settings window's GitMan page: an open Git tab's background fetch interval, background `git maintenance` and
//! the other worktrees' WIP rows, `[gitman] fetch_every_minutes`, `maintenance` and `worktree_wip`, read and saved off the UI thread with the table's other keys
//! kept; each save reaches the open tabs at once.

use std::path::PathBuf;

use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Entity, IntoElement, Render, Subscription, Window, div, prelude::*, px};
use supraman_core::Settings;
use supraman_gitman::{BackgroundSettings, background_settings, set_setting};
use supraman_ui::button;
use supraman_ui::hit::HitTargets;
use supraman_ui::settings_page::row;
use supraman_ui::text_input::{InputEvent, TextInput};
use theme::ActiveTheme as _;

use crate::view::writes::Serial;

pub const MAINTENANCE_KEY: &str = "gitman-maintenance";
pub const WORKTREE_WIP_KEY: &str = "gitman-worktree-wip";

fn digit(c: char) -> Option<char> {
    c.is_ascii_digit().then_some(c)
}

pub struct GitmanPage {
    home: Option<PathBuf>,
    hits: HitTargets,
    every: Entity<TextInput>,
    settings: BackgroundSettings,
    /// A change waits for the first read: it never saves over what the file holds.
    loaded: bool,
    error: Option<String>,
    saves: Serial,
    _every_events: Subscription,
}

impl GitmanPage {
    pub fn new(home: Option<PathBuf>, hits: HitTargets, window: &mut Window, cx: &mut Context<Self>) -> GitmanPage {
        let every = cx.new(|cx| TextInput::new("minutes", Some(digit), window, cx));
        let events = cx.subscribe(&every, |this: &mut Self, input, e: &InputEvent, cx| {
            if *e == InputEvent::Edited {
                let text = input.read(cx).text().to_string();
                this.every_edited(&text, cx);
            }
        });
        let page = GitmanPage { home, hits, every, settings: BackgroundSettings::default(), loaded: false, error: None, saves: Serial::default(), _every_events: events };
        page.read(cx);
        page
    }

    fn read(&self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else { return };
        let read = cx.background_spawn(async move { Settings::load(&home).map(|s| background_settings(&s)) });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            this.update(cx, |this, cx| {
                match read {
                    Ok(settings) => {
                        this.settings = settings;
                        this.loaded = true;
                        this.every.update(cx, |input, cx| input.set_text(&settings.fetch_every().to_string(), cx));
                    }
                    Err(e) => this.error = Some(format!("Cannot read settings.toml: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    #[cfg(test)]
    pub fn every(&self) -> &Entity<TextInput> {
        &self.every
    }

    #[cfg(test)]
    pub fn settings(&self) -> BackgroundSettings {
        self.settings
    }

    fn every_edited(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        match text.parse::<u32>() {
            Ok(minutes) => {
                self.settings.fetch_every_minutes = Some(minutes);
                self.save("fetch_every_minutes", minutes, cx);
            }
            Err(_) => {
                self.error = Some("The interval is a whole number of minutes".into());
                cx.notify();
            }
        }
    }

    pub fn toggle_maintenance(&mut self, cx: &mut Context<Self>) {
        let on = !self.settings.maintains();
        self.settings.maintenance = Some(on);
        self.save("maintenance", on, cx);
        cx.notify();
    }

    pub fn toggle_worktree_wip(&mut self, cx: &mut Context<Self>) {
        let on = !self.settings.worktree_wips();
        self.settings.worktree_wip = Some(on);
        self.save("worktree_wip", on, cx);
        cx.notify();
    }

    /// `[gitman] <key> = value` saved after the saves before it; the open tabs hear what the file then holds.
    fn save(&mut self, key: &'static str, value: impl serde::Serialize + Send + 'static, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone().filter(|_| self.loaded) else { return };
        let previous = self.saves.previous();
        let task = cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let saved = cx.background_spawn(async move { Settings::update(&home, |s| set_setting(s, key, value)) }).await;
            this.update(cx, |this, cx| {
                this.error = match saved {
                    Ok((settings, Ok(()))) => {
                        crate::git_view::background::apply(background_settings(&settings), cx);
                        None
                    }
                    Ok((_, Err(e))) => Some(format!("Cannot save settings.toml: {e}")),
                    Err(e) => Some(format!("Cannot save settings.toml: {e}")),
                };
                cx.notify();
            })
            .ok();
        });
        self.saves.push(task);
    }

    fn rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let field = div().w(px(72.)).child(self.every.clone()).into_any_element();
        let tick = button::checkbox(&self.hits, MAINTENANCE_KEY, self.settings.maintains(), cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_maintenance(cx)));
        let worktrees = button::checkbox(&self.hits, WORKTREE_WIP_KEY, self.settings.worktree_wips(), cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_worktree_wip(cx)));
        let mut rows = vec![
            row("Fetch every N minutes (0 = off)", "An open Git tab fetches every remote when it opens, each time it becomes the active tab, and then this often. A failure shows on the Pull button.", field, None, cx),
            row("Run git maintenance in the background", "Once an hour at most per repository, while no Claude session works in it: the commit-graph written, loose objects packed, packs repacked.", tick.into_any_element(), None, cx),
            row("Show other worktrees' work in progress", "A row in the graph above each other worktree's HEAD while it has changes or a Claude session works in it.", worktrees.into_any_element(), None, cx),
        ];
        if let Some(error) = &self.error {
            rows.insert(0, supraman_ui::banner::tinted(cx.theme().status().error).my_2().child(error.clone()).into_any_element());
        }
        rows
    }
}

impl Render for GitmanPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().children(self.rows(cx))
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[gpui::test]
    fn the_page_shows_the_defaults_and_saves_both_keys_with_the_tables_others_kept(cx: &mut TestAppContext) {
        cx.update(|cx| {
            supraman_ui::init_globals(cx, "test");
            supraman_ui::text_input::bind_keys(cx);
        });
        let home = tempfile::tempdir().unwrap();
        std::fs::write(Settings::path(home.path()), "[sessions]\nmax_working = 3\n\n[gitman]\npull = \"rebase\"\nprune = false\n").unwrap();
        let hits = HitTargets::default();
        let (page, cx) = cx.add_window_view(|window, cx| GitmanPage::new(Some(home.path().to_path_buf()), hits.clone(), window, cx));
        cx.run_until_parked();

        let shown = page.read_with(cx, |p, cx| (p.every().read(cx).text().to_string(), p.settings().maintains()));
        assert_eq!(shown, ("1".to_string(), true), "1 minute and maintenance on by default");
        cx.update(|_, _| ());
        assert!(hits.borrow().contains_key(MAINTENANCE_KEY), "the tick box paints");

        page.update_in(cx, |p, window, cx| p.every.update(cx, |i, cx| i.focus_all(window, cx)));
        cx.simulate_input("5x");
        cx.run_until_parked();
        let at = hits.borrow().get(MAINTENANCE_KEY).map(|h| h.bounds.center()).expect("painted");
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();

        let saved = Settings::load(home.path()).unwrap();
        let bg = background_settings(&saved);
        assert_eq!((bg.fetch_every_minutes, bg.maintenance), (Some(5), Some(false)), "a letter is refused, 5 and off saved");
        let remote = supraman_gitman::remote_settings(&saved);
        assert_eq!((remote.pull, remote.prune), (Some(supraman_gitman::write::remote::PullChoice::Rebase), Some(false)), "the table's other keys kept");
        assert_eq!(saved.sessions.max_working, 3, "other tables kept");
        assert_eq!(page.read_with(cx, |p, cx| p.every().read(cx).text().to_string()), "5");
    }

    #[gpui::test]
    fn the_worktree_wip_tick_is_on_by_default_and_saves_off(cx: &mut TestAppContext) {
        cx.update(|cx| {
            supraman_ui::init_globals(cx, "test");
            supraman_ui::text_input::bind_keys(cx);
        });
        let home = tempfile::tempdir().unwrap();
        std::fs::write(Settings::path(home.path()), "[gitman]
maintenance = false
").unwrap();
        let hits = HitTargets::default();
        let (page, cx) = cx.add_window_view(|window, cx| GitmanPage::new(Some(home.path().to_path_buf()), hits.clone(), window, cx));
        cx.run_until_parked();
        assert!(page.read_with(cx, |p, _| p.settings().worktree_wips()), "on while unset");
        cx.update(|_, _| ());
        let at = hits.borrow().get(WORKTREE_WIP_KEY).map(|h| h.bounds.center()).expect("painted");
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        let saved = background_settings(&Settings::load(home.path()).unwrap());
        assert_eq!((saved.worktree_wip, saved.maintenance), (Some(false), Some(false)), "saved off, the other key kept");
    }
}
