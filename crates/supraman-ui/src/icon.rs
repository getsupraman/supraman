//! Zed's icons (`assets/icons`, Lucide-derived, ISC — see `assets/icons/LICENSES`), compiled in and tinted with the text colour.

use gpui::{Hsla, Svg, prelude::*, px, svg};

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
    Clock,
    Check,
    Home,
    GitGraph,
    Local,
    Remote,
    Tag,
    Worktree,
    Copy,
    Modified,
    Renamed,
    SortDown,
    SortUp,
    ListFlat,
    ListTree,
    DiffSplit,
    DiffInline,
    File,
    FileDiff,
    ChevronUp,
    Untracked,
    Conflict,
    Chat,
    Hash,
    Stash,
    ChevronLeft,
    Solo,
    ArrowUpRight,
    Undo,
    Redo,
    Trash,
    GitCommit,
}

impl Icon {
    #[cfg(test)]
    const ALL: [Icon; 60] = [
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
        Icon::Clock,
        Icon::Check,
        Icon::Home,
        Icon::GitGraph,
        Icon::Local,
        Icon::Remote,
        Icon::Tag,
        Icon::Worktree,
        Icon::Copy,
        Icon::Modified,
        Icon::Renamed,
        Icon::SortDown,
        Icon::SortUp,
        Icon::ListFlat,
        Icon::ListTree,
        Icon::DiffSplit,
        Icon::DiffInline,
        Icon::File,
        Icon::FileDiff,
        Icon::ChevronUp,
        Icon::Untracked,
        Icon::Conflict,
        Icon::Chat,
        Icon::Hash,
        Icon::Stash,
        Icon::ChevronLeft,
        Icon::Solo,
        Icon::ArrowUpRight,
        Icon::Undo,
        Icon::Redo,
        Icon::Trash,
        Icon::GitCommit,
    ];

    pub fn bytes(self) -> &'static [u8] {
        match self {
            Icon::ChevronDown => include_bytes!("../assets/icons/chevron_down.svg"),
            Icon::ChevronRight => include_bytes!("../assets/icons/chevron_right.svg"),
            Icon::Close => include_bytes!("../assets/icons/close.svg"),
            Icon::ExpandAll => include_bytes!("../assets/icons/chevron_up_down.svg"),
            Icon::CollapseAll => include_bytes!("../assets/icons/chevron_down_up.svg"),
            Icon::Eye => include_bytes!("../assets/icons/eye.svg"),
            Icon::EyeOff => include_bytes!("../assets/icons/eye_off.svg"),
            Icon::FolderAdd => include_bytes!("../assets/icons/folder_add.svg"),
            Icon::Minus => include_bytes!("../assets/icons/dash.svg"),
            Icon::Plus => include_bytes!("../assets/icons/plus.svg"),
            Icon::Refresh => include_bytes!("../assets/icons/refresh.svg"),
            Icon::Settings => include_bytes!("../assets/icons/settings.svg"),
            Icon::Star => include_bytes!("../assets/icons/star.svg"),
            Icon::StarFilled => include_bytes!("../assets/icons/star_filled.svg"),
            Icon::Warning => include_bytes!("../assets/icons/warning.svg"),
            Icon::Attach => include_bytes!("../assets/icons/maximize.svg"),
            Icon::Claude => include_bytes!("../assets/icons/claude.svg"),
            Icon::Folder => include_bytes!("../assets/icons/folder.svg"),
            Icon::GitBranch => include_bytes!("../assets/icons/git_branch.svg"),
            Icon::Play => include_bytes!("../assets/icons/play_outlined.svg"),
            Icon::RelayReady => include_bytes!("../assets/icons/signal_high.svg"),
            Icon::RelayStarting => include_bytes!("../assets/icons/load_circle.svg"),
            Icon::Resume => include_bytes!("../assets/icons/rotate_ccw.svg"),
            Icon::Square => include_bytes!("../assets/icons/stop.svg"),
            Icon::Tab => include_bytes!("../assets/icons/screen.svg"),
            Icon::Task => include_bytes!("../assets/icons/pencil.svg"),
            Icon::Terminal => include_bytes!("../assets/icons/terminal.svg"),
            Icon::Work => include_bytes!("../assets/icons/list_todo.svg"),
            Icon::Clock => include_bytes!("../assets/icons/clock.svg"),
            Icon::Check => include_bytes!("../assets/icons/check.svg"),
            Icon::Home => include_bytes!("../assets/icons/home.svg"),
            Icon::GitGraph => include_bytes!("../assets/icons/git_graph.svg"),
            Icon::Local => include_bytes!("../assets/icons/screen.svg"),
            Icon::Remote => include_bytes!("../assets/icons/server.svg"),
            Icon::Tag => include_bytes!("../assets/icons/bookmark.svg"),
            Icon::Worktree => include_bytes!("../assets/icons/git_worktree.svg"),
            Icon::Copy => include_bytes!("../assets/icons/copy.svg"),
            Icon::Modified => include_bytes!("../assets/icons/pencil.svg"),
            Icon::Renamed => include_bytes!("../assets/icons/arrow_right.svg"),
            Icon::SortDown => include_bytes!("../assets/icons/arrow_down.svg"),
            Icon::SortUp => include_bytes!("../assets/icons/arrow_up.svg"),
            Icon::ListFlat => include_bytes!("../assets/icons/menu.svg"),
            Icon::ListTree => include_bytes!("../assets/icons/list_tree.svg"),
            Icon::DiffSplit => include_bytes!("../assets/icons/diff_split.svg"),
            Icon::DiffInline => include_bytes!("../assets/icons/diff_unified.svg"),
            Icon::File => include_bytes!("../assets/icons/file.svg"),
            Icon::FileDiff => include_bytes!("../assets/icons/file_diff.svg"),
            Icon::ChevronUp => include_bytes!("../assets/icons/chevron_up.svg"),
            Icon::Untracked => include_bytes!("../assets/icons/circle_help.svg"),
            Icon::Conflict => include_bytes!("../assets/icons/git_merge_conflict.svg"),
            Icon::Chat => include_bytes!("../assets/icons/chat.svg"),
            Icon::Hash => include_bytes!("../assets/icons/hash.svg"),
            Icon::Stash => include_bytes!("../assets/icons/archive.svg"),
            Icon::ChevronLeft => include_bytes!("../assets/icons/chevron_left.svg"),
            Icon::Solo => include_bytes!("../assets/icons/crosshair.svg"),
            Icon::ArrowUpRight => include_bytes!("../assets/icons/arrow_up_right.svg"),
            Icon::Undo => include_bytes!("../assets/icons/undo.svg"),
            Icon::Redo => include_bytes!("../assets/icons/redo.svg"),
            Icon::Trash => include_bytes!("../assets/icons/trash.svg"),
            Icon::GitCommit => include_bytes!("../assets/icons/git_commit.svg"),
        }
    }

    pub fn svg(self, size: f32, color: Hsla) -> Svg {
        svg().data(self.bytes()).size(px(size)).flex_none().text_color(color)
    }

    /// The share of a slot the icon fills so that thin outlines and filled marks look one size.
    fn optical(self) -> f32 {
        match self {
            Icon::GitBranch | Icon::Folder => 1.0,
            _ => 0.875,
        }
    }

    /// The icon at its optical size for a square slot of `slot` px.
    pub fn slotted(self, slot: f32, color: Hsla) -> Svg {
        self.svg(slot * self.optical(), color)
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
