//! Acceptance tests for the scheduler. Everything here uses the public API,
//! so these run against any implementation with the same surface.

use sim::{Duration, Instant, Scheduler};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn at(n: u64) -> Instant {
    Instant::ZERO + ms(n)
}

fn drain(s: &mut Scheduler<&'static str>) -> Vec<(Instant, &'static str)> {
    let mut out = Vec::new();
    while let Some(f) = s.pop() {
        assert_eq!(s.now(), f.at, "clock follows the popped event");
        out.push((f.at, f.event));
    }
    out
}

#[test]
fn events_pop_in_time_order_and_the_clock_follows() {
    let mut s = Scheduler::new();
    s.schedule_at(at(30), "c");
    s.schedule_at(at(10), "a");
    s.schedule_at(at(20), "b");
    assert_eq!(s.now(), at(0));
    assert_eq!(drain(&mut s), vec![(at(10), "a"), (at(20), "b"), (at(30), "c")]);
    assert_eq!(s.now(), at(30));
}

#[test]
fn same_time_pops_in_schedule_order_whatever_the_payload() {
    // Payloads chosen so that any ordering by payload would differ.
    let mut s = Scheduler::new();
    s.schedule_at(at(5), "z");
    s.schedule_at(at(5), "a");
    s.schedule_at(at(5), "m");
    let got: Vec<_> = drain(&mut s).into_iter().map(|(_, e)| e).collect();
    assert_eq!(got, vec!["z", "a", "m"]);
}

#[test]
fn schedule_after_is_relative_to_the_moving_clock() {
    let mut s = Scheduler::new();
    s.schedule_after(ms(10), "first");
    let f = s.pop().unwrap();
    assert_eq!(f.at, at(10));
    s.schedule_after(ms(10), "second");
    assert_eq!(s.pop().unwrap().at, at(20));
}

#[test]
#[should_panic(expected = "scheduled in the past")]
fn scheduling_in_the_past_is_a_bug() {
    let mut s = Scheduler::new();
    s.schedule_at(at(10), "x");
    s.pop();
    s.schedule_at(at(5), "y");
}

#[test]
fn cancel_removes_an_event_and_reports_whether_it_was_live() {
    let mut s = Scheduler::new();
    let a = s.schedule_at(at(1), "a");
    let b = s.schedule_at(at(2), "b");
    let c = s.schedule_at(at(3), "c");
    assert!(s.cancel(b));
    assert!(!s.cancel(b), "second cancel is a no-op");
    assert_eq!(s.pop().unwrap().event, "a");
    assert!(!s.cancel(a), "already fired");
    assert_eq!(s.pop().unwrap().event, "c");
    assert!(!s.cancel(c));
    assert_eq!(s.stats().cancelled, 1);
    assert_eq!(s.stats().fired, 2);
    assert!(s.is_empty());
}

#[test]
fn peek_and_pop_until_respect_the_bound() {
    let mut s = Scheduler::new();
    s.schedule_at(at(10), "a");
    s.schedule_at(at(20), "b");
    assert_eq!(s.peek(), Some(at(10)));
    assert_eq!(s.pop_until(at(15)).map(|f| f.event), Some("a"));
    assert_eq!(s.pop_until(at(15)), None);
    assert_eq!(s.now(), at(10), "pop_until does not move the clock to the bound");
    s.advance_to(at(15));
    assert_eq!(s.now(), at(15));
    assert_eq!(s.pop_until(at(20)).map(|f| f.event), Some("b"));
}

#[test]
#[should_panic(expected = "would skip an event")]
fn advance_past_a_pending_event_is_a_bug() {
    let mut s = Scheduler::new();
    s.schedule_at(at(10), "a");
    s.advance_to(at(11));
}

#[test]
fn idle_time_costs_nothing() {
    // A decade of nothing is one pop.
    let mut s = Scheduler::new();
    s.schedule_after(Duration::from_secs(10 * 365 * 24 * 3600), "later");
    assert_eq!(s.pop().unwrap().event, "later");
    assert_eq!(s.now().nanos(), 10 * 365 * 24 * 3600 * 1_000_000_000);
}

#[test]
fn clock_handles_see_the_same_time() {
    let mut s: Scheduler<()> = Scheduler::new();
    let c1 = s.clock();
    let c2 = s.clock();
    s.schedule_after(ms(7), ());
    s.pop();
    assert_eq!(c1.now(), at(7));
    assert_eq!(c2.now(), at(7));
}
