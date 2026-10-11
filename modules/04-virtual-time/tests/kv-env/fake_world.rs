//! The KV store end to end under the simulated environment.
//!
//! These are the Module 2 tests, ported from a hand-advanced clock to the
//! Module 4 scheduler. Where a test used to call `settle()` it now runs the
//! event queue, and where it used to `advance()` the clock it now runs for a
//! span of virtual time. The last two tests are the ones that matter most
//! for the course: a seeded workload runs twice and the trace hashes must
//! match.

use determinism::{check_determinism, Prng};
use kv_core::client::Config;
use kv_core::log::Entry;
use kv_core::{Duration, Instant, Request, Response};
use kv_env::fake::{FakeStorage, World, WorldConfig};
use std::collections::BTreeMap;

fn cfg() -> Config {
    Config {
        initial_timeout: Duration::from_millis(100),
        max_timeout: Duration::from_secs(1),
        max_attempts: 5,
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn at(millis: u64) -> Instant {
    Instant::ZERO + ms(millis)
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

#[test]
fn basic_operations() {
    let mut w = World::new(FakeStorage::new());
    w.add_client(1, cfg());
    assert_eq!(w.call(1, get("a")), Response::Value(None));
    assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
    assert_eq!(w.call(1, get("a")), Response::Value(Some(b"1".to_vec())));
    assert_eq!(w.call(1, del("a")), Response::DeleteOk { existed: true });
    assert_eq!(w.call(1, del("a")), Response::DeleteOk { existed: false });
    assert_eq!(w.call(1, get("a")), Response::Value(None));
    assert!(w.gave_up.is_empty());
    // A read is two network hops; a write adds the disk.
    assert_eq!(w.now(), at(2 * 6 + 5 * 3));
}

#[test]
fn write_is_not_visible_until_durable() {
    let mut w = World::new(FakeStorage::new());
    w.add_client(1, cfg());
    w.add_client(2, cfg());
    let rid = w.submit(1, put("a", "1"));
    // The request arrives after one network latency; the disk takes longer.
    w.run_for(ms(1));
    assert_eq!(w.driver.storage.pending(), 1, "append waits for the disk");
    assert!(w.take_completed(1, rid).is_none(), "no reply before durability");
    assert_eq!(
        w.call(2, get("a")),
        Response::Value(None),
        "readers see only applied state"
    );
    assert_eq!(w.driver.storage.pending(), 1, "still not durable at {}", w.now());
    w.run_until_idle();
    assert_eq!(w.take_completed(1, rid), Some(Response::PutOk));
    assert_eq!(w.call(2, get("a")), Response::Value(Some(b"1".to_vec())));
}

#[test]
fn lost_reply_is_retried_and_not_applied_twice() {
    let mut w = World::new(FakeStorage::new());
    w.add_client(1, cfg());
    w.drop_replies.push((1, 1)); // the first reply to request 1 vanishes
    let rid = w.submit(1, del("k"));
    // Nothing happens until the timeout, however long the node took.
    w.run_for(ms(99));
    assert!(w.take_completed(1, rid).is_none());
    assert_eq!(w.stats.replies_dropped, 1);
    w.run_until_idle();
    assert_eq!(
        w.take_completed(1, rid),
        Some(Response::DeleteOk { existed: false })
    );
    assert_eq!(w.stats.timers_fired, 1);
    let stats = w.driver.node.stats();
    assert_eq!(stats.applied, 1, "retry must not re-apply");
    assert_eq!(stats.duplicates, 1);
    assert_eq!(w.clients[&1].retries(), 1);
    // Timer at 100ms, retransmit arrives at 101ms, reply back at 102ms.
    assert_eq!(w.now(), at(102));
}

#[test]
fn client_gives_up_when_every_reply_is_lost() {
    let mut w = World::new(FakeStorage::new());
    w.add_client(1, cfg());
    for _ in 0..5 {
        w.drop_replies.push((1, 1));
    }
    let rid = w.submit(1, put("a", "1"));
    w.run_until_idle();
    assert!(w.take_completed(1, rid).is_none());
    assert_eq!(w.gave_up, vec![(1, 1)]);
    assert_eq!(w.stats.timers_fired, 5);
    // 100 + 200 + 400 + 800 + 1000 (capped) ms of waiting.
    assert_eq!(w.now(), at(2500));
    // The write *did* happen; the client just does not know. This is the
    // "fate unknown" case every distributed client has to live with.
    assert_eq!(w.driver.node.get(b"a"), Some(&b"1"[..]));
}

#[test]
fn restart_recovers_from_the_log() {
    let mut w = World::new(FakeStorage::new());
    w.add_client(1, cfg());
    w.call(1, put("a", "1"));
    w.call(1, put("b", "2"));
    w.call(1, del("a"));
    w.call(1, put("c", "3"));
    let log = w.driver.storage.log.clone();

    let mut w2 = World::new(FakeStorage::with_log(log));
    w2.add_client(1, cfg());
    assert!(w2.driver.node.is_running());
    assert_eq!(w2.driver.node.stats().recovered_entries, 4);
    assert_eq!(w2.call(1, get("a")), Response::Value(None));
    assert_eq!(w2.call(1, get("b")), Response::Value(Some(b"2".to_vec())));
    assert_eq!(w2.call(1, get("c")), Response::Value(Some(b"3".to_vec())));
}

#[test]
fn torn_tail_is_discarded_and_overwritten() {
    let mut w = World::new(FakeStorage::new());
    w.add_client(1, cfg());
    w.call(1, put("a", "1"));
    let mut log = w.driver.storage.log.clone();
    let good_len = log.len();
    // A crash mid-append: half an entry on disk, never acknowledged.
    let mut partial = Vec::new();
    Entry::Put {
        key: b"zz".into(),
        value: b"never acked".into(),
    }
    .encode(&mut partial);
    log.extend_from_slice(&partial[..partial.len() / 2]);

    let mut w2 = World::new(FakeStorage::with_log(log));
    w2.add_client(1, cfg());
    assert_eq!(w2.driver.node.stats().recovered_entries, 1);
    assert_eq!(w2.call(1, get("zz")), Response::Value(None));
    // The next append lands at the end of the *valid* prefix.
    assert_eq!(w2.call(1, put("b", "2")), Response::PutOk);
    let mut expected = w2.driver.storage.log[..good_len].to_vec();
    Entry::Put {
        key: b"b".into(),
        value: b"2".into(),
    }
    .encode(&mut expected);
    assert_eq!(w2.driver.storage.log, expected);
}

/// A seeded random workload checked against a reference model, run under
/// the determinism check. The closure must be a pure function of the seed:
/// if a later module's change makes this fail, that change leaked some
/// nondeterminism into the node, the client, the scheduler or the fake
/// environment.
fn workload(prng: &mut Prng, trace: &mut determinism::Trace) {
    let config = WorldConfig {
        seed: prng.next_u64(),
        net_jitter: ms(5),
        disk_jitter: ms(20),
        ..WorldConfig::default()
    };
    let mut w = World::with_config(FakeStorage::new(), config);
    let clients = 1 + prng.below(4);
    for id in 1..=clients {
        w.add_client(id, cfg());
    }
    let keys: Vec<Vec<u8>> = (0..8).map(|i| format!("key{i}").into_bytes()).collect();
    let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    let mut expected: Vec<(u64, u64, Response)> = Vec::new();

    for _ in 0..200 {
        let id = 1 + prng.below(clients);
        let key = prng.pick(&keys).clone();
        let (request, response) = match prng.below(3) {
            0 => (
                Request::Get { key: key.clone() },
                Response::Value(model.get(&key).cloned()),
            ),
            1 => {
                let value = format!("v{}", prng.below(1000)).into_bytes();
                model.insert(key.clone(), value.clone());
                (Request::Put { key, value }, Response::PutOk)
            }
            _ => {
                let existed = model.remove(&key).is_some();
                (Request::Delete { key }, Response::DeleteOk { existed })
            }
        };
        // The model is only valid if operations are applied in the order the
        // model saw them. Each step below runs one operation to completion
        // before the next starts, so that holds. Module 8 relaxes this and
        // needs a real linearizability checker as a result.
        // Sometimes lose the reply, so the client must retry and the node
        // must deduplicate. The model does not change: same answer either way.
        if prng.chance(1, 10) {
            w.drop_replies.push((id, next_request_id(&w, id)));
        }
        let rid = w.submit(id, request);
        expected.push((id, rid, response));
        assert!(w.run_while(|w| !w.clients[&id].is_idle()));
        // Sometimes let time pass with nothing in flight.
        if prng.chance(1, 4) {
            w.run_for(ms(prng.range(1, 150)));
        }
    }
    w.run_until_idle();
    for (id, rid, response) in expected {
        assert_eq!(
            w.take_completed(id, rid),
            Some(response),
            "client {id} request {rid}"
        );
    }
    assert!(w.gave_up.is_empty());
    for line in w.trace().lines() {
        trace.record(format_args!("{line}"));
    }
    determinism::trace!(trace, "stats {:?} {:?}", w.driver.node.stats(), w.stats);
}

fn next_request_id(w: &World, id: u64) -> u64 {
    // Clients number requests from 1; count how many this one has finished.
    1 + w.completed.iter().filter(|(c, _, _)| *c == id).count() as u64
        + w.gave_up.iter().filter(|(c, _)| *c == id).count() as u64
}

#[test]
fn seeded_workload_matches_model_and_is_deterministic() {
    for seed in 0..20 {
        check_determinism(seed, 3, workload).unwrap_or_else(|d| panic!("{d}"));
    }
}

#[test]
fn known_seed_fingerprint() {
    // Pins seed 1's trace hash. If this changes, the behaviour of the node,
    // client, scheduler or fake environment changed; update it knowingly
    // and say why. Module 4 changed it on purpose: every trace line now
    // carries a virtual timestamp from the scheduler.
    let hash = check_determinism(1, 2, workload).unwrap();
    assert_eq!(
        hash, 0xd982_fd72_6e9e_453a,
        "seed 1 trace hash changed: {hash:#018x}"
    );
}
