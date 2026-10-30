//! Zed's icons (`assets/icons`, Lucide-derived, ISC — see `assets/icons/LICENSES`), compiled in and tinted with the text colour.

use gpui::{AnyView, App, Context, Hsla, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Svg, Window, div, prelude::*, px, svg};
use theme::ActiveTheme as _;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    ChevronDown,
    ChevronRight,
    Close,
    ExpandAll,
    CollapseAll,
    Eye,
    EyeOff,
    FolderAdd,
    Minus,
    Plus,
    Refresh,
    Settings,
    Star,
    StarFilled,
    Warning,
    Attach,
    Claude,
    Folder,
    GitBranch,
    Play,
    RelayReady,
    RelayStarting,
    Resume,
    Square,
    Tab,
    Task,
    Terminal,
    Work,
}

impl Icon {
    #[cfg(test)]
    const ALL: [Icon; 28] = [
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::Close,
        Icon::ExpandAll,
        Icon::CollapseAll,
        Icon::Eye,
        Icon::EyeOff,
        Icon::FolderAdd,
        Icon::Minus,
        Icon::Plus,
        Icon::Refresh,
        Icon::Settings,
        Icon::Star,
        Icon::StarFilled,
        Icon::Warning,
        Icon::Attach,
        Icon::Claude,
        Icon::Folder,
        Icon::GitBranch,
        Icon::Play,
        Icon::RelayReady,
        Icon::RelayStarting,
        Icon::Resume,
        Icon::Square,
        Icon::Tab,
        Icon::Task,
        Icon::Terminal,
        Icon::Work,
    ];

    pub fn bytes(self) -> &'static [u8] {
        match self {
            Icon::ChevronDown => include_bytes!("../../assets/icons/chevron_down.svg"),
            Icon::ChevronRight => include_bytes!("../../assets/icons/chevron_right.svg"),
            Icon::Close => include_bytes!("../../assets/icons/close.svg"),
            Icon::ExpandAll => include_bytes!("../../assets/icons/chevron_up_down.svg"),
            Icon::CollapseAll => include_bytes!("../../assets/icons/chevron_down_up.svg"),
            Icon::Eye => include_bytes!("../../assets/icons/eye.svg"),
            Icon::EyeOff => include_bytes!("../../assets/icons/eye_off.svg"),
            Icon::FolderAdd => include_bytes!("../../assets/icons/folder_add.svg"),
            Icon::Minus => include_bytes!("../../assets/icons/dash.svg"),
            Icon::Plus => include_bytes!("../../assets/icons/plus.svg"),
            Icon::Refresh => include_bytes!("../../assets/icons/refresh.svg"),
            Icon::Settings => include_bytes!("../../assets/icons/settings.svg"),
            Icon::Star => include_bytes!("../../assets/icons/star.svg"),
            Icon::StarFilled => include_bytes!("../../assets/icons/star_filled.svg"),
            Icon::Warning => include_bytes!("../../assets/icons/warning.svg"),
            Icon::Attach => include_bytes!("../../assets/icons/maximize.svg"),
            Icon::Claude => include_bytes!("../../assets/icons/claude.svg"),
            Icon::Folder => include_bytes!("../../assets/icons/folder.svg"),
            Icon::GitBranch => include_bytes!("../../assets/icons/git_branch.svg"),
            Icon::Play => include_bytes!("../../assets/icons/play_outlined.svg"),
            Icon::RelayReady => include_bytes!("../../assets/icons/signal_high.svg"),
            Icon::RelayStarting => include_bytes!("../../assets/icons/load_circle.svg"),
            Icon::Resume => include_bytes!("../../assets/icons/rotate_ccw.svg"),
            Icon::Square => include_bytes!("../../assets/icons/stop.svg"),
            Icon::Tab => include_bytes!("../../assets/icons/screen.svg"),
            Icon::Task => include_bytes!("../../assets/icons/pencil.svg"),
            Icon::Terminal => include_bytes!("../../assets/icons/terminal.svg"),
            Icon::Work => include_bytes!("../../assets/icons/list_todo.svg"),
        }
    }

    pub fn svg(self, size: f32, color: Hsla) -> Svg {
        svg().data(self.bytes()).size(px(size)).flex_none().text_color(color)
    }
}

pub struct Tooltip(SharedString);

thread_local! {
    pub static LAST_SHOWN: std::cell::RefCell<Option<SharedString>> = const { std::cell::RefCell::new(None) };
}

impl Tooltip {
    pub fn text(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let text = text.into();
        move |_, cx| {
            LAST_SHOWN.with(|last| *last.borrow_mut() = Some(text.clone()));
            cx.new(|_| Tooltip(text.clone())).into()
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        div().pl_2().pt_2().child(
            div()
                .max_w(px(320.))
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.elevated_surface_background)
                .text_xs()
                .font_family(foreman_ui::fonts::UI)
                .text_color(colors.text)
                .shadow_md()
                .child(self.0.clone()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_16px_svg() {
        for icon in Icon::ALL {
            let text = std::str::from_utf8(icon.bytes()).unwrap();
            assert!(text.starts_with("<svg") && text.contains(r#"width="16""#), "{icon:?}: {text}");
            assert!(text.trim_end().ends_with("</svg>"), "{icon:?} is cut short");
        }
    }
}
