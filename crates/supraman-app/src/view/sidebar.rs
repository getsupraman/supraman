use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use gpui::{App, AppContext as _, Entity, Pixels};
use supraman_ui::keyed_list::KeyedList;

use super::left::{self, LeftRow, PendingReveal};
use super::left_width::LeftWidth;
use super::DEFAULT_LEFT_WIDTH;
use crate::fleet::Tree;

/// The left panel's state: its width, the tree and its rows as last built, what is open or closed, and its holds.
pub struct Sidebar {
    pub left_width: Pixels,
    pub(super) left_saves: Entity<LeftWidth>,
    pub(super) left_hold: Option<left::Hold>,
    pub(super) left_hold_timer: Option<gpui::Task<()>>,
    pub left_holds: u64,
    pub(super) tree_cache: RefCell<Option<CachedTree>>,
    pub(super) left_list: KeyedList,
    pub(super) reveal_pending: Option<(PendingReveal, Instant)>,
    pub(super) left_rows: Rc<(Rc<Tree>, Vec<LeftRow>)>,
    pub(super) collapsed: HashSet<String>,
    pub expanded: HashSet<String>,
    pub closed_expanded: HashSet<String>,
    pub(super) search_marks: HashMap<String, crate::search::Marks>,
    /// The sidebar's pane in [`supraman_ui::focus`].
    pub(super) focus: gpui::FocusHandle,
}

pub(super) struct CachedTree {
    pub at: i64,
    pub until: Option<i64>,
    pub show_stopped: bool,
    pub own_prefix: Option<String>,
    pub tree: Rc<Tree>,
}

impl Sidebar {
    pub fn new(cx: &mut App) -> Self {
        Self {
            left_width: DEFAULT_LEFT_WIDTH,
            left_saves: cx.new(|_| LeftWidth::default()),
            left_hold: None,
            left_hold_timer: None,
            left_holds: 0,
            tree_cache: RefCell::new(None),
            left_list: Default::default(),
            reveal_pending: None,
            left_rows: Rc::new((Rc::new(Tree { workspaces: Vec::new(), hidden_stopped: 0 }), Vec::new())),
            collapsed: HashSet::new(),
            expanded: HashSet::new(),
            closed_expanded: HashSet::new(),
            search_marks: HashMap::new(),
            focus: cx.focus_handle(),
        }
    }
}
