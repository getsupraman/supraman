---
name: supraman-scheduler
description: Schedule recurring work that runs on this machine through Supraman with `sm schedule` — "every monday", "daily", "weekly", "at 7", "schedule /replay-failures", "run X every night", any recurring job that needs this repo's files, its git-ignored secrets (.env), Docker or other local tools. Use this, not the built-in schedule skill (cloud routines, which cannot see this machine) and not a hand-made Windows task. One `sm schedule add` (trigger, slash command, permission profile, caps), then `sm schedule dry-run` for the next three fires and the command line; the schedule waits for the human's Approve in the Supraman window. USE FOR - running a slash command or a prompt on a schedule on this machine; "what is scheduled", "why did the run fail", "pause/resume that schedule"; changing a schedule you added in this conversation at the human's request. DO NOT USE FOR - approving a schedule, removing one, editing one you did not add in this conversation, registering a Windows task, or running a schedule now (all of them are the human's, in the Supraman window).
allowed-tools: Bash(sm schedule add:*), Bash(sm schedule edit:*), Bash(sm schedule list:*), Bash(sm schedule show:*), Bash(sm schedule pause:*), Bash(sm schedule resume:*), Bash(sm schedule dry-run:*)
---

# Supraman scheduler

A schedule runs a slash command (or a prompt) in one folder Supraman lists, unattended, at the times
its trigger says. You write it with `sm`; **only the human's Approve on its row in the Supraman window makes it
run**. Until then it is `pending`, and every change to what it runs makes it pending again.

**You run exactly these, with the Bash tool, bare `sm`:**

| Verb | Use |
|---|---|
| `sm schedule add` | create one schedule (§1) |
| `sm schedule edit <id> <options>` | change a schedule **you added in this conversation at the human's request**, with `add`'s options (§1); it goes `pending` again |
| `sm schedule dry-run <id>` | the prompt, the `claude` command line, the permissions and the next three fires; launches and writes nothing |
| `sm schedule list` | every schedule of the folder: id, status, trigger, next fire, name |
| `sm schedule show <id>` | one schedule in full, and its last runs |
| `sm schedule pause <id>` | stop it firing; nothing else changes |
| `sm schedule resume <id>` | let it fire again; it stays `pending` unless its approval still matches |

Each verb works on one listed folder's schedules: the deepest listed folder holding the current folder, or
the one holding `--folder <dir>` when you pass it (any verb takes it). `sm` says when no listed folder holds
it; `sm folder list` lists them. No other verb (§4).

## 1. "Call /<command> every monday" → one `sm schedule add`

Work out each part, then ask **once** for everything still open (§2), then add.

1. **The command.** `--command /<name>`. It must exist as `.claude/commands/<name>.md` in the folder the run
   works in (or the user's own commands); `sm` refuses an unknown one and names the folder it looked in.
   Read that file: it tells you what the run will do and so which profile it needs.
2. **The folder.** The run works in the listed folder that holds the command: run `sm` in it (or below), or
   pass `--folder <dir>`. `sm` names the folder it took in what it prints.
3. **The trigger.** `--every "weekly mon 09:00"` (days `mon..sun`, comma-separated: `weekly mon,thu 07:30`),
   `--every "daily 09:00"`, `--every "interval 2h from 08:00"`. Local wall time; DST follows the clock.
   **No time given → propose `09:00` and ask** (§2).
4. **The profile.** `--profile`, chosen from what the command does:

   | Profile | The run may | Where it works |
   |---|---|---|
   | `read` | read and search, run `sm`; anything else parks it as needs-you | the folder's checkout |
   | `edit` | also edit and write files; `git status/diff/log/add/commit`; no `git push` | the folder's checkout |
   | `auto` | what the auto-mode classifier allows | the folder's checkout |

   Every run works in the folder's checkout, the human's working copy, where its git-ignored files (`.env`)
   and tools are. A job that needs a separate git worktree makes one itself; Supraman makes none.

   `read` when the command only reads, reports or triages. **Anything more — it edits, fixes, commits, runs a
   build, Docker or `dotnet run` — needs `edit` or `auto`, and only the human picks it: ask (§2), never choose
   it yourself.** `bypassPermissions` is never a schedule's mode; `sm` refuses it. Extra rules only when the
   human names them: `--allow-tools "Bash(dotnet test:*)"`, `--deny-tools <rule>`. An `edit` run that builds,
   runs tests or starts containers needs those commands as `--allow-tools` (the profile has git, not the
   build). `dry-run` prints a `hint:` line for each such tool the command names
   (`hint: the run calls docker; add --allow-tools "Bash(docker:*)"`).
5. **The caps** (unattended runs end at them): always pass `--max-runtime 1h --max-tokens 3000000`, the
   defaults, unless the human gave others (hard limits 8h and 20000000 tokens).
6. **The name.** `--name "<Command words>"` (`--name "Autofix prod bugs"`); the id is its slug
   (`autofix-prod-bugs`). `sm` refuses an id that is taken: `sm schedule list`, then pick another
   `--name`/`--id` or ask whether the existing one is meant.

**On Windows the Bash tool is Git Bash, which rewrites an argument starting with `/` into a path**
(`/fix` → `C:/Program Files/Git/fix`; `sm` refuses it and names the fix). Write the command with two slashes,
`--command //autofix-prod-bugs`: Git Bash passes it on as `/autofix-prod-bugs`. Elsewhere one slash.

```
sm schedule add --name "Autofix prod bugs" --every "weekly mon 09:00" --command //autofix-prod-bugs --profile edit --max-runtime 1h --max-tokens 3000000
```

Then, with the id it printed:

```
sm schedule dry-run <id>
```

**Report to the human**, from what the two commands printed and nothing else:

- the schedule: id, folder, trigger, command, profile, caps;
- the **next three fires**, exactly as `next fires:` printed them;
- the **command line**: the whole `claude` argv as printed, in one code block, and the working folder (`cwd:`);
- every `hint:` line `dry-run` printed, as a question: "add `--allow-tools "Bash(docker:*)"`?"
  **Pass the hint on; never add the rule yourself.** On the human's yes, `sm schedule edit <id> --allow-tools "<rule>"` (§3);
- **"It is pending: it runs only after you Approve it on its row in the Supraman window."**

## 2. Asking

Ask before `sm schedule add`, once, with everything still open and your proposal for each:

```
To schedule /autofix-prod-bugs every Monday I need:
- the time: 09:00? (local)
- the profile: it fixes bugs, so it edits files — edit (edits and commits in the folder's checkout, no push) or auto? read would park it.
- the tools: it runs `docker compose up` — add --allow-tools "Bash(docker:*)"?
Caps: 1h runtime, 3000000 tokens unless you want others.
```

Name each tool the command file calls (`dotnet`, `docker`, `npm`, `pnpm`, `cargo`, `python`) with the
`--allow-tools` rule `dry-run` would hint, and add the rule only when the human says yes to it.

Ask the one who asked you, the way you reach them: in your reply when the request came in your prompt; by
`SendMessage` to its sender when it came in a message. End the turn with the question and wait; do not add anything meanwhile. An answer like "your call" or
"defaults" settles the time (`09:00`) and the caps, **never** a profile above `read`.

Nothing is open (time given, the command only reads) → add without asking.

## 3. Edit, list, show, explain, pause, resume

- **"Change <id>" / the human's answer to a hint.** Only for a schedule **you added in this conversation at the
  human's request**: `sm schedule edit <id>` with the options that change (`--every "daily 07:00"`,
  `--allow-tools "Bash(docker:*)"`, `--profile edit` on the human's answer, §2). The id cannot change. It goes
  `pending` again: tell the human it runs only after their Approve in the Supraman window. Any other schedule
  is the human's (§4).

- **"What is scheduled?"** `sm schedule list`; one line per schedule of the folder: id, status, trigger,
  next fire, name.
- **"Explain <id>" / "how did its last runs go?"** `sm schedule show <id>`. Say in words when it fires,
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
  | `interrupted` | sm died mid-run (sleep, logoff, crash); the next fire cleaned it up |
  | `running` | started, not ended yet |
- **"Pause <id>."** `sm schedule pause <id>`. **"Resume <id>."** `sm schedule resume <id>`; when it
  answers `pending`, tell the human it needs Approve in the Supraman window before it fires again.

## 4. Never

| Never | Instead |
|---|---|
| approve a schedule, or say it is approved before `list`/`show` says `active` | the human's Approve in the Supraman window |
| edit, write or delete a schedule's TOML (`<home>/schedules/*.toml`) or a run record | `sm schedule …` only |

| `sm schedule remove`, or `sm schedule edit` of a schedule you did not add in this conversation at the human's request | tell the human: the Supraman window (or they run `sm schedule edit`/`remove` themselves) |
| `schtasks`, `Register-ScheduledTask`, Task Scheduler, any Windows task | Supraman registers the task after Approve |
| the built-in schedule skill or a cloud routine for a job on this machine | this skill: `sm schedule add` |
| choose `edit` or `auto` without the human's answer | §2 |
| add an `--allow-tools` rule the human did not say yes to, a `dry-run` hint's included | §2 |
| run a scheduled command now to try it | `sm schedule dry-run` |
