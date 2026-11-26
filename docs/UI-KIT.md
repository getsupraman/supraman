# The UI kit

The model behind `supraman-ui`: what a control is made of, the few mechanisms every control is built
from, and the rules a feature follows when it composes them. Units of run `ui-kit` cite these sections
(`UI-KIT §4.2`). The component list in CLAUDE.md is the index of what exists; this file is why it is
shaped that way.

Reference: `.repos/gpui-kit` (longbridge, Apache-2.0, on `gpui-pre` 0.3.14, Zed's gpui republished).
We stay on Zed's gpui and Zed's editor, so its crates cannot be a dependency (two gpuis cannot share a
binary). Files we port keep their Apache-2.0 notice and say where they came from (§12).

## 1. The problem, stated once

Today's kit is a set of free functions and entities that each fuse behaviour with appearance and take a
hit key. It works control by control, but the things that cross controls have no owner:

| Symptom René sees | Mechanism behind it |
|---|---|
| no scrollbars | 10 feature `overflow_y_scroll` boxes and every list/table scroll with no bar; the kit has no scroll primitive |
| focus incomplete and buggy | focus is set during render in 49 places; carets and the terminal cursor ignore focus and window activity |
| terminal: `` ` `` then `e` types `` `e ``, no IME | the terminal has no platform input handler, so gpui's composed WM_CHAR and IME text have nowhere to go |
| commit message jumps when clicked; cannot select it | read-only and editable are two different renderings; read-only text is a plain `div` |
| context menu: "which row did I click?" | hover is a paint style, nothing records the row a menu belongs to |
| Rename branch looks unlike Set upstream; the blue band | seven question/form surfaces (dialog, band, form slot, name card, stash strip, inline row field, in-place editor) |
| a checkbox's label sometimes toggles it | the checkbox is a square; each caller adds a label its own way |
| text and icons sit a pixel apart in some rows | `ink` alignment is called in 16 places; app and work-view never call it |

Each row is one missing mechanism, not one missing fix. The kit below adds the mechanisms once.

## 2. What a control is made of

Every control is three independent layers (skill `shared-ui-components` §1):

- **Behaviour**: what the user can do and the state that follows (press, select, open, drag, type,
  scroll, focus). Owned by the kit, written once, tested once.
- **Appearance**: how each state looks, from tokens (§3). Owned by the kit.
- **Content**: what it shows (a branch name, a session's rows, the menu's items). Owned by the feature,
  passed in.

Dependencies point one way: feature → `supraman-ui` → tokens → theme. The kit never imports a feature
type, feature words or a feature's state file.

## 3. Tokens

Colour comes from the theme (`ThemeColors` plus `themes::{selection, secondary}`). Every other decision is
`supraman_ui::tokens`, by role, at the values the kit already drew with (ui-kit/2 measured them; the gallery did not move):

| Token | Values | Replaces |
|---|---|---|
| `space` | `XS 2, S 4, M 6, L 8, XL 12, XXL 16, XXXL 20, PAGE 24` | `gap_1`, `p_2`, `px(6.)`, … |
| `radius` | `S 4` (buttons, fields, pills, tick boxes), `M 6` (toolbar buttons, menus, cards), `L 8` (dialogs), `FULL` | `rounded_sm`, `rounded_md`, `rounded_lg` |
| `text` | `CAPTION 10, SMALL 12, BODY 14, TITLE 16, HEADING 18` (rems of 16 px), each at gpui's φ line height | `text_xs`, `text_sm`, `px(10.)` |
| `control` | `SMALL 20` (8 px padding, SMALL text), `MEDIUM 26` (12, BODY), `LARGE 32` (16, BODY) | `button::Size::table` |
| `icon` | `XS 10, S 12, M 14, L 16` | literal icon sizes |
| `stroke` | `HAIRLINE 1, FOCUS 2` | `border_1`, `border_2` |
| `motion` | `FAST 80 ms, NORMAL 160 ms, TURN 1 s, SCROLLBAR_IDLE 1500 ms, CARET_BLINK 530 ms` | scattered constants |
| `Layer` | `Float < Card < Prompt < OverPrompt < Menu < Dialog` | `menu::{FLOAT, FORM, CARD, OVER_FORM, PRIORITY}`, `dialog::PRIORITY` |

Colour roles the kit adds over the theme, derived once per theme load (`tokens::roles(cx)`): `surface`, `surface_raised`,
`surface_sunken`, `border_subtle`, `border_strong`, `ring`, `accent`, `on_accent`, `danger`, `warning`, `success`, `info`,
`selection`, `selection_inactive`. A component asks for a role; a theme change moves every control at once (the "physics"
test). The theme is fitted where the roles come from: `selection` holds `ACTIVE_SELECTION_SHOWS` on the panel and always
reads stronger than `selection_inactive`, which still shows; `info` holds `INFO_SATURATION`, so it never reads as muted
text (`themes.rs`).

A size or emphasis is a **dimension** of a control (`size: Size`, `tone: Tone`, `fill: Fill`), never a
second control.

## 4. The mechanisms

The kit is built from eight mechanisms. Every control is a composition of them plus appearance.

### 4.1 Interaction state

One state vector per interactive element, kept by the kit, not by gpui paint styles alone:

`hovered · pressed · focused · focus_visible · selected · open · dragging · drop_over · disabled`

`open` is the new one: an element that anchored a popup (a menu, a select, a picker, a hover card)
stays `open` until that popup closes. One function resolves the vector to appearance with a fixed
precedence (instance style → hovered → pressed → selected → open → drop_over → focus ring → disabled
last; idea from gpui-kit `base/state_style.rs`). Hover is still gpui's `.hover()` for the paint, and
`open` overrides it, so a right-clicked row keeps its "this one" fill while the pointer travels into
its menu.

`state.rs` holds the vector the call sites set (`hovered` for a hover the kit tracks itself, a menu's highlight;
`selected`, `open`, `focus_visible`, `disabled`, `dragging`, `drop_over` (§4.7)); the pointer's hover and
press stay gpui's paint.

### 4.2 Anchors and popups

Every popup has an **anchor**: the key of the element it belongs to, and a position (pointer, or the
anchor's bounds). `Popup::open(anchor, …)` records it; the anchor's element reads `open` from the
window's popup registry while it paints. One registry per window: one popup open at a time per layer,
outside click and Esc close it, focus returns to what held it (§4.3).

Popups: context menu, dropdown menu, select, picker (palette in a popover), hover card, tooltip.
Menus have Up/Down/Home/End/Enter, icons, shortcuts, a submenu (Right/Left, or the pointer resting on its item
for `motion::NORMAL`), scrolling when long, and type-ahead. An anchor whose element is not a table row or a `Button`
paints `popup::open_fill` as its first child; the `open` fill is `tokens::roles` `open`, apart from hover and selection.

### 4.3 Focus

The focus model has three levels, and only one place changes focus:

1. **Window → pane** (`focus::Pane`, F6): exists, stays.
2. **Pane → scope**: a form, a dialog, a popup is a *scope*. Tab and Shift+Tab cycle within the
   innermost scope (`focus_order`, exists); a modal scope traps the focus (`dialog` does, popups will).
3. **Scope → control**: the control holding the keys.

Rules:

- **Focus moves only in an event handler or through `focus::request(handle)`**, which applies after the
  frame paints (gpui-kit's popover defers the same way). No `window.focus` inside `render`; a ratchet
  test counts them down to zero (§11).
- **A popup remembers what held the focus when it opened** and gives it back on close, only if the focus
  is still inside the popup; if nothing held it, only what encloses the popup may take it, else nothing
  does (`focus::popup`; `restore_on_loss` asks it first).
- **Only the element holding the keys shows a caret or cursor, and only while its window is active.**
  A text field's caret and a terminal's line cursor (`|`) behave alike: shown and blinking while they
  hold the keys, hidden otherwise. One
  `Blink` entity (epoch-cancelled timer, paused by each keystroke, gpui-kit `blink_cursor.rs`) drives
  every caret and cursor.
- `focus_visible` (keyboard-reached: a key since the window's last mouse press) shows the ring, `focus::ring`,
  a painted outline, never a shadow; a mouse click on a button does not.

### 4.4 Scroll

One primitive, `ScrollArea`, wraps every scroller: a `div` with a `ScrollHandle`, `uniform_list`,
`gpui::list`. It draws the scrollbar as an overlay through one `ScrollbarHandle` trait implemented for
all three handles (port of gpui-kit `base/scrollbar.rs`, which uses the `ListState` scrollbar methods
our gpui already has):

- overlay thumb, min length 32 px, 6 px wide, 8 px while hovered or dragged;
- shown while scrolling, then fades after `motion.scrollbar_idle`; shown while the pointer is over the
  area (mode setting `[appearance] scrollbars = "auto" | "always"`);
- thumb drag keeps the grab offset; a track click pages toward the click;
- keyboard: PageUp/PageDown/Home/End on a focused scroll area that has no selection of its own;
- nested areas: the inner one takes the wheel until it reaches its edge (gpui-kit `scrollable_mask.rs`).

Lists (§4.6) and the text view (§4.5) are scroll areas. Feature code never calls `overflow_y_scroll`
again; a scan test enforces it (like `clipboard::tests`).

### 4.5 Text

One text engine, two modes, the same box:

- `TextInput` (exists): editable, IME and dead keys through `EntityInputHandler`, selection, undo.
- **`TextView`**: a read-only, plain `TextInput` kept per element id, so the same layout and the same
  selection model: mouse selection, double-click word, triple-click line, Shift+arrows once focused, Ctrl+C,
  Ctrl+A. Copy goes through `supraman_ui::clipboard`.
- **Editable is a mode, not a different widget.** `TextInput::set_read_only(bool)`: switching mode keeps the
  font, size, wrap, padding and border, so nothing moves. Only the caret appears. The commit panel's
  message is one box: click places the caret where you clicked (on a commit that can be reworded) and
  the actions (Cancel, Update Message) appear in a reserved row, not inserted above content.

### 4.6 Lists

One virtual list core under table, tree, palette, menu, the sidebar and every row list in a feature:
keyed rows, one selection (click, Up/Down, PageUp/PageDown, Home/End, type-ahead), `open` per row from
§4.2, `drop_over` from §4.7, a `ScrollArea`, row chrome (`button::row`'s fill, hover, inset, selection
by focus) applied by the list. A feature supplies rows and a row painter, never the row's hover or fill.
The hand-hovered rows in `left.rs`, `schedules.rs`, `results.rs`, the Work navigator and overview move
onto it.

`list.rs` is that core as two parts each shape composes, not one type: `Selection` (one selection, moved by
`key_of`'s one key map over `Rows`: a step, a page of the rows in view, the ends, type-ahead; `Rows::wrapping` for a
menu) and `Chrome` (a row's fills, painted by `list::row` or a table's row under the content: its band, the ghost
hover and press, the `selection` role while the list holds the keys and `selection_inactive` else, the open fill of
the popup anchored to the row). The table keeps its columns and `uniform_list`, the menu its popup and submenu, the
palette its query, the keyed list its `gpui::list`; each holds a `Selection` and paints its rows' `Chrome`. A
feature hands rows and their content; `hover` in the ratchets is at zero.

### 4.7 Drag and drop

One mechanic, three uses:

- **Resize**: `split::DraggedDivider`, shared by splits, height bars, stacks and table columns.
- **Reorder**: `dnd::Sortable<P>` over a list or the tab strip: each item says the `Edge` a payload
  over it lands on (the owner's rule: `lands(from, to)` for an index, `folder_edits::landing` and
  `group_landing` for the sidebar), the insertion line is drawn there in `Roles::drop_border` at
  `stroke::FOCUS`, a drop calls the owner; `scrolls` gives any `ScrollbarHandle`'s area edge
  auto-scroll, each move and on every `motion::EDGE_SCROLL` while the pointer rests at the edge.
  Tab strip, sidebar folders and groups.
- **Drop onto**: a typed `dnd::Payload` (`DraggedTab`, `DraggedSession`, a pill, a folder, a group)
  and `DropZone` targets, which paint `drop_over` (`Roles::drop_over` inside `drop_border`, through
  `state::resolve`) while a payload they accept hovers them and call their owner once on its drop.
  A terminal, a group's row, the sidebar's drop row, a graph pill.

Every drag starts at `dnd::source`: the shared `dnd::chip` under the pointer, the source `dragging`
(dimmed) while it is dragged, and Esc cancels it with nothing dropped.

### 4.8 Label and alignment

One `Label` primitive: an optional leading mark (icon, dot, tick box) and text, the mark on the
letters' middle by `ink::mark`, clipped with its tooltip (`clipped`). Buttons, rows, menu items,
pills, tree rows and checkboxes are built on it, so alignment is right everywhere because it is
written once.

`Checkbox::new(hits, key, checked).label("Check out after creating")`: the whole row (square and label) is one
hit target, toggles on click, Space and Enter, and carries the focus ring. Same for `Radio` and
`Switch` if they are ever needed.

## 5. Surfaces: where a question or a small form appears

A Git tab question or small form has two meanings:

- **A question or a small form about an action** (rename a branch, set an upstream, a force push,
  delete, add a remote, create a worktree, a stash message): **one `Prompt`**. It is modal to its tab,
  centred over the tab's content on a light scrim, the same panel as the main window's `dialog`
  (title, fields from `form`, answers right-aligned, Enter the default answer, Esc cancel). The tab
  stays visible behind it, so the context (the selected branch, the graph) is still seen.
- **Editing a value where it is shown** (a group's name in the sidebar, a new branch's name on the
  graph row, the commit message): **in place**, through `TextBox` (§4.5). Nothing else moves.

The `Prompt` is `dialog::Dialog::in_tab` (ui-kit/6): the main window's dialog over one tab's region, at
`Layer::Prompt`, F6's pane while it shows (`focus::Pane::covering`), its keys never reaching the tab behind
while the rest of the window keeps its own. A form keeps its title × (outside the Tab order) rather than a
Cancel answer. The band, the name card, the stash strip and the per-tab form slot are gone, and
`themes::attention` with the band (placement confirmed by René 2026-11-26).

## 6. Visual direction

A tool that lives inside the user's editor theme should not have a look of its own that fights it. The
direction is a **calm instrument**: surfaces and text from the theme; colour only for state (selection,
focus, status, danger, a lane); one accent per window (the theme's); depth by surface steps
(`surface_sunken < surface < surface_raised`), not by tinted fills. The one place we spend boldness is
**state legibility**: the focused pane, the selected row and the row a menu belongs to are always
unmistakable, in every theme, by contrast floors the kit fits (as `themes::selection` does now).

Concretely, for the things René named:

- Prompts are raised surfaces with a hairline border, not tinted bands. A danger answer is the only red.
- Buttons: Solid for the default answer, Outline for the others, Ghost in toolbars; one height per size.
- Pills and badges share one radius and one height (`control.Small`).

## 7. Accessibility floor

Every control in the kit meets this; a feature cannot opt out because it does not paint the control.

- Every action is reachable by keyboard; every focus has a visible ring (`focus_visible`).
- Text holds the theme's contrast floors (`NAME_FLOOR` 7.0 on fills, `SECONDARY` for muted text).
- Hit targets are at least `control.Small` (20 px) high; icon buttons have a tooltip with their name.
- Nothing blinks or animates while its window is inactive, and nothing pretends to hold the keys that
  does not (§4.3).
- gpui's AccessKit roles (`Role`, `aria_*`, present in our gpui) are set by the kit's primitives:
  button, checkbox, list, listitem, menu, menuitem, dialog, textbox. Screen-reader support follows
  without feature work.

## 8. Terminal input

The terminal is a text consumer like any other:

- It gets a platform input handler (`EntityInputHandler` on the terminal entity, an
  `ElementInputHandler` in `term_element`) so composed characters (`` ` `` then `e` → `é`), AltGr
  characters and IME text arrive as text and are written as UTF-8. Key bindings still run first; a key
  with `prefer_character_input` waits for its character instead of sending the bare dead key.
- Key encoding matches Windows Terminal byte for byte (a table test: every key × Shift/Ctrl/Alt, the
  CSI `1;2D` forms, `modifyOtherKeys`/kitty where claude asks for it).
- Selection stays the mouse's. Shift+arrows and every other modified key reach the program unchanged
  (René 2026-11-26: a program that ignores Shift+arrow selection does so in every terminal).
- The cursor follows §4.3: the line cursor, shown only while the terminal holds the keys.

## 9. The crate after the refactor

```
supraman-ui/src/
  tokens.rs            §3
  state.rs             §4.1 interaction state + resolve
  popup.rs             §4.2 anchors, registry
  focus/ focus_order/  §4.3
  scroll/              §4.4 ScrollArea, Scrollbar, ScrollbarHandle (ported)
  text/                §4.5 TextInput, TextView, TextBox, Blink
  list/                §4.6 core; table/, tree/, palette/ on it
  dnd.rs               §4.7 Sortable, DropZone (split's divider stays in split/)
  label.rs ink.rs      §4.8
  controls/            button, checkbox, segmented, stepper, pill, badge, menu, select, tooltip
  surfaces/            dialog (Prompt), toast, banner, hover_card, form
  patterns/            details page, settings page row, placeholder
  gallery.rs           §10
```

Not UI, moved out to `supraman-core` (or their own small crate): `open`, `stall`, `nudge`, `store`,
the logger in `lib.rs`. Feature content leaves the kit: `inbox_card`'s and `session`'s words and
mapping go to their features, built on kit cards and labels.

Controls become builder structs (`Button::new(hits, key).label(…).size(…).tone(…)`) implementing
`IntoElement` (`RenderOnce`, and `Styled` so a caller's layout follows the kit's own), in place of 30 free functions with
positional arguments. The hit key stays the element's id, so the selftests keep working. ui-kit/2 did buttons
(`Button`, `ButtonSplit`, `Button::line`).

## 10. The gallery

`supraman.exe --selftest-shots` gains a **kit gallery**: one picture per section showing every control
in every state (rest, hover, pressed, focused, selected, open, disabled, drop-over; Small/Medium; each
tone), in the dark default theme and the light selftest theme. It is the review surface for this run:
every unit's report carries the gallery's PNGs before and after, so René reviews appearance from
pictures, not from a test build.

## 11. Order of work

Diagnostics before diagnosis, foundations before migration. Units of run `ui-kit`:

1. Gallery and ratchets: the gallery (§10) and scan tests that count raw `overflow_*_scroll`,
   render-time `focus`, hand `.hover(` rows and `px(` literals in feature crates; the counts may only
   go down.
2. Tokens (§3) and the builder shape (§9) for buttons; the kit's own `px(` onto tokens.
3. Scroll (§4.4): port the scrollbar; every scroller onto `ScrollArea`.
4. Focus (§4.3): `focus::request`, no render-time focus, popup focus return, `Blink`, focus-aware caret
   and terminal cursor.
5. Interaction state, anchors and menus (§4.1, §4.2).
6. Prompt (§5): one surface; band, name card, stash strip and form slot retired; visual direction §6.
7. Text (§4.5): `TextView`, `TextBox`; the commit panel's message rebuilt on it.
8. Terminal input (§8).
9. Label, checkbox, alignment (§4.8).
10. Lists (§4.6): every feature row list onto the core.
11. Drag and drop (§4.7).
12. Crate shape (§9): infra and feature content out; the ratchets at zero; CLAUDE.md's component list
    rewritten from this file.

## 12. Porting from gpui-kit

Candidates, each a file or two with only theme and animation internals to rewrite: `base/scrollbar.rs`,
`base/scrollable_mask.rs`, `base/input/base/blink_cursor.rs`, `base/state_style.rs`, the focus restore
in `base/popover.rs`, the selection model of `base/selectable_text.rs`. A ported file keeps its
`Apache-2.0` header, names its source path and commit (`f807c1b`), and is listed in
`scripts/licenses` with the others we ship.
