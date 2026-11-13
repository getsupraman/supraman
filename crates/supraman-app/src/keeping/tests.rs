use std::collections::BTreeSet;

use super::*;

const NAME: &str = "fm-demo-tab";
const OTHER: &str = "fm-demo-alpha";

fn checked(kept: &[&str], renewed: &[(&str, Result<(), &str>)]) -> Checked {
    Checked {
        kept: kept.iter().map(|n| n.to_string()).collect::<BTreeSet<_>>(),
        renewed: renewed.iter().map(|(n, r)| (n.to_string(), r.map_err(str::to_string))).collect(),
        notices: Vec::new(),
    }
}

fn shown(r: &Renewals) -> Vec<String> {
    r.notices().cloned().collect()
}

#[test]
fn a_failed_renewal_is_a_notice_until_that_sessions_next_renewal_succeeds() {
    let mut r = Renewals::default();
    assert!(r.apply(&checked(&[NAME, OTHER], &[(NAME, Err("Workman is unreachable")), (OTHER, Ok(()))])));
    assert_eq!(shown(&r), ["Cannot renew the Workman token of fm-demo-tab: Workman is unreachable"]);

    assert!(!r.apply(&checked(&[NAME, OTHER], &[])), "a check that tries nothing keeps it");
    assert!(r.apply(&checked(&[NAME, OTHER], &[(NAME, Err("HTTP 500"))])));
    assert_eq!(shown(&r), ["Cannot renew the Workman token of fm-demo-tab: HTTP 500"], "replaced, one per session");

    assert!(r.apply(&checked(&[NAME, OTHER], &[(NAME, Ok(()))])));
    assert!(shown(&r).is_empty(), "cleared by its next renewal");
}

#[test]
fn a_renewal_notice_goes_with_its_record_and_a_dismissed_one_shows_again_once_cleared() {
    let mut r = Renewals::default();
    r.apply(&checked(&[NAME], &[(NAME, Err("down"))]));
    assert!(r.apply(&checked(&[], &[])));
    assert!(shown(&r).is_empty(), "its record is gone");

    r.apply(&checked(&[NAME], &[(NAME, Err("down"))]));
    r.dismiss("Cannot renew the Workman token of fm-demo-tab: down");
    assert!(shown(&r).is_empty());
    r.apply(&checked(&[NAME], &[(NAME, Err("down"))]));
    assert!(shown(&r).is_empty(), "dismissed while it keeps failing alike");
    r.apply(&checked(&[NAME], &[(NAME, Ok(()))]));
    r.apply(&checked(&[NAME], &[(NAME, Err("down"))]));
    assert_eq!(shown(&r).len(), 1, "a new failure after a success shows");
}
