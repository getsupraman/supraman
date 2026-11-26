// Copyright 2024 - 2026 Longbridge <https://longbridge.com>
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may not use this file except in compliance with the
// License. You may obtain a copy of the License at http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the License for the specific
// language governing permissions and limitations under the License.
//
// From gpui-kit (https://github.com/longbridge/gpui-kit) `crates/base/src/input/base/blink_cursor.rs` @ f807c1b. Changed
// for Supraman: one interval (`tokens::motion::CARET_BLINK`), no policy or idle timeout, a pause lit for a whole interval,
// and [`Blink::follow`] running it only while its owner holds the keys in an active window.

//! Every caret's and terminal cursor's blink (UI-KIT §4.3): shown and blinking while its owner holds the keys and its
//! window is active, hidden and with no timer otherwise; each keystroke shows it lit for an interval.

use gpui::{App, AppContext as _, Context, Entity, FocusHandle, Subscription, Task, Window};

use crate::tokens::motion::CARET_BLINK;

pub struct Blink {
    visible: bool,
    running: bool,
    epoch: usize,
    task: Task<()>,
}

impl Blink {
    pub fn new(_: &mut Context<Self>) -> Self {
        Self { visible: false, running: false, epoch: 0, task: Task::ready(()) }
    }

    /// `owner`'s blink: running while `focus` is focused and its window active, stopped otherwise; `owner` repaints with it.
    pub fn follow<V: 'static>(focus: &FocusHandle, window: &mut Window, cx: &mut Context<V>) -> (Entity<Blink>, [Subscription; 4]) {
        Self::follow_while(focus, |_| true, window, cx)
    }

    /// [`Blink::follow`] that also stays stopped while `wanted` says `owner` shows no caret (read-only text).
    pub fn follow_while<V: 'static>(focus: &FocusHandle, wanted: fn(&V) -> bool, window: &mut Window, cx: &mut Context<V>) -> (Entity<Blink>, [Subscription; 4]) {
        let blink = cx.new(Blink::new);
        let run = |blink: &Entity<Blink>, focus: &FocusHandle, wanted: bool, window: &Window, cx: &mut App| {
            let on = wanted && focus.is_focused(window) && window.is_window_active();
            blink.update(cx, |b, cx| b.set_running(on, cx));
        };
        let (b, f) = (blink.clone(), focus.clone());
        let focused = cx.on_focus(focus, window, move |v, window, cx| run(&b, &f, wanted(v), window, cx));
        let b = blink.clone();
        let blurred = cx.on_blur(focus, window, move |_, _, cx| b.update(cx, |b, cx| b.stop(cx)));
        let (b, f) = (blink.clone(), focus.clone());
        let activation = cx.observe_window_activation(window, move |v, window, cx| run(&b, &f, wanted(v), window, cx));
        let repaint = cx.observe(&blink, |_, _, cx| cx.notify());
        run(&blink, focus, true, window, cx);
        (blink, [focused, blurred, activation, repaint])
    }

    /// Started or stopped; nothing while it already is.
    pub fn set_running(&mut self, on: bool, cx: &mut Context<Self>) {
        match (on, self.running) {
            (true, false) => self.start(cx),
            (false, true) => self.stop(cx),
            _ => {}
        }
    }

    /// Shown, then off and on every interval.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        self.running = true;
        self.show_lit(cx);
    }

    /// Hidden, its timer dropped.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.epoch = self.epoch.wrapping_add(1);
        self.running = false;
        self.task = Task::ready(());
        self.set_visible(false, cx);
    }

    /// A keystroke or a move: shown for a whole interval before it blinks on; nothing while stopped.
    pub fn pause(&mut self, cx: &mut Context<Self>) {
        if self.running {
            self.show_lit(cx);
        }
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Whether its timer runs.
    pub fn running(&self) -> bool {
        self.running
    }

    fn show_lit(&mut self, cx: &mut Context<Self>) {
        self.epoch = self.epoch.wrapping_add(1);
        self.set_visible(true, cx);
        self.schedule(cx);
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible != visible {
            self.visible = visible;
            cx.notify();
        }
    }

    fn schedule(&mut self, cx: &mut Context<Self>) {
        let epoch = self.epoch;
        let timer = cx.background_executor().timer(CARET_BLINK);
        self.task = cx.spawn(async move |this, cx| {
            timer.await;
            _ = this.update(cx, |this, cx| {
                if epoch != this.epoch || !this.running {
                    return;
                }
                this.set_visible(!this.visible, cx);
                this.schedule(cx);
            });
        });
    }
}

#[cfg(test)]
mod tests;
