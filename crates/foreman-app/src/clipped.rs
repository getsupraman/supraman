//! One-line labels cut with an ellipsis when they do not fit (m5/26). Each painted frame measures the
//! label's text in its own text style against the width it got; a label the last frame painted cut shows
//! its full text as a tooltip on hover. Keys are the labels' element ids, one per label on screen.

use std::{cell::RefCell, collections::HashMap};

use gpui::{Bounds, Div, ElementId, Pixels, SharedString, Stateful, canvas, div, prelude::*};

use crate::view::icon::Tooltip;

/// Labels kept at most; the one painted longest ago goes first, a label no longer on screen.
const KEPT: usize = 512;

#[derive(Default)]
struct Cut {
    paints: u64,
    labels: HashMap<SharedString, (u64, Bounds<Pixels>)>,
}

impl Cut {
    fn painted(&mut self, key: &SharedString, bounds: Option<Bounds<Pixels>>) {
        let Some(bounds) = bounds else {
            self.labels.remove(key);
            return;
        };
        self.paints += 1;
        self.labels.insert(key.clone(), (self.paints, bounds));
        if self.labels.len() > KEPT
            && let Some(oldest) = self.labels.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone())
        {
            self.labels.remove(&oldest);
        }
    }
}

thread_local! {
    /// The labels painted cut, with their bounds.
    static CUT: RefCell<Cut> = RefCell::default();
}

/// Where the last frame painted the label `key`, when it painted it cut.
pub fn cut_at(key: &str) -> Option<Bounds<Pixels>> {
    CUT.with(|cut| cut.borrow().labels.get(key).map(|(_, b)| *b))
}

/// `text` on one line, cut with an ellipsis (`middle`: in the middle, as a path reads best) when it does
/// not fit, with the tooltip once it has been painted cut. Give it `min_w_0` / `flex_1` as a row needs.
pub fn clipped(key: impl Into<SharedString>, text: impl Into<SharedString>, middle: bool) -> Stateful<Div> {
    clipped_with(key, text, middle, true)
}

/// [`clipped`], without its tooltip when `own_tooltip` is false: its parent shows the full text another way.
pub fn clipped_with(key: impl Into<SharedString>, text: impl Into<SharedString>, middle: bool, own_tooltip: bool) -> Stateful<Div> {
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
                CUT.with(|set| set.borrow_mut().painted(&key, cut.then_some(bounds)));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    };
    let label = label.child(text.clone()).child(measure);
    if tooltip && own_tooltip { label.tooltip(Tooltip::text(text)) } else { label }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_painted_longest_ago_go_first() {
        let mut cut = Cut::default();
        let bounds = Bounds::default();
        for i in 0..KEPT + 10 {
            cut.painted(&format!("label-{i}").into(), Some(bounds));
            cut.painted(&"shown".into(), Some(bounds));
        }
        assert_eq!(cut.labels.len(), KEPT);
        assert!(cut.labels.contains_key("shown") && cut.labels.contains_key(format!("label-{}", KEPT + 9).as_str()));
        assert!(!cut.labels.contains_key("label-0"));
        cut.painted(&"shown".into(), None);
        assert!(!cut.labels.contains_key("shown"), "a label that fits again is forgotten");
    }
}
