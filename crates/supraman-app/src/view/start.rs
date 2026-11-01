//! The prompt tab of New task… and New scheduled task…: one prompt, a folder, a permission mode, Send. Send turns
//! the tab into a [+] session tab whose claude gets the text as its first prompt; below the form, the workspace's
//! schedules and recent sessions.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Entity, FontWeight, ScrollHandle, SharedString, Subscription, Window, div, prelude::*, px};
use supraman_core::paths::path_key;
use supraman_ui::banner;
use supraman_ui::button;
use supraman_ui::details::{self, Section};
use supraman_ui::hit::hit_target;
use supraman_ui::text_input::{InputEvent, TextInput};
use theme::ActiveTheme as _;

use super::schedules::At;
use super::{FleetView, Tab, TabKind};
use crate::fleet::SessionRow;
use crate::schedules;

/// claude's `--permission-mode` values the page offers; never `bypassPermissions`.
pub const MODES: [&str; 4] = ["default", "acceptEdits", "auto", "plan"];

/// The line a scheduled task's text follows.
pub const SCHEDULE_ASK: &str = "Set this up as a Supraman schedule (supraman-scheduler skill); ask me what is open, then add it and show the dry run:";

const RECENT: usize = 8;

pub struct StartPage {
    pub workspace: String,
    pub scheduled: bool,
    pub folder: PathBuf,
    pub mode: &'static str,
    pub input: Entity<TextInput>,
    pub error: Option<String>,
    pub scroll: ScrollHandle,
    _events: Subscription,
}

/// A scheduled task's tab: the first pending schedule of `workspace` created at or after `since` is the one it asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsksSchedule {
    pub workspace: String,
    pub since: i64,
    /// Its id and its name.
    pub found: Option<(String, String)>,
}

/// What claude's first prompt is: the text, after [`SCHEDULE_ASK`] for a scheduled task.
pub fn first_prompt(text: &str, scheduled: bool) -> String {
    if scheduled { format!("{SCHEDULE_ASK}\n{text}") } else { text.to_string() }
}

/// The workspace's home, then its folders, each once.
pub fn folders(workspace: &supraman_core::Workspace) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    if let Some(home) = workspace.home() {
        out.push((format!("{} (home)", workspace.name), home.to_path_buf()));
    }
    for f in &workspace.folders {
        if !out.iter().any(|(_, p)| path_key(p) == path_key(&f.path)) {
            out.push((f.name.clone(), f.path.clone()));
        }
    }
    out
}

/// The file the launch's pwsh runs for `command`: the path of `& '<path>' …`, else a bare name looked up as pwsh does.
pub fn launched_file(command: &str, path: &std::ffi::OsStr, pathext: &str) -> Option<PathBuf> {
    let command = command.trim();
    if let Some(quoted) = command.strip_prefix("& '") {
        return quoted.split_once('\'').map(|(p, _)| PathBuf::from(p));
    }
    let bare = !command.is_empty() && command.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    bare.then(|| supraman_core::paths::pwsh_command(command, path, pathext)).flatten()
}

/// Why `prompt` cannot go to claude as `file`: cmd.exe ends a `.cmd`'s command line at a new line and re-reads its quotes.
pub fn cmd_refusal(file: Option<&Path>, prompt: &str) -> Option<String> {
    let file = file?;
    let ext = file.extension()?.to_string_lossy().to_ascii_lowercase();
    if !matches!(ext.as_str(), "cmd" | "bat") || !prompt.contains(['\n', '"']) {
        return None;
    }
    Some(format!(
        "Not sent: claude here is {}, which runs through cmd.exe and cuts the text at a new line and mangles double quotes. \
         Install the native claude.exe (claude.ai/install), which carries any text, or write the prompt on one line without \".",
        file.display()
    ))
}

/// Written whole before the launch; the launch script deletes it once read.
fn write_prompt(home: Option<&Path>, text: &str) -> std::io::Result<PathBuf> {
    let dir = home.map_or_else(|| std::env::temp_dir().join("supraman-prompts"), |h| h.join("prompts"));
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{}-{}.txt", supraman_core::time::now_ms(), supraman_core::time::hex6()));
    supraman_core::fs::write_atomic(&file, text.as_bytes())?;
    Ok(file)
}

impl FleetView {
    /// A new prompt tab, one per opening; `folder` preselects one of the workspace's.
    pub fn open_start(&mut self, workspace: &str, folder: Option<PathBuf>, scheduled: bool, window: &mut Window, cx: &mut Context<Self>) -> Option<u64> {
        self.menu = None;
        let ws = self.workspaces.iter().find(|w| w.name == workspace)?;
        let choices = folders(ws);
        let folder = folder
            .filter(|f| choices.iter().any(|(_, p)| path_key(p) == path_key(f)))
            .or_else(|| choices.first().map(|(_, p)| p.clone()))?;
        let placeholder = if scheduled { "What should run, and when? Enter sends, Shift+Enter adds a line." } else { "What should claude do? Enter sends, Shift+Enter adds a line." };
        let input = cx.new(|cx| TextInput::new(placeholder, None, window, cx).enter_confirms());
        let tab = self.next_tab_id;
        self.next_tab_id += 1;
        let events = cx.subscribe_in(&input, window, move |this: &mut Self, _, event: &InputEvent, _, cx| match event {
            InputEvent::Confirm => this.send_start(tab, cx),
            InputEvent::Edited => {
                if let Some(page) = this.start_page_mut(tab) {
                    page.error = None;
                }
                cx.notify();
            }
            InputEvent::ConfirmBack | InputEvent::Cancel | InputEvent::Refused(_) => {}
        });
        let title = if scheduled { format!("New scheduled task · {workspace}") } else { format!("New task · {workspace}") };
        let page = StartPage { workspace: workspace.to_string(), scheduled, folder: folder.clone(), mode: MODES[0], input: input.clone(), error: None, scroll: ScrollHandle::new(), _events: events };
        self.tabs.push(Tab::new(tab, title, TabKind::Start(page), Some(folder)));
        self.active = Some(tab);
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        Some(tab)
    }

    pub fn start_page(&self, tab: u64) -> Option<&StartPage> {
        match &self.tab(tab)?.kind {
            TabKind::Start(page) => Some(page),
            _ => None,
        }
    }

    fn start_page_mut(&mut self, tab: u64) -> Option<&mut StartPage> {
        match &mut self.tabs.iter_mut().find(|t| t.id == tab)?.kind {
            TabKind::Start(page) => Some(page),
            _ => None,
        }
    }

    /// The prompt box keeps the focus, so typing goes on after a pick.
    pub fn set_start_choice(&mut self, tab: u64, folder: Option<PathBuf>, mode: Option<&'static str>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.start_page_mut(tab) {
            let input = page.input.clone();
            input.update(cx, |input, cx| input.focus(window, cx));
            if let Some(folder) = folder {
                page.folder = folder;
            }
            if let Some(mode) = mode {
                page.mode = mode;
            }
            cx.notify();
        }
    }

    /// The tab becomes a [+] session tab in the chosen folder, titled by the text's first line.
    pub fn send_start(&mut self, tab: u64, cx: &mut Context<Self>) {
        let Some(page) = self.start_page(tab) else { return };
        let text = page.input.read(cx).text().trim().to_string();
        let (workspace, scheduled, folder, mode) = (page.workspace.clone(), page.scheduled, page.folder.clone(), page.mode);
        if text.is_empty() {
            if let Some(page) = self.start_page_mut(tab) {
                page.error = Some("Nothing to send.".into());
            }
            cx.notify();
            return;
        }
        let prompt = first_prompt(&text, scheduled);
        let path = std::env::var_os("PATH").unwrap_or_default();
        let pathext = std::env::var("PATHEXT").unwrap_or_default();
        if let Some(why) = cmd_refusal(launched_file(&self.claude_command, &path, &pathext).as_deref(), &prompt) {
            if let Some(page) = self.start_page_mut(tab) {
                page.error = Some(why);
            }
            cx.notify();
            return;
        }
        let file = match write_prompt(self.home.as_deref(), &prompt) {
            Ok(file) => file,
            Err(e) => {
                if let Some(page) = self.start_page_mut(tab) {
                    page.error = Some(format!("Not sent: cannot write the prompt to the home ({e})."));
                }
                cx.notify();
                return;
            }
        };
        let since = supraman_core::time::now_ms();
        let first = super::FirstPrompt { file, mode };
        let claude = self.claude_command.clone();
        self.hand_claude_in(Some(tab), folder, crate::compose::first_line(&text), claude, None, None, Some(first), cx);
        if scheduled && let Some(t) = self.tab_mut(tab) {
            t.asks_schedule = Some(AsksSchedule { workspace, since, found: None });
        }
    }

    /// After each schedule poll: a scheduled task's tab finds the pending schedule its session added.
    pub(super) fn find_asked_schedules(&mut self) {
        for tab in &mut self.tabs {
            let Some(ask) = tab.asks_schedule.as_mut().filter(|a| a.found.is_none()) else { continue };
            let rows = self.schedules.get(&ask.workspace).map(Vec::as_slice).unwrap_or_default();
            ask.found = rows
                .iter()
                .filter(|r| r.status == supraman_core::schedule::Status::Pending && r.schedule.created_at >= ask.since)
                .min_by_key(|r| r.schedule.created_at)
                .map(|r| (r.schedule.id.clone(), r.title().to_string()));
        }
    }

    pub(super) fn render_schedule_ask(&self, tab: u64, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ask = self.tab(tab)?.asks_schedule.as_ref()?;
        let (id, name) = ask.found.clone()?;
        let status = cx.theme().status().clone();
        let workspace = ask.workspace.clone();
        Some(
            banner::tinted(status.warning)
                .flex()
                .items_center()
                .gap_2()
                .m_2()
                .child(div().flex_1().min_w_0().child(SharedString::from(format!("Schedule {name} waits for your approval"))))
                .child(button::bordered(&self.hits, format!("start-schedule-open-{tab}"), "Open", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_schedule(&workspace, &id, At::Top, cx);
                })))
                .into_any_element(),
        )
    }

    fn start_sections(&self, workspace: &str) -> Vec<Section> {
        let mut out = Vec::new();
        let rows = self.schedules.get(workspace).map(Vec::as_slice).unwrap_or_default();
        if !rows.is_empty() {
            let mut s = Section::new("schedules", "Schedules");
            for r in rows {
                s = s.link(schedules::status_words(r).0, r.title(), format!("schedule:{}", r.schedule.id));
            }
            out.push(s);
        }
        let tree = self.tree();
        let mut recent: Vec<(&str, &SessionRow)> = tree
            .workspaces
            .iter()
            .filter(|n| n.name == workspace)
            .flat_map(|n| n.repos.iter().flat_map(|r| r.sessions.iter().map(move |s| (r.name.as_str(), s))))
            .collect();
        recent.sort_by_key(|(_, r)| std::cmp::Reverse(r.session.started_at.unwrap_or_default()));
        if !recent.is_empty() {
            let mut s = Section::new("recent", "Recent sessions");
            for (repo, r) in recent.into_iter().take(RECENT) {
                s = s.link(repo, crate::fleet::display_name(&r.session), format!("session:{repo}\u{0}{}", r.session.session_id));
            }
            out.push(s);
        }
        out
    }

    fn open_start_link(&mut self, workspace: &str, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = target.strip_prefix("schedule:") {
            self.open_schedule(workspace, id, At::Top, cx);
        } else if let Some((repo, session_id)) = target.strip_prefix("session:").and_then(|t| t.split_once('\u{0}')) {
            let row = self.tree().workspaces.into_iter().filter(|n| n.name == workspace).flat_map(|n| n.repos).flat_map(|r| r.sessions).find(|r| r.session.session_id == session_id);
            if let Some(row) = row {
                self.open_row(workspace, repo, &row, window, cx);
            }
        }
    }

    pub(super) fn render_start_page(&self, tab: u64, page: &StartPage, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ws = self.workspaces.iter().find(|w| w.name == page.workspace)?;
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let label = |text: &'static str| div().flex_none().w(px(88.)).text_xs().text_color(colors.text_muted).child(text);
        let line = |name: &'static str| div().flex().flex_wrap().items_center().gap_2().min_w_0().child(label(name));
        let mut folder_line = line("Folder");
        for (i, (name, path)) in folders(ws).into_iter().enumerate() {
            let key = format!("start-folder-{i}");
            let selected = path_key(&path) == path_key(&page.folder);
            folder_line = folder_line.child(button::choice(SharedString::from(key.clone()), SharedString::from(name), selected, cx).relative().child(hit_target(&self.hits, key)).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| this.set_start_choice(tab, Some(path.clone()), None, window, cx),
            )));
        }
        let mut mode_line = line("Mode");
        for mode in MODES {
            let key = format!("start-mode-{mode}");
            mode_line = mode_line.child(button::choice(SharedString::from(key.clone()), mode, page.mode == mode, cx).relative().child(hit_target(&self.hits, key)).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| this.set_start_choice(tab, None, Some(mode), window, cx),
            )));
        }
        let form = div()
            .flex()
            .flex_col()
            .gap_3()
            .pb_4()
            .child(
                div()
                    .id("start-prompt")
                    .relative()
                    .child(page.input.clone())
                    .child(hit_target(&self.hits, "start-prompt".into())),
            )
            .when(page.scheduled, |d| {
                d.child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(format!("Sent after the line: {SCHEDULE_ASK}"))))
            })
            .child(folder_line)
            .child(mode_line)
            .children(page.error.clone().map(|e| banner::tinted(status.error).relative().child(SharedString::from(e)).child(hit_target(&self.hits, "start-error".into()))));
        let actions = vec![
            button::bordered(&self.hits, "start-send", "Send", cx)
                .font_weight(FontWeight::SEMIBOLD)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.send_start(tab, cx);
                }))
                .into_any_element(),
            button::bordered(&self.hits, "start-cancel", "Cancel", cx)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.close_tab(tab, cx);
                }))
                .into_any_element(),
        ];
        let sections = self.start_sections(&page.workspace);
        let workspace = page.workspace.clone();
        let on_link: details::OnLink<Self> = Rc::new(move |this: &mut Self, target: &str, window, cx| this.open_start_link(&workspace, target, window, cx));
        let title = if page.scheduled { format!("New scheduled task in {}", page.workspace) } else { format!("New task in {}", page.workspace) };
        let key = format!("start-{tab}");
        let page = details::Page { key: &key, title, pill: None, sections: &sections, actions, scroll: &page.scroll, on_link: Some(on_link), top: Some(form.into_any_element()) };
        Some(details::render(page, &self.hits, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use supraman_core::{Folder, Workspace};

    #[test]
    fn a_scheduled_task_asks_for_a_schedule_on_its_own_first_line() {
        assert_eq!(first_prompt("fix \"it\"\nnow", false), "fix \"it\"\nnow");
        let asked = first_prompt("every monday at 7: replay failures", true);
        assert_eq!(asked.lines().collect::<Vec<_>>(), [SCHEDULE_ASK, "every monday at 7: replay failures"]);
    }

    #[test]
    fn the_folders_are_the_home_then_each_folder_once() {
        let ws = Workspace::new("demo", r"C:\demo", vec![Folder::new("Alpha", r"C:\demo\Alpha"), Folder::new("Same", r"c:\DEMO")]);
        let names: Vec<String> = folders(&ws).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["demo (home)", "Alpha"]);
        assert!(!MODES.contains(&"bypassPermissions"));
    }

    #[test]
    fn a_claude_cmd_refuses_a_new_line_or_a_double_quote_and_names_claude_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let (npm, native) = (tmp.path().join("npm"), tmp.path().join("native"));
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(npm.join("claude.cmd"), "").unwrap();
        std::fs::write(native.join("claude.exe"), "").unwrap();
        let ext = ".COM;.EXE;.BAT;.CMD";
        let only_cmd = std::env::join_paths([&npm]).unwrap();
        let cmd = launched_file("claude", &only_cmd, ext);
        assert_eq!(cmd, Some(npm.join("claude.cmd")));
        let why = cmd_refusal(cmd.as_deref(), "two\nlines").unwrap();
        assert!(why.contains("claude.cmd") && why.contains("the native claude.exe") && why.contains("carries any text"), "{why}");
        assert!(cmd_refusal(cmd.as_deref(), "say \"hi\"").is_some());
        assert_eq!(cmd_refusal(cmd.as_deref(), "one line, no quotes"), None);
        let exe = launched_file("claude", &std::env::join_paths([&native, &npm]).unwrap(), ext);
        assert_eq!(cmd_refusal(exe.as_deref(), "two\n\"lines\""), None);
        assert_eq!(cmd_refusal(launched_file("claude", &std::ffi::OsString::new(), ext).as_deref(), "a\nb"), None, "no claude: the launch says so");
        assert_eq!(launched_file(r"& 'C:\x\claude.cmd' --selftest-argv 'out'", &only_cmd, ext), Some(PathBuf::from(r"C:\x\claude.cmd")));
    }

    #[test]
    fn the_prompt_file_holds_the_text_byte_for_byte() {
        let home = tempfile::tempdir().unwrap();
        let text = "say \"hi\" to $env:USERNAME\n'quoted' `tick` % é 中\r\nend";
        let file = write_prompt(Some(home.path()), text).unwrap();
        assert!(file.starts_with(home.path().join("prompts")));
        assert_eq!(std::fs::read(&file).unwrap(), text.as_bytes());
    }
}
