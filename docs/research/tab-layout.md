# Tab layout: tab groups and a split view (tabview/6)

René asked for Edge's coloured, collapsible tab groups and Edge's split screen, which shows two pages side by
side. This note says what each one solves in Supraman's right pane, how the two fit together and fit the tab code
that tabview/2–5 built, and what each one costs in gpui. §8 recommends a design and splits it into units.
**Nothing here is decided.** René approves or amends §8 and answers §9. The units then become Workman items.

Sources: `crates/supraman-ui/src/tabs.rs` (+ `tabs/fit.rs`, `tabs/list.rs`), `crates/supraman-ui/src/split.rs`;
in `crates/supraman-app/src`: `view.rs` (`Tab`, `TabKind`, `activate_tab`, `cycle_tab`, `close_tab`, `key_down`),
`view/right.rs`, `view/right/tab_menu.rs`, `view/tab_keys.rs`, `view/reopen.rs`, `view/drop.rs`, `view/git_tab.rs`,
`view/work_tab.rs`, `view/term.rs` (`chord`), `saved_tabs.rs`; `crates/supraman-git-view/src/git_view/refresh.rs`
(`set_shown`); `docs/KEYS.md`, `docs/ARCHITECTURE.md`.

## 1. The tabs as they are

| Part | Today | Where |
|---|---|---|
| Model | `FleetView::tabs: Vec<Tab>` in strip order and `active: Option<u64>`. A tab is one `TabKind`: Attach, Shell (plain or a hand claude), Work, Git, Schedule, Details, Start, or Saved (a Resume placeholder) | `view.rs:140`, `:161`, `:269–270` |
| Strip | `tab_strip` is one header per tab. Tabs share the width evenly between `TAB_MIN` 120 and `TAB_MAX` 240 px; below that the strip scrolls sideways. The ⌄ button lists every tab in the shared palette. The scroll and the list are kept under the `STRIP` keyed state, one strip per window | `tabs.rs:160`, `:171`; `tabs/fit.rs` |
| Content | `render_tab_content` paints only the active tab: a page entity, or a terminal grid with its find bar and drop layer | `view/right.rs:79` |
| One per thing | A Git tab per repo folder (`open_or_focus`) and a Work tab per workspace (`open_work`). A new one is pushed last and made active | `view/git_tab.rs:15`, `view/work_tab.rs:12` |
| Keys | `CHORDS` are bound on `FleetView`'s context ahead of `key_down`: Ctrl+Tab/PgUp/PgDn, Ctrl+1..9, Ctrl+Shift+PgUp/PgDn to move a tab, Ctrl+T/W (passed on to a terminal under `outside_terminals`), Ctrl+Shift+T/W, Ctrl+Alt+T. `key_down` sends every other key to the active tab's terminal | `view/tab_keys.rs:31`, `view.rs:2400`, `:2426` |
| Menu | `tab_items`: Open Git, Open Workman, Copy folder path, Open folder, Close, Close other tabs, Close tabs to the right, Reopen closed tab, then a session tab's row items | `view/right/tab_menu.rs:51` |
| Drag | A tab drags as a `DraggedTab`. Dropped on a header, the tab moves there (`on_move`). Dropped on a terminal, the session's SendMessage name is pasted into it (`terminal_drop` covers the whole terminal) | `view/drop.rs:46` |
| Saved | `SavedTabs { tabs: Vec<SavedTab>, active }` in `<home>/tabs.json`. Schedule, Details and Start tabs are not saved. At start each saved tab goes through `plan` → `Reopen` or a reason; tabs that fail are reported in one notice | `saved_tabs.rs:104`, `:146`; `view/reopen.rs:50`, `:131` |
| Closed | The last 20 `SavedTab`s go on a stack. Ctrl+Alt+T plans the newest one again and pushes it last | `view/tab_keys.rs:66`, `view/reopen.rs:167` |
| Shown | `show_git_tabs` tells each Git tab whether it is `active`. A hidden Git tab holds its live refreshes as `pending` and runs them when it is shown again. `is_marked` highlights the sidebar row of the session the active tab runs | `view/git_tab.rs:29`, `refresh.rs:147`, `view.rs:1997` |

Size: `view.rs` has 2,893 lines, about 2,650 of them outside tests. C144(6) says it must not grow, so any new state
or method lives elsewhere (§6).

## 2. What each one solves here

Three situations come up again and again:

- **S1, one task.** A claude session tab works in repo X. René watches the agent and that repo's Git tab (the
  agent's row, the WIP) at the same time, and now and then the workspace's Work tab (the run and its criteria).
- **S2, several sessions across workspaces.** Two or three workspaces, each with a few session tabs plus its Git
  and Work tabs, all interleaved in one strip in the order they were opened.
- **S3, 30+ tabs.** The strip scrolls (tabview/2). Each title is cut to 120 px, and the ⌄ list is the way to find
  a tab.

| Need | Today | Groups | Split |
|---|---|---|---|
| S1: see the agent and its graph at once | not possible: Ctrl+Tab back and forth, and the Git tab holds its refreshes while hidden | no: still one page shown | **yes**: the session on one side, the Git tab on the other, both live |
| S1: keep the task's three tabs together | by hand (drag, Ctrl+Shift+PgUp/PgDn). Open Git and Open Workman push the new tab last, far from its source | **yes**: Open Git from a grouped tab opens it in that group, next to its source | partly: two of the three |
| S2: tell which workspace a tab belongs to | the title prefix `ws · repo · name` (cut at 120 px) and the hover card | **yes**: a colour per group and a named chip before its tabs | no |
| S2: put a workspace away for now | close its tabs (Ctrl+Alt+T brings back one at a time) | **yes**: collapse the group to its chip. The tabs keep running and stay in the saved file | no |
| S3: fewer headers to scroll past | the ⌄ list | **yes**: a collapsed group takes one chip's width | each pair still has two headers (§4.1) |
| S3: find a tab | the ⌄ list filters by title | the list can show and filter by group name | no |

In short: **the split solves S1**, the case René meets daily. **Groups solve S2 and S3**, which grow with the
number of tabs. Neither one replaces the other.

## 3. Edge's behaviour, as a reference

Edge's behaviour is described here in our own words, from Edge's current desktop builds as generally known. Edge
was not checked again for this note, so a detail can differ.

**Tab groups.** A right-click on a tab adds it to a new group or to an existing one. A group has a short name,
which may be empty, and a colour from a small fixed set. Its chip sits before its tabs, and the group's tabs carry
a coloured line. A click on the chip collapses the group to the chip alone; another click expands it. A tab opened
from a grouped tab joins that group. A tab dragged into a group's span joins it, and dragged out it leaves. The chip
drags the whole group. The chip's menu names the group, recolours it, adds a new tab to it, ungroups it, closes it,
or moves it to a new window. Groups survive a restart along with the session. Reopening a closed group brings back
all of its tabs.

**Split screen.** Two pages show side by side in one tab slot, with a divider between them that drags. One side
has the focus and is marked. Each side navigates on its own, and a link can open in the other side. The pair shows
as one entry in the strip with both pages' icons. A split is started from a toolbar button or a tab's right-click,
and the user picks the other page from the open tabs. Closing one side leaves the other as a plain tab. There are
exactly two sides.

**What we take and what we leave.** We take the chip, the colours, collapse, "a tab opened from a grouped tab
joins its group", the drag rules, and a focused side that is marked. We leave moving a group to a new window
(Supraman has one window of tabs), "a link opens in the other side" (no tab here navigates), and the pair as a
single strip entry (§4.1).

## 4. Design options

### 4.1 The split: where it lives

| Option | Shape | Keys and `active` | Cost | Verdict |
|---|---|---|---|---|
| **A. A pair of neighbouring tabs** | Two tabs next to each other in `tabs` are marked as a pair. Their headers are drawn joined (one outline, a thin seam). Activating either one shows both, split by a `SplitPane`. The tab that has the focus is `active` | unchanged: `active` is still one tab id. Ctrl+Tab from the left side reaches the right side, which simply moves the focus across. Ctrl+1..9 still counts tabs | small: one id pair and a ratio per pair. The strip only draws the join | **recommended** |
| B. Edge's single entry | A pair is one strip entry holding two tabs | `active` becomes "entry + side". Ctrl+1..9, the ⌄ list, Close, the menu and `DraggedTab` all have to learn about entries | large: every place that indexes `tabs` changes | no: A gives the same view with none of that |
| C. Window-wide panes | The right pane splits into two panes, each with its own strip and active tab (as VS Code's editor groups) | two `active`s, two strips (`STRIP` keyed state per pane), and a drop target per pane | largest: a second strip, moving tabs between panes, a per-pane saved layout | no: S1 needs one pair, not two workspaces of tabs |

Rules for A:
- A tab is in at most one pair, and a pair's tabs are always next to each other. Moving or dragging one side moves
  the pair. Dragging a side out of the pair ends the pair.
- Closing one side ends the pair. The other side stays as a plain, active tab.
- A tab is shown at most once. The same `Entity<Terminal>` or `GitView` is never painted twice, so "show this tab
  in both sides" cannot happen.
- Below `2 × min` width (`SplitPane`'s `min_first`/`min_second`) only the focused side shows. The pair comes back
  when the window is wide enough again.
- Side by side only. A split above and below is §9 Q3.

### 4.2 Groups: who makes them

| Option | What happens | For | Against |
|---|---|---|---|
| G-auto. One group per workspace, always | every tab whose folder is in a workspace (`tab_card::place`) is in its group. A plain shell outside any workspace is ungrouped | nothing to do. The colour is stable per workspace | no user control. A shell `cd`'d elsewhere switches group. A task spanning two workspaces cannot be one group |
| **G-user. User-made, Edge's rules, with a workspace helper** | groups are made from the tab menu. A tab opened from a grouped tab joins that group. *Group tabs by workspace* (the strip's ⌄ menu) makes or fills one group per workspace, coloured by the workspace's name | René's mental model of Edge. S1 can be one group across workspaces. The helper covers G-auto's main case on demand | one click for René to get G-auto's result |
| G-session. A group per claude session (its tab, its Git, its Work) | opening Git or Workman from a session tab groups them | S1 exactly | many groups of 2–3 tabs. A Git tab shared by two sessions has no single group |

**Recommended: G-user.** Rules:
- A group's tabs are always next to each other in `tabs`. A group has a name (may be empty), one of the ten
  colours, and a collapsed flag.
- The colours are `themes::lanes`' ten, already contrast-fitted, refitted on `tab_bar_background` by the same
  `nearest_holding` search. *Group tabs by workspace* picks a colour by a stable hash of the workspace's name.
- A collapsed group shows only its chip. Collapsing the group that holds the active tab activates the nearest tab
  outside it (Edge does the same), or nothing when there is none. A collapsed group's tabs keep running, and a Git
  tab among them is hidden (`set_shown(false)`).
- A pair (A) lies inside one group or outside every group. Joining a group takes the pair along.

### 4.3 How they combine with Open Git and Open Workman (tabview/4–5)

| Item | Today | With groups | With the split |
|---|---|---|---|
| Open Git | focuses the repo's Git tab, or opens it last | a new Git tab opens **right after the source tab, in its group**. An existing one is focused where it is, never moved | new item **Open Git beside**: pair the source tab with the repo's Git tab (opened right after the source, or moved next to it) |
| Open Workman | the Work tab of the bound workspace, on its first active run | same rule as Open Git | **Open Workman beside**, same rule |
| new: Show beside ▸ | — | — | a submenu of the open tabs (most recently used first); the pick moves next to this tab and pairs with it |
| new: End side by side | — | — | on a paired tab only. The other side stays where it is |
| new: Add to new group, Add to group ▸, Remove from group | — | the tab (with its pair) joins or leaves. A new group opens its name field on the chip | — |
| chip menu: Name and colour, Collapse/Expand, New tab in group, Ungroup, Close group | — | Close group closes every tab through `close_tab` (detach included), pushes one group entry for Ctrl+Alt+T, and asks first (`Confirm`) when more than one tab runs claude | — |

"Beside" moves an existing tab next to its source, because a pair's tabs must be neighbours. A Git tab that is
already in another pair leaves it first, and that pair's other tab stays as a plain tab. The menu items are dimmed
with a reason in each case: "Already beside this tab", "This tab is the Git tab", or (while reading) "Looking for
its repository…".

### 4.4 Dragging (`DraggedTab`)

| Drop | Today | Proposed |
|---|---|---|
| on a header | move there | move there. Between two tabs of a group it joins the group; outside every group it leaves it. A side of a pair dropped away from its partner ends the pair |
| on a group chip | — | join the group at its end. Dropped on a collapsed chip, the group stays collapsed |
| a chip dragged (new payload `DraggedGroup { id }`) | — | the group moves as one block, landing only between groups or ungrouped tabs |
| on a terminal | pastes the dragged tab's SendMessage name | unchanged across the terminal |
| on the content's right edge band (new, only while a `DraggedTab` is over it) | — | show the dragged tab beside the active tab. The band is about 56 px wide on the content's right edge, painted over the terminal's paste layer and taking the drop first |

The edge band is the one real clash with today's behaviour: the paste layer covers the whole terminal. A narrow
band on the right edge keeps the paste everywhere else. Whether the band is wanted at all is §9 Q6. The menu and
the chord do the same job without it.

## 5. Persistence

`SavedTabs` gains two optional lists. Indices count into `tabs` as saved, after the unsaved kinds (Schedule,
Details, Start) are left out.

```rust
pub struct SavedTabs {
    pub tabs: Vec<SavedTab>,
    pub active: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<SavedGroup>,   // { name, colour: u8, collapsed, first: usize, len: usize }
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pairs: Vec<SavedPair>,     // { left: usize, ratio: f32 } — the right side is left + 1
}
```

| Case | What reopens |
|---|---|
| an old file (no `groups`, no `pairs`) | the flat strip, exactly as today: `serde(default)` |
| a new file read by an older Supraman | the flat strip: the unknown fields are ignored (`SavedTabs` does not deny them), so going back a version is safe |
| a group whose tabs partly fail `plan` | the group with the tabs that reopened, in order. A group left empty is dropped. The failed tabs are in the one notice, as today |
| a pair where one side fails | the other side as a plain tab |
| a pair where a side was not saved (a Details tab beside a terminal) | the saved side as a plain tab |
| a pair inside a group | the pair stays inside it: the saved `first..first+len` covers both sides |
| tabs opened while the saved ones load | they come after, ungrouped, as today (`apply_reopen` puts the reopened tabs first) |
| the active tab in a collapsed group | the group reopens expanded. A file never holds that state, because collapsing moves `active` out |

**Ctrl+Alt+T's stack.** Today it holds `SavedTab`s. The stack becomes `Closed::Tab { saved, group, at }` or
`Closed::Group { group, tabs }`:
- A tab comes back into its group if that group is still open (matched by an id kept for this run), next to where
  it was. Otherwise it comes back ungrouped, last, as today.
- A closed group comes back whole, as one step (Edge does the same), with its name, colour and its tabs in order.
  Tabs that cannot come back are named in one toast.
- A pair does not come back as a pair. Each side is its own `Closed::Tab`. Pairs are cheap to make again, and a
  closed side is often the one that was in the way.

`saved_tabs::plan` stays per tab. New code in `saved_tabs.rs` maps the old indices to the reopened ids (a pure
function, unit-tested over failures at the start, middle and end of a group and of a pair).

## 6. Keyboard

### 6.1 The chords that exist, with groups and a split

| Chord | Today | Proposed |
|---|---|---|
| Ctrl+Tab, Ctrl+PgDn / Ctrl+Shift+Tab, Ctrl+PgUp | next / previous tab, wrapping | the same over the **shown** tabs: a collapsed group's tabs are skipped. Inside a pair, this moves the focus to the other side, since both are shown |
| Ctrl+1..8, Ctrl+9 | the nth tab, Ctrl+9 the last | the nth shown tab (collapsed groups skipped), Ctrl+9 the last shown |
| Ctrl+Shift+PgUp / PgDn | move the active tab one place | the same, plus: at a group's edge the first press takes the tab out of its group without moving it, and the next press moves it. Moving into a group's edge joins the group the same way. A side of a pair moves the pair. Pure: `tabs::moved_grouped` |
| Ctrl+T / Ctrl+W | new task tab / close the active tab | the new tab opens after the active tab, in its group. Ctrl+W closes the focused side of a pair, and the other side stays |
| Ctrl+Shift+T / Ctrl+Shift+W | the same, in every tab | the same |
| Ctrl+Alt+T | reopen the last closed tab | reopen the last closed tab or group (§5) |
| the terminal's chords (`term.rs:chord`) | Ctrl+C/V/Insert, Ctrl+=/-/0, Ctrl+Shift+C/V/F/=/Up/Down/Home/End/PgUp/PgDn, Shift+Insert | unchanged. They go to the focused side's terminal, because `key_down` routes to `active` (`view.rs:2426`) and `active` is the focused side |

Ctrl+Shift+PgUp/PgDn is in both `CHORDS` (move the tab) and the terminal's `chord` (scroll a page). `CHORDS` is
matched first, so moving wins, as KEYS.md already says. Nothing here changes that.

### 6.2 New chords

A Ctrl+letter belongs to the program in a terminal tab (claude, PSReadLine), so the new chords use Ctrl+Alt, as
Ctrl+Alt+T does. On René's German and Italian layouts Ctrl+Alt is AltGr, and AltGr types characters on
Q E M 2 3 7 8 9 0 ß + < (German) and E è ò à + (Italian). The letters below type nothing there.

| Chord | Does |
|---|---|
| Ctrl+Alt+S | show the previously active tab (most recently used) beside the active tab. On a paired tab, end the pair |
| Ctrl+Alt+G | add the active tab to a new group and open its name field. On a grouped tab, collapse or expand its group (collapsing moves the focus out, §4.2) |

Nothing else is needed: Ctrl+Tab already crosses a pair (§6.1), and a group is entered from the ⌄ list or with
Ctrl+1..9. Swapping a pair's sides and resizing them stay mouse-only (the menu and the divider). The divider's
double-click resets 50/50, as every `SplitPane` does. Both chords go in `CHORDS`, bound ahead of `key_down`, and
never go to a terminal. KEYS.md lists them.

## 7. gpui cost and where the state lives

### 7.1 What exists, what is new

| Piece | Exists | New |
|---|---|---|
| side-by-side panes with a drag | `split::SplitPane` (min widths, a default on double-click; the owner keeps the ratio) | the ratio per pair, kept as a fraction so it survives a window resize |
| strip state | `STRIP` keyed state (scroll, room, ⌄ list) | none. The strip stays one per window |
| strip widths | `fit::tab_width`, `span`, `revealed` assume equal widths | chips (their name's width, up to 120 px) and a collapsed group's zero-width tabs. `span` becomes an offsets table (a prefix sum) built once per paint. `revealed` stays the same over it |
| headers | `TabHeader { id, label, title, tooltip }` | `TabHeader::group: Option<(colour, first, last)>` for the coloured underline and rounded ends, and `joined: bool` for a pair's seam. The chips are a second kind of strip item, `StripItem::Chip { id, name, colour, collapsed, count }` |
| ⌄ list | `tabs/list.rs` over `Palette` | `TabEntry::group` adds a dimmed group name before the title, and filtering covers it. A collapsed group's tabs are listed, and picking one expands that group |
| drag payloads | `DraggedTab`, `DraggedSession`, `chip` | `DraggedGroup`; the edge band's drop target (§4.4) |
| menus | `ContextMenu`, `tab_items` | the items of §4.3, and the chip's own menu through the same `ContextMenu` |
| colours | `themes::lanes`, `nearest_holding` | refitting on `tab_bar_background`, and a stable hash from workspace to colour |
| long lists | `KeyedList` | not needed: the strip is a row, and the ⌄ list is already the palette |

### 7.2 Two contents at once

`render_tab_content` paints one tab today. A pair paints two, each in a `SplitPane` side.

| Content | Cost of being shown beside another tab | What has to change |
|---|---|---|
| Terminal | `terminal_grid` sizes its PTY to its bounds, so pairing a terminal resizes its ConPTY and claude redraws at about half the width. At 1920 px with the sidebar that is about 750 px, roughly 90 columns at the default font, which claude's TUI handles. Unpairing resizes it back. Both grids paint each frame, but only the focused one blinks a cursor | focus follows a click: a mouse-down on either side sets `active` (and with it the key routing) before the event goes on. The focused side gets a 2 px `border_focused` outline. The find bar, the detach banner and the schedule question stay per tab |
| Git tab | `show_git_tabs` treats only `active` as shown, so an unfocused Git side would hold its refreshes, which defeats S1. The graph, ref panel and commit panel need width: their own `SplitPane` minimums add up to roughly 900 px with every panel open | **shown** means "in the active pair or active", not "active". `background::shown` (the fetch on activation) fires when a side becomes visible. Before the split unit merges, a criterion checks the Git tab at 640 px: every hit key of the graph and toolbar is fully visible, or the ref panel closes to its header |
| Work tab | three columns (navigator, list, detail) | the same 640 px check. `escape` goes to the focused side only |
| Schedule, Details, Start, Saved pages | `details::Page`, which already wraps | nothing |
| `is_marked` (the sidebar's mark) | the active tab's session | both shown sides' sessions are marked, and the focused one with the focused border (§9 Q7) |
| `Occupancy`, the agent rows | per session, not per tab | nothing |
| hit keys | `terminal-<id>`, `tab-<id>`, … are per tab | the selftests see both sides. New keys: `split-<left id>` for the divider, `tab-group-<id>` for a chip, `tab-group-<id>-menu`, `split-edge` for the drop band |
| `pane_toast` | one over the content | it stays one, over the whole content |

### 7.3 Where the new state lives (C144(6))

`view.rs` is far over 1,500 lines and must not grow. The plan:
- **First, a move with no change in behaviour (unit L0):** `Tab`, `TabKind`, `Detach`, `tab`/`tab_mut`,
  `activate_tab`, `cycle_tab`, `tab_runs`, `is_marked`, `session_tab` and the tab's constructor move from
  `view.rs` into `view/tabs.rs` (about 150 lines out). That makes room for the one field below.
- **The arithmetic** goes in `supraman_ui::tabs::layout` (a new file next to `fit.rs`): `Layout { groups,
  pairs }` keyed by tab id, with the rules as pure functions: `normalize` (groups next to each other, pairs inside
  one group), `moved_grouped`, `shown` (the ids a collapse hides), `cycled_shown`, `nth_shown`,
  `reorder_with_layout` (a pair or a group moves as one), `closed` (what a close leaves). Unit tests cover all of
  them. It knows nothing of `Tab`, so the strip and the app share it.
- **The app's state** goes on its own entity, `view/layout.rs` `TabLayout` (the `Layout`, the pair ratios, the
  MRU list for Ctrl+Alt+S and "Show beside ▸", group names and colours). `FleetView` holds one
  `Entity<TabLayout>` field. The strip, `tab_keys.rs`, `tab_menu.rs` and `reopen.rs` read it.
  `render_tab_content` moves from `view/right.rs` into `view/right/content.rs`, so the pair rendering has a file
  of its own.
- `saved_tabs.rs` (330 lines) takes `SavedGroup`, `SavedPair` and the index mapping.

## 8. Recommendation

**Build both, the split first.** The split solves S1, the situation René is in many times a day: watching an agent
next to its graph. It needs only a pair of ids and one `SplitPane`. Groups pay off as the tab count grows (S2,
S3), where tabview/2's overflow and the ⌄ list already work. Groups are the larger change to the strip: chips,
variable widths, a new drag payload.

Design: **A. a pair of neighbouring tabs** for the split (§4.1), and **G-user: user-made groups** with *Group tabs
by workspace* and "a tab opened from a grouped tab joins its group" (§4.2). The two combine as §4.3–§6 say.

### Units

| Unit | Outcome | Criteria | Proof |
|---|---|---|---|
| **L0** tabs out of view.rs | `Tab`, `TabKind` and the tab helpers live in `view/tabs.rs`, and `view.rs` shrinks by about 150 lines | no change in behaviour. `view.rs` loses at least 100 non-test lines | the existing tests and `--selftest`; a `wc` in the report |
| **L1** split: pair and show | a tab shows beside another: the menu's *Show beside ▸*, *Open Git beside*, *Open Workman beside*, *End side by side*; Ctrl+Alt+S; the divider; focus follows a click; the focused side is outlined; an unfocused Git side stays live; Ctrl+Tab crosses a pair; closing a side ends the pair | `tabs::layout` pair rules (neighbours, move as one, close leaves the other, a tab in at most one pair). Keys reach the focused side's terminal. A Git tab on the unfocused side applies a live refresh. Below `2 × min` only the focused side shows | unit tests for `layout` (pairs) and `tab_items` (the new items, dimmed with their reasons). **One selftest step**: a terminal tab beside a Git tab; click each side; typed text reaches the clicked side's terminal; both sides' hit keys are fully visible; a commit made outside shows in the Git side without activating it. A unit test cannot do this step, because it needs focus, layout and the watcher in a real window |
| **L2** split: strip, persistence, narrow pages | the pair's headers drawn joined; `SavedPair` saved and reopened; the Git and Work tabs usable at 640 px | an old file reopens flat. A pair with a failed side reopens the other side alone. Every hit key of the Git toolbar and graph is visible at 640 px | unit tests: `saved_tabs` round trip with pairs and the index mapping with failures. Selftest: the Git tab at 640 px (the existing hit-visibility check, run at that width) |
| **G1** groups: model, strip, menu | groups are made, named, coloured, collapsed and expanded, ungrouped and closed from the menus and the chip; the chip's colour on the tabs' underline; a tab opened from a grouped tab (Ctrl+T, Open Git, Open Workman) joins its group, next to its source; Ctrl+Alt+G; Ctrl+Tab, Ctrl+1..9 and Ctrl+Shift+PgUp/PgDn as §6.1 | `tabs::layout` group rules (next to each other, `moved_grouped`, `cycled_shown`, `nth_shown`, collapsing moves `active` out). The strip's offsets table reveals the active tab with chips present. The colours hold their contrast floors on `tab_bar_background` in the dark and light themes | unit tests for `layout`, `fit`'s offsets and the colour fit. Selftest: make a group of three, collapse it by its chip, check the strip shows the chip and not the tabs (hit keys) and that Ctrl+Tab skips them; expand it again |
| **G2** groups: persistence, Ctrl+Alt+T, workspace helper, ⌄ list | `SavedGroup` saved and reopened; the `Closed` stack (§5); *Group tabs by workspace*; group names in the ⌄ list | an old file reopens flat. A group with failed tabs reopens with the rest. A closed group comes back whole in one Ctrl+Alt+T. The helper gives each workspace one group with a stable colour, and leaves tabs outside any workspace alone | unit tests: round trip, index mapping, the `Closed` stack, the helper over `tab_card::place` |
| **G3** dragging | drag into and out of groups, chip drags (`DraggedGroup`), the content's right-edge band to show beside | §4.4's table. The terminal's paste drop still works outside the band | unit tests for the drop rules (a pure `dropped`-style function, as in `view/drop.rs`). Selftest: drag a tab onto a chip, and onto the edge band. Only if §9 Q6 says yes |

Each unit updates `docs/KEYS.md` and the `supraman_ui::tabs` entry in `CLAUDE.md` for what it adds. L0 comes before
L1, and L1 before L2. G1 can start after L1, and G2 and G3 follow G1.

## 9. Questions for René

1. **The split's shape.** Should a pair be two neighbouring tabs drawn joined (A, recommended), or one strip entry
   as in Edge (B)? Window-wide panes with two strips (C) are left out.
2. **Who makes groups.** User-made groups with a *Group tabs by workspace* command (recommended), or always one
   group per workspace without asking?
3. **Above and below.** Side by side only (recommended for now), or a stacked split too, for a wide terminal over
   the Git graph?
4. **Chords.** Are Ctrl+Alt+S (beside the last tab, again to end) and Ctrl+Alt+G (new group, then collapse or
   expand) right, or do you want other keys?
5. **Ctrl+Alt+T and groups.** Should a closed group come back whole in one press (recommended, as Edge does), and
   a pair come back as two plain tabs?
6. **Dragging into a split.** Is the right-edge drop band (G3) wanted, or are the menu and the chord enough?
7. **The sidebar's mark.** Should both shown sides' sessions be marked (recommended), or only the focused one?
8. **The order.** Split first (L0–L2), then groups (G1–G3), or groups first because the 30+ tab case hurts more
   today?
