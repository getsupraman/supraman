---
name: supraman-scheduler
description: Set up, list, pause, resume and explain Supraman's scheduled runs of a workspace with `fm schedule` — e.g. "call /autofix-prod-bugs every monday using foreman's scheduler" becomes one `fm schedule add` (trigger, slash command, permission profile, caps), then `fm schedule dry-run` to show the next three fires and the command line; the schedule then waits for the human's Approve in the Supraman window. USE FOR - a request to run a slash command or a prompt on a schedule through Supraman; "what is scheduled", "why did the run fail", "pause/resume that schedule". DO NOT USE FOR - approving a schedule, editing or removing one, registering a Windows task, or running a schedule now (all of them are the human's, in the Supraman window).
allowed-tools: Bash(fm schedule add:*), Bash(fm schedule list:*), Bash(fm schedule show:*), Bash(fm schedule pause:*), Bash(fm schedule resume:*), Bash(fm schedule dry-run:*)
---

# Supraman scheduler

A schedule runs a slash command (or a prompt) in one folder of a Supraman workspace, unattended, at the times
its trigger says. You write it with `fm`; **only the human's Approve on its row in the Supraman window makes it
run**. Until then it is `pending`, and every change to what it runs makes it pending again.

**You run exactly these, with the Bash tool, bare `fm`:**

| Verb | Use |
|---|---|
| `fm schedule add` | create one schedule (§1) |
| `fm schedule dry-run [<w>] <id>` | the prompt, the `claude` command line, the permissions and the next three fires; launches and writes nothing |
| `fm schedule list [<w>]` | every schedule of the workspace: id, status, folder, trigger, next fire, name |
| `fm schedule show [<w>] <id>` | one schedule in full, and its last runs |
| `fm schedule pause [<w>] <id>` | stop it firing; nothing else changes |
| `fm schedule resume [<w>] <id>` | let it fire again; it stays `pending` unless its approval still matches |

`<w>` is the workspace. Your role text names it when you are a Supraman session; otherwise leave it out and
`fm` takes the workspace whose folders hold the current folder (it names the candidates when that is
ambiguous; then name one). No other verb (§4).

## 1. "Call /<command> every monday" → one `fm schedule add`

Work out each part, then ask **once** for everything still open (§2), then add.

1. **The command.** `--command /<name>`. It must exist as `.claude/commands/<name>.md` in the folder the run
   works in (or the user's own commands); `fm` refuses an unknown one and names the folder it looked in.
   Read that file: it tells you what the run will do and so which profile it needs.
2. **The folder.** `--folder <folder>`: the workspace folder that holds the command. Leave it out when the
   current folder is inside it or the workspace has one folder; `fm` says when it needs one
   (`fm workspace show <w>` lists the folders).
3. **The trigger.** `--every "weekly mon 09:00"` (days `mon..sun`, comma-separated: `weekly mon,thu 07:30`),
   `--every "daily 09:00"`, `--every "interval 2h from 08:00"`. Local wall time; DST follows the clock.
   **No time given → propose `09:00` and ask** (§2).
4. **The profile.** `--profile`, chosen from what the command does:

   | Profile | The run may | Where it works |
   |---|---|---|
   | `read` | read and search, run `fm`; anything else parks it as needs-you | the folder's checkout |
   | `worktree-edit` | also edit and write files; `git status/diff/log/add/commit`; push its own run branch | its own git worktree, never the checkout |
   | `auto` | what the auto-mode classifier allows | its own git worktree |

   `read` when the command only reads, reports or triages. **Anything more — it edits, fixes, commits, opens a
   branch or a PR, runs a build — needs `worktree-edit` or `auto`, and only the human picks it: ask (§2),
   never choose it yourself.** `bypassPermissions` is never a schedule's mode; `fm` refuses it. Extra rules
   only when the human names them: `--allow-tools "Bash(dotnet test:*)"`, `--deny-tools <rule>`. A
   `worktree-edit` fix that builds or runs tests needs those commands as `--allow-tools` (the profile has
   git, not the build): say which in §2, e.g. `--allow-tools "Bash(cargo test:*)"`; `dry-run` names it too.
5. **The caps** (unattended runs end at them): always pass `--max-runtime 1h --max-tokens 3000000`, the
   defaults, unless the human gave others (hard limits 8h and 20000000 tokens).
6. **The name.** `--name "<Command words>"` (`--name "Autofix prod bugs"`); the id is its slug
   (`autofix-prod-bugs`). `fm` refuses an id that is taken: `fm schedule list`, then pick another
   `--name`/`--id` or ask whether the existing one is meant.

**On Windows the Bash tool is Git Bash, which rewrites an argument starting with `/` into a path**
(`/fix` → `C:/Program Files/Git/fix`, which `fm` refuses). Write the command with two slashes,
`--command //autofix-prod-bugs`: Git Bash passes it on as `/autofix-prod-bugs`. Elsewhere one slash.

```
fm schedule add --name "Autofix prod bugs" --every "weekly mon 09:00" --command //autofix-prod-bugs --profile worktree-edit --max-runtime 1h --max-tokens 3000000
```

Then, with the id it printed:

```
fm schedule dry-run <id>
```

**Report to the human**, from what the two commands printed and nothing else:

- the schedule: id, workspace, folder, trigger, command, profile, caps;
- the **next three fires**, exactly as `next fires:` printed them;
- the **command line**: the whole `claude` argv as printed, in one code block, and the working folder (`cwd:`);
- **"It is pending: it runs only after you Approve it on its row in the Supraman window."**

## 2. Asking

Ask before `fm schedule add`, once, with everything still open and your proposal for each:

```
To schedule /autofix-prod-bugs every Monday I need:
- the time: 09:00? (local)
- the profile: it fixes bugs, so it edits files — worktree-edit (edits in its own worktree) or auto? read would park it.
Caps: 1h runtime, 3000000 tokens unless you want others.
```

Ask the one who asked you, the way you reach them: in your reply when the request came in your prompt; by
`SendMessage` to its sender when it came in a message (a repo session asks `fm-<w>-workspace`, which asks the
human). End the turn with the question and wait; do not add anything meanwhile. An answer like "your call" or
"defaults" settles the time (`09:00`) and the caps, **never** a profile above `read`.

Nothing is open (time given, the command only reads) → add without asking.

## 3. List, show, explain, pause, resume

- **"What is scheduled?"** `fm schedule list [<w>]`; one line per schedule: id, status, folder, trigger,
  next fire, name.
- **"Explain <id>" / "how did its last runs go?"** `fm schedule show [<w>] <id>`. Say in words when it fires,
  what it runs, with which profile and caps, and its status:
  - `pending` — waiting for Approve in the Supraman window, or changed since its approval
    (`(changed since approval)`): it does not fire until approved again;
  - `active` — approved, fires on its trigger;
  - `paused` — kept, does not fire.

  Then its `last runs` (newest first: run id, start, how it fired, outcome):

  | Outcome | Means |
  |---|---|
  | `done`, `ok` | the run finished its work |
  | `nothing_to_do` | its gate script found nothing, no session ran |
  | `skipped` | not run: an earlier run was still going, or no session slot was free |
  | `needs_you` | the run stopped to ask; it is in Supraman's inbox |
  | `failed`, `refused` | it could not run or start; the text after the colon says why |
  | `capped_runtime`, `capped_tokens` | stopped at its cap |
  | `interrupted` | fm died mid-run (sleep, logoff, crash); the next fire cleaned it up |
  | `running` | started, not ended yet |
- **"Pause <id>."** `fm schedule pause [<w>] <id>`. **"Resume <id>."** `fm schedule resume [<w>] <id>`; when it
  answers `pending`, tell the human it needs Approve in the Supraman window before it fires again.

## 4. Never

| Never | Instead |
|---|---|
| approve a schedule, or say it is approved before `list`/`show` says `active` | the human's Approve in the Supraman window |
| edit, write or delete a schedule's TOML (`<home>/schedules/<w>.toml`) or a run record | `fm schedule …` only |
| `fm schedule edit` or `fm schedule remove` | tell the human: the Supraman window (or they run `fm schedule edit`/`remove` themselves) |
| `schtasks`, `Register-ScheduledTask`, Task Scheduler, any Windows task | Supraman registers the task after Approve |
| choose `worktree-edit` or `auto` without the human's answer | §2 |
| run a scheduled command now to try it | `fm schedule dry-run` |
