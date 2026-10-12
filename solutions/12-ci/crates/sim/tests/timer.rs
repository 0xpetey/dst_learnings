//! Acceptance tests for the timer helper.

use sim::{Duration, Instant, Scheduler, Timer};

fn at(ms: u64) -> Instant {
    Instant::ZERO + Duration::from_millis(ms)
}

#[test]
fn reconcile_schedules_once_per_distinct_deadline() {
    let mut s = Scheduler::new();
    let mut t = Timer::new();
    t.reconcile(&mut s, Some(at(10)), || "a");
    t.reconcile(&mut s, Some(at(10)), || panic!("not needed"));
    assert_eq!(s.stats().scheduled, 1);
    t.reconcile(&mut s, Some(at(20)), || "b");
    assert_eq!(s.stats().cancelled, 1);
    let f = s.pop().unwrap();
    assert_eq!((f.at, f.event), (at(20), "b"));
    assert!(t.fired(f.id));
    assert_eq!(t.deadline(), None);
    assert!(s.pop().is_none(), "the cancelled event never fires");
}

#[test]
fn reconcile_to_none_cancels() {
    let mut s = Scheduler::new();
    let mut t = Timer::new();
    t.reconcile(&mut s, Some(at(10)), || ());
    t.reconcile(&mut s, None, || ());
    assert_eq!(t.deadline(), None);
    assert!(s.pop().is_none());
}

#[test]
fn a_foreign_id_is_not_ours() {
    let mut s = Scheduler::new();
    let mut t = Timer::new();
    let other = s.schedule_at(at(1), ());
    t.reconcile(&mut s, Some(at(2)), || ());
    assert!(!t.fired(other));
    assert_eq!(t.deadline(), Some(at(2)));
}
