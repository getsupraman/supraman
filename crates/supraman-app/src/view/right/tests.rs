use std::collections::BTreeSet;

use gpui::{AnyElement, Context, FocusHandle, IntoElement, Render, ScrollHandle, TestAppContext, Window, div, prelude::*, px};
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::patterns::details::{self, Section};

use super::area;

const PANELS: [(&str, &str); 4] = [("t1", "terminal"), ("t2", "terminal"), ("d1", "details"), ("d2", "details")];

/// Two terminal contents and two details pages side by side, as four panels of a dock, each under its scope.
struct Panels {
    hits: HitTargets,
    focus: Vec<FocusHandle>,
    scroll: ScrollHandle,
    sections: Vec<Section>,
}

impl Panels {
    fn content(&self, pane: &str, hits: &HitTargets, cx: &mut Context<Self>) -> AnyElement {
        if pane == "terminal" {
            return div().relative().size_full().child(hit_target(hits, "terminal-1".into())).into_any_element();
        }
        let page = details::Page { key: "details-w", title: "w".into(), pill: None, sections: &self.sections, actions: Vec::new(), scroll: &self.scroll, on_link: None, top: None, over: None, bar: None };
        details::render(page, hits, cx)
    }
}

impl Render for Panels {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = supraman_ui::focus::root(div().flex().w(px(1600.)).h(px(600.)));
        for (i, (scope, pane)) in PANELS.into_iter().enumerate() {
            let hits = self.hits.scoped(scope);
            let content = self.content(pane, &hits, cx);
            row = row.child(div().flex().flex_col().w(px(400.)).h_full().child(area([content], Some(pane), &self.focus[i], &hits)));
        }
        row
    }
}

#[gpui::test]
fn two_terminal_contents_and_two_details_pages_register_their_panes_and_keys_apart(cx: &mut TestAppContext) {
    cx.update(|cx| supraman_ui::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, cx| Panels {
        hits: HitTargets::default(),
        focus: PANELS.iter().map(|_| cx.focus_handle()).collect(),
        scroll: ScrollHandle::new(),
        sections: vec![Section::new("facts", "Facts").row("State", "working").path("Folder", std::path::Path::new("p:/w/alpha"))],
    });
    cx.update(|window, _| window.activate_window());
    for _ in 0..2 {
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        cx.update(|_, _| ());
    }

    let order: Vec<String> = cx.update(|window, cx| supraman_ui::focus::order(window, cx).into_iter().map(|k| k.to_string()).collect());
    let expected: Vec<String> = PANELS.iter().map(|(scope, pane)| format!("{scope}/{pane}")).collect();
    assert_eq!(order, expected, "each content registers its pane under its own scope");

    let keys: BTreeSet<String> = view.read_with(cx, |v, _| v.hits.borrow().keys().cloned().collect());
    for scope in ["t1", "t2"] {
        assert!(keys.contains(&format!("{scope}/terminal-1")), "{scope}'s terminal paints under its scope: {keys:?}");
    }
    for scope in ["d1", "d2"] {
        assert!(keys.contains(&format!("{scope}/details-w-section-facts")), "{scope}'s details page paints under its scope: {keys:?}");
    }
    let bare: Vec<&String> = keys.iter().filter(|k| !PANELS.iter().any(|(s, _)| k.starts_with(&format!("{s}/")))).collect();
    assert!(bare.is_empty(), "no content paints a key outside its scope: {bare:?}");
}
