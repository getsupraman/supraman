# GitMan groundwork: analysis of GitKraken 12.6.1

Ground work for WorkState `autowork/roadmap/3` (GitMan). Date 2026-10-28. Analysed install: `%LOCALAPPDATA%\gitkraken\app-12.6.1`.

## 0. Method and legal frame

- Unpacked `resources/app.asar` (442 MB → 927 MB) with `@electron/asar`. **No source maps ship any more**; bundles are minified webpack (`main.bundle.js` 7 MB, `render.bundle.js` 13 MB, `requestHandler`, `worker`).
- Readable ground truth: `package.json` (full dependency list), `@gitkraken/gitkraken-components` `.d.ts` types, the Rust sources of `@axosoft/node-spawn-server` and `@axosoft/rust-socket-bridge`, `strings.json` (5,526 UI strings), keybinding JSON, theme JSONC. No GitKraken binary was executed beyond reading files.
- **License:** the GitKraken Components License (and the EULA) forbid reverse engineering and "creating software with the same or similar functionality". EU Software Directive Art. 5(3)/Art. 8 lets a lawful user observe and study a program to learn its *ideas and principles*, and that right cannot be contracted away. So this report records ideas, architecture and behaviour only. **GitMan copies no GitKraken code, assets, themes, icons or strings, and depends on no GitKraken package.** Everything below is reimplemented from the description.

## 1. Stack at a glance

| Layer | GitKraken 12.6.1 | GitMan |
|---|---|---|
| Shell | Electron 41, React 17, redux + redux-saga + Immutable, jotai | gpui (Zed rev 03efd49), Foreman crates |
| Git read | git CLI (`rev-list`, `for-each-ref`, `status --porcelain=v2`), libgit2 (nodegit fork) as legacy fallback | **gix** in-process |
| Git write/network | Bundled Git for Windows 2.54 + LFS + ssh + gpg | system **git CLI** |
| Spawning | Rust msgpack spawn server (to keep Node's event loop free) | background executor + process helper (no server) |
| Credentials | Rust askpass bridge → TCP socket to app | small askpass binary → named pipe + token |
| Watcher | nsfw | `notify` + debouncer |
| Graph | `@gitkraken/gitkraken-components` (react-virtualized, SVG edges) | own gpui graph element |
| Diff/merge | Monaco diff editor, own 3-way merge tool | FileMan's (Zed) editor, later |
| Terminal | xterm.js + node-pty | foreman-term |

## 2. Git backend

**Dual backend, CLI as the default.** The setting `gitBinaryEnabled` (default true) picks the CLI. 58 operations exist twice (`gitX` / `nodegitX`), which shows the cost of maintaining two backends. Features such as squash merge exist only on the CLI path. About 90 `runGitCmd` builders exist. Their conventions:
- message and refspec input on stdin (`commit --file=-`, `fetch --stdin`, `rev-list --stdin`);
- NUL-separated output formats, parsed as a stream;
- `-c` overrides that make output safe to parse (for example `advice.*=false`), plus `--no-optional-locks`;
- environment: `GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=""`, `TERM=dumb`.

**libgit2 is still used for:** clone with an in-process LFS filter, ahead/behind and descendant-of, merge analysis, blame and submodule status. GitMan can do all of these with gix or the CLI.

**Locking.** Every operation declares the resources it needs (INDEX, INDEX_READ, CONFIG, REPO_SETTINGS, GITIGNORE, GITATTRIBUTES), and a per-repo scheduler serialises operations that conflict. GitMan should adopt this idea.

**Loading the graph:**
- **Tips:** the SHA of every ref.
- **Walk:** `rev-list --date-order --ignore-missing --max-count=N+1 --stdin` with a NUL format of parents, author, committer and raw body. Fetching N+1 tells it whether more commits exist.
- **Limits:** 2000 commits by default (minimum 500), lazy-load pages of 200, and the rest of the graph rehydrated when idle.
- **Caching:** commits are cached by SHA, and a reload is skipped when the tips and the count have not changed.
- **Refs:** one `for-each-ref` call with a NUL format that includes `upstream:track`, so ahead/behind costs no extra call.
- **WIP:** `status --porcelain=v2 --untracked-files=all --branch --renames`.
- **Maintenance:** the app runs `git maintenance` itself (commit-graph, prefetch, gc).

**Watching.** nsfw, debounced 2 s on the workdir and 200 ms on `.git`/worktrees. Events go through a gitignore filter, then are classified as index, HEAD/heads, stash, remotes/tags or worktrees, and **each class triggers only its own refresh**. Config files and the commit template have their own watchers.

**Credentials and signing.** A tiny Rust binary is set as `GIT_ASKPASS`, `SSH_ASKPASS` (`SSH_ASKPASS_REQUIRE=force`) and `gpg.program`. It connects to `127.0.0.1:$PORT`, sends the env, argv and command id as JSON, and returns the answer or an exit code. Prompt kinds are USERNAME, PASSWORD, PASSPHRASE and PASSPHRASE_SSH_SIGN. For gpg it forwards stdin, and the app runs the real gpg with `--pinentry-mode=loopback`. `GIT_SSH_COMMAND` points at a known ssh, and Git Credential Manager is supported.

**Hooks.** GitKraken points git at its own hooks folder, which wraps each real hook and writes BEGIN/EXIT markers to stderr. The UI parses the markers to show hook output per hook.

**Interactive rebase.** `GIT_SEQUENCE_EDITOR` is set to a shim that hands the todo list to the app over the same socket, the app shows its rebase panel, and the shim writes back the edited todo.

**Undo/redo.** The journal is the reflog of `refs/worktree/gitkraken/undo`:
- each entry is a marker commit with a trailer naming the action and the head before and after;
- undo replays the inverse action;
- before a discard, the changes are stashed automatically so the discard can be undone;
- supported actions: checkout, commit, discard, delete branch, remove remote, reset, rebase.

**Other processes.** The main renderer does all git work. `requestHandler` is a hidden window for HTTP to hosting services. Web workers handle commit search and the diff3 calculation. `gk.exe` is a separate Go CLI product (cobra, go-git, MCP) and is not the app's engine.

### Backend decision for GitMan (proposal)

| Operation class | Backend |
|---|---|
| Graph walk, refs, ahead/behind, trees, blobs, tree-to-tree diffs, blame | gix (reads the commit-graph file, multithreaded, no FFI) |
| Status / WIP | gix status; `git status --porcelain=v2 -z` as fallback and cross-check (fsmonitor, sparse checkout, submodules) |
| Commit (hooks, signing), merge, rebase incl. interactive, cherry-pick, revert, stash, reset, worktree, submodule, sparse-checkout, LFS, maintenance | git CLI |
| fetch, push, pull, clone | git CLI + askpass bridge |
| Partial staging (hunks and lines) | build the patch from the gix diff, then `git apply --cached -` |
| git2-rs | not used; a stopgap only if gix lacks a mutation |

A single backend with no duplicates: gix for reads, the CLI for writes. Every operation goes through one per-repo resource-lock scheduler, with command ids for logs and cancellation.

## 3. Commit graph (design to reimplement)

**Input row.** sha, parents, author, email, date, message, type (commit, merge, stash, work-dir, merge-conflict, rebase-warning), heads (current, upstream, worktree), remotes (owner, avatar, host type), tags (annotated). The WIP row is synthesised from the work-dir stats and parented to HEAD.

**Processed row.** lane, colour lane, `edges[lane] = {starting | passThrough | ending, parent, dashed}`, max lane, ref groups (a local branch and its remotes merged into one pill), the refs of the nearest descendant (shown as "ghost refs" on hover), and a timeline bucket.

**Lane assignment.** One pass from the newest row down:
- **Bookkeeping:** a set of used lanes, the lane reserved for each parent that is not placed yet, and a list of lanes to free when a given sha is reached.
- **The row's lane:** a reservation made by a child, otherwise the lowest free lane. A pinned branch always gets lane 0.
- **First parent:** if two children reserve it, the lower lane wins and the other lane is freed when the parent is reached. Stash reservations lose. A parent that has a merge child never moves.
- **Further merge parents** get fresh lanes.
- **Edges:** a carried lane ends at its target and passes through otherwise. Each parent starts an edge in its own lane. Stash and WIP edges are dashed.
- **Filter mode** puts every row in lane 0 and draws no edges.
- **Hiding a branch** prunes commits that only it reaches.

**Geometry.** Rows are 28 px. Lane x = pad + lane·w + w/2. A cross-lane edge runs horizontally at mid-row, turns through a quarter arc of radius 11, then runs vertically. Colours come from a 10-colour palette with `lane % 10`.

**Zones.** ref, graph, message, author, datetime, sha and changes. Each zone has a persisted width, hidden flag, order and mode (compact, rich, text). Resizing keeps the zones filling the width. Headers can be dragged to reorder and show an icon below a minimum width. The graph zone scrolls horizontally, and nodes of off-screen lanes are clamped into faded gutters.

**Rendering.**
- Each zone is its own virtualised grid, with the scroll shared between them.
- Edges per row are one cached SVG, keyed by an edge hash.
- Avatars are lazy-loaded with a 100 ms debounce.
- A scroll-marker strip beside the scrollbar shows head, upstream, local, remote, tags, stashes, search hits, selection and PRs.
- The changes bar is normalised to the min and max over all rows.

**Interactions.**
- Click selects; Ctrl-click toggles a row; Shift-click selects a range.
- Up/Down move by row. Shift+Up/Down move topologically (first parent, non-stash child). H jumps to HEAD, Shift+H to its upstream.
- Double-click is a host callback; the host uses it to check out.
- Context menus are built by the host from the row's context data.
- Dropping a ref on another ref opens a menu: merge, rebase, interactive rebase, fast-forward, push, or start a PR.
- A branch or tag can be created inline on a row.
- Highlighting: dim merge commits, dim rows outside the selected commit's ancestry, and highlight on ref hover.

**gpui mapping.**
- The processing step is a pure function off the UI thread, re-run for each page.
- `uniform_list` with one shared scroll.
- Edges drawn with `paint_path` and quads, caching paths by edge hash if profiling shows a need.
- The lane palette comes from the theme's accents/players palette, per autowork-b0. Do not add 10 new tokens.
- The WIP summary field is the shared `TextInput`.
- The graph is a thin component. Git semantics (drop menu, checkout) live in GitMan.

## 4. Features and UI layout

**Layout:**
- tab bar (one tab per repo, plus repo management);
- toolbar: undo, redo, pull (merge / ff-only / rebase), push, branch, stash, pop, terminal, fetch;
- left ref panel: Local, Remote, PRs, Issues, Tags, Stashes, Submodules, Worktrees, Gitflow, Agents; with filter, hide/solo and maximise;
- centre: the graph, replaced by the file or diff view when a file opens;
- right detail panel: WIP staging and commit message, or commit details;
- bottom: terminal;
- a status bar;
- Ctrl+J and Ctrl+K toggle the left and right panels.

**Feature tiers in GitKraken.**
- Free: everything in the MVP column below.
- Paid: private and self-hosted repos, the 3-way merge tool, multiple profiles, self-hosted integrations, Conflict Prevention.
- Cloud: Launchpad, Cloud Patches, Code Suggest, Workspaces, Teams, AI with credits.

**Agents view (new in 12.x).** Coding-agent sessions per worktree with approve/deny prompts, plus hooks installed into Claude Code, Codex and Copilot CLI. **This overlaps Foreman directly. In GitMan, Foreman's sessions are the agents view.**

**Themes.** JSONC with `meta{name, scheme}` and a `themeValues` token map: about 171 tokens, LESS-style `fade`/`lighten`, 10 graph lane colours, and a base file of 358 derived tokens. GitMan does **not** adopt this format. It uses Foreman's Zed theme JSON (`foreman-ui/src/themes.rs`, about 150 tokens).

**Keymap.** An Atom-style keymap: CSS-selector scope → chord → command. In GitMan this maps onto gpui key contexts and actions. The defaults are listed in the scope table.

### Proposed scope

| MVP | Later | Out of scope |
|---|---|---|
| Repo open/clone/init, recent repos, one tab per repo | Interactive rebase panel (sequence-editor shim) | Launchpad, Cloud Patches, Code Suggest, Teams |
| Commit graph with zones, paging, search highlight/filter, hide/solo/pin branch | 3-way merge tool (needs FileMan editor) | Licensing, onboarding, webviews |
| Ref panel: local, remote, tags, stashes, worktrees, submodules, filter | File history and blame | Conflict Prevention (needs a server) |
| Staging of files, hunks and lines; discard; commit; amend; template | GPG/SSH signing; hooks output view | Hosted AI credits |
| Diff view: split and inline (a read-only viewer until FileMan) | PRs and issues through GitHub/GitLab (reuse foreman-workstate OAuth) | A separate Agents view (Foreman is it) |
| Toolbar: fetch, pull, push, branch, stash, pop | LFS, Gitflow, sparse checkout, shallow clone | |
| Undo/redo journal on a reflog ref | Profiles (git identity) | |
| Askpass bridge, per-repo lock scheduler, watcher classes | Drag ref onto ref (merge, rebase, push) | |
| Command palette, commit search | Bring-your-own-endpoint AI commit messages (could use Claude via Foreman) | |

## 5. What Foreman can share (from autowork-b0, master 38f318c)

Almost all the generic UI is still inside the binary crate `foreman-app`. **m5/44 extracts it** into `foreman-ui` (or a new `foreman-widgets`) and writes `docs/ARCHITECTURE.md`. GitMan depends on that.

| Need | Today | Where it belongs |
|---|---|---|
| Text input (single/multi-line) | `foreman-app/src/text_input.rs` | foreman-ui (m5/44) |
| Virtualised table with columns | none (gpui `uniform_list`) | foreman-ui, new |
| Tree view | fleet-specific `view/left.rs` | foreman-ui, on the table |
| Tab strip | `view.rs` Tab/TabKind, `tab_card.rs` | generalise in m5/44 |
| Split panels | one fixed divider in `view.rs` | foreman-ui, new |
| Context menu | `view/menu.rs` (actions are Foreman-specific) | split in m5/44 |
| Dialog / confirm | `view/delete.rs` | foreman-ui, new |
| Toast / notices | `view.rs`, `view/notices.rs` | extract in m5/44 |
| Fuzzy palette | matcher `search.rs`, list `view/results.rs` | foreman-ui |
| Keymap | gpui actions plus `bind_keys`; `docs/KEYS.md` | shared helpers in foreman-ui |
| Theme | `foreman-ui/src/themes.rs` (Zed JSON) | reuse; lane colours from accents |
| Terminal | `foreman-term`, `foreman-ui/src/term_element.rs` | reuse |
| Process helper | claude-specific `foreman-claude/src/cli.rs` | generic one in foreman-core |
| File watcher | none (transcripts are polled) | `notify` watcher in foreman-core |
| Askpass server | none | gitman crate (or foreman-core if shared) |
| Credentials | `foreman-workstate/src/secret.rs` (Credential Manager) | move to foreman-core |
| Settings | `foreman-core/src/settings.rs`, `settings_window.rs` | plugin page registration plus a namespaced `[gitman]` table |
| Diff/text view | none | FileMan's editor; no GitMan-only editor |
| HTTP/OAuth | `foreman-workstate` http.rs, auth.rs (PKCE) | reuse for GitHub/GitLab |

## 6. Zed as a source (checkout `.repos/zed` @ 03efd49, Foreman's pinned rev)

**Licence.** Every Zed crate GitMan would touch (`git`, `git_ui`, `askpass`, `net`, `buffer_diff`, `editor`, `multi_buffer`, `project`, `workspace`, `theme`, `ui`, `picker`, `fuzzy`, `settings`, `fs`, `text`, `rope`, `language`, `terminal`, `extension_host`) is **GPL-3.0-or-later**; only `gpui` is Apache-2.0. Foreman's own crates are already `GPL-3.0-or-later`, so reusing Zed's code is licence-compatible. It needs the GPL notices in THIRD_PARTY.md. This is the opposite of GitKraken, which may be studied only.

**Zed's git layer (`crates/git`) is CLI-only.** It does not use git2 or gix any more. A `GitRepository` trait sits over a `GitBinary` command builder. Graph data comes from `rev-list` with the format `%H%x00%P%x00%D`, in chunks of 1000. The crate already includes askpass, blame, commit and status. Its dependencies are light: gpui, rope, text, sum_tree, util and askpass, with no workspace and no project. **GitMan can depend on it directly** at the pinned rev.

**Zed already has a gpui commit graph:** `git_ui/src/git_graph.rs`, about 8,500 lines.
- It has lanes, `PathBuilder` curves, `uniform_list`, a split view, a changed-files tree and search.
- Its lane colours come from the theme's accent colours, which confirms the palette question: no new tokens.
- It depends on editor, project, workspace, picker and markdown, so it cannot be a dependency. It can serve as the code base for GitMan's graph, forked into the gitman crate under GPL with attribution. The **design follows GitKraken (§3)**: see §7.3.

**Revised backend proposal.** Start on **Zed's `git` crate (CLI)**, the same model GitKraken is converging on. Add **gix** only where a measurement shows that process spawning hurts, for example the graph walk on very large repos or status on every watcher tick. This replaces the "gix for reads" proposal in §2. Assumption A1 (gix status on Windows) becomes relevant only if gix is added.

## 7. Settled and open decisions

### 7.1 Password-prompt (askpass) channel: **settled; the gpg part is left open for a test**

| | GitKraken: TCP loopback | Named pipe | Zed: AF_UNIX socket file |
|---|---|---|---|
| How | `127.0.0.1:$PORT` from env; Rust bridge exe | `\\.\pipe\…`; Foreman's peer protocol already uses one | socket in a private temp dir; on Windows the host's own exe is `SSH_ASKPASS` and `ZED_ASKPASS_SOCKET` names the socket |
| Who can connect | any local process of any user: needs a token | default DACL also lets Everyone read: needs an explicit DACL or a token | whoever can open the temp dir, i.e. the user (NTFS ACL) |
| Portability | everywhere | Windows only | Windows 10 1803+ and Unix, same code |
| Risks | firewall/AV heuristics on a listening port; port in env leaks to child processes | a second, Windows-only transport | needs a Windows AF_UNIX implementation (Zed's `net` crate has one); a socket file left behind after a crash |
| Ready-made code | none we may use | client only (`foreman-claude/src/peer.rs`) | Zed `askpass` + `net` (GPL, pinned rev); password kept with `CryptProtectMemory` and zeroised |

**Decision: reuse Zed's `askpass` crate.** It uses an AF_UNIX socket in a private per-session temp dir, and `gitman.exe`/`foreman.exe` answers as `SSH_ASKPASS`/`GIT_ASKPASS` when the socket env var is set. It needs no token, no listening port and no extra binary, and it comes with Zed's `git` crate anyway.

**Left open until a test can run:**
- **gpg signing passphrases on Windows.** Zed's gpg wrapper is Unix-only (`gpg_wrapper_path()` returns `None` on Windows). GitKraken proxies gpg with `--pinentry-mode=loopback`.
- **The order of Git Credential Manager and askpass for HTTPS.** GCM normally prompts with its own UI.

The test matrix is: Git for Windows ssh.exe, Windows OpenSSH, plink, GCM over HTTPS, gpg and ssh signing.

### 7.2 Git executable: **settled as system git; the minimum version is left open for a test**

| | Bundle Git for Windows (GitKraken) | Require system git (Zed on Windows/Linux) |
|---|---|---|
| Size | GitKraken's `git/` folder is **349 MB** (plus LFS, ssh, gpg) | 0 |
| Updates | we must ship every git security fix | the user's update channel |
| Behaviour | can differ from the user's terminal git (version, credential helpers, ssh) | identical to the user's terminal and to Claude Code sessions |
| Failure mode | none at start | git missing or too old: needs a clear screen |
| Licence | GPL-2 binaries to redistribute, with source offer | none |

**Decision: require a system git.** Find it like Zed does: `which` in the repo's environment PATH, then the global PATH. Allow a path override in GitMan's settings. Check the version at start and show a "git missing or too old" screen. Foreman's users already need Git for Windows, because Claude Code on Windows runs on Git Bash. That claim is unverified; check it against Claude Code's install docs. Zed bundles git only in its macOS app bundle.

**Left open until a test can run:** the minimum version. Candidates by feature: porcelain v2 (2.11), `--no-optional-locks` (2.15), `maintenance` (2.29), `rev-list` behaviour Zed relies on. The test runs the git test suite against the oldest supported Git for Windows. The dev machine has 2.44.0.

### 7.3 Still open

1. **The diff viewer before FileMan exists.** Zed's diff stack (`buffer_diff`, `multi_buffer`, `editor`) depends on project and workspace. This is shared with FileMan's extraction question, which autowork-b0 is analysing. Do not build a GitMan-only editor.
2. **Graph design: settled by René (2026-10-28).** GitMan follows **GitKraken's design**:
   - zones and columns;
   - a ref pill zone on the left;
   - avatar nodes and the WIP row;
   - the scroll-marker strip;
   - drag ref onto ref;
   - ghost refs;
   - the pinned lane and its dimming.

   Colours, icons and wording are our own, and C61 still forbids copying GitKraken's assets or code. Zed's `git_graph.rs` is an **implementation base only**: lane bookkeeping, `PathBuilder` curves, `uniform_list` and chunked `rev-list` loading. A unit may fork it or write its own; either way the result looks like GitKraken, not Zed.
3. **The undo journal ref.** Our own, e.g. `refs/worktree/gitman/undo`, with our own trailer format.
