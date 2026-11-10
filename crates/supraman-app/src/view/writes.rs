//! The view's writes that must land in the order they were made.

use futures::FutureExt as _;

/// A poll's report gives way to these ([`MaxWrites`]).
#[derive(Default)]
pub(super) struct Writes {
    pub(super) sends: Serial,
    pub(super) max: MaxWrites,
}

/// The human's sends, each started only after the one before it has finished, so they arrive in the order sent.
#[derive(Default)]
pub(crate) struct Serial(Option<futures::future::Shared<futures::future::LocalBoxFuture<'static, ()>>>);

impl Serial {
    pub(crate) fn previous(&self) -> Option<futures::future::Shared<futures::future::LocalBoxFuture<'static, ()>>> {
        self.0.clone()
    }

    pub(crate) fn push(&mut self, next: impl std::future::Future<Output = ()> + 'static) {
        self.0 = Some(next.boxed_local().shared());
    }
}

/// A tick's limit counts only when no write is pending and the tick started after the last one landed, so a stale read never undoes a click.
#[derive(Default)]
pub(super) struct MaxWrites {
    pub(super) serial: Serial,
    set: u64,
    landed: u64,
    tick_saw: u64,
}

impl MaxWrites {
    pub(super) fn set(&mut self) -> u64 {
        self.set += 1;
        self.set
    }

    pub(super) fn landed(&mut self, generation: u64) {
        self.landed = self.landed.max(generation);
    }

    pub(super) fn tick_starts(&mut self) {
        self.tick_saw = self.landed;
    }

    pub(super) fn report_counts(&self) -> bool {
        self.landed == self.set && self.tick_saw == self.landed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poll_report_never_undoes_a_busy_limit_set_since_its_tick_started() {
        let mut w = MaxWrites::default();
        w.tick_starts();
        assert!(w.report_counts(), "nothing set: the report is the file's value");
        let one = w.set();
        assert!(!w.report_counts(), "set, not yet written");
        w.tick_starts();
        w.landed(one);
        assert!(!w.report_counts(), "this tick read the file before the write landed");
        let two = w.set();
        w.tick_starts();
        assert!(!w.report_counts(), "a second set is pending");
        w.landed(two);
        assert!(!w.report_counts());
        w.tick_starts();
        assert!(w.report_counts(), "a tick started after the last write landed");
        w.landed(one);
        assert!(w.report_counts(), "an older generation never moves it back");
    }

    #[test]
    fn a_send_starts_only_after_the_one_queued_before_it_has_finished() {
        use futures::task::LocalSpawnExt as _;
        let mut pool = futures::executor::LocalPool::new();
        let spawner = pool.spawner();
        let order = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sends = Serial::default();
        for (n, yields) in [(1, 50), (2, 0)] {
            let (previous, order) = (sends.previous(), order.clone());
            let handle = spawner
                .spawn_local_with_handle(async move {
                    if let Some(previous) = previous {
                        previous.await;
                    }
                    for _ in 0..yields {
                        let mut yielded = false;
                        futures::future::poll_fn(|cx| {
                            if std::mem::replace(&mut yielded, true) {
                                return std::task::Poll::Ready(());
                            }
                            cx.waker().wake_by_ref();
                            std::task::Poll::Pending
                        })
                        .await;
                    }
                    order.borrow_mut().push(n);
                })
                .unwrap();
            sends.push(handle);
        }
        pool.run_until(sends.previous().unwrap());
        assert_eq!(*order.borrow(), [1, 2]);
    }
}
