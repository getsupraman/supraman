# plugins/supraman

Loaded with `--plugin-dir <this dir>` on sessions Supraman launches; never installed into user settings.

Every hook runs `${CLAUDE_PLUGIN_ROOT}/bin/sm.exe hook <event>` in exec form (no shell). Claude Code expands only
its own placeholders in an exec-form `command`, not arbitrary env vars (`${FOREMAN_FM}` fails with "Executable not
found in $PATH"), so `sm.exe` has to sit in `bin/` here: `sm plugin install <this dir>` copies
the running `sm.exe` there. `bin/` is gitignored.

The session needs `SUPRAMAN_HOME` (and `FOREMAN_HOME`, which an older `fm` reads), `FOREMAN_ROLE`, `FOREMAN_WORKSPACE`, `FOREMAN_SESSION` (its name, what `sm tell` signs
with) and, for a repo session, `FOREMAN_REPO` in its environment,
passed as `--settings '{"env":{…}}'` (env on the `claude --bg` process does not reach the session). The app
writes `roles/{workspace,relay,repo}.md` to `<home>/roles/` at start (`{sm}`/`{home}` filled in) and installs
`bin/sm.exe` from its own build. Claude Code puts this plugin's `bin/` on the session's PATH, so roles call `sm` by name, with the Bash tool: every launch passes `--allowedTools "Bash(sm:*)" "Bash(<abs sm>:*)"`
(a `PowerShell(…)` rule did not match). `sm send` exits 1 when `FOREMAN_ROLE` is set and non-empty: its drop
reaches the receiver from `fm-<w>-relay`, the human's voice, so a session reaches another one with `SendMessage` only.

Skills: `skills/workspace-controller` (`/supraman:workspace-controller`, loaded by `roles/workspace.md`) and
`skills/repo-controller` (`/supraman:repo-controller`, loaded by `roles/repo.md`; it runs `/workman:controller` from
the user-installed workman plugin, which loads with no `extra_plugin_dirs`). The roles stay short: who the session is, which skill to
invoke, what to do on start and after a compaction; the skills hold the rules.

Workman is https://work.supraman.ai, reached only through Supraman's OAuth sign-in (Settings → Workman →
Sign in); there is no static token and no local stack. Supraman mints each session its own token for its role and
passes it with `--mcp-config <file> --strict-mcp-config`; not signed in, a session has no `work_*` tools.
`sm work overview|board` reads the workspace's project with an access token of the same sign-in.

- every event: one line in `<home>/events/<session_id>.jsonl` (`supraman_core::Event`)
- SessionStart (startup, resume, compact, clear): stdout = `FOREMAN-ROLE <role> v<sha8>` + `<home>/roles/<role>.md`
  with `{workspace}` replaced by `FOREMAN_WORKSPACE` and `{repo}` by `FOREMAN_REPO`
