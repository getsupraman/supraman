# Keys and mouse

Foreman's terminal tabs act like Windows Terminal with its defaults (René's settings: copy on Ctrl+C,
paste on Ctrl+V, find on Ctrl+Shift+F, no copy on select, plain-text copy). Code: `crates/foreman-app/src/view/term.rs`
(`chord`), `crates/foreman-ui/src/term_element.rs` (mouse), `crates/foreman-term` (selection, cursor, colours).

## Window

| Keys | Does |
|---|---|
| Ctrl+Tab, Ctrl+PgDn | next tab |
| Ctrl+Shift+Tab, Ctrl+PgUp | previous tab |
| Ctrl+, | settings |
| F5 | refresh: read the running sessions (`claude agents`) and the closed ones' transcripts now, as the action bar's ↻ does. Not sent to the program in a terminal tab |

## Terminal tab

| Keys | Does |
|---|---|
| Ctrl+C | with a selection: copy it (plain text, line breaks kept, trailing blanks trimmed) and clear it; without one: ^C to the program |
| Ctrl+Shift+C, Ctrl+Insert | copy the selection and clear it |
| Ctrl+V, Ctrl+Shift+V, Shift+Insert | paste (bracketed when the program asked for it; CRLF → CR). Ctrl+V with no text on the clipboard goes to the program as ^V |
| Ctrl+Shift+F | find bar: type to search (case-insensitive unless the text has a capital), Enter next match up (older output), Shift+Enter down, Esc closes |
| Ctrl+= (Ctrl++), Ctrl+-, Ctrl+0 | font size up, down, reset — per tab |
| Ctrl+wheel | font size up / down |
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

When the program turned mouse reporting on, clicks and drags go to it; hold Shift to select instead.

## Cursor and colours

The cursor is a bar that blinks every 530 ms and stays lit while you type or it moves; a program that sets
its own shape (DECSCUSR) wins. ANSI colours 0–15 are Windows Terminal's Campbell scheme; the foreground,
background and selection come from the Foreman theme.
