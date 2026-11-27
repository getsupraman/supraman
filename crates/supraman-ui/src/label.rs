//! [`Label`]: a leading mark and text on one line, the mark on the letters' middle ([`crate::ink`]), the text cut with
//! its tooltip ([`crate::clipped`]). Every mark beside words in the kit is one.

use gpui::{AnyElement, App, Div, Hsla, IntoElement, Pixels, RenderOnce, SharedString, StyleRefinement, Svg, Window, div, prelude::*};
use theme::ActiveTheme as _;

use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::marks::Mark;
use crate::tokens::{self, Text};

/// What sits before a [`Label`]'s text.
pub enum LabelMark {
    /// Sized by the label's text and coloured by [`Label::mark_color`], else its text colour.
    Icon(Icon),
    /// The caller's own size and colour.
    Svg(Svg),
    Dot(Mark),
    /// [`crate::controls::button::tick`] in the accent colour: a read-only tick box.
    Tick(bool),
    Element(AnyElement),
}

impl From<Icon> for LabelMark {
    fn from(icon: Icon) -> Self {
        LabelMark::Icon(icon)
    }
}

impl From<Svg> for LabelMark {
    fn from(svg: Svg) -> Self {
        LabelMark::Svg(svg)
    }
}

impl From<Mark> for LabelMark {
    fn from(mark: Mark) -> Self {
        LabelMark::Dot(mark)
    }
}

impl From<AnyElement> for LabelMark {
    fn from(el: AnyElement) -> Self {
        LabelMark::Element(el)
    }
}

enum Words {
    Text(SharedString),
    Element(AnyElement),
}

/// A mark and text. `RenderOnce`: an owner's hit key goes on its parent; [`Label::hit`] records its parts for a test.
#[derive(IntoElement)]
pub struct Label {
    words: Option<Words>,
    key: Option<(SharedString, bool)>,
    mark: Option<LabelMark>,
    slot: Option<Pixels>,
    size: Option<Text>,
    icon_size: Option<f32>,
    color: Option<Hsla>,
    mark_color: Option<Hsla>,
    muted: bool,
    centred: bool,
    gap: Pixels,
    hit: Option<(HitTargets, SharedString)>,
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

impl Label {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self::with(Some(Words::Text(text.into())))
    }

    /// A mark alone: the slot a row keeps for one beside text the owner paints.
    pub fn none() -> Self {
        Self::with(None)
    }

    /// The owner's own words (a [`crate::clipped::pair`], a [`crate::clipped::path_label`], a painted cell) in the text's place.
    pub fn element(words: impl IntoElement) -> Self {
        Self::with(Some(Words::Element(words.into_any_element())))
    }

    fn with(words: Option<Words>) -> Self {
        Self {
            words,
            key: None,
            mark: None,
            slot: None,
            size: None,
            icon_size: None,
            color: None,
            mark_color: None,
            muted: false,
            centred: false,
            gap: tokens::space::M,
            hit: None,
            style: StyleRefinement::default(),
            children: Vec::new(),
        }
    }

    pub fn mark(mut self, mark: impl Into<LabelMark>) -> Self {
        self.mark = Some(mark.into());
        self
    }

    pub fn mark_if(mut self, mark: Option<impl Into<LabelMark>>) -> Self {
        self.mark = mark.map(Into::into);
        self
    }

    /// The mark centred in a slot `width` wide, kept while there is no mark, so names in a list align.
    pub fn slot(mut self, width: Pixels) -> Self {
        self.slot = Some(width);
        self
    }

    /// Cut with an ellipsis (in the middle while `middle`) under `key`, its whole text the tooltip while cut.
    pub fn clipped(mut self, key: impl Into<SharedString>, middle: bool) -> Self {
        self.key = Some((key.into(), middle));
        self
    }

    pub fn size(mut self, size: Text) -> Self {
        self.size = Some(size);
        self
    }

    pub fn icon_size(mut self, size: f32) -> Self {
        self.icon_size = Some(size);
        self
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub fn mark_color(mut self, color: Hsla) -> Self {
        self.mark_color = Some(color);
        self
    }

    /// In [`crate::themes::secondary`].
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }

    /// The letters moved onto the box's middle and the mark left there: a label centred in a fixed-height control (a button, a pill).
    pub fn centred(mut self) -> Self {
        self.centred = true;
        self
    }

    pub fn gap(mut self, gap: Pixels) -> Self {
        self.gap = gap;
        self
    }

    /// Records its mark under `<key>-mark` and its text under `<key>-text`.
    pub fn hit(mut self, hits: &HitTargets, key: impl Into<SharedString>) -> Self {
        self.hit = Some((hits.clone(), key.into()));
        self
    }

    fn icon_px(&self) -> f32 {
        self.icon_size.unwrap_or(match self.size {
            Some(t) if t == tokens::text::CAPTION => tokens::icon::XS,
            Some(t) if t == tokens::text::SMALL => tokens::icon::S,
            Some(t) if t == tokens::text::TITLE || t == tokens::text::HEADING => tokens::icon::L,
            _ => tokens::icon::M,
        })
    }

    fn mark_element(&mut self, text: Hsla, cx: &App) -> Option<AnyElement> {
        let color = self.mark_color.unwrap_or(text);
        let mark = match self.mark.take() {
            None => None,
            Some(LabelMark::Icon(icon)) => Some(icon.svg(self.icon_px(), color).into_any_element()),
            Some(LabelMark::Svg(svg)) => Some(svg.into_any_element()),
            Some(LabelMark::Dot(mark)) => Some(crate::marks::dot(mark).into_any_element()),
            Some(LabelMark::Tick(on)) => Some(crate::controls::button::tick(on, cx.theme().colors().text_accent, cx).into_any_element()),
            Some(LabelMark::Element(el)) => Some(el),
        };
        let hit = self.hit.as_ref().map(|(hits, key)| hit_target(hits, format!("{key}-mark")));
        let mark = match (mark, self.slot) {
            (None, None) => return None,
            (mark, Some(w)) => div().relative().flex().flex_none().items_center().justify_center().w(w).children(mark.map(|m| div().relative().flex().flex_none().children(Some(m)).children(hit))),
            (Some(mark), None) => div().relative().flex().flex_none().child(mark).children(hit),
        };
        Some(if self.centred { mark.into_any_element() } else { crate::ink::mark(mark).into_any_element() })
    }

    fn words_element(&mut self) -> Option<AnyElement> {
        let words = match (self.words.take()?, self.key.take()) {
            (Words::Text(text), Some((key, middle))) => {
                let key = self.hit.as_ref().map_or_else(|| key.clone(), |(hits, _)| hits.key(&key).into());
                crate::clipped::clipped(key, text, middle).flex_shrink(1.).into_any_element()
            }
            (Words::Text(text), None) => div().whitespace_nowrap().child(text).into_any_element(),
            (Words::Element(el), _) => el,
        };
        let hit = self.hit.as_ref().map(|(hits, key)| hit_target(hits, format!("{key}-text")));
        let words = div().relative().flex().flex_grow(1.).items_center().min_w_0().flex_shrink(1.).child(words).children(hit);
        Some(if self.centred { crate::ink::text(words).into_any_element() } else { words.into_any_element() })
    }
}

impl RenderOnce for Label {
    fn render(mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors();
        let color = self.color.unwrap_or(if self.muted { crate::themes::secondary(colors) } else { colors.text });
        let line: Div = div().flex().items_center().gap(self.gap).min_w_0().whitespace_nowrap();
        let line = match (self.size, self.color.is_some() || self.muted) {
            (Some(size), true) => line.text_size(size.size).text_color(color),
            (Some(size), false) => line.text_size(size.size),
            (None, true) => line.text_color(color),
            (None, false) => line,
        };
        let mark = self.mark_element(color, cx);
        let words = self.words_element();
        let mut line = line.children(mark).children(words);
        line.style().refine(&self.style);
        line.children(self.children)
    }
}

impl Styled for Label {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Label {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

#[cfg(test)]
mod tests;
