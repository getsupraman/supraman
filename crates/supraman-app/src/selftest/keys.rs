use gpui::{AsyncWindowContext, KeyDownEvent, Keystroke, Modifiers, PlatformInput};

/// As gpui_windows makes them: `enter` for a new line, `space`, else the character itself.
pub(crate) fn keystroke(cx: &mut AsyncWindowContext, key: &str, key_char: Option<&str>, modifiers: Modifiers) -> anyhow::Result<()> {
    let event = KeyDownEvent {
        keystroke: Keystroke { modifiers, key: key.into(), key_char: key_char.map(String::from) },
        is_held: false,
        prefer_character_input: false,
    };
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::KeyDown(event), cx);
    })?;
    Ok(())
}

/// Each character a keystroke that no binding takes, which gpui then hands to the input's handler as a real key's WM_CHAR is.
pub(crate) fn type_text(cx: &mut AsyncWindowContext, text: &str) -> anyhow::Result<()> {
    for ch in text.chars() {
        let c = ch.to_string();
        let keystroke = match ch {
            '\n' => {
                keystroke(cx, "enter", None, Modifiers::default())?;
                continue;
            }
            ' ' => Keystroke { modifiers: Modifiers::default(), key: "space".into(), key_char: Some(c) },
            _ => Keystroke { modifiers: Modifiers { shift: ch.is_uppercase(), ..Default::default() }, key: c.to_lowercase(), key_char: Some(c) },
        };
        cx.update(|window, cx| window.dispatch_keystroke(keystroke, cx))?;
    }
    Ok(())
}
