//! Module 13 acceptance tests: the replica under turmoil.
//!
//! Six things must hold:
//!
//! 1. Every peer message survives the wire format the port gave it.
//! 2. A cluster with no faults answers every request.
//! 3. A run is a pure function of its seed, in one process and across
//!    two.
//! 4. Partitions, holds and crashes are dealt and survived: the history
//!    is linearizable, the committed prefixes agree, and the quiet phase
//!    completes.
//! 5. The planted stale read of Module 9 is caught here too.
//! 6. A write at a lower offset keeps the bytes past it. Seed 34 found
//!    out what happens otherwise.

use kv_core::replica::{Message, Status};
use kv_core::{Bug, Duration, Request, RequestEnvelope, Response, ResponseEnvelope};
use kv_turmoil::wire::Datagram;
use kv_turmoil::{run, FaultConfig, RunConfig, Schedule};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::process::Command;

fn config(seed: u64, requests: u64) -> RunConfig {
    RunConfig::from_seed(seed, requests)
}

#[test]
fn every_peer_message_survives_the_wire() {
    let runs = vec![(0, 3), (2, 7), (5, 7)];
    let envelope = RequestEnvelope {
        client: 7,
        request_id: 42,
        request: Request::Put {
            key: b"k1".to_vec(),
            value: b"c7-42".to_vec(),
        },
    };
    let messages = vec![
        Message::Beat {
            boot: 3,
            seq: 99,
            view: 5,
            status: Status::Primary,
            log_view: 5,
            runs: runs.clone(),
            top: 7,
            durable: 7,
            commit: 6,
            whole: true,
        },
        Message::Beat {
            boot: 0,
            seq: 1,
            view: 0,
            status: Status::Reading,
            log_view: 0,
            runs: Vec::new(),
            top: 0,
            durable: 0,
            commit: 0,
            whole: false,
        },
        Message::StartViewChange { view: 6 },
        Message::DoViewChange {
            view: 6,
            log_view: 5,
            runs: runs.clone(),
            commit: 6,
            whole: false,
        },
        Message::StartView {
            view: 6,
            runs: runs.clone(),
            commit: 7,
        },
        Message::Prepare {
            view: 6,
            seq: 8,
            commit: 7,
            bytes: vec![0xAB; 512],
        },
        Message::PrepareOk {
            boot: 1,
            view: 6,
            durable: 8,
        },
        Message::Pull { view: 6, from: 3 },
        Message::Forward {
            view: 6,
            envelope: envelope.clone(),
        },
        Message::Fetch { view: 6, seq: 2 },
        Message::Missing { view: 6, seq: 2 },
    ];
    for message in messages {
        let datagram = Datagram::Peer {
            from: 2,
            message: message.clone(),
        };
        let bytes = datagram.to_bytes();
        assert_eq!(Datagram::decode(&bytes).unwrap(), datagram, "{message}");
        // Every status has to fit in a beat.
        let mut cut = bytes.clone();
        cut.push(0);
        assert!(Datagram::decode(&cut).is_err(), "trailing bytes are rejected");
    }
    let request = Datagram::Request(envelope);
    assert_eq!(Datagram::decode(&request.to_bytes()).unwrap(), request);
    let reply = Datagram::Reply(ResponseEnvelope {
        client: 7,
        request_id: 42,
        response: Response::Value(Some(b"v".to_vec())),
    });
    assert_eq!(Datagram::decode(&reply.to_bytes()).unwrap(), reply);
    assert!(Datagram::decode(&[]).is_err());
    assert!(Datagram::decode(&[9]).is_err(), "an unknown tag is rejected");
    // A beat that claims a billion runs in twenty bytes is a lie, and is
    // refused before anything is allocated for it.
    let mut lie = vec![3, 0, 20];
    lie.extend_from_slice(&[0; 25]);
    lie.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(Datagram::decode(&lie).is_err());
}

#[test]
fn a_reliable_cluster_answers_every_request() {
    let mut c = config(5, 20);
    c.faults = FaultConfig::NONE;
    c.nodes = 3;
    let report = run(&c);
    assert_eq!(report.clients.gave_up, 0, "{report}");
    assert_eq!(report.clients.completed, report.clients.submitted, "{report}");
    assert_eq!(report.clients.submitted, 6 * 30, "{report}");
    assert_eq!(report.history, 180, "{report}");
    assert_eq!(
        (
            report.faults.partitions,
            report.faults.holds,
            report.faults.crashes
        ),
        (0, 0, 0)
    );
    assert!(report.schedule.events.is_empty());
    let forwarded: u64 = report.replicas.iter().map(|r| r.stats.forwarded).sum();
    assert!(forwarded > 0, "two of three replicas forward: {report}");
    assert!(report.replicas.iter().all(|r| r.net.rejected == 0), "{report}");
}

#[test]
fn a_run_is_a_pure_function_of_the_seed() {
    let c = config(2, 20);
    let a = run(&c);
    let b = run(&c);
    assert_eq!(a.hash, b.hash);
    assert_eq!(a.clients, b.clients);
    assert_eq!(a.faults, b.faults);
    assert_eq!(a.replicas, b.replicas);
    assert_eq!(a.steps, b.steps);
    assert_eq!(a.check, b.check);
    let other = run(&config(3, 20));
    assert_ne!(a.hash, other.hash, "two seeds are two runs");
}

#[test]
fn the_binary_agrees_with_the_library() {
    let bin = Path::new(env!("CARGO_BIN_EXE_kv-turmoil"));
    let out = Command::new(bin)
        .args(["--seed", "2", "--requests", "20"])
        .output()
        .expect("run kv-turmoil");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .find(|l| l.starts_with("trace_hash="))
        .expect("a hash line");
    let hash = u64::from_str_radix(&line["trace_hash=".len()..], 16).unwrap();
    assert_eq!(hash, run(&config(2, 20)).hash, "{stdout}");
}

#[test]
fn partitions_holds_and_crashes_are_dealt_and_survived() {
    // Seed 2 draws partitions, seed 5 crashes, seed 34 twelve crashes, and
    // seed 1 all three families on five nodes. Every run checks its own
    // history, logs and quiet phase, and panics otherwise.
    let mut partitions = 0;
    let mut holds = 0;
    let mut crashes = 0;
    let mut view_changes = 0;
    let mut gave_up = 0;
    for seed in [1, 2, 5, 34] {
        let report = run(&config(seed, 30));
        partitions += report.faults.partitions;
        holds += report.faults.holds;
        crashes += report.faults.crashes;
        view_changes += report.replicas.iter().map(|r| r.stats.view_changes).sum::<u64>();
        gave_up += report.clients.gave_up;
        assert_eq!(report.clients.quiet_completed, report.clients.quiet_submitted);
        assert!(report.replicas.iter().all(|r| r.net.rejected == 0), "{report}");
        assert_eq!(
            report.schedule.events.len() as u64,
            2 * (report.faults.partitions + report.faults.holds + report.faults.crashes),
            "every fault has an end"
        );
    }
    assert!(
        partitions > 0 && holds > 0 && crashes > 0,
        "{partitions} {holds} {crashes}"
    );
    assert!(view_changes > 0, "the faults reached the protocol");
    // Requests that gave up under the faults are allowed. They stay in
    // the history as pending, and the checker lets them land or not.
    let _ = gave_up;
}

#[test]
fn a_stale_primary_is_caught() {
    // A primary that steps down and keeps answering reads. The partition
    // between replicas leaves the clients talking to the old primary,
    // and the first seeds all find it.
    let mut caught = None;
    for seed in [0, 2, 3] {
        let mut c = config(seed, 30);
        c.bugs = vec![Bug::StaleReadAfterStepDown];
        let result = catch_unwind(AssertUnwindSafe(|| run(&c)));
        if let Err(payload) = result {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "?".to_string());
            assert!(message.starts_with("safety:"), "{message}");
            assert!(message.contains("not linearizable"), "{message}");
            caught = Some(seed);
            break;
        }
    }
    assert!(caught.is_some(), "no seed saw the stale read");
}

#[test]
fn the_schedule_is_drawn_from_the_seed() {
    use determinism::Prng;
    let chaos = Duration::from_secs(20);
    let a = Schedule::draw(&mut Prng::from_seed(9), 3, chaos, &FaultConfig::default());
    let b = Schedule::draw(&mut Prng::from_seed(9), 3, chaos, &FaultConfig::default());
    assert_eq!(a, b);
    assert!(a.events.windows(2).all(|w| w[0].0 <= w[1].0), "in time order");
    assert!(
        a.events.iter().all(|(at, _)| *at <= chaos),
        "inside the chaos phase"
    );
    assert!(!a.events.is_empty(), "seed 9 draws at least one fault");
    let none = Schedule::draw(&mut Prng::from_seed(9), 3, chaos, &FaultConfig::NONE);
    assert!(none.events.is_empty());
    let one = Schedule::draw(&mut Prng::from_seed(9), 1, chaos, &FaultConfig::default());
    assert!(one.events.is_empty(), "a cluster of one has no pair to part");
}

#[test]
fn a_write_at_a_lower_offset_keeps_the_bytes_past_it() {
    use kv_core::env::Storage;
    use kv_core::replica::Input;
    use kv_turmoil::{new_disks, SharedStorage};
    let disks = new_disks(1);
    let mut storage = SharedStorage::new(disks.clone(), 0);
    storage.append(1, 2048, &[7; 512]);
    storage.append(2, 0, &[9; 512]);
    storage.read_all(3);
    let mut inputs = Vec::new();
    storage.poll_completions(&mut inputs);
    assert_eq!(inputs.len(), 3);
    let Input::ReadComplete { data, .. } = &inputs[2] else {
        panic!("the read completes last");
    };
    assert_eq!(data.len(), 2560);
    assert_eq!(&data[..512], &[9; 512]);
    assert_eq!(&data[2048..], &[7; 512]);
    assert_eq!(disks.borrow()[0].bytes.len(), 2560);
}
