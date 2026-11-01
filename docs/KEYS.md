# Keys and mouse

Supraman's terminal tabs act like Windows Terminal with its defaults (René's settings: copy on Ctrl+C,
paste on Ctrl+V, find on Ctrl+Shift+F, no copy on select, plain-text copy). Code: `crates/supraman-app/src/view/term.rs`
(`chord`), `crates/supraman-ui/src/term_element.rs` (mouse), `crates/supraman-term` (selection, cursor, colours).

## Window

| Keys | Does |
|---|---|
| Ctrl+Tab, Ctrl+PgDn | next tab |
| Ctrl+Shift+Tab, Ctrl+PgUp | previous tab |
| Ctrl+, | settings |
| Ctrl+Shift+E | the search box over the tree, its text selected; also from a terminal tab (VS Code's Explorer chord; Ctrl+Shift+F is the terminal's find bar, and a Ctrl+letter belongs to the program in the tab) |
| F5 | refresh: read the running sessions (`claude agents`) and the closed ones' transcripts now, as the action bar's ↻ does, while no terminal tab is active (no tab, or a Work tab). With a terminal tab active, F5 goes to its program; use ↻ there |

## Text inputs (search box, Add workspace name, find bar, task box)

One input (`crates/supraman-app/src/text_input.rs`), the keys of a Windows edit box. The task box (Task…, Reply…)
is its multi-line kind: lines wrap, it grows to 8 rows and then scrolls (the wheel too). Characters, dead keys
(`^` `´` `` ` ``) and IME compositions come from Windows' own text input, so German and Italian layouts type
as anywhere else.

| Keys / mouse | Does |
|---|---|
| Left, Right, Home, End | move the caret (with a selection: Left/Right go to its start/end); in the task box Home/End go to the start/end of the row as it is shown |
| Up, Down | task box: the row above / below, keeping the column (past the first / last row: the start / end) |
| Ctrl+Home, Ctrl+End | start / end of the text |
| Ctrl+Left, Ctrl+Right | previous / next word (`-`, `.`, space separate words) |
| Shift+ any of those | select |
| Ctrl+A | select all |
| Backspace, Delete; Ctrl+Backspace, Ctrl+Delete | delete a character; a word |
| Ctrl+C / Ctrl+Insert, Ctrl+X / Shift+Delete, Ctrl+V / Shift+Insert | copy, cut, paste (line breaks pasted as spaces; the task box keeps them) |
| Ctrl+Z; Ctrl+Y, Ctrl+Shift+Z | undo (typing runs to a space are one step); redo |
| Enter | Add workspace: save. Search: nothing. Find bar: next match up. Task box: a new line |
| Shift+Enter | find bar: next match down. Task box: a new line |
| Ctrl+Enter | task box: send |
| Esc | Search: clear the box (the tree is back); on an empty box, give the keys back to the window. Add workspace: cancel. Find bar: close. Task box: close, nothing sent. An open menu closes first |
| click, drag, Shift+click | place the caret, select, extend the selection |
| double-click, triple-click | select the word, select all |

The Add workspace name takes lowercase letters, digits and `-`: capitals are lowered, a space or `_` becomes
`-`, anything else is refused with a hint under the field. A click anywhere outside an input gives the keys
back to the window (the terminal).

## Terminal tab

| Keys | Does |
|---|---|
| Ctrl+C | with a selection: copy it (plain text, line breaks kept, trailing blanks trimmed) and clear it; without one: ^C to the program |
| Ctrl+Shift+C, Ctrl+Insert | copy the selection and clear it |
| Ctrl+V, Ctrl+Shift+V, Shift+Insert | paste (bracketed when the program asked for it; CRLF → CR). Ctrl+V with no text on the clipboard goes to the program as ^V |
| Ctrl+Shift+F | find bar: type to search (case-insensitive unless the text has a capital), Enter next match up (older output), Shift+Enter down, Esc closes |
| Ctrl+= (Ctrl++), Ctrl+-, Ctrl+0 | font size up, down, reset — every terminal, kept in settings.toml (m5/41) |
| Ctrl+wheel | font size up / down, every terminal |
| Ctrl+Shift+Up / Down | scroll one line |
| Ctrl+Shift+PgUp / PgDn | scroll one page |
| Ctrl+Shift+Home / End | scroll to the top / bottom |
| Shift+Enter | ESC CR (a new line in Claude Code's prompt) |

Everything else goes to the program, including Shift+PgUp/PgDn (Windows Terminal binds no default to them).

## Mouse in a terminal

| Mouse | Does |
|---|---|
| drag | select |
| double-click | select a word, up to Windows Terminal's `wordDelimiters` (`` /\()"'-.,:;<>~!@#$%^&*|+=[]{}~?│``) |
| triple-click | select the line |
| Shift+click | extend the selection |
| Alt+drag | block (rectangle) selection |
| right-click | with a selection: copy it and clear it; without one: paste |
| wheel | scrollback; in a full-screen program, arrow keys or its own mouse reports |
| hover a link | an OSC 8 link (a program's `[text](url)`, underlined) shows its URL in the terminal's bottom left corner |
| Ctrl+hover, Ctrl+click | the pointer is a hand over a link or a URL; Ctrl+click opens it: `http`, `https`, `mailto` and `file` only, any other scheme is refused with a note |

When the program turned mouse reporting on, clicks and drags go to it; hold Shift to select instead.

## Cursor and colours

The cursor is a bar that blinks every 530 ms and stays lit while you type or it moves; a program that sets
its own shape (DECSCUSR) wins. ANSI colours 0–15 are Windows Terminal's Campbell scheme; the foreground,
background and selection come from the Supraman theme.
