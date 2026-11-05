//! A tree list over flat `/`-separated paths: folders derived from the paths and listed before files at
//! each level, expanded and collapsed per folder, filtered by a substring of the whole path, each folder
//! carrying an aggregate of its subtree. Its rows are a headless [`Table`], so only the rows in view are
//! laid out. [`TreeState`] keeps the tree, the open folders (by path, so a rebuild keeps them), the
//! filter, the order and the selection.

use std::{cell::RefCell, collections::HashMap, collections::HashSet, rc::Rc};

use gpui::{AnyElement, App, Context, Div, FocusHandle, Pixels, SharedString, Stateful, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::hit::HitTargets;
use crate::icon::Icon;
use crate::matcher::find_all;
use crate::table::{Column, Table, TableState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    #[default]
    Ascending,
    Descending,
}

impl Order {
    pub fn reversed(self) -> Self {
        match self {
            Order::Ascending => Order::Descending,
            Order::Descending => Order::Ascending,
        }
    }
}

enum Kind<T, A> {
    File(T),
    Folder { children: Vec<usize>, aggregate: A },
}

struct Node<T, A> {
    name: SharedString,
    path: SharedString,
    depth: usize,
    parent: Option<usize>,
    kind: Kind<T, A>,
}

/// The nodes of a tree; folders hold their children in display order.
pub struct Tree<T, A> {
    nodes: Vec<Node<T, A>>,
    roots: Vec<usize>,
    order: Order,
}

impl<T, A: Default> Tree<T, A> {
    /// The tree of `entries` (`\` read as `/`), each folder's aggregate `fold`ed over every file below it.
    pub fn build(entries: impl IntoIterator<Item = (String, T)>, fold: impl Fn(&mut A, &T), order: Order) -> Self {
        let mut tree = Tree { nodes: Vec::new(), roots: Vec::new(), order: Order::Ascending };
        let mut folders: HashMap<SharedString, usize> = HashMap::new();
        let mut aggregates: HashMap<usize, A> = HashMap::new();
        for (path, payload) in entries {
            let path = normalize(&path);
            let (dir, name) = match path.rsplit_once('/') {
                Some((dir, name)) => (Some(dir), name),
                None => (None, path.as_str()),
            };
            if name.is_empty() {
                continue;
            }
            let parent = dir.map(|dir| tree.folder(dir, &mut folders, &mut aggregates));
            let mut up = parent;
            while let Some(folder) = up {
                fold(aggregates.entry(folder).or_default(), &payload);
                up = tree.nodes[folder].parent;
            }
            tree.push(name, &path, parent, Kind::File(payload));
        }
        for (folder, value) in aggregates {
            if let Kind::Folder { aggregate, .. } = &mut tree.nodes[folder].kind {
                *aggregate = value;
            }
        }
        tree.order = order;
        tree.sort_all();
        tree
    }

    fn folder(&mut self, dir: &str, folders: &mut HashMap<SharedString, usize>, aggregates: &mut HashMap<usize, A>) -> usize {
        if let Some(&at) = folders.get(dir) {
            return at;
        }
        let (parent, name) = match dir.rsplit_once('/') {
            Some((up, name)) => (Some(self.folder(up, folders, aggregates)), name),
            None => (None, dir),
        };
        let at = self.push(name, dir, parent, Kind::Folder { children: Vec::new(), aggregate: A::default() });
        folders.insert(SharedString::from(dir.to_string()), at);
        aggregates.insert(at, A::default());
        at
    }
}

impl<T, A> Tree<T, A> {
    fn push(&mut self, name: &str, path: &str, parent: Option<usize>, kind: Kind<T, A>) -> usize {
        let at = self.nodes.len();
        let depth = parent.map_or(0, |p| self.nodes[p].depth + 1);
        self.nodes.push(Node { name: name.to_string().into(), path: path.to_string().into(), depth, parent, kind });
        match parent.map(|p| &mut self.nodes[p].kind) {
            Some(Kind::Folder { children, .. }) => children.push(at),
            _ => self.roots.push(at),
        }
        at
    }

    /// Folders first at every level, then the names A–Z ignoring case (Z–A when `Descending`).
    pub fn sort(&mut self, order: Order) {
        if order != self.order {
            self.order = order;
            self.sort_all();
        }
    }

    fn sort_all(&mut self) {
        let order = self.order;
        let keys: Vec<(bool, String)> = self.nodes.iter().map(|n| (matches!(n.kind, Kind::File(_)), n.name.to_lowercase())).collect();
        let by = |a: &usize, b: &usize| {
            let names = keys[*a].1.cmp(&keys[*b].1).then_with(|| self.nodes[*a].name.cmp(&self.nodes[*b].name));
            keys[*a].0.cmp(&keys[*b].0).then(if order == Order::Descending { names.reverse() } else { names })
        };
        let mut roots = std::mem::take(&mut self.roots);
        roots.sort_by(by);
        let mut lists: Vec<(usize, Vec<usize>)> = Vec::new();
        for (i, node) in self.nodes.iter().enumerate() {
            if let Kind::Folder { children, .. } = &node.kind {
                let mut children = children.clone();
                children.sort_by(by);
                lists.push((i, children));
            }
        }
        for (i, sorted) in lists {
            if let Kind::Folder { children, .. } = &mut self.nodes[i].kind {
                *children = sorted;
            }
        }
        self.roots = roots;
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Every folder's path.
    pub fn folders(&self) -> impl Iterator<Item = &SharedString> {
        self.nodes.iter().filter(|n| matches!(n.kind, Kind::Folder { .. })).map(|n| &n.path)
    }

    /// The folder at `path`'s aggregate.
    pub fn aggregate(&self, path: &str) -> Option<&A> {
        self.nodes.iter().find_map(|n| match &n.kind {
            Kind::Folder { aggregate, .. } if n.path == path => Some(aggregate),
            _ => None,
        })
    }

    /// Which nodes `filter` keeps: a file whose path holds it, ignoring case, and every folder above one.
    fn kept(&self, filter: &str) -> Option<Vec<bool>> {
        let needle = filter.replace('\\', "/");
        if needle.is_empty() {
            return None;
        }
        let mut kept = vec![false; self.nodes.len()];
        for (i, node) in self.nodes.iter().enumerate() {
            if !matches!(node.kind, Kind::File(_)) || find_all(&node.path, &needle).is_empty() {
                continue;
            }
            kept[i] = true;
            let mut up = node.parent;
            while let Some(at) = up.filter(|at| !kept[*at]) {
                kept[at] = true;
                up = self.nodes[at].parent;
            }
        }
        Some(kept)
    }

    /// The rows shown: a depth-first walk into the `open` folders, only `kept` nodes when filtered.
    fn rows(&self, open: &HashSet<SharedString>, kept: Option<&[bool]>) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self.roots.iter().rev().copied().collect();
        while let Some(at) = stack.pop() {
            if kept.is_some_and(|k| !k[at]) {
                continue;
            }
            out.push(at);
            if let Kind::Folder { children, .. } = &self.nodes[at].kind
                && open.contains(&self.nodes[at].path)
            {
                stack.extend(children.iter().rev());
            }
        }
        out
    }
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_matches('/').to_string()
}

/// One row as the owner's painter sees it.
pub struct Row<'a, T, A> {
    pub name: &'a SharedString,
    pub path: &'a SharedString,
    pub depth: usize,
    pub item: Item<'a, T, A>,
    pub selected: bool,
}

pub enum Item<'a, T, A> {
    File(&'a T),
    Folder { aggregate: &'a A, open: bool },
}

struct Inner<T, A> {
    tree: Tree<T, A>,
    open: HashSet<SharedString>,
    filter: String,
    kept: Option<Vec<bool>>,
    order: Order,
    rows: Vec<usize>,
    selected: Option<SharedString>,
}

impl<T, A> Inner<T, A> {
    fn refresh(&mut self) {
        self.rows = self.tree.rows(&self.open, self.kept.as_deref());
    }

    fn row_of(&self, path: &str) -> Option<usize> {
        self.rows.iter().position(|at| self.tree.nodes[*at].path == path)
    }

    fn node(&self, row: usize) -> Option<&Node<T, A>> {
        self.rows.get(row).map(|at| &self.tree.nodes[*at])
    }
}

/// What the owner keeps of a tree between frames; cloning shares it.
pub struct TreeState<T, A> {
    inner: Rc<RefCell<Inner<T, A>>>,
    table: TableState,
}

impl<T, A> Clone for TreeState<T, A> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), table: self.table.clone() }
    }
}

impl<T: 'static, A: Default + 'static> TreeState<T, A> {
    pub fn new(cx: &mut App) -> Self {
        let inner = Inner {
            tree: Tree { nodes: Vec::new(), roots: Vec::new(), order: Order::Ascending },
            open: HashSet::new(),
            filter: String::new(),
            kept: None,
            order: Order::default(),
            rows: Vec::new(),
            selected: None,
        };
        Self { inner: Rc::new(RefCell::new(inner)), table: TableState::new(cx) }
    }

    /// Rebuilds the tree from `entries`, keeping the open folders, the filter, the order and the selection
    /// by path. Build it off the UI thread with [`Tree::build`] and hand it to [`TreeState::set_tree`]
    /// when the entries are many.
    pub fn set_entries(&self, entries: impl IntoIterator<Item = (String, T)>, fold: impl Fn(&mut A, &T)) {
        let order = self.inner.borrow().order;
        self.set_tree(Tree::build(entries, fold, order));
    }

    pub fn set_tree(&self, mut tree: Tree<T, A>) {
        let mut inner = self.inner.borrow_mut();
        tree.sort(inner.order);
        inner.tree = tree;
        let filter = std::mem::take(&mut inner.filter);
        drop(inner);
        self.set_filter(&filter);
    }
}

impl<T, A> TreeState<T, A> {
    pub fn focus_handle(&self) -> &FocusHandle {
        self.table.focus_handle()
    }

    pub fn order(&self) -> Order {
        self.inner.borrow().order
    }

    pub fn set_order(&self, order: Order) {
        let mut inner = self.inner.borrow_mut();
        inner.order = order;
        inner.tree.sort(order);
        inner.refresh();
    }

    pub fn filter(&self) -> String {
        self.inner.borrow().filter.clone()
    }

    /// Keeps the files whose whole path holds `filter` (ignoring case, `\` as `/`) with the folders above
    /// them, and opens those folders.
    pub fn set_filter(&self, filter: &str) {
        let mut inner = self.inner.borrow_mut();
        inner.filter = filter.to_string();
        inner.kept = inner.tree.kept(filter);
        if let Some(kept) = &inner.kept {
            let opened: Vec<SharedString> = inner
                .tree
                .nodes
                .iter()
                .zip(kept)
                .filter(|(n, k)| **k && matches!(n.kind, Kind::Folder { .. }))
                .map(|(n, _)| n.path.clone())
                .collect();
            inner.open.extend(opened);
        }
        inner.refresh();
    }

    /// The folder at `path`'s aggregate.
    pub fn aggregate(&self, path: &str) -> Option<A>
    where
        A: Clone,
    {
        self.inner.borrow().tree.aggregate(path).cloned()
    }

    pub fn is_open(&self, path: &str) -> bool {
        self.inner.borrow().open.contains(path)
    }

    /// Opens a closed folder and closes an open one.
    pub fn toggle(&self, path: &str) {
        let mut inner = self.inner.borrow_mut();
        if !inner.open.remove(path) {
            inner.open.insert(SharedString::from(path.to_string()));
        }
        inner.refresh();
    }

    pub fn expand_all(&self) {
        let mut inner = self.inner.borrow_mut();
        let all: Vec<SharedString> = inner.tree.folders().cloned().collect();
        inner.open.extend(all);
        inner.refresh();
    }

    pub fn collapse_all(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.open.clear();
        inner.refresh();
    }

    /// The paths of the rows shown, top to bottom.
    pub fn visible(&self) -> Vec<SharedString> {
        let inner = self.inner.borrow();
        inner.rows.iter().map(|at| inner.tree.nodes[*at].path.clone()).collect()
    }

    pub fn selected(&self) -> Option<SharedString> {
        self.inner.borrow().selected.clone()
    }

    /// Selects the row of `path` (none with `None`) and scrolls it into view when it is shown.
    pub fn select(&self, path: Option<&str>) {
        let row = {
            let mut inner = self.inner.borrow_mut();
            inner.selected = path.map(|p| SharedString::from(p.to_string()));
            path.and_then(|p| inner.row_of(p))
        };
        if let Some(row) = row {
            self.table.reveal(row);
        }
    }

    fn select_row(&self, row: usize) -> Option<SharedString> {
        let path = self.inner.borrow().node(row).map(|n| n.path.clone());
        self.inner.borrow_mut().selected = path.clone();
        self.table.select(Some(row));
        self.table.reveal(row);
        path
    }

    /// Left closes the selected open folder, else selects its folder; Right opens a closed folder, else
    /// selects its first row. The newly selected path, if the selection moved.
    fn arrow(&self, key: &str) -> Option<Option<SharedString>> {
        let (row, folder, open, parent) = {
            let inner = self.inner.borrow();
            let row = inner.row_of(inner.selected.as_ref()?)?;
            let node = inner.node(row)?;
            let parent = node.parent.and_then(|p| inner.row_of(&inner.tree.nodes[p].path));
            let folder = matches!(node.kind, Kind::Folder { .. }).then(|| node.path.clone());
            let open = folder.as_ref().is_some_and(|f| inner.open.contains(f));
            (row, folder, open, parent)
        };
        match (key, folder) {
            ("left", Some(f)) if open => self.toggle(&f),
            ("left", _) => return Some(parent.and_then(|p| self.select_row(p))),
            ("right", Some(f)) if !open => self.toggle(&f),
            ("right", Some(_)) => return Some(self.select_row(row + 1)),
            ("right", None) => {}
            _ => return None,
        }
        Some(None)
    }
}

/// A folder's chevron slot; a file's painter starts with a slot this wide, so its name lines up with its sibling folders'.
pub const LEAD: f32 = 16.;

type PaintFn<V, T, A> = dyn Fn(&mut V, Row<'_, T, A>, &mut Window, &mut Context<V>) -> AnyElement;
type SelectFn<V> = dyn Fn(&mut V, &SharedString, &mut Window, &mut Context<V>);

/// One frame's tree list, built in the owner's render. Rows paint under `<id>-row-<path>`.
pub struct TreeList<V, T, A> {
    id: SharedString,
    state: TreeState<T, A>,
    hits: HitTargets,
    row_height: Pixels,
    indent: Pixels,
    paint: Rc<PaintFn<V, T, A>>,
    on_select: Option<Rc<SelectFn<V>>>,
}

impl<V: 'static, T: 'static, A: 'static> TreeList<V, T, A> {
    /// `paint` draws a row's content right of its indent, and of a folder's chevron (see [`LEAD`]).
    pub fn new(
        id: impl Into<SharedString>,
        state: &TreeState<T, A>,
        hits: &HitTargets,
        paint: impl Fn(&mut V, Row<'_, T, A>, &mut Window, &mut Context<V>) -> AnyElement + 'static,
    ) -> Self {
        Self { id: id.into(), state: state.clone(), hits: hits.clone(), row_height: px(24.), indent: px(14.), paint: Rc::new(paint), on_select: None }
    }

    pub fn row_height(mut self, height: Pixels) -> Self {
        self.row_height = height;
        self
    }

    pub fn indent(mut self, indent: Pixels) -> Self {
        self.indent = indent;
        self
    }

    /// Called after a click or a key selects a row.
    pub fn on_select(mut self, f: impl Fn(&mut V, &SharedString, &mut Window, &mut Context<V>) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    pub fn render(self, cx: &mut Context<V>) -> Stateful<Div> {
        let (rows, selected) = {
            let inner = self.state.inner.borrow();
            (inner.rows.len(), inner.selected.as_ref().and_then(|p| inner.row_of(p)))
        };
        self.state.table.select(selected);
        let column = [Column::new("name", "", px(0.), px(0.))];
        let (keys, picked, clicked, arrows) = (self.state.clone(), self.state.clone(), self.state.clone(), self.state.clone());
        let (on_select, on_arrow) = (self.on_select.clone(), self.on_select.clone());
        Table::new(self.id.clone(), &self.state.table, &self.hits, column, &[px(0.)], rows, self.cell())
            .headless()
            .row_height(self.row_height)
            .row_key(move |ix| keys.inner.borrow().node(ix).map_or_else(String::new, |n| n.path.to_string()))
            .on_select(move |this, ix, window, cx| {
                if let (Some(path), Some(f)) = (picked.select_row(ix), &on_select) {
                    f(this, &path, window, cx);
                }
            })
            .on_click(move |_, ix, _, _| {
                let folder = clicked.inner.borrow().node(ix).filter(|n| matches!(n.kind, Kind::Folder { .. })).map(|n| n.path.clone());
                if let Some(folder) = folder {
                    clicked.toggle(&folder);
                }
            })
            .on_key(move |this, key, window, cx| match arrows.arrow(key) {
                None => false,
                Some(moved) => {
                    if let (Some(path), Some(f)) = (moved, &on_arrow) {
                        f(this, &path, window, cx);
                    }
                    true
                }
            })
            .render(cx)
    }

    fn cell(&self) -> impl Fn(&mut V, usize, usize, &mut Window, &mut Context<V>) -> AnyElement + 'static {
        let (state, paint, indent) = (self.state.clone(), self.paint.clone(), self.indent);
        move |this, ix, _, window, cx| {
            let inner = state.inner.borrow();
            let Some(node) = inner.node(ix) else { return div().into_any_element() };
            let muted = cx.theme().colors().text_muted;
            let (item, chevron) = match &node.kind {
                Kind::File(payload) => (Item::File(payload), None),
                Kind::Folder { aggregate, .. } => {
                    let open = inner.open.contains(&node.path);
                    (Item::Folder { aggregate, open }, Some(if open { Icon::ChevronDown } else { Icon::ChevronRight }))
                }
            };
            let selected = inner.selected.as_ref() == Some(&node.path);
            let row = Row { name: &node.name, path: &node.path, depth: node.depth, item, selected };
            let content = paint(this, row, window, cx);
            div()
                .flex()
                .items_center()
                .w_full()
                .min_w_0()
                .pl(indent * node.depth as f32)
                .children(chevron.map(|c| div().flex().flex_none().items_center().justify_center().w(px(LEAD)).child(c.svg(12., muted))))
                .child(div().flex().flex_1().min_w_0().items_center().child(content))
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Change {
        Added,
        Modified,
        Deleted,
    }

    #[derive(Default, Debug, PartialEq)]
    struct Badge {
        added: usize,
        modified: usize,
        deleted: usize,
    }

    fn count(badge: &mut Badge, change: &Change) {
        match change {
            Change::Added => badge.added += 1,
            Change::Modified => badge.modified += 1,
            Change::Deleted => badge.deleted += 1,
        }
    }

    fn entries() -> Vec<(String, Change)> {
        [
            ("src/main.rs", Change::Modified),
            ("src/ui/view.rs", Change::Added),
            ("README.md", Change::Modified),
            ("src/ui/src/lib.rs", Change::Deleted),
            ("docs\\guide.md", Change::Modified),
            ("src/ui/src/Mod.rs", Change::Modified),
        ]
        .into_iter()
        .map(|(p, c)| (p.to_string(), c))
        .collect()
    }

    fn state(cx: &mut App) -> TreeState<Change, Badge> {
        let state = TreeState::new(cx);
        state.set_entries(entries(), count);
        state
    }

    fn paths(state: &TreeState<Change, Badge>) -> Vec<String> {
        state.visible().iter().map(|p| p.to_string()).collect()
    }

    #[gpui::test]
    fn a_tree_is_built_from_flat_paths_and_expands_and_collapses(cx: &mut TestAppContext) {
        let state = cx.update(state);
        assert_eq!(paths(&state), ["docs", "src", "README.md"], "folders first, a backslash read as a slash");
        state.expand_all();
        assert_eq!(
            paths(&state),
            ["docs", "docs/guide.md", "src", "src/ui", "src/ui/src", "src/ui/src/lib.rs", "src/ui/src/Mod.rs", "src/ui/view.rs", "src/main.rs", "README.md"],
            "a file beside folders comes after them; src at two levels is two folders"
        );
        state.toggle("src/ui");
        assert_eq!(paths(&state)[2..], ["src", "src/ui", "src/main.rs", "README.md"]);
        state.collapse_all();
        assert_eq!(paths(&state), ["docs", "src", "README.md"]);
        state.toggle("src");
        state.toggle("src/ui");
        state.set_entries(entries().into_iter().filter(|(p, _)| p != "src/main.rs"), count);
        assert_eq!(paths(&state), ["docs", "src", "src/ui", "src/ui/src", "src/ui/view.rs", "README.md"], "a rebuild keeps the open folders");
    }

    #[gpui::test]
    fn a_filter_keeps_the_matching_files_with_their_folders_and_opens_them(cx: &mut TestAppContext) {
        let state = cx.update(state);
        state.set_filter("UI\\V");
        assert_eq!(paths(&state), ["src", "src/ui", "src/ui/view.rs"], "a substring of the whole path, ignoring case");
        state.set_filter("mod");
        assert_eq!(paths(&state), ["src", "src/ui", "src/ui/src", "src/ui/src/Mod.rs"]);
        state.set_filter("nothing");
        assert!(paths(&state).is_empty());
        state.set_filter("");
        assert_eq!(paths(&state)[..4], ["docs", "src", "src/ui", "src/ui/src"], "the folders the filter opened stay open");
    }

    #[test]
    fn a_folders_aggregate_covers_its_whole_subtree() {
        let tree = Tree::build(entries(), count, Order::Ascending);
        assert_eq!(tree.aggregate("src"), Some(&Badge { added: 1, modified: 2, deleted: 1 }));
        assert_eq!(tree.aggregate("src/ui"), Some(&Badge { added: 1, modified: 1, deleted: 1 }));
        assert_eq!(tree.aggregate("src/ui/src"), Some(&Badge { added: 0, modified: 1, deleted: 1 }));
        assert_eq!(tree.aggregate("docs"), Some(&Badge { added: 0, modified: 1, deleted: 0 }));
        assert_eq!(tree.aggregate("README.md"), None, "a file has none");
    }

    #[gpui::test]
    fn the_order_and_its_reverse_keep_folders_first(cx: &mut TestAppContext) {
        let state = cx.update(|cx| TreeState::<Change, Badge>::new(cx));
        let names = ["b.rs", "A.rs", "c.rs", "z/x.rs", "a/x.rs"];
        state.set_entries(names.iter().map(|n| (n.to_string(), Change::Added)), count);
        assert_eq!(paths(&state), ["a", "z", "A.rs", "b.rs", "c.rs"], "A to Z ignoring case");
        state.set_order(state.order().reversed());
        assert_eq!(paths(&state), ["z", "a", "c.rs", "b.rs", "A.rs"]);
        state.set_entries(names.iter().map(|n| (n.to_string(), Change::Added)), count);
        assert_eq!(paths(&state), ["z", "a", "c.rs", "b.rs", "A.rs"], "a rebuild keeps the order");
    }

    struct Files {
        state: TreeState<Change, Badge>,
        hits: HitTargets,
        painted: Vec<String>,
        selected: Vec<String>,
    }

    impl Render for Files {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let list = TreeList::new("t", &self.state, &self.hits, |this: &mut Files, row, _, _| {
                this.painted.push(row.path.to_string());
                div().child(row.name.clone()).into_any_element()
            })
            .on_select(|this, path, _, _| this.selected.push(path.to_string()));
            div().w(px(400.)).h(px(300.)).child(list.render(cx))
        }
    }

    fn files(cx: &mut TestAppContext) -> (Entity<Files>, &mut VisualTestContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (files, cx) = cx.add_window_view(|window, cx| {
            let state = TreeState::new(cx);
            state.set_entries((0..20_000).map(|i| (format!("dir{:03}/file{:03}.rs", i / 100, i % 100), Change::Modified)), count);
            window.focus(state.focus_handle(), cx);
            Files { state, hits: HitTargets::default(), painted: Vec::new(), selected: Vec::new() }
        });
        cx.run_until_parked();
        (files, cx)
    }

    fn redraw(files: &Entity<Files>, cx: &mut VisualTestContext) -> Vec<String> {
        files.update(cx, |f, cx| {
            f.painted.clear();
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|_, _| ());
        files.read_with(cx, |f, _| f.painted.clone())
    }

    #[gpui::test]
    fn only_the_rows_in_view_are_laid_out_and_the_arrows_walk_the_tree(cx: &mut TestAppContext) {
        let (files, cx) = files(cx);
        files.read_with(cx, |f, _| f.state.expand_all());
        let painted: HashSet<String> = redraw(&files, cx).into_iter().collect();
        assert_eq!(files.read_with(cx, |f, _| f.state.visible().len()), 20_200);
        assert!(!painted.is_empty() && painted.len() <= 14, "{} rows painted: {painted:?}", painted.len());
        files.read_with(cx, |f, _| assert!(f.hits.borrow().contains_key("t-row-dir000/file000.rs") && !f.hits.borrow().contains_key("t-row-dir100")));

        cx.simulate_keystrokes("down left");
        files.read_with(cx, |f, _| assert_eq!(f.state.visible()[..2], ["dir000", "dir001"], "Left closes the open folder"));
        cx.simulate_keystrokes("right right");
        files.read_with(cx, |f, _| {
            assert!(f.state.is_open("dir000"), "Right opens a closed folder");
            assert_eq!(f.state.selected().as_deref(), Some("dir000/file000.rs"), "then selects its first row");
        });
        cx.simulate_keystrokes("down down left");
        files.read_with(cx, |f, _| {
            assert_eq!(f.state.selected().as_deref(), Some("dir000"), "Left on a file selects its folder");
            assert_eq!(f.selected, ["dir000", "dir000/file000.rs", "dir000/file001.rs", "dir000/file002.rs", "dir000"]);
        });

        files.read_with(cx, |f, _| f.state.collapse_all());
        redraw(&files, cx);
        let hit = files.read_with(cx, |f, _| f.hits.borrow().get("t-row-dir001").copied()).expect("a row paints under its path");
        cx.simulate_click(hit.bounds.center(), Modifiers::none());
        files.read_with(cx, |f, _| {
            assert!(f.state.is_open("dir001"), "a click on a folder's row opens it");
            assert_eq!(f.state.selected().as_deref(), Some("dir001"));
        });
    }

    struct Names {
        state: TreeState<Change, Badge>,
        hits: HitTargets,
    }

    impl Render for Names {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let list = TreeList::new("n", &self.state, &self.hits, |this: &mut Names, row, _, _| {
                let name = div().relative().child(row.name.clone()).child(crate::hit::hit_target(&this.hits, format!("name-{}", row.path)));
                let lead = matches!(row.item, Item::File(_)).then(|| div().flex_none().w(px(LEAD)));
                div().flex().children(lead).child(name).into_any_element()
            });
            div().w(px(400.)).h(px(300.)).child(list.render(cx))
        }
    }

    #[gpui::test]
    fn a_files_name_starts_where_its_sibling_folders_names_do(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (names, cx) = cx.add_window_view(|_, cx| {
            let state = TreeState::new(cx);
            state.set_entries(entries(), count);
            state.expand_all();
            Names { state, hits: HitTargets::default() }
        });
        cx.run_until_parked();
        let x = |path: &str, cx: &mut VisualTestContext| names.read_with(cx, |n, _| n.hits.borrow().get(path).map(|h| h.bounds.left())).unwrap_or_else(|| panic!("{path} paints"));
        let top = x("name-src", cx);
        assert_eq!(x("name-README.md", cx), top, "a top-level file's name at the top-level folders' names");
        assert_eq!(x("name-docs", cx), top);
        let inner = x("name-src/ui", cx);
        assert_eq!(inner, top + px(14.), "one level in is one indent");
        assert_eq!(x("name-src/main.rs", cx), inner, "a file beside a folder at its name");
    }
}
