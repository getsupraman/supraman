//! The prompt chips of a run or a blocked item: Workman's `/api/v1/prompts`, labels, counts and text as served.

use gpui::{ClipboardItem, FontWeight};
use supraman_ui::button;
use supraman_ui::fonts;
use supraman_workman::Prompt;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PromptsState {
    Loading,
    Ready(Vec<Prompt>),
    Failed(String),
}

#[derive(Default)]
pub(crate) struct Prompts {
    pub(crate) by_scope: HashMap<String, PromptsState>,
    pending: HashSet<String>,
    /// The expanded chip: its scope and key.
    pub(crate) open: Option<(String, String)>,
}

impl Prompts {
    pub(crate) fn collapse(&mut self) -> bool {
        self.open.take().is_some()
    }

    fn shown(&self, scope: &str) -> &[Prompt] {
        match self.by_scope.get(scope) {
            Some(PromptsState::Ready(prompts)) => prompts,
            _ => &[],
        }
    }
}

pub(crate) const NOTHING_RUNS: &str = "Paste it into a fresh Claude session. Nothing runs from here.";

pub(crate) fn chip_label(p: &Prompt) -> String {
    match p.count {
        Some(n) => format!("{} ({n})", p.label),
        None => p.label.clone(),
    }
}

impl WorkView {
    pub(crate) fn read_prompts(&mut self, scope: String, cx: &mut Context<Self>) {
        let Some(client) = self.client(cx) else { return };
        self.prompts.by_scope.entry(scope.clone()).or_insert(PromptsState::Loading);
        cx.spawn(async move |this, cx| {
            let asked = scope.clone();
            let read = cx.background_executor().spawn(async move { client.prompts(&asked).map_err(read_failed) }).await;
            this.update(cx, |this, cx| {
                match read {
                    Ok(p) => {
                        this.prompts.by_scope.insert(scope, PromptsState::Ready(p.prompts));
                    }
                    Err(_) if matches!(this.prompts.by_scope.get(&scope), Some(PromptsState::Ready(_))) => {}
                    Err(why) => {
                        this.prompts.by_scope.insert(scope, PromptsState::Failed(why));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn schedule_prompts(&mut self, scope: String, cx: &mut Context<Self>) {
        if self.prompts.pending.insert(scope.clone()) {
            after_debounce(cx, move |this, cx| {
                this.prompts.pending.remove(&scope);
                this.read_prompts(scope, cx);
            });
        }
    }

    fn toggle_prompt(&mut self, scope: &str, key: &str, cx: &mut Context<Self>) {
        let wanted = (scope.to_string(), key.to_string());
        self.prompts.open = if self.prompts.open.as_ref() == Some(&wanted) { None } else { Some(wanted) };
        cx.notify();
    }

    /// The chips of `scope`, painted under `work-prompt-<place>-<key>`, and the expanded one's text with Copy and Close.
    pub(super) fn render_prompts(&self, scope: &str, place: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompts = self.prompts.shown(scope);
        if prompts.is_empty() {
            return None;
        }
        let colors = cx.theme().colors().clone();
        let open = self.prompts.open.as_ref().filter(|(s, _)| s == scope).and_then(|(_, key)| prompts.iter().find(|p| &p.key == key));
        let chips = prompts.iter().map(|p| {
            let (scope, key) = (scope.to_string(), p.key.clone());
            let on = open.is_some_and(|o| o.key == p.key);
            button::action(&self.hits, format!("work-prompt-{place}-{}", p.key), chip_label(p), button::Tone::Neutral, button::Fill::Outline, cx)
                .when(on, |d| d.bg(colors.element_selected))
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| this.toggle_prompt(&scope, &key, cx)))
        });
        let mut out = div().flex().flex_col().gap_2().child(div().flex().flex_wrap().gap_1().children(chips.collect::<Vec<_>>()));
        if let Some(p) = open {
            let (text, label) = (p.text.clone(), p.label.clone());
            out = out.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.element_background)
                    .child(div().font_family(fonts::MONO).text_xs().whitespace_normal().child(SharedString::from(p.text.clone())))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(
                                button::action(&self.hits, format!("work-prompt-{place}-copy"), "Copy", button::Tone::Neutral, button::Fill::Outline, cx)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                                        this.toast.show(format!("Copied the {label} prompt"), |this: &mut WorkView| &mut this.toast, cx);
                                    })),
                            )
                            .child(button::action(&self.hits, format!("work-prompt-{place}-close"), "Close", button::Tone::Neutral, button::Fill::Outline, cx).on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                this.prompts.collapse();
                                cx.notify();
                            })))
                            .child(div().text_xs().text_color(colors.text_muted).child(NOTHING_RUNS)),
                    ),
            );
        }
        Some(out.into_any_element())
    }
}
