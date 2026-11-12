use std::{cell::RefCell, collections::HashMap};

use gpui::{Bounds, Div, ElementId, Hsla, Pixels, SharedString, Stateful, canvas, div, prelude::*};

use crate::tooltip::Tooltip;

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
    static CUT: RefCell<Cut> = RefCell::default();
}

pub fn cut_at(key: &str) -> Option<Bounds<Pixels>> {
    CUT.with(|cut| cut.borrow().labels.get(key).map(|(_, b)| *b))
}

/// Give it `min_w_0` / `flex_1` as a row needs.
pub fn clipped(key: impl Into<SharedString>, text: impl Into<SharedString>, middle: bool) -> Stateful<Div> {
    clipped_with(key, text, middle, true)
}

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
    let measure = measure(key.clone(), text.clone(), tooltip);
    let label = label.child(text.clone()).child(measure);
    if tooltip && own_tooltip { label.tooltip(Tooltip::text(text)) } else { label }
}

/// Records under `key` whether `text` is wider than the bounds it is laid over; `shown` is what the last frame recorded.
fn measure(key: SharedString, text: SharedString, shown: bool) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            let style = window.text_style();
            let size = style.font_size.to_pixels(window.rem_size());
            let run = style.to_run(text.len());
            let width = window.text_system().shape_line(text.clone(), size, &[run], None).width;
            let cut = width > bounds.size.width + gpui::px(0.5);
            // a label that turned cut gets its tooltip, one that fits again loses it, in the next frame
            if cut != shown {
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
}

/// A bright part, then muted words cut first: `bright` (the element showing `bright_text`, its own ellipsis) keeps its width up to the line's,
/// `muted` in `muted_color` shrinks to nothing before it; `bright muted` is the tooltip while either is cut (`cut_at` `<key>-bright`, `<key>-muted`).
pub fn pair(key: impl Into<SharedString>, bright_text: impl Into<SharedString>, bright: impl IntoElement, muted: impl Into<SharedString>, muted_color: Hsla) -> Stateful<Div> {
    let (key, bright_text, muted) = (key.into(), bright_text.into(), muted.into());
    let (bright_key, muted_key) = (SharedString::from(format!("{key}-bright")), SharedString::from(format!("{key}-muted")));
    let bright_cut = cut_at(&bright_key).is_some();
    let tooltip = bright_cut || cut_at(&muted_key).is_some();
    let whole = SharedString::from(format!("{bright_text} {muted}"));
    let bright = div().relative().flex().flex_none().max_w_full().min_w_0().overflow_hidden().child(bright).child(measure(bright_key, bright_text, bright_cut));
    let muted = div().flex().flex_1().min_w_0().pl_1().text_color(muted_color).child(clipped_with(muted_key, muted, false, false));
    let line = div().id(ElementId::Name(key)).flex().items_center().min_w_0().overflow_hidden().whitespace_nowrap().child(bright).child(muted);
    if tooltip { line.tooltip(Tooltip::text(whole)) } else { line }
}

/// [`clipped`] over up to `lines` wrapped lines, the last cut with an ellipsis; the whole text as a tooltip while cut.
pub fn clamped(key: impl Into<SharedString>, text: impl Into<SharedString>, lines: usize) -> Stateful<Div> {
    let (key, text) = (key.into(), text.into());
    let tooltip = cut_at(&key).is_some();
    let measure = {
        let (key, text) = (key.clone(), text.clone());
        canvas(
            move |bounds, window, _| {
                let style = window.text_style();
                let size = style.font_size.to_pixels(window.rem_size());
                let run = style.to_run(text.len());
                let wrapped = window
                    .text_system()
                    .shape_text(text.clone(), size, &[run], Some(bounds.size.width + gpui::px(0.5)), None)
                    .map(|shaped| shaped.iter().map(|line| line.wrap_boundaries().len() + 1).sum::<usize>())
                    .unwrap_or(1);
                let cut = wrapped > lines;
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
    let label = div()
        .id(ElementId::Name(key))
        .relative()
        .min_w_0()
        .overflow_hidden()
        .line_clamp(lines)
        .text_ellipsis()
        .child(text.clone())
        .child(measure);
    if tooltip { label.tooltip(Tooltip::text(text)) } else { label }
}

/// A `/` path as one line: its `dir/` in `folder`, cut in the middle under `dir_key` when narrow, then its name in `name`.
pub fn path_label(dir_key: impl Into<SharedString>, path: &str, folder: Hsla, name: Hsla) -> Div {
    let (dir, file) = match path.rsplit_once('/') {
        Some((dir, file)) => (format!("{dir}/"), file.to_string()),
        None => (String::new(), path.to_string()),
    };
    div()
        .flex()
        .min_w_0()
        .whitespace_nowrap()
        .when(!dir.is_empty(), |d| d.child(clipped(dir_key, dir, true).flex_shrink(1.).text_color(folder)))
        .child(div().flex_none().text_color(name).child(SharedString::from(file)))
}

#[cfg(test)]
mod tests;
