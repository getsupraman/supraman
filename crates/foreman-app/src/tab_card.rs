//! A terminal tab's program title and its hover card (m5/29). A program names its tab with OSC 0/2, as
//! claude does with its session name (`✳ name` idle, `◐`/`◑` while working); the tab shows that, cut
//! to fit, and its own label when there is none. Hovering the tab shows a card as Edge's tab hover does:
//! the whole title, then what is known of the tab, one line per field.

use std::{cell::RefCell, path::Path};

use foreman_core::Workspace;

use gpui::{App, AnyView, SharedString, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

/// Longer program titles are cut to this many characters, with an ellipsis.
pub const MAX_TITLE: usize = 200;

/// `raw` as a tab label: control and bidi-format characters dropped, runs of whitespace as one space,
/// trimmed, at most [`MAX_TITLE`] characters. Nothing left reads as no title at all.
pub fn sanitize(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut space = false;
    let mut count = 0;
    for c in raw.chars() {
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if c.is_control() || is_format(c) {
            continue;
        }
        if count == MAX_TITLE {
            out.push('…');
            break;
        }
        if space {
            out.push(' ');
            count += 1;
            space = false;
            if count == MAX_TITLE {
                out.push('…');
                break;
            }
        }
        out.push(c);
        count += 1;
    }
    (!out.is_empty()).then_some(out)
}

/// Bidi embeddings, overrides and isolates, zero-width and BOM characters: they would reorder or hide
/// what the tab shows.
fn is_format(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// What a tab is, as its card says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `claude` started from `+` or resumed in the tab: a human drives it.
    Hand,
    /// `claude attach` to a background session.
    Attach,
    Shell,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Hand => "hand",
            Kind::Attach => "bg attach",
            Kind::Shell => "shell",
        }
    }
}

/// What the card shows; `None` fields are left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    pub title: String,
    pub workspace: Option<String>,
    pub repo: Option<String>,
    pub folder: Option<String>,
    pub branch: Option<String>,
    pub session: Option<String>,
    pub kind: Option<Kind>,
    /// A hand tab's WorkState credential: Foreman's token overrides the user's `workstate` entry
    /// ([`crate::hand`]).
    pub workstate: Option<String>,
    /// Epoch milliseconds.
    pub started: Option<i64>,
}

impl Card {
    /// The lines under the title, as `(label, value)`.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = Vec::new();
        let mut put = |label, value: Option<String>| {
            if let Some(v) = value.filter(|v| !v.is_empty()) {
                rows.push((label, v));
            }
        };
        put("workspace", self.workspace.clone());
        put("repo", self.repo.clone());
        put("folder", self.folder.clone());
        put("branch", self.branch.clone());
        put("session", self.session.clone());
        put("kind", self.kind.map(|k| k.label().to_string()));
        put("WorkState", self.workstate.clone());
        put("started", self.started.and_then(local_time));
        rows
    }

    /// The card as text: the title, then one `label  value` line per row. The selftest compares this.
    pub fn text(&self) -> String {
        let mut text = self.title.clone();
        for (label, value) in self.rows() {
            text.push_str(&format!("\n{label}  {value}"));
        }
        text
    }
}

fn local_time(ms: i64) -> Option<String> {
    use chrono::TimeZone as _;
    let at = chrono::Local.timestamp_millis_opt(ms).single()?;
    let today = chrono::Local::now().date_naive() == at.date_naive();
    Some(if today { at.format("%H:%M") } else { at.format("%Y-%m-%d %H:%M") }.to_string())
}

/// The branch checked out at `dir` or the nearest folder above it with a `.git`; `None` when detached
/// or not in a repo.
pub fn branch_at(dir: &Path) -> Option<String> {
    foreman_core::branch_at(dir)
}

/// `name · id`, or the id alone for an unnamed session.
pub fn session_label(name: &str, id: &str) -> String {
    if name.is_empty() { id.to_string() } else { format!("{name} · {id}") }
}

/// The workspace holding `cwd` (the innermost when roots nest) and the repo of it holding `cwd`, by
/// name. Case- and `/`-insensitive, as [`crate::fleet::shell_cwd`].
pub fn place(cwd: &Path, workspaces: &[Workspace]) -> (Option<String>, Option<String>) {
    let key = |p: &Path| -> Vec<String> { p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect() };
    let at = key(cwd);
    let inner = |p: &Path| at.starts_with(&key(p)).then(|| p.components().count());
    let Some(ws) = workspaces.iter().filter(|w| inner(&w.root).is_some()).max_by_key(|w| w.root.components().count()) else {
        return (None, None);
    };
    let repo = ws.repos.iter().filter_map(|r| inner(&r.path).map(|depth| (depth, r))).max_by_key(|(depth, _)| *depth);
    (Some(ws.name.clone()), repo.map(|(_, r)| r.name.clone()))
}

thread_local! {
    /// The card shown last, as [`Card::text`]; the selftest reads it.
    pub static LAST_CARD: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// A tooltip builder for the tab: the card is put together when it is shown, so its branch is current.
pub fn tooltip(card: impl Fn() -> Card + 'static) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| {
        let card = card();
        LAST_CARD.with(|last| *last.borrow_mut() = Some(card.text()));
        cx.new(|_| CardView(card)).into()
    }
}

struct CardView(Card);

impl Render for CardView {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let quiet = colors.text.opacity(0.62);
        let rows = self.0.rows().into_iter().map(|(label, value)| {
            div()
                .flex()
                .gap_2()
                .child(div().flex_none().w(px(64.)).text_color(quiet).child(label))
                .child(div().min_w_0().child(SharedString::from(value)))
        });
        div().pl_2().pt_2().child(
            div()
                .w(px(360.))
                .px_3()
                .py_2()
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.elevated_surface_background)
                .text_xs()
                .font_family(foreman_ui::fonts::UI)
                .text_color(colors.text)
                .shadow_md()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().pb_1().child(SharedString::from(self.0.title.clone())))
                .children(rows),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_dropped_and_whitespace_folded() {
        assert_eq!(sanitize("✳ fix the tray").as_deref(), Some("✳ fix the tray"));
        assert_eq!(sanitize("  a\x07b\x1b[31m  c\r\n d\t e ").as_deref(), Some("ab[31m c d e"));
        assert_eq!(sanitize("x\u{9b}y\u{90}z\u{85}w").as_deref(), Some("xyz w"), "C1 controls; NEL is a line break");
        assert_eq!(sanitize("a\u{202E}cba\u{202C}\u{200B}!").as_deref(), Some("acba!"), "bidi override, zero width");
    }

    #[test]
    fn an_empty_or_blank_title_is_none() {
        assert_eq!(sanitize(""), None);
        assert_eq!(sanitize(" \t\r\n\x1b\u{FEFF}"), None);
    }

    #[test]
    fn a_long_title_is_cut_with_an_ellipsis() {
        let long = "é".repeat(MAX_TITLE + 50);
        let cut = sanitize(&long).unwrap();
        assert_eq!(cut.chars().count(), MAX_TITLE + 1);
        assert!(cut.ends_with('…'));
        let exact = "x".repeat(MAX_TITLE);
        assert_eq!(sanitize(&exact).as_deref(), Some(exact.as_str()), "exactly the cap is not cut");
        let spaced = format!("{} y", "x".repeat(MAX_TITLE - 1));
        assert_eq!(sanitize(&spaced).unwrap().chars().count(), MAX_TITLE + 1, "a space counts");
    }

    #[test]
    fn the_card_lists_only_known_fields_in_order() {
        let card = Card {
            title: "✳ fix the tray".into(),
            workspace: Some("autowork".into()),
            folder: Some(r"P:\x".into()),
            branch: Some(String::new()),
            kind: Some(Kind::Attach),
            ..Card::default()
        };
        assert_eq!(card.text(), "✳ fix the tray\nworkspace  autowork\nfolder  P:\\x\nkind  bg attach");
        let hand = Card {
            title: "✳ mw-workflow".into(),
            kind: Some(Kind::Hand),
            workstate: Some("Foreman token · orchestrator · mw-workflow".into()),
            ..Card::default()
        };
        assert_eq!(hand.text(), "✳ mw-workflow\nkind  hand\nWorkState  Foreman token · orchestrator · mw-workflow");
    }

    #[test]
    fn the_place_is_the_innermost_workspace_and_its_repo() {
        let repo = |name: &str, path: &str| foreman_core::Repo {
            name: name.into(),
            path: path.into(),
            has_claude_md: false,
            has_agents_md: false,
            branch: None,
            enabled: true,
        };
        let outer = Workspace::new("outer", "P:/Projects", vec![]);
        let aw = Workspace::new("autowork", r"P:\Projects\AutoWork", vec![repo("AutoWork", r"P:\Projects\AutoWork"), repo("zed", r"P:\Projects\AutoWork\.repos\zed")]);
        let all = [outer, aw];
        let at = |p: &str| place(Path::new(p), &all);
        assert_eq!(at(r"p:\projects\autowork\.repos\zed\crates"), (Some("autowork".into()), Some("zed".into())));
        assert_eq!(at(r"P:\Projects\AutoWork\docs"), (Some("autowork".into()), Some("AutoWork".into())));
        assert_eq!(at(r"P:\Projects\Other"), (Some("outer".into()), None));
        assert_eq!(at(r"D:\x"), (None, None));
        assert_eq!(session_label("", "abc"), "abc");
        assert_eq!(session_label("fix-tray", "abc"), "fix-tray · abc");
    }

    #[test]
    fn the_branch_comes_from_the_nearest_git() {
        let root = std::env::temp_dir().join(format!("fm-branch-{}", std::process::id()));
        let inner = root.join("src").join("deep");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/m5/29\n").unwrap();
        assert_eq!(branch_at(&inner).as_deref(), Some("m5/29"));
        std::fs::write(root.join(".git").join("HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(branch_at(&inner), None, "detached");
        std::fs::remove_dir_all(&root).ok();
    }
}
