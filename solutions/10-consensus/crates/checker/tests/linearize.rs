//! Module 8 acceptance tests for the checker alone: histories built by
//! hand, with the answers known.
//!
//! The pictures in the comments use one line per client. `├──┤` is an
//! operation from call to return, and `├──` without a return is one the
//! client gave up on.

use checker::kv::{self, KvModel};
use checker::linearize::{self, Verdict};
use checker::model::{check_sequential, SequentialError};
use checker::{History, Op};
use kv_core::{Instant, Request, Response};

const LIMIT: u64 = 100_000;

fn at(ms: u64) -> Instant {
    Instant::from_nanos(ms * 1_000_000)
}
fn put(k: &str, v: &str) -> Request {
    Request::Put {
        key: k.into(),
        value: v.into(),
    }
}
fn get(k: &str) -> Request {
    Request::Get { key: k.into() }
}
fn del(k: &str) -> Request {
    Request::Delete { key: k.into() }
}
fn value(v: &str) -> Response {
    Response::Value(Some(v.into()))
}
const NONE: Response = Response::Value(None);

/// A completed operation.
fn done(client: u64, request: Request, call: u64, ret: u64, response: Response) -> Op<Request, Response> {
    Op {
        client,
        request_id: 0,
        request,
        call: at(call),
        ret: Some(at(ret)),
        response: Some(response),
    }
}

/// An operation the client gave up on.
fn open(client: u64, request: Request, call: u64) -> Op<Request, Response> {
    Op {
        client,
        request_id: 0,
        request,
        call: at(call),
        ret: None,
        response: None,
    }
}

fn history(mut ops: Vec<Op<Request, Response>>) -> History<Request, Response> {
    // Request ids per client, in call order, as a real client would issue.
    ops.sort_by_key(|op| op.call);
    let mut next = std::collections::BTreeMap::new();
    for op in &mut ops {
        let n = next.entry(op.client).or_insert(1u64);
        op.request_id = *n;
        *n += 1;
    }
    History::from_ops(ops)
}

fn check(h: &History<Request, Response>) -> Verdict<Response> {
    linearize::check::<KvModel>(h, LIMIT)
}

#[test]
fn a_sequential_history_is_checked_by_the_model_alone() {
    //  c1  ├ put x=1 ┤          ├ get x → 1 ┤
    //  c2             ├ del x ┤               ├ get x → None ┤
    let h = history(vec![
        done(1, put("x", "1"), 0, 10, Response::PutOk),
        done(2, del("x"), 20, 30, Response::DeleteOk { existed: true }),
        done(1, get("x"), 40, 50, NONE),
    ]);
    assert!(h.is_sequential());
    assert_eq!(h.max_concurrency(), 1);
    assert!(check_sequential::<KvModel>(&h).is_ok());
    let v = check(&h);
    assert!(v.is_linearizable(), "{v:?}");
    assert_eq!(v.stats().steps, 3, "one candidate per step, no search");

    // The same history with one wrong answer.
    let h = history(vec![
        done(1, put("x", "1"), 0, 10, Response::PutOk),
        done(2, del("x"), 20, 30, Response::DeleteOk { existed: true }),
        done(1, get("x"), 40, 50, value("1")),
    ]);
    assert_eq!(
        check_sequential::<KvModel>(&h),
        Err(SequentialError::Mismatch {
            index: 2,
            expected: NONE,
            got: value("1")
        })
    );
    let Verdict::Illegal(illegal) = check(&h) else {
        panic!("a stale read is not linearizable")
    };
    assert_eq!(illegal.linearized, vec![0, 1]);
    assert_eq!(illegal.stuck, vec![(2, NONE)]);
    assert!(illegal
        .to_string()
        .starts_with("not linearizable: after 2 operations"));
}

#[test]
fn a_concurrent_read_may_see_either_side_of_a_write() {
    //  c1  ├──── put x=1 ────┤
    //  c2     ├── get x ──┤
    for response in [NONE, value("1")] {
        let h = history(vec![
            done(1, put("x", "1"), 0, 20, Response::PutOk),
            done(2, get("x"), 5, 15, response),
        ]);
        assert!(!h.is_sequential());
        assert_eq!(h.first_overlap(), Some((0, 1)));
        assert_eq!(
            check_sequential::<KvModel>(&h),
            Err(SequentialError::NotSequential { first: 0, second: 1 })
        );
        assert!(check(&h).is_linearizable());
    }
}

#[test]
fn a_value_cannot_be_unseen() {
    //  c1  ├──── put x=1 ────┤
    //  c2     ├ get x → 1 ┤      ├ get x → None ┤
    let h = history(vec![
        done(1, put("x", "1"), 0, 20, Response::PutOk),
        done(2, get("x"), 5, 10, value("1")),
        done(2, get("x"), 25, 30, NONE),
    ]);
    let Verdict::Illegal(illegal) = check(&h) else {
        panic!("the second read went back in time")
    };
    assert_eq!(illegal.linearized, vec![0, 1]);
    assert_eq!(illegal.stuck, vec![(2, value("1"))]);

    // Swap the responses and the history is fine: the write took effect
    // between the two reads, inside its own window.
    let h = history(vec![
        done(1, put("x", "1"), 0, 20, Response::PutOk),
        done(2, get("x"), 5, 10, NONE),
        done(2, get("x"), 25, 30, value("1")),
    ]);
    let Verdict::Linearizable { order, .. } = check(&h) else {
        panic!("linearizable")
    };
    assert_eq!(order, vec![1, 0, 2]);
}

#[test]
fn real_time_order_is_not_negotiable() {
    //  c1  ├ put x=1 ┤
    //  c2               ├ put x=2 ┤
    //  c3                            ├ get x → 1 ┤
    let h = history(vec![
        done(1, put("x", "1"), 0, 10, Response::PutOk),
        done(2, put("x", "2"), 20, 30, Response::PutOk),
        done(3, get("x"), 40, 50, value("1")),
    ]);
    assert!(
        !check(&h).is_linearizable(),
        "the second put returned before the read"
    );

    //  c1  ├ put x=1 ┤
    //  c2       ├─────── put x=2 ───────┤
    //  c3                  ├ get x → 1 ┤
    let h = history(vec![
        done(1, put("x", "1"), 0, 10, Response::PutOk),
        done(2, put("x", "2"), 5, 50, Response::PutOk),
        done(3, get("x"), 20, 40, value("1")),
    ]);
    assert!(
        check(&h).is_linearizable(),
        "the second put may land after the read"
    );
}

#[test]
fn a_write_the_client_gave_up_on_may_or_may_not_have_happened() {
    //  c1  ├── put x=1 ──  (gave up)
    //  c2            ├ get x → 1 ┤
    let h = history(vec![
        open(1, put("x", "1"), 0),
        done(2, get("x"), 10, 20, value("1")),
    ]);
    assert_eq!((h.completed(), h.pending()), (1, 1));
    let Verdict::Linearizable { order, .. } = check(&h) else {
        panic!("the write happened")
    };
    assert_eq!(order, vec![0, 1], "the pending write was placed");

    //  c1  ├── put x=1 ──  (gave up)
    //  c2            ├ get x → None ┤
    let h = history(vec![open(1, put("x", "1"), 0), done(2, get("x"), 10, 20, NONE)]);
    let Verdict::Linearizable { order, .. } = check(&h) else {
        panic!("the write did not happen")
    };
    assert_eq!(order, vec![1], "the pending write was left out");

    //  c1  ├── put x=1 ──  (gave up)
    //  c2            ├ get x → 1 ┤   ├ get x → None ┤
    let h = history(vec![
        open(1, put("x", "1"), 0),
        done(2, get("x"), 10, 20, value("1")),
        done(2, get("x"), 30, 40, NONE),
    ]);
    assert!(!check(&h).is_linearizable(), "it cannot both happen and not");

    // A pending write may take effect late, after operations that
    // started after it. It never blocks them.
    //  c1  ├── put x=1 ──  (gave up)
    //  c2     ├ get x → None ┤   ├ get x → 1 ┤
    let h = history(vec![
        open(1, put("x", "1"), 0),
        done(2, get("x"), 10, 20, NONE),
        done(2, get("x"), 30, 40, value("1")),
    ]);
    let Verdict::Linearizable { order, .. } = check(&h) else {
        panic!("the write landed late")
    };
    assert_eq!(order, vec![1, 0, 2]);
}

#[test]
fn keys_are_independent_and_a_pending_read_is_dropped() {
    //  c1  ├ put x=1 ┤  ├ put y=1 ┤
    //  c2      ├─── get x → None ───┤
    //  c3      ├── get y ──  (gave up)
    let h = history(vec![
        done(1, put("x", "1"), 0, 10, Response::PutOk),
        done(1, put("y", "1"), 20, 30, Response::PutOk),
        done(2, get("x"), 5, 40, NONE),
        open(3, get("y"), 5),
    ]);
    let parts = kv::partition(&h);
    let keys: Vec<&[u8]> = parts.iter().map(|(k, _)| k.as_slice()).collect();
    assert_eq!(keys, vec![&b"x"[..], &b"y"[..]]);
    assert_eq!(parts[0].1.len(), 2);
    assert_eq!(parts[1].1.len(), 1, "the pending read of y is gone");
    let report = kv::check(&h, LIMIT);
    assert!(report.is_linearizable(), "{report:?}");
    assert_eq!(report.keys, 2);
    assert_eq!(report.stats.ops, 3);

    // Make x fail: the read of x returned 1 and then another read of x
    // returned None after the put returned.
    let h = history(vec![
        done(1, put("x", "1"), 0, 10, Response::PutOk),
        done(1, put("y", "1"), 20, 30, Response::PutOk),
        done(2, get("x"), 50, 60, NONE),
    ]);
    let report = kv::check(&h, LIMIT);
    let failed = report.failed.expect("x is broken");
    assert_eq!(failed.key, b"x");
    assert_eq!(failed.history.len(), 2, "the indices refer to x's history");
    assert!(matches!(failed.verdict, Verdict::Illegal(_)));
}

#[test]
fn the_budget_is_honoured() {
    let h = history(vec![
        done(1, put("x", "1"), 0, 20, Response::PutOk),
        done(2, put("x", "2"), 0, 20, Response::PutOk),
        done(3, get("x"), 0, 20, value("2")),
    ]);
    assert!(check(&h).is_linearizable());
    assert!(matches!(
        linearize::check::<KvModel>(&h, 2),
        Verdict::Unknown(stats) if stats.steps == 2
    ));
}

#[test]
fn the_cache_keeps_a_wide_history_cheap() {
    // Twelve clients each put their own value to x at once, and a reader
    // sees one of them. Without the cache, the search is 12! deep in the
    // worst case. With it, every (set, state) pair is visited once.
    let mut ops: Vec<Op<Request, Response>> = (1..=12)
        .map(|c| done(c, put("x", &c.to_string()), 0, 100, Response::PutOk))
        .collect();
    ops.push(done(13, get("x"), 200, 210, value("7")));
    let h = history(ops);
    let v = check(&h);
    assert!(v.is_linearizable());
    assert!(v.stats().steps < 100_000, "{:?}", v.stats());
    // The impossible variant has to exhaust the search: the reader saw a
    // value nobody wrote. Eight writers have 8! = 40 320 orders. The
    // state after any set of them depends only on the last one, so the
    // cache sees 8 * 2^7 = 1024 configurations and tries at most eight
    // candidates from each.
    let mut ops: Vec<Op<Request, Response>> = (1..=8)
        .map(|c| done(c, put("x", &c.to_string()), 0, 100, Response::PutOk))
        .collect();
    ops.push(done(9, get("x"), 200, 210, value("99")));
    let h = history(ops);
    let v = check(&h);
    assert!(matches!(v, Verdict::Illegal(_)), "{v:?}");
    let stats = v.stats();
    assert!(stats.cached <= 1024 + 8, "{stats:?}");
    assert!(stats.steps < 10_000, "{stats:?}");
    // Twelve writers would be 12 * 2^11 configurations, which is why the
    // budget exists and why the lesson says to keep the hot set small.
}

#[test]
fn a_history_is_well_formed_or_it_panics() {
    let mut h: History<Request, Response> = History::new();
    h.call(1, 1, get("x"), at(0));
    h.ret(1, 1, NONE, at(5));
    h.call(1, 2, get("x"), at(10));
    h.give_up(1, 2);
    h.call(1, 3, get("x"), at(20));
    assert_eq!((h.len(), h.completed(), h.pending()), (3, 1, 2));
    let r = std::panic::catch_unwind(move || h.call(1, 4, get("x"), at(30)));
    assert!(r.is_err(), "two operations open on one client");
}
