# plugins/supraman

Loaded with `--plugin-dir <this dir>` on sessions Supraman launches; never installed into user settings.

Every hook runs `${CLAUDE_PLUGIN_ROOT}/bin/sm.exe hook <event>` in exec form (no shell). Claude Code expands only
its own placeholders in an exec-form `command`, not arbitrary env vars, so `sm.exe` has to sit in `bin/` here: `sm plugin install <this dir>` copies
the running `sm.exe` there. `bin/` is gitignored.

The session needs `SUPRAMAN_HOME`, `SUPRAMAN_ROLE`, `SUPRAMAN_SESSION` (its name) and, in a listed folder,
`SUPRAMAN_REPO` (that folder's name) in its environment; a scheduled run gets them through `--settings '{"env":{…}}'` (env on
the `claude --bg` process does not reach the session). The app writes `roles/hand.md` to `<home>/roles/` at start
(`{sm}`/`{home}` filled in) and installs `bin/sm.exe` from its own build. Claude Code puts this plugin's `bin/` on
the session's PATH, so a session calls `sm` by name, with the Bash tool: every launch passes
`--allowedTools "Bash(sm:*)" "Bash(<abs sm>:*)"` (a `PowerShell(…)` rule does not match).

Skills: `skills/supraman-scheduler` (`/supraman:supraman-scheduler`, named by `roles/hand.md`).

Workman is https://work.supraman.ai, reached only through Supraman's OAuth sign-in (Settings → Workman →
Sign in); there is no static token and no local stack. Supraman mints each session its own token and
passes it with `--mcp-config <file> --strict-mcp-config`; not signed in, a session has no `work_*` tools.

- every event: one line in `<home>/events/<session_id>.jsonl` (`supraman_core::Event`)
- SessionStart (startup, resume, compact, clear): stdout = `SUPRAMAN-ROLE <role> v<sha8>` + `<home>/roles/<role>.md`
  with `{repo}` replaced by `SUPRAMAN_REPO`
