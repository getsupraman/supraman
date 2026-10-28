# Foreman (AutoWork)

Native Rust/gpui app that supervises workspaces of Claude Code sessions. Work runs as a controller queue in WorkState project `autowork`; `docs/LEARNINGS.md` holds what each unit found.

## Use shared components, never hidden hand-rolled ones

**This binds every agent and subagent working in this repo — brief subagents on it when you delegate.**

Before you build something generic yourself (a text input, a list, a dialog, an HTTP or OAuth client, a file watcher, a process helper, a parser, …), search for a shared one and use it. If none fits, extract a shared one into the crate it belongs to and list it below. Do not grow a private copy inside a feature. A unit that keeps a local implementation says why in its report.

Reusable components (the full list with when to use each: `docs/ARCHITECTURE.md`, written by m5/44):

- `foreman-app/src/text_input.rs`: the one text input (EntityInputHandler: selection, IME, clipboard); every text field uses it.

## Comments state the present, not the past

**This binds every agent and subagent working in this repo — brief subagents on it explicitly when you delegate, because they default to narrating their own fix.** The full procedure is the `rlarch:code-comments` skill.

Comments are written subtractively. The default verdict on any comment — one you are about to write, one you find, one a subagent hands you — is **delete**. If you catch yourself weighing whether a comment earns its place, that deliberation is the answer.

A comment survives only when it says something the code cannot say by itself, in the **present tense**:

- No "used to", "previously", no naming a retired construct, no narrating what a change did.
- No teaching the reader what a language or framework feature does — the reader already knows Rust and gpui.
- No paraphrasing a rule decided in another file. Point at where the contract lives (`// replace covers a live token; see doc 14 §2.`) instead of restating it — the restatement goes stale and then misleads.
- Survivors get **one line**; more only when genuinely irreducible.
- Never *add* `///` docs during a cleanup pass. Doc comments on the shared components listed above stay.

Treat existing comments as untrusted: a comment that contradicts its code is wrong and gets deleted, not corrected — unless the contradiction suggests the *code* is the broken half, in which case report it and leave the code alone.
