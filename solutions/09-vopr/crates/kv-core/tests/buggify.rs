//! Module 7 acceptance tests: fault points fire only when enabled, at the
//! configured rate, and the node and heartbeat survive each one.

use determinism::{Chance, Prng};
use kv_core::buggify::{Buggify, Site};
use kv_core::heartbeat::{self, Heartbeat};
use kv_core::node::{Input, Node, Output};
use kv_core::{Duration, Instant, Request, RequestEnvelope, Response, ResponseEnvelope};

fn req(client: u64, id: u64, request: Request) -> Input {
    Input::Request(RequestEnvelope {
        client,
        request_id: id,
        request,
    })
}
fn put(k: &str, v: &str) -> Request {
    Request::Put {
        key: k.into(),
        value: v.into(),
    }
}
fn reply(client: u64, id: u64, response: Response) -> Output {
    Output::Reply(ResponseEnvelope {
        client,
        request_id: id,
        response,
    })
}

/// A running node with `buggify` installed.
fn started(buggify: Buggify) -> (Node, Vec<Output>) {
    let mut out = Vec::new();
    let mut node = Node::with_buggify(buggify, &mut out);
    out.clear();
    node.on_input(Input::ReadComplete { op: 0, data: vec![] }, &mut out);
    assert!(node.is_running());
    (node, out)
}

#[test]
fn every_site_has_a_name_and_a_place_in_the_table() {
    assert_eq!(Site::ALL.len(), Site::COUNT);
    let mut names: Vec<&str> = Site::ALL.iter().map(|s| s.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Site::COUNT, "names are unique");
    for (i, s) in Site::ALL.iter().enumerate() {
        assert_eq!(s.index(), i);
        assert_eq!(s.to_string(), s.name());
    }
}

#[test]
fn off_never_fires_and_spends_no_randomness() {
    let mut b = Buggify::off();
    for _ in 0..1000 {
        for site in Site::ALL {
            assert!(!b.fire(site));
        }
    }
    assert_eq!(b.total_fired(), 0);
    assert!(b.enabled_sites().is_empty());
}

#[test]
fn a_site_that_is_not_enabled_never_fires() {
    let mut b = Buggify::new(&[Site::DropReply], Chance::ALWAYS, Prng::from_seed(1));
    assert!(b.is_enabled(Site::DropReply));
    assert!(!b.is_enabled(Site::DropRequest));
    for _ in 0..1000 {
        assert!(!b.fire(Site::DropRequest));
        assert!(!b.fire(Site::SkipBeat));
        assert!(b.fire(Site::DropReply));
    }
    assert_eq!(b.fired(Site::DropReply), 1000);
    assert_eq!(b.fired(Site::DropRequest), 0);
    assert_eq!(b.total_fired(), 1000);
    assert_eq!(b.take_log().len(), 1000);
    assert!(b.take_log().is_empty(), "the log drains");
}

#[test]
fn an_enabled_site_fires_at_its_rate() {
    let mut b = Buggify::new(&Site::ALL, Chance::percent(25), Prng::from_seed(2));
    let fires = (0..40_000).filter(|_| b.fire(Site::DropRequest)).count();
    assert!((9_500..10_500).contains(&fires), "{fires}");
    b.set_fire(Chance::NEVER);
    assert!(!b.fire(Site::DropRequest));
    b.disable_all();
    assert!(b.enabled_sites().is_empty());
}

#[test]
fn the_enabled_set_is_a_function_of_the_seed() {
    let draw = |seed| Buggify::draw_sites(Chance::percent(25), &mut Prng::from_seed(seed));
    assert_eq!(draw(5), draw(5));
    let mut per_site = [0u32; Site::COUNT];
    let mut distinct = std::collections::BTreeSet::new();
    for seed in 0..400 {
        let sites = draw(seed);
        distinct.insert(sites.clone());
        for s in sites {
            per_site[s.index()] += 1;
        }
    }
    for (s, n) in Site::ALL.iter().zip(per_site) {
        assert!((60..140).contains(&n), "{s} enabled in {n} of 400 runs");
    }
    assert!(distinct.len() >= 10, "{} distinct sets", distinct.len());
}

#[test]
fn a_dropped_reply_is_answered_from_the_session_on_the_retry() {
    let (mut node, mut out) = started(Buggify::new(
        &[Site::DropReply],
        Chance::ALWAYS,
        Prng::from_seed(0),
    ));
    node.on_input(req(1, 1, put("k", "v")), &mut out);
    assert!(matches!(out[0], Output::Append { .. }));
    out.clear();
    node.on_input(Input::AppendComplete { op: 1 }, &mut out);
    assert!(out.is_empty(), "the reply fell on the floor");
    assert_eq!(node.get(b"k"), Some(&b"v"[..]), "but the write is applied");
    assert_eq!(node.buggify().fired(Site::DropReply), 1);
    assert_eq!(node.buggify_mut().take_log(), vec![Site::DropReply]);

    // The client retries; the buffer has room this time.
    node.buggify_mut().set_fire(Chance::NEVER);
    node.on_input(req(1, 1, put("k", "v")), &mut out);
    assert_eq!(out, vec![reply(1, 1, Response::PutOk)]);
    assert_eq!(node.stats().applied, 1, "once");
    assert_eq!(node.stats().duplicates, 1);
}

#[test]
fn a_dropped_request_leaves_no_trace_in_the_node() {
    let (mut node, mut out) = started(Buggify::new(
        &[Site::DropRequest],
        Chance::ALWAYS,
        Prng::from_seed(0),
    ));
    node.on_input(req(1, 1, put("k", "v")), &mut out);
    assert!(out.is_empty());
    assert_eq!(node.stats().requests, 0, "never counted as received");
    assert_eq!(node.pending_writes(), 0, "no session, no append");
    node.buggify_mut().set_fire(Chance::NEVER);
    node.on_input(req(1, 1, put("k", "v")), &mut out);
    assert!(
        matches!(out[0], Output::Append { .. }),
        "the retry is the first the node hears of it"
    );
    assert_eq!(node.stats().duplicates, 0);
}

#[test]
fn a_duplicated_reply_is_two_identical_replies() {
    let (mut node, mut out) = started(Buggify::new(
        &[Site::DuplicateReply],
        Chance::ALWAYS,
        Prng::from_seed(0),
    ));
    node.on_input(req(1, 1, Request::Get { key: b"k".into() }), &mut out);
    let expected = reply(1, 1, Response::Value(None));
    assert_eq!(out, vec![expected.clone(), expected]);
    assert_eq!(node.stats().requests, 1);
}

#[test]
fn a_skipped_beat_keeps_the_sequence_numbers_dense() {
    let interval = Duration::from_millis(100);
    let config = heartbeat::Config {
        interval,
        suspect_after: Duration::from_millis(350),
    };
    let mut out = Vec::new();
    let b = Buggify::new(&[Site::SkipBeat], Chance::percent(50), Prng::from_seed(4));
    let mut hb = Heartbeat::with_buggify(b, 0, &[1], config, Instant::ZERO, &mut out);
    // One tick per round, on the beat. (`next_deadline` also stops at the
    // moment the silent peer becomes suspect, which is not a round.)
    for round in 1..=100u32 {
        hb.on_input(Instant::ZERO + interval * round, heartbeat::Input::Tick, &mut out);
    }
    let seqs: Vec<u64> = out
        .iter()
        .filter_map(|o| match o {
            heartbeat::Output::Send {
                message: heartbeat::Message::Beat { seq },
                ..
            } => Some(*seq),
            _ => None,
        })
        .collect();
    let skipped = hb.buggify().fired(Site::SkipBeat);
    assert!(skipped > 20 && skipped < 80, "{skipped}");
    assert_eq!(
        seqs.len() as u64,
        101 - skipped,
        "one beat per round that was not skipped"
    );
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>(), "no gaps");
    assert_eq!(hb.stats().sent, seqs.len() as u64);
    assert_eq!(
        hb.next_deadline(),
        Some(Instant::ZERO + interval * 101),
        "the rounds kept their rhythm"
    );
}

#[test]
fn the_production_constructors_have_no_fault_points() {
    let mut out = Vec::new();
    let node = Node::new(&mut out);
    assert_eq!(node.buggify().total_fired(), 0);
    let mut out = Vec::new();
    let hb = Heartbeat::new(0, &[1], heartbeat::Config::default(), Instant::ZERO, &mut out);
    assert_eq!(hb.buggify().total_fired(), 0);
}
