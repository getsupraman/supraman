//! One-line labels cut with an ellipsis when they do not fit (m5/26). Each painted frame measures the
//! label's text in its own text style against the width it got; a label the last frame painted cut shows
//! its full text as a tooltip on hover. Keys are the labels' element ids, one per label on screen.

use std::{cell::RefCell, collections::HashMap};

use gpui::{Bounds, Div, ElementId, Pixels, SharedString, Stateful, canvas, div, prelude::*};

use crate::view::icon::Tooltip;

thread_local! {
    /// The labels the last frame painted cut, with their bounds.
    static CUT: RefCell<HashMap<SharedString, Bounds<Pixels>>> = RefCell::default();
}

/// Where the last frame painted the label `key`, when it painted it cut.
pub fn cut_at(key: &str) -> Option<Bounds<Pixels>> {
    CUT.with(|cut| cut.borrow().get(key).copied())
}

/// `text` on one line, cut with an ellipsis (`middle`: in the middle, as a path reads best) when it does
/// not fit, with the tooltip once it has been painted cut. Give it `min_w_0` / `flex_1` as a row needs.
pub fn clipped(key: impl Into<SharedString>, text: impl Into<SharedString>, middle: bool) -> Stateful<Div> {
    let (key, text) = (key.into(), text.into());
    let label = div()
        .id(ElementId::Name(key.clone()))
        .relative()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap();
    let label = if middle { label.text_ellipsis_middle() } else { label.text_ellipsis() };
    let tooltip = cut_at(&key).is_some();
    let measure = {
        let (key, text) = (key.clone(), text.clone());
        canvas(
            move |bounds, window, _| {
                let style = window.text_style();
                let size = style.font_size.to_pixels(window.rem_size());
                let run = style.to_run(text.len());
                let width = window.text_system().shape_line(text.clone(), size, &[run], None).width;
                let cut = width > bounds.size.width + gpui::px(0.5);
                // a label that turned cut gets its tooltip, one that fits again loses it, in the next frame
                if cut != tooltip {
                    window.refresh();
                }
                CUT.with(|set| {
                    let mut set = set.borrow_mut();
                    if cut {
                        set.insert(key.clone(), bounds);
                    } else {
                        set.remove(&key);
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    };
    let label = label.child(text.clone()).child(measure);
    if tooltip { label.tooltip(Tooltip::text(text)) } else { label }
}
