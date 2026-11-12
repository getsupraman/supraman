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

mod pair {
    use gpui::{Context, Entity, Modifiers, Render, TestAppContext, VisualTestContext, Window, px};

    use super::*;
    use crate::hit::{HitTargets, hit_target};
    use crate::tooltip::LAST_SHOWN;

    const BRIGHT: &str = "feature-branch";
    const MUTED: &str = "P:/Projects/somewhere/far/away";

    struct Holder {
        hits: HitTargets,
        width: f32,
    }

    impl Render for Holder {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let bright = div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(BRIGHT).child(hit_target(&self.hits, "name".into()));
            let line = pair("line", BRIGHT, bright, MUTED, gpui::black()).child(hit_target(&self.hits, "line".into()));
            div().w(px(self.width)).flex().child(line.relative().flex_1())
        }
    }

    fn holder(width: f32, cx: &mut TestAppContext) -> (Entity<Holder>, &mut VisualTestContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (view, cx) = cx.add_window_view(|_, _| Holder { hits: HitTargets::default(), width });
        cx.run_until_parked();
        (view, cx)
    }

    fn resize(view: &Entity<Holder>, width: f32, cx: &mut VisualTestContext) {
        view.update(cx, |h, cx| {
            h.width = width;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    fn name_width(view: &Entity<Holder>, cx: &mut VisualTestContext) -> f32 {
        f32::from(view.read_with(cx, |h, _| h.hits.borrow()["name"].bounds.size.width))
    }

    fn tooltip(view: &Entity<Holder>, cx: &mut VisualTestContext) -> Option<SharedString> {
        let at = view.read_with(cx, |h, _| h.hits.borrow()["line"].bounds.center());
        LAST_SHOWN.with(|l| l.borrow_mut().take());
        cx.simulate_mouse_move(at, None, Modifiers::none());
        cx.executor().advance_clock(std::time::Duration::from_secs(2));
        cx.run_until_parked();
        let shown = LAST_SHOWN.with(|l| l.borrow().clone());
        cx.simulate_mouse_move(gpui::point(px(0.), px(300.)), None, Modifiers::none());
        cx.executor().advance_clock(std::time::Duration::from_secs(2));
        cx.run_until_parked();
        shown
    }

    fn whole_and_none_cut(cx: &mut TestAppContext) -> (Entity<Holder>, &mut VisualTestContext, f32) {
        let (view, cx) = holder(800., cx);
        let whole = name_width(&view, cx);
        assert!(cut_at("line-bright").is_none() && cut_at("line-muted").is_none(), "both fit at 800 px");
        assert_eq!(tooltip(&view, cx), None, "nothing cut, no tooltip");
        (view, cx, whole)
    }

    #[gpui::test]
    fn the_muted_part_is_cut_first_and_the_whole_text_is_the_tooltip(cx: &mut TestAppContext) {
        let (view, cx, whole) = whole_and_none_cut(cx);
        resize(&view, whole + 30., cx);
        assert!(cut_at("line-muted").is_some(), "the muted part is cut first");
        assert!(cut_at("line-bright").is_none(), "the bright part stays whole while the muted part shrinks");
        assert!((name_width(&view, cx) - whole).abs() < 0.5, "the bright part keeps its width");
        assert_eq!(tooltip(&view, cx).as_deref(), Some(format!("{BRIGHT} {MUTED}").as_str()));
    }

    #[gpui::test]
    fn narrower_than_the_bright_part_it_is_cut_too(cx: &mut TestAppContext) {
        let (view, cx, whole) = whole_and_none_cut(cx);
        resize(&view, whole / 2., cx);
        assert!(cut_at("line-muted").is_some() && cut_at("line-bright").is_some(), "both cut");
        assert!(name_width(&view, cx) <= whole / 2. + 0.5, "cut to the line's width");
        assert_eq!(tooltip(&view, cx).as_deref(), Some(format!("{BRIGHT} {MUTED}").as_str()));
    }
}
