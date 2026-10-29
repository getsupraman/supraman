# plugins/foreman

Loaded with `--plugin-dir <this dir>` on sessions Foreman launches; never installed into user settings.

Every hook runs `${CLAUDE_PLUGIN_ROOT}/bin/fm.exe hook <event>` in exec form (no shell). Claude Code expands only
its own placeholders in an exec-form `command`, not arbitrary env vars (`${FOREMAN_FM}` fails with "Executable not
found in $PATH"), so `fm.exe` has to sit in `bin/` here: `fm plugin install <this dir>` copies
the running `fm.exe` there. `bin/` is gitignored.

The session needs `FOREMAN_HOME`, `FOREMAN_ROLE`, `FOREMAN_WORKSPACE`, `FOREMAN_SESSION` (its name, what `fm tell` signs
with) and, for a repo session, `FOREMAN_REPO` in its environment,
passed as `--settings '{"env":{…}}'` (env on the `claude --bg` process does not reach the session). The app
writes `roles/{workspace,relay,repo}.md` to `<FOREMAN_HOME>/roles/` at start (`{fm}`/`{home}` filled in) and installs
`bin/fm.exe` from its own build. Claude Code puts this plugin's `bin/` on the session's PATH, so roles call `fm` by name, with the Bash tool: every launch passes `--allowedTools "Bash(fm:*)" "Bash(<abs fm>:*)"`
(a `PowerShell(…)` rule did not match). `fm send` exits 1 when `FOREMAN_ROLE` is set and non-empty: its drop
reaches the receiver from `fm-<w>-relay`, the human's voice, so a session reaches another one with `SendMessage` only.

Skills: `skills/workspace-controller` (`/foreman:workspace-controller`, loaded by `roles/workspace.md`) and
`skills/repo-controller` (`/foreman:repo-controller`, loaded by `roles/repo.md`; it runs `/rlarch:controller` from
the user-installed rlarch plugin, which loads with no `extra_plugin_dirs`). The roles stay short: who the session is, which skill to
invoke, what to do on start and after a compaction; the skills hold the rules.

WorkState is https://workstate.dikaia.ai, reached only through Foreman's OAuth sign-in (Settings → WorkState →
Sign in); there is no static token and no local stack. Foreman mints each session its own token for its role and
passes it with `--mcp-config <file> --strict-mcp-config`; not signed in, a session has no `work_*` tools.
`fm work overview|board` reads the workspace's project with an access token of the same sign-in.

- every event: one line in `<FOREMAN_HOME>/events/<session_id>.jsonl` (`foreman_core::Event`)
- SessionStart (startup, resume, compact, clear): stdout = `FOREMAN-ROLE <role> v<sha8>` + `<FOREMAN_HOME>/roles/<role>.md`
  with `{workspace}` replaced by `FOREMAN_WORKSPACE` and `{repo}` by `FOREMAN_REPO`
