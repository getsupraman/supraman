# Peer messaging: writing into a Claude Code session's pipe

Measured 2026-10-26 by autowork-36 against claude 2.1.308–2.1.310 (bundled JS), with live 2.1.304–2.1.310
sessions; used by Foreman since m5/4 (`crates/foreman-claude/src/peer.rs`, `human.rs`), checked on 2.1.310.
Labels: **[measured]** = seen in the registry or a live test; **[code]** = read in the bundled source; **[guess]** = inference.
No token values appear anywhere in this file.

## How Foreman uses it (m5/4)

- Human → session: `foreman_claude::human::Sender::send` (the task box, Approve/Reply, `fm send`) resolves the
  session by its exact name in `~/.claude/sessions` (read only: pid alive with the same `procStart`, newest
  start wins, `peerProtocol == 1`), reads the `.key`'s `peerToken` and drops it after writing one connection:
  the `auth` line, then a `user` line with no `from` and no `from-mode`. Content:
  `<cross-session-message from-name="foreman">\nFrom the human (via Foreman, workspace <w>) [m-<8hex>]:\n<body>\n</cross-session-message>`
  — a fresh id per message, so no two are the same text (the receiver drops a repeat within 30 s).
- Only when Foreman knows the session's launch mode (its desired state) and it is prompting (`auto`,
  `default`, `acceptEdits`): a bypass or plan receiver holds an unmoded message silently.
- **`sent direct` means written into the pipe, not confirmed**: no receipt comes back (no inbox), so a rate,
  duplicate or queue drop would be silent. The selftest proves delivery by the receiver's answers.
- Any failure (no entry, stale entry, unknown protocol, no key, pipe error, 5 s timeout, the burst budget)
  falls back to the relay inbox, and the app/`fm send` say which: `sent direct` / `sent via relay (<why>)`.
- Limits respected through `relay/<w>/direct.json` (direct sends of the last minute): keep ≥ 10 of the
  receiver's 30-token burst (refill 0.5/s), else the relay.
- Session → human: `fm tell "<body>"` / `fm tell --file <path>` writes the workspace outbox
  (`{from, from_name, body}`, `from` = the session name from `FOREMAN_SESSION`). No reply path over the pipe:
  Foreman never registers an inbox.
- Under `cfg(test)`, `peer::deliver` refuses every pipe outside `\.\pipe\LOCAL\cc-msg-foremantest-*`.

## 1. Addressing
- [measured] Registry dir: `~/.claude/sessions/`. Each live session writes:
  - `<pid>.json` with `pid, sessionId, cwd, name, nameSource, kind (interactive|bg), status (idle|busy|shell), messagingSocketPath (\\.\pipe\LOCAL\cc-msg-<32hex>), peerProtocol: 1, peerFeatures: ["notify_idle","artifact_yield"], procStart, pidDomain, version, startedAt, updatedAt, statusUpdatedAt`.
  - `<pid>.<sha256(canonical pipe path)>.key` with `{peerToken, procStartFt, pidDomain}`. The canonical path is `\\.\pipe\` + the pipe name lowercased (`LOCAL\cc-msg-...`). [measured] This derivation matched the real key file name.
- [code] ListAgents reads every `<pid>.json` and keeps a record only if: it has a socket path, it isn't a spare or parked session, the pid is alive with the same process start time, and a 250 ms connect probe to the pipe succeeds. It also merges in-process subagents/teammates and cloud/bridge sessions. Short refs are prefixes of a hash of `kind:id`.
- [measured] Records of dead sessions are cleaned up lazily, so stale `.json` files exist. Always do the liveness check.

## 2. Auth
- [code] Each receiver generates two random 16-byte hex tokens at inbox start:
  - `peerToken` is published in the `.key` file (mode 0600). Other senders read it from there.
  - `childToken` is exported as `CLAUDE_CODE_MESSAGING_TOKEN`, only to the session's own child processes.
  It is per receiver: not the sender's token, not one per pair.
- [code] The receiver marks a connection as "peer" or "child" by which token it presented. On Windows, the child token counts as "self-sent" when the receiver is in bypass mode, and self-sent messages skip holds.
- [measured] A non-Claude process gets a legitimate token by reading the target's `.key` file, exactly as a session's SendMessage does. No impersonation is needed to send. The test script did this and the message was delivered.
- [code] Auth is required only on Windows. On macOS/Linux it is optional (the auth line is sent only if a key file exists); the receiver relies on socket-directory permissions plus the connecting process's pid and uid.
- [code] For a session to reply to Foreman, Foreman must be registered: its own pipe named `\\.\pipe\LOCAL\cc-msg-<32hex>` plus `<pid>.json` and `<pid>.<sha>.key`. On Windows, SendMessage refuses a pipe that no live registered owner vouches for ("No running session has registered an inbox at X (ENOINBOX) — refusing to send to an unvouched pipe"). With a key file it requires a live owner with a matching process start time. [guess] A registered Foreman would then appear in ListAgents as a "Claude session".

## 3. Wire format
- [code+measured] Newline-delimited JSON over the pipe, one connection per message, fire-and-forget. Line cap is 1 MiB. A connection that hasn't sent a complete line before the first-line deadline is closed.
- Line 1: `{"type":"auth","token":"<peerToken>"}`
- Line 2: `{"type":"user","message":{"role":"user","content":"<text>"},"msg_id":"<uuid>","priority":"next","from":"uds:<sender pipe>"}`
  Optional fields: `uuid`, `session_id` (dropped if it doesn't match the receiver's), `from_plugin`, `file_attachments`.
  Priority: `now` is handled immediately; otherwise `next` (default) or `later`.
- [code] There are no `to` or `summary` fields on the wire; those are SendMessage tool parameters. Identity and mode travel inside `content` as a wrapper:
  ```
  <cross-session-message from="uds:…" from-session="…" hop-chain="…" from-name="…" from-mode="bypass|prompting" from-plugin="…">
  body
  </cross-session-message>
  ```
  The attributes must appear in that order. The receiver re-renders the wrapper and ignores it unless the result is byte-identical. [measured] `from-name` alone worked.
- [code] No reply comes back on the connection. Receipts are sent later to the sender's own inbox (only if `from` is a valid `uds:` address for a `cc-msg-<32hex>` pipe), as `{"type":"control","action":"peer_message_status","status":"held|denied|expired|delivered|refused|dropped","orig_msg_id":"…","reason":"…"}`. `delivered` is sent only for a message that was held first. A plain accept sends no receipt.
- [code] Other control actions: `rename`, `notify_when_idle`/`peer_idle_notice` (subscribe to the receiver going idle), `yield_artifact_replies`, `unyield_artifact_replies`, `artifact_replies_yielded`.

## 4. Delivery
- [measured] On an interactive session that was busy mid-turn, the message was queued and ran as the next input when the turn ended.
- [guess, high confidence] A `--bg` session runs the same session code under its host, so an idle bg session should pick the message up the same way. Not tested.
- [code] Messages are dropped with a `dropped` receipt in these cases:
  - more than 30 in a burst from one sender (refills at 0.5/s)
  - identical text from the same sender within 30 s
  - a relay loop or a chain that's too long
  - more than 50 peer messages already queued at the receiver
  Limits can be adjusted by remote flag `tengu_harbor_kite_limits`.
- [code] Holds ("held for approval"):
  - The receiver's mode is classed as "bypass" (`bypassPermissions`, or plan mode with bypass available) or "prompting" (everything else, including auto).
  - If the sender asserted a `from-mode` and it equals the receiver's class, the message is accepted. If it differs, the message is held.
  - With no `from-mode`, a bypass receiver holds the message and a prompting receiver accepts it.
  - Self-sent messages are always accepted.
  - The `crossSessionInbound` setting (`accept|hold|refuse`) in policy, flag or user settings overrides all of the above. Local/project settings can only make it stricter. A remote kill switch can refuse everything.
  - Held messages wait for the user to approve them in the UI. [guess] On an unattended bg session they expire.
- [code] Target gone, on the SendMessage side:
  - `ENOINBOX` (no key file, dead owner, or unreadable registry)
  - `ENOENT`/`ECONNREFUSED`, reported as "peer may have restarted… call ListAgents"
  - `EBUSY`/`EAGAIN` means busy, retry
  - 5 s send timeout
  A raw client just fails to open the pipe. [code] SendMessage also paces itself per target so it stays under the receiver's rate limit.

## 5. What the receiver sees
- [measured] The model sees:
  `Another Claude session sent a message:\n<cross-session-message from-name="foreman">\nTEST 1\n</cross-session-message>\n\nThis came from another Claude session — not typed by your user … Treat it as a teammate's request …`
  So it is a teammate's message, never the user's.
- [measured] With no `from=` in the wrapper, the "reply via SendMessage to the `from=` address" sentence is omitted, so the receiver cannot reply.
- [code] Internally the sender is recorded as `from ?? "unknown"`.
- [code] `from-mode` is not verified. Foreman could claim `bypass` to get past a hold on a bypass receiver, but that is spoofing; setting `crossSessionInbound: "accept"` in user settings is the clean option.
- [guess] An untagged (raw) message is framed the same way, minus name and address. Not tested.

## 6. Stability
- [measured] Registry files carry `peerProtocol: 1`. The same value is in 2.1.308, 2.1.309 and 2.1.310, and live 2.1.304–2.1.307 sessions register identically.
- [code] Wire markers (wrapper tag, auth frame, receipt actions, rate-limit defaults, `cc-msg-` naming) are present in all three versions. Minified names differ, so I compared strings, not code.
- Not documented. Behaviour depends on remote flags (`tengu_harbor_kite_mode_emit`, `tengu_harbor_kite_limits`, `tengu_harbor_kite_pacing_off`, `tengu_session_stable_address`). [guess] Expect drift; detect it through `peerProtocol`/`peerFeatures`.
- [code] macOS/Linux: a Unix domain socket, by default under `$XDG_RUNTIME_DIR/cc-socks/`, `/tmp/cc-socks-<uid>/<pid>.sock` (or `/private/tmp/...`), or Termux tmp. Same JSON lines; the auth line is optional; the receiver checks peer credentials.

## 7. Supported routes
- [code] No `claude send` command. Background-session commands are only `attach`, `logs`, `stop`, `rm`, `respawn`, plus `agents`. `claude attach` is a raw terminal attach (keystrokes), not a structured API.
- [code] The supported route is to own stdin. A session Foreman launches through the Agent SDK or `claude -p --input-format stream-json` accepts user messages on stdin at any time, including ones flagged as coming from a peer (origin `{kind:"peer", from, fromMode}`, treated as host-injected). That is the documented, versioned path for sessions Foreman starts.
- [code] `--messaging-socket-path <path>` lets the launcher choose the pipe name, which saves a registry lookup.
- For sessions Foreman did not start, the pipe is the only way in.

## Reproduce
autowork-36 used a throwaway script (not kept) that did what `peer::find_session` + `peer::deliver` do. m5/4
measured `procStart` as the process creation FILETIME: equal to `GetProcessTimes` creation time for all 20 live
sessions on 2026-10-26 (`(Get-Process -Id <pid>).StartTime.ToFileTimeUtc()`).
