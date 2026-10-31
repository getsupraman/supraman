use std::time::Duration;

use anyhow::bail;
use gpui::{AsyncWindowContext, Entity};

use supraman_ui::notices::dismiss_key;

use crate::{
    selftest::{click, until},
    selftest_manage::shot,
    view::{FleetView, notices::Notice},
};

const FIRST: &str = "fm-selftest: Workman session hand-selftest-1, not fm-selftest-x: replace does not override that";
const SECOND: &str = "fm-selftest: a second hand notice";
const ACTION: &str = "fm-selftest: Cannot do the thing";
const WITHIN: Duration = Duration::from_secs(10);

/// The hits are the last frame's, so a frame from before a change would point elsewhere.
fn hand_x(v: &FleetView, text: &str) -> Option<String> {
    let hits = v.hits.borrow();
    let count = v.hand_notices.len();
    if !(0..count).all(|i| hits.contains_key(&dismiss_key(Notice::Hand, i))) || hits.contains_key(&dismiss_key(Notice::Hand, count)) {
        return None;
    }
    v.hand_notices.iter().position(|t| t == text).map(|i| dismiss_key(Notice::Hand, i))
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    crate::selftest_messages::pointer_off(view, cx).await?;
    view.update(cx, |v, cx| {
        v.hand_notices.push(FIRST.into());
        v.hand_notices.push(SECOND.into());
        v.action_error = Some(ACTION.into());
        cx.notify();
    });
    let action_x = dismiss_key(Notice::Action, 0);
    let first_x = until(view, cx, WITHIN, "the hand notices and the action error painted with a ×", |v| {
        let action = v.hits.borrow().contains_key(&action_x);
        (action && hand_x(v, SECOND).is_some()).then(|| hand_x(v, FIRST)).flatten()
    })
    .await?;
    shot("m5-37-notices-before", cx).await;

    click(view, &first_x, cx).await?;
    let second = until(view, cx, WITHIN, "a frame after the first hand notice's ×", |v| {
        (!v.hand_notices.iter().any(|t| t == FIRST)).then(|| hand_x(v, SECOND)).flatten()
    })
    .await;
    if second.is_err() || view.read_with(cx, |v, _| v.action_error.as_deref() != Some(ACTION)) {
        bail!("× on the first hand notice did not remove it alone: (hand notices, action error) {:?}; {:?}", view.read_with(cx, |v, _| (v.hand_notices.clone(), v.action_error.clone())), second.err());
    }
    shot("m5-37-notices-after", cx).await;

    view.update(cx, |v, cx| {
        v.hand_notices.push(FIRST.into());
        cx.notify();
    });
    let again_x = until(view, cx, WITHIN, "the first hand notice shown again once raised again", |v| hand_x(v, FIRST)).await?;
    click(view, &again_x, cx).await?;
    let second_x = until(view, cx, WITHIN, "the raised-again notice gone after its ×", |v| {
        (!v.hand_notices.iter().any(|t| t == FIRST)).then(|| hand_x(v, SECOND)).flatten()
    })
    .await?;
    click(view, &second_x, cx).await?;
    until(view, cx, WITHIN, "the second hand notice gone, its × unpainted", |v| {
        (v.hand_notices.is_empty() && !v.hits.borrow().contains_key(&dismiss_key(Notice::Hand, 0))).then_some(())
    })
    .await?;
    click(view, &action_x, cx).await?;
    until(view, cx, WITHIN, "the action error gone after its ×", |v| v.action_error.is_none().then_some(())).await?;

    Ok("× on one of 2 hand notices removed only it (the other and the action error stayed); raised again it showed again and its × removed it; × removed the second and the action error".into())
}
