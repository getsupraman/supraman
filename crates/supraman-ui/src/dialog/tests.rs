use gpui::{Bounds, Entity, KeyBinding, MouseButton, MouseDownEvent, TestAppContext, VisualTestContext, div, px, size};

use super::*;
use crate::focus::{self, Pane};

gpui::actions!(dialog_test, [CloseTab]);

struct Asking {
    hits: HitTargets,
    dialog: Dialog,
    confirm: Option<Confirm<u8>>,
    answers: Vec<Answer>,
    pane: FocusHandle,
    inside: FocusHandle,
    behind: Vec<String>,
    popup: Option<FocusHandle>,
    close_once_open: bool,
    _restore: gpui::Subscription,
}

impl Render for Asking {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = focus::root(div().id("behind"))
            .relative()
            .size_full()
            .key_context("Behind")
            .track_focus(&self.pane)
            .on_action(cx.listener(|this, _: &CloseTab, _, _| this.behind.push("close-tab".into())))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, _| this.behind.push(e.keystroke.key.clone())))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _, _| this.behind.push("click".into())))
            .child(div().id("inside").size_4().track_focus(&self.inside))
            .child(hit_target(&self.hits, "behind".into()))
            .child(Pane::new("pane", &self.pane).paint(&self.hits))
            .children(std::mem::take(&mut self.close_once_open).then(|| closer(cx.weak_entity())));
        let dialog = self.confirm.as_ref().map(|c| {
            self.dialog.ask(&self.hits, "confirm", c, |this: &mut Asking, answer, _, cx| {
                this.answers.push(answer);
                this.confirm = None;
                cx.notify();
            }, cx)
        });
        over(content, dialog).children(self.popup.as_ref().map(|p| div().id("popup").size_4().track_focus(p)))
    }
}

/// Closes the question once every defer of its first frame has run, before another frame draws: an answer clicked that fast.
fn closer(view: gpui::WeakEntity<Asking>) -> impl IntoElement {
    gpui::canvas(|_, _, _| {}, move |_, _, window, cx| {
        window.defer(cx, move |window, cx| {
            window.defer(cx, move |_, cx| {
                view.update(cx, |v, cx| {
                    v.confirm = None;
                    cx.notify();
                })
                .ok();
            })
        })
    })
}

fn open(cx: &mut TestAppContext) -> (Entity<Asking>, &mut VisualTestContext) {
    cx.update(|cx| {
        crate::init_globals(cx, "test");
        cx.bind_keys([KeyBinding::new("ctrl-w", CloseTab, Some("Behind"))]);
    });
    let (view, cx) = cx.add_window_view(|window, cx| Asking {
        hits: HitTargets::default(),
        dialog: Dialog::default(),
        confirm: None,
        answers: Vec::new(),
        pane: cx.focus_handle(),
        inside: cx.focus_handle(),
        behind: Vec::new(),
        popup: None,
        close_once_open: false,
        _restore: focus::restore_on_loss(window, cx),
    });
    cx.update(|window, _| window.activate_window());
    let inside = view.read_with(cx, |v, _| v.inside.clone());
    cx.update(|window, cx| window.focus(&inside, cx));
    cx.run_until_parked();
    (view, cx)
}

fn ask(view: &Entity<Asking>, confirm: Confirm<u8>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| {
        v.confirm = Some(confirm);
        cx.notify();
    });
    cx.run_until_parked();
}

fn holding(view: &Entity<Asking>, cx: &mut VisualTestContext) -> Option<String> {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let v = view.read(cx);
        if v.inside.is_focused(window) {
            return Some("inside".into());
        }
        v.dialog.focused(window, cx).map(|k| k.trim_start_matches("confirm-").to_string())
    })
}

fn at(view: &Entity<Asking>, key: &str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    view.read_with(cx, |v, _| v.hits.borrow().get(key).map(|h| h.bounds)).unwrap_or_else(|| panic!("{key} paints"))
}

fn taken(view: &Entity<Asking>, cx: &mut VisualTestContext) -> (Vec<Answer>, Vec<String>) {
    cx.run_until_parked();
    view.update(cx, |v, _| (std::mem::take(&mut v.answers), std::mem::take(&mut v.behind)))
}

fn three() -> Confirm<u8> {
    Confirm::ask("Remove the group supraman? Its folders move to the top level.", "Move folders out", 1).or("Remove folders too", 2).titled("Remove group")
}

#[gpui::test]
fn it_opens_with_the_default_answer_focused_and_tab_and_shift_tab_cycle_the_answers(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    ask(&view, three(), cx);
    assert_eq!(holding(&view, cx).as_deref(), Some("yes"), "the default answer holds the focus as it opens");
    let mut walked = Vec::new();
    for _ in 0..3 {
        cx.simulate_keystrokes("tab");
        walked.push(holding(&view, cx).unwrap_or_default());
    }
    assert_eq!(walked, ["alt", "cancel", "yes"], "Tab: each answer, wrapping");
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(holding(&view, cx).as_deref(), Some("cancel"), "Shift+Tab: back, wrapping");
    view.update(cx, |v, cx| {
        v.confirm = None;
        cx.notify();
    });
    cx.run_until_parked();
    ask(&view, Confirm::ask("Delete the session?", "Delete", 1).danger(), cx);
    assert_eq!(holding(&view, cx).as_deref(), Some("cancel"), "Cancel is the default of a question that destroys");
}

#[gpui::test]
fn enter_and_space_press_the_focused_answer_and_esc_cancels_and_the_focus_goes_back(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    ask(&view, three(), cx);
    cx.simulate_keystrokes("tab enter");
    assert_eq!(taken(&view, cx), (vec![Answer::Alt], vec![]), "Enter pressed the focused answer");
    assert_eq!(holding(&view, cx).as_deref(), Some("inside"), "the focus back where it was");
    ask(&view, three(), cx);
    cx.simulate_keystrokes("tab tab space");
    assert_eq!(taken(&view, cx).0, [Answer::No]);
    ask(&view, three(), cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(taken(&view, cx).0, [Answer::No], "Esc cancels");
    ask(&view, Confirm::report("Nothing was deleted."), cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(taken(&view, cx).0, [Answer::Yes], "a report's only answer");
}

#[gpui::test]
fn nothing_behind_it_takes_a_key_a_chord_or_a_click_while_it_shows(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_keystrokes("x ctrl-w");
    assert_eq!(taken(&view, cx).1, ["x", "close-tab"], "the content takes them without a dialog");
    ask(&view, three(), cx);
    cx.simulate_keystrokes("x ctrl-w f6");
    assert_eq!(holding(&view, cx).as_deref(), Some("yes"), "F6 does not leave it");
    let behind = at(&view, "behind", cx);
    cx.simulate_mouse_down(behind.origin + gpui::point(px(5.), px(5.)), MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_up(behind.origin + gpui::point(px(5.), px(5.)), MouseButton::Left, gpui::Modifiers::none());
    assert_eq!(taken(&view, cx), (vec![], vec![]), "the scrim takes the keys and the click, and answers nothing");
}

#[gpui::test]
fn the_panel_is_centred_over_the_whole_window_and_its_text_wraps_inside_it(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    ask(&view, Confirm::ask("A question long enough to wrap. ".repeat(12), "Yes", 1).titled("Long"), cx);
    let [scrim, behind, panel, text, yes, cancel] = ["confirm-scrim", "behind", "confirm-dialog", "confirm-text", "confirm-yes", "confirm-cancel"].map(|k| at(&view, k, cx));
    assert_eq!(scrim, behind, "the scrim covers the whole window");
    assert!(panel.size.width <= WIDTH && (panel.center() - scrim.center()).x.abs() < px(1.) && (panel.center() - scrim.center()).y.abs() < px(1.), "centred: {panel:?} on {scrim:?}");
    assert!(text.right() <= panel.right() && text.size.height > px(40.), "wrapped inside: {text:?}");
    assert!(yes.top() > text.bottom() && cancel.right() <= panel.right() && yes.right() < cancel.left(), "the answers under it, right-aligned");
    cx.simulate_resize(size(px(300.), px(200.)));
    let [scrim, panel] = ["confirm-scrim", "confirm-dialog"].map(|k| at(&view, k, cx));
    assert!(panel.left() >= scrim.left() + MARGIN - px(1.) && panel.bottom() <= scrim.bottom() - MARGIN + px(1.), "never past the window: {panel:?} in {scrim:?}");
}

#[gpui::test]
fn a_question_closed_before_a_frame_showed_its_focus_gives_the_keys_back(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let popup = view.update(cx, |v, cx| v.popup.insert(cx.focus_handle()).clone());
    cx.run_until_parked();
    cx.update(|window, cx| window.focus(&popup, cx));
    cx.run_until_parked();
    assert!(cx.update(|window, _| popup.is_focused(window)), "a menu holds the focus");
    view.update(cx, |v, cx| {
        v.popup = None;
        v.confirm = Some(Confirm::ask("Delete the session?", "Delete", 1).danger());
        v.close_once_open = true;
        cx.notify();
    });
    assert_eq!(holding(&view, cx).as_deref(), Some("inside"), "the focus back in the pane, not on the closed question's answer");
    assert!(view.read_with(cx, |v, _| v.confirm.is_none()), "closed before a second frame");
    cx.simulate_keystrokes("x");
    assert_eq!(taken(&view, cx).1, ["x"], "a key reaches the pane");
}
