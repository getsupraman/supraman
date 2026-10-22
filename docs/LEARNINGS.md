# Learnings

Measured facts that shape the design. Newest last; each with its date and how it was observed.

## 2026-10-22 — setup

- **WorkState was running but unregistered.** `workstate-api-1` had been up 47h and healthy, with
  `/health` 200, but `claude mcp get workstate` said not found. The setup skill's state table covered
  this ("server up, not registered"). → The app's tear-up must check registration as well as health.
- **Controller token gives 19 tools including `work_constrain`**, so the running image is new enough
  for `/rlarch:controller`. It is not yet proven to equal `feat/workstate` HEAD (M0.1).
- **A local-scope MCP entry shadows the user-scope one.** Registered `--scope user` from
  `Skills/mcp`; `claude mcp get` run *there* reports `Scope: Local config`. An older per-directory
  entry exists for that folder and takes precedence. From any other folder it resolves to user scope.
  → The app should inspect `claude mcp get workstate` **from each repo's cwd** before launching a
  session there, not once globally.
- **MCP tools only appear in a new session.** Registration in the current session never adds the
  tools to it. → The app registers MCP/plugins *before* launching sessions, never mid-session.
- **`feat/workstate` is ahead of `master`** in the Skills repo (master merged an earlier tip).
- **`claude agents --json`** lists interactive *and* background sessions; background rows carry
  `state` (seen: `blocked`), interactive rows carry `status` (`idle`/`busy`) and a `pid`.
- **Session peer registry**: `~/.claude/sessions/<pid>.json` holds `sessionId`, `cwd`, `name`,
  `status`, `peerProtocol: 1`, `peerFeatures: ["notify_idle","artifact_yield"]`,
  `messagingSocketPath: \\.\pipe\LOCAL\cc-msg-<hex>`, with a sibling `.key` file. `SendMessage`
  rides on this. `~/.claude/daemon/` holds `roster.json`, `dispatch`, `attach-journal`, `pty-pids`
  for `--bg` sessions; the daemon exits after 5s idle with no clients.
- **Zed licensing**: `gpui` Apache-2.0; `terminal`, `terminal_view`, `ui`, `theme`, `workspace`
  GPL-3.0. GPL accepted for this project.
