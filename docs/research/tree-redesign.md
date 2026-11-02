# Tree redesign: proposal (m5/57)

The left tree's row kinds as the code draws them today, three alternative designs that fit the folders model
(m5/55, m5/56), and a recommendation. **Nothing here is decided**; René approves or amends the choice in §4, and the
implementation becomes a follow-up item.

Sources: `crates/foreman-app/src/view/left.rs` (tree, Inbox, Messages, notices, compose, Add workspace),
`view/menu.rs` (menus), `view/manage.rs` `menu_for`, `fleet.rs` (`build`), `search.rs` and `view/results.rs`,
`view/notices.rs`, `supervise.rs` (`RelayStatus`), `foreman-ui` `button`, `menu`, `hover_card`, `banner`, `notices`.
Keys used by the selftests were counted in `crates/foreman-app/src/selftest*.rs`.

## 1. Inventory

The left panel, top to bottom: action bar · Inbox · Messages · notices · search box · tree (or search results) ·
toast. Only the Inbox and Messages hold still under a moving pointer (m5/18); tree rows do not.

### 1.1 Action bar (`render_action_bar`)

| Shows | Actions | Keys |
|---|---|---|
| icons: add workspace, expand all, collapse all, show/hide stopped (with the hidden count), refresh; right: CLI-untested warning, `busy ≤ n` stepper, settings | click each; tooltips | `add-workspace`, `expand-all`, `collapse-all`, `toggle-stopped`, `refresh`, `cli-warning`, `busy-dec`, `busy-value`, `busy-inc`, `bar-settings` |

All are `button::toolbar` with an `Icon` and a `Tooltip`. The tree reuses nothing from it.

### 1.2 Workspace row (`render_left`, 30 px)

| | |
|---|---|
| Shows | chevron · **name** (bold) · `Work` button · root path (clipped) · `N sessions` · `started`/`stopped` (green/secondary, empty with no desired state) · relay label when started or live: `relay starting` / `relay ready` / `direct` / `relay watcher expired, not re-armed (armed n min ago)` (green or warning) · `Task…` (started and the relay accepts) · `Start`/`Stop` |
| Click | toggles open/closed (`collapsed` key `workspace:<ws>`) |
| Double-click | nothing |
| Right-click menu | `start`/`stop` · `work` · `task` (if it can) · `rescan` · `open-folder` · `open-terminal` · `delete-closed` (if any) · `remove`; title = workspace name |
| Hover | row background only; no hover card |
| Keys | `ws-row-<ws>`, `work-<ws>`, `task-<ws>`, `start-<ws>`/`stop-<ws>`, `ws-root-<ws>` and `relay-<ws>` (clipped labels) |
| Below it | the compose box (`compose`, `compose-box`, `compose-text`, `compose-send`, `compose-cancel`) when `Task…` is open |

Three bordered buttons and four text labels share one 30 px line; at the default panel width the root path and the
relay label are the ones that get clipped.

### 1.3 Supervised rows (the workspace session, the relay, wanted repo sessions)

`fleet::build` puts every session whose name is in the workspace's desired `wanted` list into
`WorkspaceNode::supervised`, drawn right under the workspace row at 26 px indent, before the repos. That list holds:

- `fm-<ws>-workspace`, the workspace session (inserted by Start, `supervise.rs` `start_workspace`);
- `fm-<ws>-relay`, the relay, only while it is wanted (demand-started by `sync_relay`, retired when idle);
- `fm-<ws>-<repo>` for each repo `fm want` added.

They are ordinary session rows (§1.5) with `repo = "supervised"`: same dot, state, name, kind · id, star, pills and
menu. Nothing marks which one is the workspace session or the relay except the name suffix. The relay's state is
also on the workspace row as its label (§1.2), so the relay shows twice while it runs.

### 1.4 Folder/repo row (26 px indent 26)

| | |
|---|---|
| Shows | chevron (only with sessions) · `./name` or `name (root)` · `(branch)` · session count (with sessions) · `disabled` tag (disabled, row dimmed); quiet text when it has no session · `+` (enabled) |
| Click | toggles (`repo:<ws>/<repo>`) |
| Right-click menu | `new-claude` (enabled) · `open-terminal` · `open-folder` · `disable`/`enable`; title `<ws> · ./<repo>` |
| Hover | background; `+` tooltip "New claude session in this repo" |
| Keys | `repo-row-<ws>/<repo>`, `new-claude-<ws>/<repo>`, `branch-<ws>/<repo>` |
| Below it | its sessions at indent 46, then `N more closed…` (`more-<ws>/<repo>`, click shows all) |

Disabled repos are listed last with no sessions. m5/56 c5 removes them from the tree.

### 1.5 Session row (24 px line, indent 26 or 46)

| | |
|---|---|
| Shows | health dot · state (78 px: `running`, `idle`, `blocked`, `stopped`, `process gone`, `unknown`, `closed`) · name (or `(unnamed)`, search marks highlighted) · `kind · shortid` · ☆/★ · pills: `resume` (not attachable, resumable), `tab` (open in a tab other than the active one), `attach` (live background) · second lines: `another Foreman` tag (foreign `fm-` row, tooltip), `waiting for: …` (warning), `hook: <last event> n ago · compacted …` (`fm-` rows only) |
| States | blocked: warning tint 0.12/0.18; stopped, process gone and foreign: dimmed; **selected** (the active tab runs it): `ghost_element_selected` and a 2 px focused left border |
| Click | nothing in the tree; in search results, opens its tab or attaches (`open_result`) |
| Double-click | nothing |
| Right-click menu | `attach` or `resume` · `stop-session` (`Stop` for `fm-`, `Stop…` otherwise) · `copy-send-name` (when the peer registry has it) · `copy-name` · `copy-id` · `show-in-work` (claimed item) · `star`/Unstar · `delete` (stopped, not in a tab, not wanted) |
| Hover | background; star tooltip; the foreign tag's tooltip |
| Keys | `session-row-<sessionId>`, `marked-<sessionId>` (selected), `session-name-<id>`, `session-id-<id>`, `star-<sessionId>`, `resume-<id>`, `tab-link-<id>`, `attach-<id>`, `foreign-<sessionId>`, `waiting-<id>`, `hook-<id>` |

Worktrees have **no row of their own**: a session whose cwd is a linked worktree is placed under its main repo
(`main_repos`, `assign_with`) and looks like any other session there; nothing on the row names the worktree.

### 1.6 Inbox (`render_inbox`)

Header `INBOX · n` (`· k new, shown when the pointer leaves` while held). Rows:
- **needs-you** (supervisor `NeedsYou`): kind label in its colour (`blocked`, `trust needed`, `gave up`, `launch failed`,
  `duplicate`, `token expired`, `other server`, `stop failed`) · `<ws> · <name>` · age · detail; one remedy button
  (`Attach`, `Retry`, `Run in terminal`, `Re-mint`, `Restart as administrator`), key `inbox-<verb>-<name>`. No click on the row.
- **Workman rows**: `Workman · <kind>` · `<ws> · <ref>` · since · what · the MCP call (mono, never run, C21);
  click opens the item in the Work tab; × dismisses locally. Keys `work:<ws>/<key>` (row), `work_view::dismiss_key`.

Area key `inbox-area`.

### 1.7 Messages (`render_messages`)

Header `MESSAGES · n`. Each outbox message: sender (bold) · workspace · age · body (`MESSAGE_LINES`, tables in mono)
· ask state (`Approve` + `Reply…`, `Reply…`, `answered: …`, `superseded…`); `Attach` when the sender is a live
attachable session; ×. Click expands/collapses. Keys `msg-<ws>/<file>`, `msg-attach-…`, `msg-approve-…`,
`msg-reply-…`, `msg-answered-…`, `msg-dismiss-…`, `msg-table-…`, area `messages-area`.

### 1.8 Notices

Banners above the search box: CLI failed, registry error, skipped workspace, stale session list (no ×); plain
notices with × (`notice-dismiss-<setup|supervise|hand|inbox|action>-<n>`, `foreman_ui::notices`); the Workman
notice (`ws-setup-notice`, `ws-setup-signin`/`-cancel`/`-retry`/`-settings`/`-dismiss`); the start warning;
"Removing <ws>…"; the confirm (`confirm`); the Add workspace review (`add-review`, `add-name`, `add-repo-<name>`,
`add-save`, `add-cancel`); "No workspaces registered". The toast floats at the panel's bottom (`toast`).

### 1.9 Search results (`render_results`)

Same nesting with `result-ws-<ws>`, `result-repo-<ws>/<repo>` (click reveals and clears the search) and
`result-session-<sessionId>` wrapping the normal session row (click opens/attaches).

### 1.10 What the selftests lean on

| Key | Where | Note |
|---|---|---|
| `ws-row-<ws>` | `selftest_manage`, `_left`, `_search` | right-click; 7f measures its top against the Inbox seam |
| `start-<ws>`, `stop-<ws>` | `selftest_supervise`, `_manage`, `_left` | 11 clicks |
| `task-<ws>` | `selftest_supervise` t0 | the hold test aims the pointer at it while Messages grows |
| `work-<ws>` | `selftest_work` | |
| `repo-row-<ws>/<repo>`, `new-claude-<ws>/<repo>` | `selftest_manage`, `_left` | |
| `session-row-<sessionId>` | 5 files | right-click → `menu-…` |
| `attach-<id>`, `resume-<id>`, `star-<sessionId>`, `tab-link-<id>`, `foreign-…`, `marked-…` | `selftest.rs`, `_closed`, `_refresh`, `_mark`, `_left` | |
| `WorkspaceNode::supervised` (not a key) | `selftest_supervise.rs:297`, step g, `selftest_delete.rs:49` | read straight from `tree()` |
| menu key lists | `selftest_manage` | asserts the workspace menu is **exactly** `[start, work, rescan, open-folder, open-terminal, remove]` and the disabled repo's is `[open-terminal, open-folder, enable]` |

A hit key exists only while its element paints, and two painted elements with one key keep the last one's bounds.

## 2. What every alternative must handle (from m5/55, m5/56)

- **A session under two workspaces.** The same session paints once per workspace that contains its cwd. Today's
  element ids (`session-<id>`) and hit keys (`session-row-<sessionId>`, `star-…`, `attach-…`) assume one row per
  session. Every alternative scopes the second and later occurrences: the element id and keys gain `@<ws>`
  (`session-row-<sessionId>@gaia`); the occurrence in the workspace that supervises it, or else the first
  workspace in registry order, keeps today's unscoped key, so existing selftests keep working. `menu_for` already
  takes the workspace from `MenuTarget::Session`, so the menu needs no change.
- **Plain folders.** No branch, no worktree placement, no `(root)` test against a git root. Label `./name` as today.
- **"supervised in <ws>".** A folder in two workspaces whose session is enabled in one: the other shows the folder
  with a quiet `supervised in <ws>` tag and no Enable; clicking the tag reveals the owning workspace (like
  `reveal`).
- **Disabled folders** leave the tree (m5/56 c5); search still finds them, labelled `disabled`.
- **Home.** The workspace's `home` replaces `root` on the workspace row; a workspace with several folders has no
  single root to show.

## 3. Alternatives

Legend: `●` health dot (green running, blue idle, amber blocked, grey stopped), `▸`/`▾` disclosure, `░` hover
background, `▌` selected (active tab), `[Btn]` bordered button, `(pill)` pill, `⟨icon⟩` an icon button.

### A. Minimal: name what is there, fit the folders model

Keep every row kind, every button and every menu. Change only what m5/55/56 force and what is confusing now.

```
 ▾ larchsys   [Work]  P:\Projects\LarchSys   7 sessions  started   [Task…] [Stop]
     ● running  workspace            bg · 1a2b3c4d  ☆  (attach)
       hook: turn ended 12 s ago
     ● idle     relay  ready         bg · 5e6f7a8b  ☆  (attach)
   ▾ ./AutoWork (master)  3                                          +
     ● running  fm-larchsys-autowork bg · 9c0d1e2f  ☆  (attach)
     ● blocked  m5-57 tree           bg · 77aa88bb  ☆  (attach)   ← amber tint
       waiting for: permission prompt
░▌   ● idle     Fix the tray         int · bbbb0005 ★  (tab)      ← hover + selected
   ▸ ./NodeApi                1      folder · also in gaia
   ▸ ./Skills (main)                 supervised in foreman

 ▾ gaia       [Work]  home P:\Projects\Gaia   2 sessions  stopped           [Start]
   ▾ ./Gaia (main)  1                                                +
     ● idle     gaia-ui              bg · 3344aa55  ☆  (attach)
   ▾ ./NodeApi                1      folder                          +
     ● running  node bindings        int · cc00dd11 ☆  ↗ also in larchsys
```

- **Workspace session and relay** stay in the supervised block, but the row shows its **role** (`workspace`,
  `relay`) instead of `fm-<ws>-workspace` (the full name stays in the menu's Copy name and the hover card). The
  relay label leaves the workspace row; its text moves onto the relay row's state column (`ready`, `starting`,
  `expired`), and while the relay is not running the workspace row shows `direct` only if messages go direct.
- **Supervised repo sessions** (`fm-<ws>-<repo>`) move from the top block to their folder, first in its list.
- **Icons on hover:** none new. Menus unchanged.
- **Two workspaces:** the row paints in both; the non-owning occurrence carries a quiet `↗ also in <ws>`; the
  folder row says `also in gaia` when its folder belongs to another workspace too.
- **Plain folders:** a `folder` tag where the branch would be. **Supervised elsewhere:** `supervised in <ws>` tag,
  no `+`, no Enable in its menu.

Cost: `fleet.rs` (role per supervised row, supervised repo sessions under their folder, owner per occurrence),
`view/left.rs` (labels, tags, scoped keys), `view/results.rs` (same keys), `supervise.rs` unchanged. No shared
component changes. Selftests: `relay-<ws>` is unused by them; `selftest_supervise` reads `RelayStatus::label`
directly and keeps working. `selftest_supervise` reads `WorkspaceNode::supervised` itself (`selftest_supervise.rs:297`
looks for the workspace session there; step g counts `supervised.len()`), so moving `fm-<ws>-<repo>` under its folder
changes step g's expected count; `selftest_delete.rs:49` walks both lists and is unaffected. Risk: low.

### B. The workspace row is the workspace session

Hide the workspace session's and the relay's rows; the workspace row takes their state and controls. Row actions
become icons that are always laid out and become visible on row hover, keyboard focus or selection.

```
 ▾ ● larchsys   P:\Projects\LarchSys · 7          ◉relay  ⟨☰⟩⟨✎⟩⟨⤢⟩⟨■⟩      ← not hovered: icons faint
 ░ ▾ ● larchsys   P:\Projects\LarchSys · 7        ◉relay  ⟨☰⟩⟨✎⟩⟨⤢⟩⟨■⟩      ← hovered: icons full
       ☰ Work   ✎ Task…   ⤢ Attach workspace session   ■ Stop   (tooltips)
     ▾ ⎇ AutoWork  master · 3                                   ⟨+⟩⟨>_⟩
       ● running  fm-larchsys-autowork   bg · 9c0d1e2f        ⟨☆⟩⟨⤢⟩
       ● blocked  m5-57 tree             bg · 77aa88bb        ⟨☆⟩⟨⤢⟩   ← amber
         waiting for: permission prompt
 ░▌    ● idle     Fix the tray           int · bbbb0005       ⟨★⟩⟨▭⟩   ← hover + selected
     ▸ 📁 NodeApi  folder · 1 · also in gaia
     ▸ ⎇ Skills   main · supervised in foreman

 ▸ ○ gaia       home P:\Projects\Gaia · 2                         ⟨☰⟩   ⟨▶⟩
```

- **The dot before the name** is the workspace session's health (`●` running/idle/blocked in its colour, `○` when
  stopped or not wanted). Blocked tints the workspace row amber, as a blocked session row is tinted now. `waiting
  for: …` of the workspace session appears as the workspace row's second line.
- **Relay:** one glyph with its status colour (`◉` ready, `◌` starting, `⚠` expired, nothing when direct or
  stopped); its tooltip is today's label. The relay is never a row.
- **Workspace row icons** (left to right, fixed slots so nothing shifts): Work `☰`, Task… `✎` (started, relay
  accepts), Attach `⤢` (workspace session attachable; else Resume), Start `▶` / Stop `■`. The keys stay
  `work-<ws>`, `task-<ws>`, `start-<ws>`/`stop-<ws>`; Attach gets `attach-ws-<ws>`.
- **Workspace menu** gains the workspace session's items, under a separator: Attach/Resume workspace session, Copy
  SendMessage name, Copy name, Copy id, Show in Work tab; the relay's Copy name and Stop relay go there too. It
  loses nothing.
- **Folder row:** a kind icon (`⎇` git, `📁` plain) replaces `./`; hover icons `+` New claude and `>_` Open terminal;
  Open folder, Enable/Disable, Remove from workspace stay in the menu.
- **Session row:** the pills become icons: Attach `⤢`, Resume `↻`, Tab `▭`; star stays as an icon. Stop, Copy, Show
  in Work, Delete stay in the menu. The state word stays (it is what a glance needs); `kind · id` stays.
- **A hover card** (`foreman_ui::hover_card`) on the workspace row lists: home, folders, workspace session (name,
  health, id, hook), relay (status), Workman project; on a session row: cwd, worktree, branch, hook, claimed item.
  The tab strip already uses the same card.
- **Two workspaces / plain / supervised elsewhere:** as in A.

Cost: `view/left.rs` (workspace and session rows rebuilt), `view/menu.rs` (`workspace_items` gains the session's
items; `session_items` reused for them), `fleet.rs` (`WorkspaceNode` carries the workspace session and relay apart
from `supervised`), `view/icon.rs` (new icons: play, square, maximize/arrow-up-right, pencil, list, terminal,
git-branch, folder, rotate — copied from Zed's set like the others). Shared: `foreman_ui::button` gains a
hover-revealed variant (an icon button with `group_hover` visibility, painted always so its hit key exists);
`foreman_ui::hover_card` is reused as is. Selftests: the workspace menu's exact key list in `selftest_manage`
changes (new items) and must be updated; `selftest_supervise.rs:297` and step g read the workspace session and relay
from `WorkspaceNode::supervised`, which no longer holds them; `task-<ws>` and `start-/stop-<ws>` keep their keys but
change size and place, so 7f's seam measure and t0's aim need re-checking. Risk: medium.

### C. Icon-first, one line per row, detail in hover cards

B's workspace row, and the same discipline on every row: a kind icon, the name, one state, and a fixed icon slot
cluster on the right; every secondary label moves into the hover card.

```
 ▾ ◆ larchsys     ●  ◉                                      ⟨☰⟩⟨✎⟩⟨⤢⟩⟨■⟩
   ▾ ⎇ AutoWork      3                                       ⟨+⟩⟨>_⟩
       ● fm-larchsys-autowork                                 ⟨⤢⟩
       ● m5-57 tree                     blocked               ⟨⤢⟩      ← amber
 ░▌    ● Fix the tray                   ★                     ⟨▭⟩
   ▸ 📁 NodeApi       1  ⇄
 ┌───────────────────────────────────────────┐                ← hover card on "m5-57 tree"
 │ m5-57 tree                                │
 │ state     blocked · waiting for: permission prompt │
 │ id        bg · 77aa88bb                   │
 │ cwd       P:\…\AutoWork.worktrees\m5-57   │
 │ worktree  m5/57 of AutoWork               │
 │ hook      turn ended 12 s ago             │
 │ also in   —                               │
 └───────────────────────────────────────────┘
```

- The state word shows only when it is not the normal one (`blocked`, `stopped`, `process gone`); running and idle
  are the dot's colour alone. `kind · id`, the hook line and `waiting for` move into the card (blocked keeps its
  tint).
- `⇄` marks a folder or session shown in more than one workspace; the card names the others.
- A session in a linked worktree gets a `⎇` badge after its name and the card names the worktree (the only design
  that shows worktrees, since the card has room).
- Workspace session and relay: as B.

Cost: everything in B, plus every row of `render_session` and `render_results`; the hit keys `session-name-…`,
`session-id-…`, `waiting-…`, `hook-…` disappear (no selftest clicks them, but the search highlights live in
`session-name-`/`session-id-` and move into the card, which search cannot highlight). Selftests that read the
painted state or hook text, and the screenshot comparisons, change. Risk: high; the most information leaves the
line, and a blocked session's `waiting for` is no longer readable without hovering.

### Comparison

| | A minimal | B workspace row is the session | C icon-first |
|---|---|---|---|
| Workspace session | row, labelled `workspace` | the workspace row's dot, icons and menu | as B |
| Relay | row, state in its state column | one glyph + tooltip | as B |
| Supervised repo session | first under its folder | first under its folder | first under its folder |
| On hover | nothing new | Work, Task, Attach, Start/Stop; folder + and terminal; session attach/resume/tab | as B, plus all detail in a card |
| In the menu | as today | as today, plus the workspace session's and relay's items | as B |
| Two workspaces | `↗ also in <ws>` | `↗ also in <ws>` | `⇄`, names in the card |
| Plain / supervised elsewhere | `folder` tag / `supervised in <ws>` | kind icon / `supervised in <ws>` | kind icon / `⇄` + card |
| Worktree | not shown | in the session card | badge + card |
| Files | `fleet.rs`, `left.rs`, `results.rs` | + `menu.rs`, `icon.rs`, `foreman_ui::button` | + every session row and the search marks |
| Selftest keys | unchanged (scoped `@<ws>` only for second occurrences); step g's `supervised` count | workspace menu list, `supervised` reads; positions of `task-`/`start-`/`stop-` | B, plus painted texts and shots |
| Risk | low | medium | high |

## 4. Recommendation

**Build A with m5/55–56, then B as its own unit; do not build C.**

1. **A ships with the folders work.** m5/55 and m5/56 change what the tree rows mean (folders, home, overlap,
   "supervised in"); A is the smallest change that shows that model correctly, and the scoped keys for a session
   under two workspaces are needed whichever design follows. Labelling the supervised rows by role and moving
   `fm-<ws>-<repo>` under its folder removes today's main confusion (which of the `fm-` rows is the workspace
   session) at no risk to the selftests.
2. **B is the target design.** The workspace session is the workspace's controller; its health and controls belong
   on the workspace row, and the relay is plumbing that needs one status glyph, not a row. Hover icons in fixed,
   always-painted slots shorten the crowded 30 px workspace line (three bordered buttons, four labels) without
   moving anything under the pointer and without losing a hit key. The menus lose nothing, so every action stays
   reachable by right-click and by the selftests' menu path.
3. **Not C.** It hides `waiting for` and the session's id behind a hover, which is what a glance at a blocked
   session needs, and it costs the search highlights.

**To approve or amend:**

- [ ] A with m5/55–56 (role labels, supervised repo sessions under their folder, `folder` / `supervised in <ws>` /
      `also in <ws>` tags, scoped keys for second occurrences).
- [ ] B as a follow-up unit: workspace row = workspace session (dot, `waiting for` line, Attach icon, menu items),
      relay as a glyph, hover icons in fixed slots on workspace, folder and session rows, a hover card on the
      workspace and session rows, a hover-revealed icon button added to `foreman_ui::button`.
- [ ] Amendments: which icons, whether the state word stays for running/idle, whether a workspace session that is
      blocked also raises its row to the top of the tree.
