//! Acceptance tests for the heartbeat state machine. Everything here goes
//! through the public API, with time fed by hand.

use kv_core::heartbeat::{Config, Heartbeat, Input, Liveness, Message, Output};
use kv_core::{Duration, Instant, NodeId};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn at(n: u64) -> Instant {
    Instant::ZERO + ms(n)
}
fn beat(from: NodeId, seq: u64) -> Input {
    Input::Message {
        from,
        message: Message::Beat { seq },
    }
}

#[test]
fn first_beats_go_out_at_construction() {
    let mut out = Vec::new();
    let hb = Heartbeat::new(0, &[1, 2], Config::default(), Instant::ZERO, &mut out);
    assert_eq!(
        out,
        vec![
            Output::Send {
                to: 1,
                message: Message::Beat { seq: 1 }
            },
            Output::Send {
                to: 2,
                message: Message::Beat { seq: 1 }
            },
        ]
    );
    assert_eq!(hb.next_deadline(), Some(at(100)));
    assert_eq!(hb.liveness(1), Liveness::Unknown);
    assert_eq!(hb.up_peers(), 0);
}

#[test]
fn beats_repeat_every_interval() {
    let mut out = Vec::new();
    let mut hb = Heartbeat::new(0, &[1], Config::default(), Instant::ZERO, &mut out);
    out.clear();
    for i in 1..=5 {
        hb.on_input(at(100 * i), Input::Tick, &mut out);
    }
    let seqs: Vec<u64> = out
        .iter()
        .filter_map(|o| match o {
            Output::Send {
                message: Message::Beat { seq },
                ..
            } => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(seqs, vec![2, 3, 4, 5, 6]);
    assert_eq!(hb.stats().sent, 6);
    // The detector keeps beating at a peer it suspects. Silence since the
    // start is noticed at the first tick at or after 350 ms.
    assert!(out.contains(&Output::PeerDown(1)));
    assert_eq!(hb.liveness(1), Liveness::Down);
}

#[test]
fn a_silent_peer_goes_down_and_a_beat_brings_it_back() {
    let mut out = Vec::new();
    let mut hb = Heartbeat::new(0, &[1], Config::default(), Instant::ZERO, &mut out);
    out.clear();
    hb.on_input(at(100), Input::Tick, &mut out);
    hb.on_input(at(200), Input::Tick, &mut out);
    hb.on_input(at(300), Input::Tick, &mut out);
    assert!(out.iter().all(|o| matches!(o, Output::Send { .. })));
    assert_eq!(
        hb.next_deadline(),
        Some(at(350)),
        "suspicion comes before the next beat"
    );
    out.clear();
    hb.on_input(at(350), Input::Tick, &mut out);
    assert_eq!(out, vec![Output::PeerDown(1)]);
    assert_eq!(hb.liveness(1), Liveness::Down);
    assert_eq!(
        hb.next_deadline(),
        Some(at(400)),
        "a down peer has no suspicion deadline"
    );
    out.clear();
    hb.on_input(at(360), beat(1, 7), &mut out);
    assert_eq!(out, vec![Output::PeerUp(1)]);
    assert_eq!(hb.next_deadline(), Some(at(400)));
    assert_eq!(hb.stats().downs, 1);
    assert_eq!(hb.stats().ups, 1);
    assert_eq!(hb.up_peers(), 1);
}

#[test]
fn a_heard_peer_is_suspected_from_its_last_beat() {
    let mut out = Vec::new();
    let mut hb = Heartbeat::new(0, &[1], Config::default(), Instant::ZERO, &mut out);
    hb.on_input(at(40), beat(1, 1), &mut out);
    assert_eq!(
        hb.next_deadline(),
        Some(at(100)),
        "next beat is sooner than 390 ms"
    );
    hb.on_input(at(100), Input::Tick, &mut out);
    hb.on_input(at(200), Input::Tick, &mut out);
    hb.on_input(at(300), Input::Tick, &mut out);
    assert_eq!(hb.next_deadline(), Some(at(390)));
    out.clear();
    hb.on_input(at(390), Input::Tick, &mut out);
    assert_eq!(out, vec![Output::PeerDown(1)]);
}

#[test]
fn duplicates_and_stragglers_are_counted_not_trusted() {
    let mut out = Vec::new();
    let mut hb = Heartbeat::new(0, &[1], Config::default(), Instant::ZERO, &mut out);
    hb.on_input(at(10), beat(1, 3), &mut out);
    hb.on_input(at(11), beat(1, 3), &mut out);
    hb.on_input(at(12), beat(1, 2), &mut out);
    hb.on_input(at(13), beat(1, 4), &mut out);
    let s = hb.stats();
    assert_eq!((s.received, s.duplicates, s.reordered), (4, 1, 1));
    assert_eq!(hb.liveness(1), Liveness::Up);
    assert_eq!(hb.stats().ups, 1, "no flapping on duplicates");
}

#[test]
fn a_late_tick_sends_one_beat() {
    let mut out = Vec::new();
    let mut hb = Heartbeat::new(0, &[1], Config::default(), Instant::ZERO, &mut out);
    out.clear();
    hb.on_input(at(1000), Input::Tick, &mut out);
    let sends = out.iter().filter(|o| matches!(o, Output::Send { .. })).count();
    assert_eq!(sends, 1);
    assert_eq!(hb.next_deadline(), Some(at(1100)));
}

#[test]
fn no_peers_no_deadline() {
    let mut out = Vec::new();
    let hb = Heartbeat::new(0, &[], Config::default(), Instant::ZERO, &mut out);
    assert!(out.is_empty());
    assert_eq!(hb.next_deadline(), None);
}

#[test]
#[should_panic(expected = "unknown node")]
fn a_beat_from_a_stranger_is_a_bug() {
    let mut out = Vec::new();
    let mut hb = Heartbeat::new(0, &[1], Config::default(), Instant::ZERO, &mut out);
    hb.on_input(at(1), beat(9, 1), &mut out);
}

#[test]
#[should_panic(expected = "listed itself")]
fn a_node_is_not_its_own_peer() {
    let mut out = Vec::new();
    Heartbeat::new(0, &[0, 1], Config::default(), Instant::ZERO, &mut out);
}

#[test]
#[should_panic(expected = "time go backwards")]
fn inputs_must_not_go_back_in_time() {
    let mut out = Vec::new();
    let mut hb = Heartbeat::new(0, &[1], Config::default(), at(10), &mut out);
    hb.on_input(at(5), Input::Tick, &mut out);
}
