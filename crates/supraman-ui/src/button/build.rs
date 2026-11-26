//! [`Button`]: every clickable button, built once and painted by the kit.

use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, ClickEvent, Context, Div, ElementId, FocusHandle, Hsla, IntoElement, MouseButton, MouseDownEvent, Pixels, RenderOnce, SharedString,
    Stateful, StyleRefinement, Svg, Window, div, prelude::*,
};
use theme::ActiveTheme as _;

use super::{Fill, Press, Size, Tone, base, icon_look, look};
use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::label::Label;
use crate::tokens::{self, radius, space, stroke};
use crate::tooltip::Tooltip;

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type OnPress = Rc<dyn Fn(&Press, &mut Window, &mut App)>;
type OnMouseDown = (MouseButton, Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>);

/// What a button shows before its label, or alone.
pub enum Glyph {
    /// Sized and coloured by the button.
    Icon(Icon),
    /// The caller's own size and colour.
    Svg(Svg),
}

impl From<Icon> for Glyph {
    fn from(icon: Icon) -> Self {
        Glyph::Icon(icon)
    }
}

impl From<Svg> for Glyph {
    fn from(svg: Svg) -> Self {
        Glyph::Svg(svg)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A labelled button in a tone and fill.
    Action,
    /// A square of `side` around its icon: a ghost, or bordered in its tone's colours once a tone is given.
    Square(Pixels),
    /// An action bar's: its icon, then any children, the ghost's fills.
    Toolbar,
}

/// Every clickable button. `key` is its element id, its hover group and its [`hit_target`] key; a label's letters paint under
/// `<key>-label`, its icon under `<key>-icon`, a turning icon under `<key>-turning`. Its own styles come first, so a caller's (`Styled`) win.
#[derive(IntoElement)]
pub struct Button {
    hits: HitTargets,
    key: SharedString,
    kind: Kind,
    label: Option<SharedString>,
    glyph: Option<Glyph>,
    after: Option<Icon>,
    tone: Option<Tone>,
    fill: Fill,
    size: Size,
    dimmed: bool,
    why: Option<SharedString>,
    turning: bool,
    on: bool,
    /// A popup anchored to its key is open ([`crate::popup::is_open`]).
    open: bool,
    focus: Option<(FocusHandle, OnPress)>,
    reveal: Option<(SharedString, bool)>,
    tooltip: Option<Rc<dyn Fn(&mut Window, &mut App) -> AnyView>>,
    on_click: Option<OnClick>,
    on_mouse_down: Option<OnMouseDown>,
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

impl Button {
    /// A labelled button: Neutral, Solid, [`Size::Small`] until told otherwise.
    pub fn new(hits: &HitTargets, key: impl Into<SharedString>) -> Self {
        Self {
            hits: hits.clone(),
            key: key.into(),
            kind: Kind::Action,
            label: None,
            glyph: None,
            after: None,
            tone: None,
            fill: Fill::Solid,
            size: Size::Small,
            dimmed: false,
            why: None,
            turning: false,
            on: false,
            open: false,
            focus: None,
            reveal: None,
            tooltip: None,
            on_click: None,
            on_mouse_down: None,
            style: StyleRefinement::default(),
            children: Vec::new(),
        }
    }

    /// An action bar's button: its icon, then any children, in the ghost's fills; the selected fill while [`Button::on`].
    pub fn toolbar(hits: &HitTargets, key: impl Into<SharedString>) -> Self {
        Self { kind: Kind::Toolbar, ..Self::new(hits, key) }
    }

    /// A row of buttons [`super::GAP`] apart, centred on one line.
    pub fn line() -> Div {
        div().flex().flex_none().items_center().gap(super::GAP)
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Before the label in its text colour, centred as its letters are; an icon-only or toolbar button's one glyph.
    pub fn icon(mut self, glyph: impl Into<Glyph>) -> Self {
        self.glyph = Some(glyph.into());
        self
    }

    /// After the label, in its text colour.
    pub fn icon_after(mut self, icon: Icon) -> Self {
        self.after = Some(icon);
        self
    }

    /// A `side` square around its icon alone, which takes the text colour hovered; a ghost unless a [`Button::tone`] is given.
    pub fn icon_only(mut self, side: Pixels) -> Self {
        self.kind = Kind::Square(side);
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
    }

    pub fn fill(mut self, fill: Fill) -> Self {
        self.fill = fill;
        self
    }

    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    /// Dimmed with the plain cursor while `why` is given, which becomes its tooltip; its click is then a no-op.
    pub fn dimmed(mut self, why: Option<impl Into<SharedString>>) -> Self {
        if let Some(why) = why {
            self.dimmed = true;
            self.why = Some(why.into());
        }
        self
    }

    /// [`Button::dimmed`] with no reason shown.
    pub fn dimmed_if(mut self, dimmed: bool) -> Self {
        self.dimmed |= dimmed;
        self
    }

    /// Its icon turns once a second while work it started runs.
    pub fn turning(mut self, turning: bool) -> Self {
        self.turning = turning;
        self
    }

    /// A toolbar button's selected fill.
    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    /// Takes the keyboard under `handle` (a [`crate::focus_order::FocusOrder`] member): [`ring`]ed in the focus colour while
    /// focused, over any other ring; Space or Enter `press`.
    pub fn focus(mut self, handle: &FocusHandle, press: impl Fn(&Press, &mut Window, &mut App) + 'static) -> Self {
        self.focus = Some((handle.clone(), Rc::new(press)));
        self
    }

    /// [`Button::focus`] under `handle`, `act` run by a click and by Space or Enter alike.
    pub fn acting<V: 'static>(self, handle: &FocusHandle, act: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static, cx: &mut Context<V>) -> Self {
        let act = Rc::new(act);
        let pressed = act.clone();
        self.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| act(this, window, cx)))
            .focus(handle, cx.listener(move |this, _: &Press, window, cx| pressed(this, window, cx)))
    }

    /// Always laid out and painted (slot and hit key kept), shown while `group` is hovered or `shown`.
    pub fn reveal(mut self, group: impl Into<SharedString>, shown: bool) -> Self {
        self.reveal = Some((group.into(), shown));
        self
    }

    /// A tooltip built as gpui's are ([`Tooltip::text`] for words); a dimmed button's why replaces it.
    pub fn tooltip(mut self, build: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.tooltip = Some(Rc::new(build));
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    pub fn on_mouse_down(mut self, button: MouseButton, handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_mouse_down = Some((button, Rc::new(handler)));
        self
    }

    /// The button as its element, for a caller that reads or adds to its style.
    pub fn build(mut self, cx: &App) -> Stateful<Div> {
        let glyph = self.glyph.take();
        let mut el = match self.kind {
            Kind::Action => self.action(glyph, cx),
            Kind::Square(side) => self.square(side, glyph, cx),
            Kind::Toolbar => self.toolbar_frame(glyph, cx),
        };
        if self.dimmed {
            el = el.opacity(tokens::DIMMED).cursor_default();
        }
        if let Some((group, false)) = self.reveal.take() {
            el = el.opacity(0.).group_hover(group, |s| s.opacity(1.));
        }
        match (self.why.take(), self.tooltip.take()) {
            (Some(why), _) => el = el.tooltip(Tooltip::text(why)),
            (None, Some(build)) => el = el.tooltip(move |window, cx| build(window, cx)),
            (None, None) => {}
        }
        match (self.on_click.take(), self.dimmed) {
            (Some(_), true) => el = el.on_click(|_, _, cx| cx.stop_propagation()),
            (Some(click), false) => el = el.on_click(move |e, window, cx| click(e, window, cx)),
            (None, _) => {}
        }
        if let Some((button, down)) = self.on_mouse_down.take() {
            el = el.on_mouse_down(button, move |e, window, cx| down(e, window, cx));
        }
        if let Some((handle, press)) = self.focus.take() {
            el = crate::focus_order::pressable(el, &handle, move |p: &Press, window, cx| press(p, window, cx));
        }
        el.style().refine(&self.style);
        el.children(self.children)
    }

    fn action(&mut self, glyph: Option<Glyph>, cx: &App) -> Stateful<Div> {
        let tone = self.tone.unwrap_or(Tone::Neutral);
        let look = look(tone, self.fill, cx);
        let hover_border = if tone == Tone::Neutral { cx.theme().colors().border_selected } else { look.border };
        let key = self.key.clone();
        let label = self.label.take().unwrap_or_default();
        let letters = div().relative().child(label).child(hit_target(&self.hits, format!("{key}-label")));
        let control = self.size.control();
        let glyph = glyph.map(|g| svg_of(g, look.text, tokens::icon::S));
        let (glyph, turning_hit) = match glyph {
            Some(svg) if self.turning => (Some(turning(svg, &key)), Some(hit_target(&self.hits, format!("{key}-turning")))),
            Some(svg) => (Some(svg.into_any_element()), None),
            None => (None, None),
        };
        base(&self.hits, key.clone(), move |s| s.bg(look.hover).border_color(hover_border), look.active)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(space::S)
            .rounded(radius::S)
            .border(stroke::HAIRLINE)
            .border_color(look.border)
            .bg(if self.open { look.hover } else { look.fill })
            .text_color(look.text)
            .whitespace_nowrap()
            .child(Label::element(letters).mark_if(glyph.map(|g| div().relative().flex().flex_none().child(g).child(hit_target(&self.hits, format!("{key}-icon"))).into_any_element())).gap(space::S).centred().children(self.after.take().map(|icon| icon.svg(tokens::icon::M, look.text))))
            .children(turning_hit)
            .h(control.height)
            .px(control.padding)
            .text_size(control.text.size)
    }

    fn square(&self, side: Pixels, glyph: Option<Glyph>, cx: &App) -> Stateful<Div> {
        let key = self.key.clone();
        let colors = cx.theme().colors();
        let open = self.open.then(|| tokens::roles(cx).open);
        let frame = |el: Stateful<Div>| el.flex().flex_none().items_center().justify_center().size(side).rounded(radius::S).when_some(open, |d, bg| d.bg(bg));
        match self.tone {
            None => {
                let text = colors.text;
                let glyph = glyph.map(|g| svg_of(g, colors.text_muted, tokens::icon::M).group_hover(key.clone(), move |s| s.text_color(text)));
                frame(base(&self.hits, key.clone(), |s| s.bg(colors.ghost_element_hover), colors.ghost_element_active)).children(glyph)
            }
            Some(tone) => {
                let look = icon_look(tone, cx);
                let glyph = glyph.map(|g| svg_of(g, look.text, tokens::icon::M).group_hover(key.clone(), move |s| s.text_color(look.hover_text)));
                frame(base(&self.hits, key.clone(), move |s| s.bg(look.hover), look.active)).border(stroke::HAIRLINE).border_color(look.border).bg(look.fill).children(glyph)
            }
        }
    }

    fn toolbar_frame(&self, glyph: Option<Glyph>, cx: &App) -> Stateful<Div> {
        let key = self.key.clone();
        let colors = cx.theme().colors();
        let control = Size::Medium.control();
        let frame = base(&self.hits, key.clone(), |s| s.bg(colors.ghost_element_hover).text_color(colors.text), colors.ghost_element_active)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(space::S)
            .h(control.height)
            .min_w(control.height)
            .px(TOOLBAR_PAD)
            .rounded(radius::M)
            .text_size(tokens::text::SMALL)
            .text_color(colors.text_muted)
            .map(|d| {
                let state = crate::state::State { selected: self.on, open: self.open, ..Default::default() };
                let fills = crate::state::Fills { selected: Some(colors.ghost_element_selected), open: Some(tokens::roles(cx).open), ..Default::default() };
                match crate::state::resolve(state, fills).fill {
                    Some(bg) => d.bg(bg),
                    None => d,
                }
            });
        let Some(glyph) = glyph else {
            return frame;
        };
        let text = colors.text;
        let glyph = svg_of(glyph, colors.text_muted, tokens::icon::L).group_hover(key.clone(), move |s| s.text_color(text));
        if !self.turning {
            return frame.child(glyph);
        }
        frame.child(hit_target(&self.hits, format!("{key}-turning"))).child(turning(glyph, &key))
    }
}

/// A toolbar button's side padding.
const TOOLBAR_PAD: Pixels = gpui::px(5.);

fn svg_of(glyph: Glyph, color: Hsla, size: f32) -> Svg {
    match glyph {
        Glyph::Icon(icon) => icon.svg(size, color),
        Glyph::Svg(svg) => svg,
    }
}

fn turning(svg: Svg, key: &str) -> AnyElement {
    crate::icon::turning(svg, ElementId::Name(format!("{key}-turning").into())).into_any_element()
}

impl RenderOnce for Button {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.open = crate::popup::is_open(&self.key, window, cx);
        self.build(cx)
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

/// A split button: `main` and its `menu` half (a ⌄), each a [`Button::toolbar`] under its own key, joined in one group by a rule.
#[derive(IntoElement)]
pub struct ButtonSplit {
    main: Button,
    menu: Button,
}

impl ButtonSplit {
    pub fn new(main: Button, menu: Button) -> Self {
        Self { main, menu }
    }
}

/// The rule between a split's halves, and its menu half's least width and side padding.
const SPLIT_RULE: Pixels = gpui::px(14.);
const SPLIT_MENU: Pixels = gpui::px(18.);

impl RenderOnce for ButtonSplit {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let rule = cx.theme().colors().border;
        div()
            .flex()
            .flex_none()
            .items_center()
            .child(self.main.rounded_r_none())
            .child(div().flex_none().w(stroke::HAIRLINE).h(SPLIT_RULE).bg(rule))
            .child(self.menu.rounded_l_none().min_w(SPLIT_MENU).px(space::XS))
    }
}
