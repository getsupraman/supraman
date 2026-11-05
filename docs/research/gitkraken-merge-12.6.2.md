# GitKraken 12.6.2: merge view, merge editor and auto-merge

Research for GitMan's merge feature (Workman: supraman/gitman-merge). It follows `gitkraken-12.6.1.md` and uses the same method: the installed app's `app.asar` was extracted and its renderer bundle beautified and read. The source is `%LOCALAPPDATA%\gitkraken\app-12.6.2\resources\app.asar`.

**C61 applies.** This file describes behaviour and the git commands GitKraken runs. GitMan copies none of GitKraken's code, strings, icons, assets or packages. Where an algorithm is named, GitMan takes a public one (git's own or a crate), not GitKraken's.

Each claim below was read in the bundle. Things that could not be determined are marked **(not verified)**.

---

## 1. Where merging happens

| Surface | What it is |
|---|---|
| Merge op | Drag a branch onto another, or use the context menu's merge (ff / no-ff / ff-only / squash). It runs `git merge` and has a separate git-flow path. |
| Conflict WIP node | A dedicated graph row ("merge conflict" node type) replaces the WIP row while the index has unmerged entries. It shows a "N conflicted files" message and selects itself automatically. |
| Commit panel in merge state | The commit button turns into *Commit and Merge* (disabled until something is staged) plus *Abort Merge*. For a rebase it is *Continue* or *Skip* plus *Abort Rebase*. |
| Conflicted file row menu | Open in built-in merge tool; Open in external tool (if configured); Take current (ours) for the whole file; Take incoming (theirs) for the whole file; Mark as resolved (= stage). |
| Built-in merge tool | A three-pane panel: A (ours) on the left, B (theirs) on the right, Output below. |
| External merge tool | Launched with temp files for base, ours and theirs. |
| Conflict prevention | Predicts conflicts against a detected base branch before you merge (§7). |
| AI auto-resolve | One button that resolves every conflict hunk in the open file (§6). |

## 2. The merge operation (git CLI path)

### 2.1 Ordinary merge (`_gitMerge`)

1. Takes locks on the index, `.gitattributes` and the repo settings. The UI blocks other writers meanwhile.
2. **Autostash** of the dirty work tree, with its own stash message, unless the merge is silent or part of a pull.
3. If the target branch is not the checked-out one, it **checks out the target first** (recursing into submodules when the user enables that). If the checkout fails it runs `reset --hard HEAD`, reapplies the stash, and throws.
4. Builds the merge options from the user's preference:
   - none → plain `git merge`
   - no-ff → `--no-ff`
   - ff-only → `--ff-only`
   - squash → `--squash`
5. Adds `-c merge.suppressDest=<git-flow master branch>` when a git-flow config exists.
6. Runs `git merge <source>` through the askpass/remote wrapper, so a gpg prompt or a credential prompt can appear.
7. Output containing `Already up to date.` → reports "up to date".
8. **Squash:** after the merge it stages everything and stops. The user commits.
9. After a merge that switched branches, it **switches back** to the original branch, unless HEAD is now detached.
10. Reapplies the autostash (drop on success). A failed reapply is reported separately ("unstash failed", with or without conflicts).
11. Re-runs LFS setup if `.gitattributes` changed, then updates submodules.
12. Shows a success toast reading *merged X into Y*.

**Failure branches:**
- gpg cancelled, gpg not found, key unusable, no smartcard or no passphrase → `git merge --abort`, switch back, reapply the stash, clear the pending commit message.
- Output contains `CONFLICT` while the merge was done on a branch that was **not** the original HEAD → it **asks** whether to stay on the target with the conflicts (stash kept) or abort. Abort means `merge --abort`, switch back, reapply the stash.
- Otherwise the conflict error leaves the repo in merge state with `MERGE_MSG` prepared, and the conflict node appears.

The nodegit fallback has the same shape but decides itself: up to date / fast-forward / real merge (from the merge state), refusal under ff-only, and an error when there is no common ancestor.

### 2.2 The git-flow merge (`mergeWithMergeCommit`): the stricter path

This path is worth copying as behaviour because it isolates the merge from the user's config:

- `update-ref ORIG_HEAD HEAD`.
- **Pre-check for work-tree loss:** counts the staged changes (`diff --cached --name-status`, rename detection off, submodules included) plus the work-tree changes the merge would overwrite. When the count is above zero it runs `merge --quit` and refuses with "N uncommitted changes would be overwritten by merge".
- Clears stale `CHERRY_PICK_HEAD` and `REVERT_HEAD`.
- When the source is already an ancestor, it writes `MERGE_HEAD` and `MERGE_MODE=no-ff` itself and creates the merge commit anyway (no-ff semantics).
- Otherwise it runs `git merge --no-commit --no-ff --allow-unrelated-histories -X <strategy options>` with these **pinned settings** (`-c`):
  - `merge.renameLimit=<merge.renamelimit | diff.renamelimit | 1000>`
  - `merge.renames=true`
  - `pull.twohead=ort`
  - `merge.autoStash=false`
  - `rerere.enabled=false`
  - `merge.directoryRenames=false`
  - `merge.verifySignatures=false`
  - `merge.renormalize=false`
  - `branch.<target>.mergeOptions=` (emptied, so the user's per-branch merge flags do not apply)
- Deletes `AUTO_MERGE`, restores files the merge dropped by mistake ("missing files"), writes `MERGE_MSG`.
- Builds the commit itself: `commit-tree` from the index with parents `[target, source]`, UTF-8 encoding, hooks off, signing through the askpass prompt if `commit.gpgsign`.
- Then `update-ref HEAD <new> <old>` with the reflog message `commit (merge): <summary>`, and `merge --quit`.
- Removes the work trees of submodules the merge dropped.

**On conflict (`handleFailedMerge`):**
- Reads the unmerged paths from `ls-files -u -z`.
- Writes `MERGE_MSG` as `<merge message>` + blank line + `#Conflicts:` + one path per line.
- **Rewrites the conflict markers in the work-tree files** (relabel):
  - The theirs label (`>>>>>>> <sha>`) becomes the source branch name. It keeps a `:<path>` suffix for renamed files.
  - The base label (`|||||||`) gets the merge base's commit summary when the file has a base.
  - The marker length honours the `conflict-marker-size` attribute (markers are resized when it is not 7).
  - Files with `merge=binary` or `merge=unset` are left alone.
  - The file is read and written as latin1, so the bytes pass through unchanged.
- Restores missing files except the conflicted ones.

**Rebase conflicts** get the same relabel treatment:
- `<<<<<<< HEAD` becomes the upstream branch name.
- The `parent of <sha>` and `<sha> (<summary>)` labels become readable.
- `drop_redundant_commits` is written into `rebase-merge/`.

### 2.3 Conflict detection inside git

These readers are used, and nothing else:
- `ls-files -u -z` for all unmerged entries;
- per path, stages 1, 2 and 3 with mode and blob id;
- `cat-file` for each stage's blob.

GitKraken does **not** parse the conflict markers in the work-tree file to build its editor (§3.1).

## 3. The built-in merge editor

### 3.1 How a conflicted file is loaded (`generateDiff3`)

1. Read stages 1/2/3 for the path (ancestor, ours, theirs). **The work-tree file is ignored.** Edits the user already made to the conflicted file are not shown and are overwritten on save **(an edge case GitMan should handle better, §8)**.
2. **One side missing** (modify/delete, add/delete):
   - From the editor: a prompt.
     - With a base: *keep modified* (write the existing side), *remove* (delete the file and stage it), *keep base* (write the ancestor).
     - Without a base: *keep new* or *remove*.
     - Cancel is always offered.
     - The message names which branch deleted the file and which modified it, using the merge's two branch names.
   - From *Take current / Take incoming*: no prompt. It writes that side, or deletes the file and stages the deletion when that side is the missing one.
   - The file mode comes from the chosen stage. `chmod` runs on non-Windows only.
3. **Binary mix check:** if base, ours and theirs do not agree on binary vs text, the in-app merge refuses ("cannot solve with in-app merge"). The user goes to the external tool or take-side.
4. **Encoding:**
   - Each side is decoded with the repo's encoding setting or a per-file override. The first side that decodes as text fixes the encoding for all three.
   - The panel header has an encoding picker; changing it re-runs the load.
   - The chosen encoding is used again when writing.
5. **Line endings:**
   - **Each side is split on its own detected line ending.** A file that is CRLF on one side and LF on the other therefore conflicts on every line **(GitKraken does nothing about it; GitMan should normalise, §8)**.
   - On write, the ending comes from the current work-tree file and `core.autocrlf`. For an LFS file it is detected from the work-tree bytes. A missing file falls back to the autocrlf default.
6. **Binary, large and images:**
   - Nothing decodes as text, or any side is over **10 MB** (`maxLoadFilesize = 1e7`) → a **binary model**: pick a whole side. The output is the base until a side is picked.
   - All present sides are images → an **image model**: the three images side by side and a side picker.
   - Saving a binary or image model with no side picked and **no base** is refused (nothing to write).
7. **Own three-way line merge.** GitKraken re-merges the three texts itself, in a **shared web worker**:
   - The algorithm is classic LCS-based diff3: an LCS diff of base↔ours and base↔theirs, overlapping hunks grouped, and *false conflicts* (both sides made the same change) folded into clean text.
   - The result is a list of **chunks**, each either `ok` (clean lines) or `conflict {ours, base, theirs}` with indices.
8. **Auto-resolve:** if that re-merge comes out with **no conflict chunk**, GitKraken does not open the editor. It:
   - writes the merged text with the file's line ending and encoding;
   - stages the path;
   - shows a short "auto-resolved" toast.

   This catches files that git marked conflicted but whose two sides made the same change, or that line up differently under this merge's LCS. **Risk:** two merge algorithms can disagree, so a file git called conflicted can be resolved silently (§8).
9. A **sha256 over the conflict chunks** is the model's identity. A reload only replaces the in-memory model when that hash or the encoding changed. This keeps the user's in-progress picks across refreshes that change nothing.

### 3.2 The model

The file is a list of hunks in order.

- A **clean** hunk has identical left, right and output lines.
- A **conflict** hunk has:
  - its `conflictIndex`;
  - `ancestor`, `left` and `right` lines (each with side, line index and an in-output flag);
  - `output` (the lines picked or typed, initially empty);
  - `hasBeenEdited`;
  - `size` (the tallest of its four columns, so the three panes stay aligned).
- **An empty side gets one placeholder line** (flagged empty). A deletion can still be ticked and taken; it contributes nothing to the output.
- **An unresolved conflict hunk shows the ancestor's lines in the Output pane** (greyed) until something is picked or typed. It is not shown as empty.
- **Operations:**
  - tick or untick one line;
  - take or untake one side of a hunk;
  - take or untake a whole side (header check box);
  - clear one hunk or all hunks;
  - reset to the state at load (enabled only when something changed);
  - reset to the state before AI.
- **Order of picked lines:**
  - Lines enter the output **in the order they were picked**. Ticking a right line and then a left line puts right first.
  - *Take side* first removes that side's lines from the output, then appends all of them. So *take A, take B* gives A then B, and *take B, take A* gives B then A.
  - This is how a user builds "both, in my order".
- **Typing in Output:**
  - Any edit inside a conflict hunk first copies the ancestor into the output (if it was still empty), then applies the edit and marks the hunk edited.
  - Edits are mapped to hunks through a line map from output row to (hunk, line). An edit that spans hunks touches each of them.
  - A typed line that equals an unmodified line keeps the "unmodified" type.
- **Undo and redo are disabled in the Output editor** (the model's undo and redo are no-ops). Only *Reset* goes back **(GitMan should keep undo, §8)**.
- **The free tier gets a read-only Output.** The user can only tick and take; typing needs a paid licence. *(Pricing behaviour, for reference only.)*

### 3.3 Panes and navigation

- **Header:** the file path with a warning icon, "N conflicts", an encoding picker, a word-wrap toggle, the AI button, *Open in <external tool>* when configured, **Save**, and Close.
- **Side headers:**
  - **Merge:** A = the merge's base branch name with HEAD's commit; B = the merge head branch with `MERGE_HEAD`'s commit.
  - **Rebase:** A = the onto branch with HEAD's commit; B = the commit being applied.
  - **Cherry-pick:** A = HEAD's branch; B = `CHERRY_PICK_HEAD`'s commit.
  - **Other states with conflicts** (stash apply, revert): A = HEAD only, B blank.
  - Each header has a take-all check box. It shows ticked when the whole side is in the output, and clicking it again untakes.
- **Output header:** previous/next conflict buttons with the current index, *Reset merge* (shown when reset is possible or an AI result is shown), and an AI summary toggle.
- **Opening** jumps to the first conflict.
- **Scroll sync:**
  - Vertical sync is **proportional per hunk**: the scrolled pane's position becomes (hunk index, percent within hunk), and the other two panes go to the same hunk at the same percent. Panes of different heights stay aligned hunk by hunk.
  - Horizontal scroll is mirrored directly.
  - Scrolling also updates the current conflict index.
  - A re-entrancy flag stops the panes from echoing each other.
- **Lazy widgets:** the per-line and per-hunk check box widgets are attached only for rows in view (plus one row either side) and detached when they scroll out.
- **Editor settings follow the user's profile:** font family and size, tab size, line numbers, syntax highlighting (language from the file path), word wrap.
- **The Output editor is hardened:**
  - a few multi-cursor and line-move key bindings are disabled;
  - the context menu loses rename symbol, change all occurrences, go to references, peek and the command palette;
  - alt-click (multi-cursor) is blocked.

### 3.4 Save (`persistMergeToDisk`)

1. Re-check that the repo is still the open one.
2. Unresolved conflicts left:
   - **Text model:** a three-way prompt.
     - *Save output*: unresolved hunks are written as their **ancestor** text.
     - *Save with conflicts*: unresolved hunks are written as `<<<<<<<` / ours / `=======` / theirs / `>>>>>>>` with **no labels and no base section**.
     - Cancel.
   - **Binary model:** a confirm, "save the ancestor".
3. Write:
   - text: the joined result with the line ending (§3.1.5) in the file's encoding;
   - binary: the chosen blob;
   - image: the chosen buffer.
4. Delete the `<file>~*` leftovers next to the file (git's `file~ours` style artifacts from rename and directory conflicts).
5. If the repo is in cherry-pick state, clean up the cherry-pick state **(on every save in that state, not only the last file; not verified as intended)**.
6. **Stage the path. Saving always marks the file resolved, even when it was saved "with conflicts" (§8).**

### 3.5 Take current / Take incoming (whole file, many files)

For each selected path:
- reuse the open model (reset first), or load it with the take-side strategy;
- take that side for every hunk;
- save without prompts, then stage.

Missing-side files are written or deleted as in §3.1.2.

### 3.6 Mark as resolved

This is `git add <path>`. There is **no check for leftover conflict markers** (§8).

## 4. The external merge tool

- Settings name a tool (detected tools are listed) or "git config default".
- A `trustExitCode` flag comes from the tool's details.
- The temp files go **next to the file**: `<file>-base`, `<file>-ours`, `<file>-theirs`. The base is an **empty file** when there is no ancestor. Blobs are written with git's smudge, so filters apply.
- **Both sides present:**
  - It runs the tool's command line with a fixed-up environment.
  - Exit code 127 → "failed to launch <tool>".
  - **Trusted exit code:** success means exit 0.
  - **Otherwise:** success means the file's bytes changed from before the launch.
  - On success it stages the path.
- **One side missing:** the same keep modified / remove / keep base prompt as §3.1.2. The work-tree file is replaced by the chosen temp file (copy).
- **Always:** the three temp files are deleted, and `<file>.orig` too when the "delete .orig after merge" setting is on.
- A missing or invalid tool configuration offers to open the tool settings.

## 5. The conflict WIP node and commit

- The node type is "merge conflict". It counts the conflicted files and is auto-selected when conflicts appear. The commit message box is prefilled from `MERGE_MSG`, and a merge-message hook can run.
- **Commit and Merge** commits the staged index (the merge commit). It is disabled until something is staged.
- **Abort Merge** runs `git merge --abort`. The rebase equivalents are Continue, Skip and Abort.
- Errors are mapped to readable messages: "merge conflicts", "rebase conflicts", and stash re-apply conflicts.

## 6. AI auto-resolve

- One button resolves **all** conflicts in the open file. (The code also supports a single conflict index, but the UI passes none.)
- **Gate:**
  - AI must be available (plan, sign-in, verified e-mail, trial);
  - a **per-profile consent to send code** is checked before any request;
  - when the user is not entitled, a trial or sign-up flow opens instead.
- **Request:**
  - for each conflict hunk: the ancestor, head (ours) and incoming (theirs) code;
  - plus context: `diff-tree` with **0 context lines** of the source commit (merge head, rebase head or cherry-pick head) against HEAD for this file, keeping only the hunks that overlap the conflict lines;
  - plus metadata: the file's line count, the hunk count, the diff's line count and the language.
  - When there is no source sha it shows an info toast and stops.
- **Response:** per conflict, the resolved code, a confidence (0–1, shown as %) and an explanation.
- **Apply:**
  - It resets to the loaded state (remembering the pre-AI state).
  - Each resolved line is matched against that hunk's left and right lines (by content, last occurrence first). A match is taken as a pick of that side's line. A line that matches neither becomes an edited line.
  - Every line is tagged "AI generated", and an **AI summary panel** lists confidence and explanation per conflict.
  - *Reset* goes back to the pre-AI state.
- Nothing is saved automatically. The user still clicks Save.

## 7. Conflict prevention (before merging)

- `git merge-tree --write-tree --name-only --no-messages -z --stdin` over **ref pairs** (several pairs in one call), parsed per pair into conflicted or clean plus the conflicted paths.
- **The base branch is detected** first from the reflog (`branch: Created from …`), keeping it only if it still matches the ref patterns. Otherwise a "likely parent" lookup: the nearest ref from the user's include and exclude patterns, excluding upstreams.
- Results are cached per (ours, theirs) pair. A conflict on the current HEAD's pair raises the indicator.
- The full stages for a preview come from `merge-tree --write-tree --allow-unrelated-histories -X …`.
- **Preview isolation:** every configured custom merge driver is overridden with `-c merge.<name>.driver=exit 1`, so the preview never runs user code.
- A team feature, *remote conflict detection*, uploads the WIP diff to GitKraken's server when the org allows it. **Out of scope for GitMan.**

## 8. Edge cases: what GitKraken handles, and what GitMan should do better

| # | Case | GitKraken | GitMan should |
|---|---|---|---|
| 1 | Both modified (text) | 3-pane editor | same |
| 2 | Both added, no base | 3-pane with empty ancestor | same; base pane shows "no common ancestor" |
| 3 | Modified/deleted, either way | prompt: keep modified / remove / keep base | same, plus show the diff of the kept side |
| 4 | Added by one side only, conflicting with a deletion | prompt: keep new / remove | same |
| 5 | Rename/rename, rename/delete, directory rename | nothing specific; `merge.directoryRenames=false` in git-flow; `~ours` artifacts deleted on save | show rename info from `merge-tree` messages / `ls-files -u` stages under two paths; let the user pick the path. **Gap in GK.** |
| 6 | Submodule (gitlink) conflict | none found **(not verified)** | pick a commit for the gitlink (ours / theirs / other); never open the text editor |
| 7 | Symlink conflict | mode kept on non-Windows; content written as a file **(not verified)** | treat as side-pick; on Windows respect `core.symlinks` |
| 8 | Mode-only conflict (exec bit) | mode from the chosen stage | show the mode change; pick |
| 9 | Binary / mixed binary-text / >10 MB | side picker; mixed refused | same; the threshold is a setting |
| 10 | Images | side by side + picker | same (gpui image); later an onion-skin/diff view |
| 11 | LFS files | stage blobs are pointers; line ending from the work tree **(not verified whether the editor shows pointers)** | smudge the stages through `git lfs smudge` / `cat-file --filters` before showing; pick-side for binaries |
| 12 | `merge=binary` / `merge=union` / custom drivers / `conflict-marker-size` | honoured on relabel; drivers neutralised in the preview | read `check-attr`; union needs no editor; a custom driver's result is the starting output |
| 13 | CRLF on one side, LF on the other | every line conflicts | normalise endings before the 3-way merge, write in the work tree's convention (`core.autocrlf`, `eol` attr) |
| 14 | Different encodings / BOM | one encoding for all three, picker to reload | detect per side; `supraman_core::text::decode`; keep the BOM on write |
| 15 | No trailing newline on one side | not special-cased | keep git's "\ No newline" semantics; never add or drop the final newline silently |
| 16 | User already edited the conflicted file | **ignored and overwritten** (built from stages) | start from the work-tree file: parse its markers (any length, with or without diff3 base) into hunks; fall back to stages when the markers are gone or broken |
| 17 | File re-merges cleanly with the app's own algorithm | silently written + staged + toast | do not silently resolve: use git's own merge result (`git merge-file -p --diff3 --marker-size N` on the three stages), so GitMan never disagrees with git; offer "accept clean result" as an explicit action |
| 18 | Same change on both sides inside a conflict | folded as clean | same (git does this too with zealous/zdiff3) |
| 19 | Save with unresolved hunks | prompt: base text, or markers without labels; **then stages anyway** | write markers **with labels** (and the base with diff3), and **do not stage** while markers remain |
| 20 | Mark resolved with markers left | stages blindly | check for conflict markers (`git diff --check`, or a scan with the attribute's marker size) and warn |
| 21 | Undo in the output | disabled; only Reset | full undo/redo in the editor; Reset as well |
| 22 | Line-pick order | output in pick order | same, plus visible "move up/down" or drag in the output |
| 23 | Conflict state other than merge (rebase, cherry-pick, revert, stash apply, am) | headers per state; stash/revert headers blank | header per state from `REBASE_HEAD` / `CHERRY_PICK_HEAD` / `REVERT_HEAD` / stash ref; continue/skip/abort per state |
| 24 | Merge started from a branch that is not HEAD | checkout target; on conflict ask: stay or abort and return | same; and refuse when the target is checked out in another worktree |
| 25 | Dirty work tree | autostash / pre-count of overwritten changes | pre-check first (`merge-tree` + changed paths) and ask; an autostash is an option, not a default |
| 26 | User config surprises (rerere, autostash, mergeOptions, verifySignatures, renormalize) | pinned with `-c` | pin the same set; rerere as a user setting (GitKraken turns it off) |
| 27 | gpg signing | askpass prompt; abort on cancel | share the askpass channel (§7.1 of the 12.6.1 report) |
| 28 | Hooks | merge-message hook; post-merge hook; hooks off for commit-tree in git-flow | run the user's hooks on the merge commit (default on) |
| 29 | Huge conflict count / huge file | lazy widgets; proportional scroll | Zed editor with blocks per hunk, only rows in view; conflict list virtualised |
| 30 | Repo changed while the editor is open | "repo changed" errors; model kept per path by hash | watch the stages (index mtime); when the stages change, reload and say so; keep picks if the hash is unchanged |
| 31 | Sparse checkout / skip-worktree | index bits preserved on unstage | keep the bits on stage/unstage |
| 32 | Case-only path conflicts on Windows | none found | detect and warn |

## 9. What GitMan needs to be feature complete

### Git layer
GitMan's read layer is read-only by rule (C146). Merge is the first write, so it lives under **supraman/gitman-actions**.

- **Merge op:** ff / no-ff / ff-only / squash, `-X ours|theirs|ignore-space-change|…`, allow unrelated histories, the pinned `-c` set (§2.2), switching target and returning, an optional autostash.
- **Abort / continue / skip / quit** for merge, rebase, cherry-pick, revert, stash apply and am.
- **Unmerged stages:** `ls-files -u -z` (mode, blob, stage, path) and `cat-file --filters` per stage (smudged content). Missing stages tell the conflict kind.
- **Clean-merge text per file:** `git merge-file -p --diff3 -L <ours label> -L <base label> -L <theirs label> --marker-size N ours base theirs` on temp files, or `merge-tree --write-tree` for the whole tree. Parse the markers into hunks. This keeps GitMan identical to git and needs no merge algorithm of its own. An in-process 3-way merge, if ever needed, uses a public crate, never GitKraken's worker.
- **Attributes:** `check-attr merge conflict-marker-size eol text binary` per path.
- **Resolve:** write the file (encoding, line ending, BOM, mode), then `git add` / `git rm` / `update-index --cacheinfo` for gitlinks.
- **Conflict prevention:** `merge-tree --write-tree --name-only -z --stdin` over pairs, base branch detection (reflog `Created from`, then the nearest ref), a cache per pair, custom drivers neutralised.
- **Merge commit:** `git commit` with `MERGE_MSG` (honouring hooks and signing) by default. The "plumbing" path (`commit-tree` + `update-ref`) only where hooks must not run.

### UI
- A conflict node in the graph (replacing WIP), conflicted files in the commit panel with a conflict-kind badge per file, and the panel's Commit and Merge / Abort buttons per state.
- **Merge editor tab** (one per path) with three panes on `supraman_editor` and the Output **editable**. That is new: `supraman_editor` is read-only today, so an editable variant is a shared-component extension, not a private editor.
- **Per hunk:** take ours / theirs / both (ours then theirs) / both (theirs then ours) / base, and per-line ticks.
- **Whole side:** take all ours / theirs.
- Previous/next conflict with a counter, proportional per-hunk scroll sync, and the base shown on demand (diff3 view).
- Encoding and line-ending indicators with a reload.
- Save, which stages only when no markers are left; Save with markers; Reset; undo.
- **Side-pick view** for binary, image, submodule, symlink and mode conflicts.
- **Delete/modify chooser** (keep ours / keep theirs / keep base / delete) with a preview.
- **External merge tool** (later): temp files beside the file, launch, trust exit code or compare bytes, cleanup (`.orig` too).
- **Headers** per state, with branch names and commit subjects.
- **Conflict prediction indicator** on the branch and the graph (later).

### Explicitly out of scope / later
- AI resolution. If wanted, it goes through Claude sessions Supraman already supervises, with the same inputs (base/ours/theirs per hunk + incoming diff), confidence and an explanation per hunk, never auto-saved.
- Remote/team conflict detection.
- GitKraken's paid-tier gating.

## 10. Test fixtures GitMan needs (`supraman_gitman::fixture`)

- Both modified, with 1 and with many hunks.
- Both added.
- Modify/delete, each way.
- Rename/rename and rename/delete.
- Submodule pointer conflict.
- Symlink vs file.
- Exec-bit-only conflict.
- Binary conflict, and binary vs text.
- An image conflict.
- An LFS pointer conflict.
- CRLF vs LF, and UTF-16 vs UTF-8.
- No final newline on one side.
- `conflict-marker-size=12`.
- `merge=union`.
- A custom merge driver configured.
- A user-edited conflict file.
- A conflict whose two sides made the same change inside a larger change.
- 10,000 conflicts in one file.
- Each of: merge, rebase, cherry-pick, revert, stash apply.
- A merge started from a branch that is not HEAD.
- A dirty work tree the merge would overwrite.
