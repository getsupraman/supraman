# plugins/foreman

Loaded with `--plugin-dir <this dir>` on sessions Foreman launches; never installed into user settings.

Every hook runs `${CLAUDE_PLUGIN_ROOT}/bin/fm.exe hook <event>` in exec form (no shell). Claude Code expands only
its own placeholders in an exec-form `command`, not arbitrary env vars (`${FOREMAN_FM}` fails with "Executable not
found in $PATH", measured in m2/2), so `fm.exe` has to sit in `bin/` here: `fm plugin install <this dir>` copies
the running `fm.exe` there. `bin/` is gitignored.

The session needs `FOREMAN_HOME`, `FOREMAN_ROLE` and (for platform roles) `FOREMAN_PLATFORM` in its environment,
passed as `--settings '{"env":{…}}'` (env on the `claude --bg` process does not reach the session, m2/1). The app
writes `roles/platform.md` to `<FOREMAN_HOME>/roles/` at start and installs `bin/fm.exe` from its own build.

- every event: one line in `<FOREMAN_HOME>/events/<session_id>.jsonl` (`foreman_core::Event`)
- SessionStart (startup, resume, compact, clear): stdout = `FOREMAN-ROLE <role> v<sha8>` + `<FOREMAN_HOME>/roles/<role>.md`
  with `{platform}` replaced by `FOREMAN_PLATFORM`
