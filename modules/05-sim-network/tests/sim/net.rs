//! Acceptance tests for the network model. The model decides fates; these
//! tests check the decisions without a scheduler in the way.

use determinism::Prng;
use sim::{Chance, Delivery, Dropped, Duration, Instant, Net, NetConfig, Shape};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn at(n: u64) -> Instant {
    Instant::ZERO + ms(n)
}
fn reliable() -> Net<u8> {
    Net::new(NetConfig::reliable(ms(1)))
}
fn prng() -> Prng {
    Prng::from_seed(1)
}

#[test]
fn reliable_delivers_once_after_the_latency() {
    let mut net = reliable();
    let mut p = prng();
    let d = net.send(at(10), 0, 1, &mut p).unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!((d[0].at, d[0].copy), (at(11), 0));
    assert_eq!(net.in_flight(), 1);
    assert_eq!(net.deliver(d[0].id), Ok((0, 1)));
    assert_eq!(net.in_flight(), 0);
    let s = net.stats();
    assert_eq!((s.sent, s.delivered, s.lost, s.partitioned), (1, 1, 0, 0));
}

#[test]
fn loss_drops_at_send_time() {
    let mut net: Net<u8> = Net::new(NetConfig {
        loss: Chance::ALWAYS,
        ..NetConfig::default()
    });
    let mut p = prng();
    for _ in 0..10 {
        assert_eq!(net.send(at(0), 0, 1, &mut p), Err(Dropped::Loss));
    }
    assert_eq!(net.stats().lost, 10);
    assert_eq!(net.in_flight(), 0, "a lost message is not in flight");
}

#[test]
fn a_duplicate_is_two_copies_of_one_message() {
    let mut net: Net<u8> = Net::new(NetConfig {
        duplicate: Chance::ALWAYS,
        ..NetConfig::default()
    });
    let mut p = prng();
    let d = net.send(at(0), 0, 1, &mut p).unwrap();
    assert_eq!(d.len(), 2);
    assert_eq!(d[0].id, d[1].id);
    assert_eq!((d[0].copy, d[1].copy), (0, 1));
    assert_eq!(net.in_flight(), 1);
    assert_eq!(net.deliver(d[0].id), Ok((0, 1)));
    assert_eq!(net.in_flight(), 1, "one copy still on the wire");
    assert_eq!(net.deliver(d[1].id), Ok((0, 1)));
    assert_eq!(net.in_flight(), 0);
    assert_eq!(net.stats().duplicated, 1);
    assert_eq!(net.stats().delivered, 2);
}

#[test]
fn reordering_is_an_extra_delay_within_the_bound() {
    let mut net: Net<u8> = Net::new(NetConfig {
        reorder: Chance::ALWAYS,
        reorder_delay_max: ms(50),
        ..NetConfig::reliable(ms(1))
    });
    let mut p = prng();
    let mut delayed = 0;
    for _ in 0..100 {
        let d = net.send(at(0), 0, 1, &mut p).unwrap();
        assert!(d[0].at >= at(1) && d[0].at <= at(51), "{:?}", d[0].at);
        if d[0].at > at(1) {
            delayed += 1;
        }
    }
    assert!(delayed > 90);
    assert_eq!(net.stats().reordered, 100);
}

#[test]
fn latency_stays_in_its_range() {
    let mut net: Net<u8> = Net::new(NetConfig {
        latency_min: ms(2),
        latency_max: ms(9),
        ..NetConfig::default()
    });
    let mut p = prng();
    let mut distinct = std::collections::BTreeSet::new();
    for _ in 0..1000 {
        let d = net.send(at(0), 0, 1, &mut p).unwrap();
        assert!(d[0].at >= at(2) && d[0].at <= at(9));
        distinct.insert(d[0].at);
    }
    assert!(distinct.len() > 100, "jitter is not a constant");
}

#[test]
fn cut_is_one_way_and_restore_undoes_it() {
    let mut net = reliable();
    let mut p = prng();
    net.cut(0, 1);
    assert!(!net.is_connected(0, 1));
    assert!(net.is_connected(1, 0));
    assert_eq!(net.send(at(0), 0, 1, &mut p), Err(Dropped::Partition));
    assert!(net.send(at(0), 1, 0, &mut p).is_ok());
    net.restore(0, 1);
    assert!(net.send(at(0), 0, 1, &mut p).is_ok());
    assert_eq!(net.stats().partitioned, 1);
}

#[test]
fn partition_and_heal_are_two_way() {
    let mut net = reliable();
    net.partition(0, 1);
    assert!(!net.is_connected(0, 1) && !net.is_connected(1, 0));
    assert_eq!(net.cuts().count(), 2);
    net.heal(0, 1);
    assert_eq!(net.cuts().count(), 0);
}

#[test]
fn split_cuts_every_cross_link_and_nothing_else() {
    let mut net = reliable();
    net.split(&[0, 1], &[2, 3]);
    assert_eq!(net.cuts().count(), 8);
    assert!(net.is_connected(0, 1) && net.is_connected(2, 3));
    assert!(!net.is_connected(0, 2) && !net.is_connected(3, 1));
    net.heal_all();
    assert_eq!(net.cuts().count(), 0);
}

#[test]
fn a_link_cut_after_send_drops_the_message_at_delivery() {
    let mut net = reliable();
    let mut p = prng();
    let d = net.send(at(0), 0, 1, &mut p).unwrap();
    net.cut(0, 1);
    assert_eq!(net.deliver(d[0].id), Err(Dropped::Partition));
    assert_eq!(net.in_flight(), 0);
    let s = net.stats();
    assert_eq!((s.delivered, s.partitioned), (0, 1));
}

#[test]
fn drop_next_loses_exactly_what_was_asked() {
    let mut net = reliable();
    let mut p = prng();
    net.drop_next(0, 1);
    net.drop_next(0, 1);
    assert_eq!(net.send(at(0), 0, 1, &mut p), Err(Dropped::Loss));
    assert_eq!(net.send(at(0), 0, 1, &mut p), Err(Dropped::Loss));
    assert!(net.send(at(0), 0, 1, &mut p).is_ok());
    assert!(net.send(at(0), 1, 0, &mut p).is_ok(), "other direction untouched");
    assert_eq!(net.stats().lost, 2);
}

#[test]
#[should_panic(expected = "never sent")]
fn the_ledger_rejects_a_message_nobody_sent() {
    let mut a = reliable();
    let mut b = reliable();
    let mut p = prng();
    let d = a.send(at(0), 0, 1, &mut p).unwrap();
    let _ = b.deliver(d[0].id);
}

#[test]
#[should_panic(expected = "never sent")]
fn the_ledger_rejects_a_copy_delivered_twice() {
    let mut net = reliable();
    let mut p = prng();
    let d = net.send(at(0), 0, 1, &mut p).unwrap();
    let _ = net.deliver(d[0].id);
    let _ = net.deliver(d[0].id);
}

#[test]
#[should_panic(expected = "talk to itself")]
fn sending_to_yourself_is_a_bug() {
    let mut net = reliable();
    let _ = net.send(at(0), 3, 3, &mut prng());
}

#[test]
#[should_panic(expected = "latency_min")]
fn an_inverted_latency_range_is_rejected() {
    let _: Net<u8> = Net::new(NetConfig {
        latency_min: ms(5),
        latency_max: ms(1),
        ..NetConfig::default()
    });
}

#[test]
fn shapes_are_sets_of_cuts() {
    fn cuts(shape: &Shape<u8>) -> Vec<(u8, u8)> {
        let mut net = reliable();
        net.apply(shape);
        net.cuts().collect()
    }
    assert_eq!(
        cuts(&Shape::Isolate {
            node: 0,
            rest: vec![1, 2]
        }),
        vec![(0, 1), (0, 2), (1, 0), (2, 0)]
    );
    assert_eq!(
        cuts(&Shape::Halves {
            a: vec![0],
            b: vec![1, 2]
        }),
        vec![(0, 1), (0, 2), (1, 0), (2, 0)]
    );
    assert_eq!(
        cuts(&Shape::Bridge {
            bridge: 1,
            a: vec![0],
            b: vec![2]
        }),
        vec![(0, 2), (2, 0)],
        "the bridge keeps both sides"
    );
    assert_eq!(
        cuts(&Shape::OneWay {
            from: vec![0],
            to: vec![1, 2]
        }),
        vec![(0, 1), (0, 2)],
        "replies still flow"
    );
    assert_eq!(
        cuts(&Shape::Ring {
            order: vec![0, 1, 2, 3]
        }),
        vec![(0, 2), (1, 3), (2, 0), (3, 1)],
        "each node keeps its two neighbours"
    );
    assert_eq!(
        cuts(&Shape::Ring { order: vec![0, 1, 2] }),
        vec![],
        "a ring of three is complete"
    );
}

#[test]
fn random_shapes_fit_the_cluster() {
    let mut p = prng();
    for _ in 0..200 {
        match Shape::random(&mut p, &[0u8, 1]) {
            Shape::Bridge { .. } | Shape::Ring { .. } => panic!("needs more nodes"),
            Shape::Halves { a, b } | Shape::OneWay { from: a, to: b } => {
                assert!(!a.is_empty() && !b.is_empty());
            }
            Shape::Isolate { rest, .. } => assert_eq!(rest.len(), 1),
        }
        assert!(!matches!(Shape::random(&mut p, &[0u8, 1, 2]), Shape::Ring { .. }));
    }
    let mut seen = [false; 5];
    for _ in 0..500 {
        let shape = Shape::random(&mut p, &[0u8, 1, 2, 3, 4]);
        let kind = match &shape {
            Shape::Isolate { rest, .. } => {
                assert_eq!(rest.len(), 4);
                0
            }
            Shape::Halves { a, b } => {
                assert_eq!(a.len() + b.len(), 5);
                1
            }
            Shape::Bridge { a, b, .. } => {
                assert!(!a.is_empty() && !b.is_empty());
                assert_eq!(a.len() + b.len(), 4);
                2
            }
            Shape::OneWay { from, to } => {
                assert_eq!(from.len() + to.len(), 5);
                3
            }
            Shape::Ring { order } => {
                assert_eq!(order.len(), 5);
                4
            }
        };
        seen[kind] = true;
    }
    assert!(
        seen.iter().all(|&s| s),
        "every shape is drawn eventually: {seen:?}"
    );
}

#[test]
fn a_random_config_is_a_function_of_the_seed() {
    let a = NetConfig::random(&mut Prng::from_seed(5));
    let b = NetConfig::random(&mut Prng::from_seed(5));
    let c = NetConfig::random(&mut Prng::from_seed(6));
    assert_eq!(a, b);
    assert_ne!(a, c);
    for seed in 0..100 {
        let cfg = NetConfig::random(&mut Prng::from_seed(seed));
        cfg.validate();
        assert!(cfg.latency_max - cfg.latency_min <= ms(50));
        assert!(cfg.reorder_delay_max >= ms(10));
    }
}

#[test]
fn the_same_seed_decides_the_same_fates() {
    fn run(seed: u64) -> Vec<Result<Vec<Delivery>, Dropped>> {
        let mut p = Prng::from_seed(seed);
        let config = NetConfig::random(&mut p);
        let mut net: Net<u8> = Net::new(config);
        (0..200).map(|i| net.send(at(i), 0, 1, &mut p)).collect()
    }
    assert_eq!(run(11), run(11));
    assert_ne!(run(11), run(12));
}

#[test]
fn display_is_what_the_trace_shows() {
    let mut net = reliable();
    let d = net.send(at(0), 0, 1, &mut prng()).unwrap();
    assert_eq!(d[0].id.to_string(), "m0");
    assert_eq!(Chance::new(1, 4).to_string(), "1/4");
    assert_eq!(Dropped::Partition.to_string(), "partition");
    let shape: Shape<u8> = Shape::Halves {
        a: vec![0, 1],
        b: vec![2],
    };
    assert_eq!(shape.to_string(), "halves [0 1] | [2]");
    let shape: Shape<u8> = Shape::OneWay {
        from: vec![0],
        to: vec![1],
    };
    assert_eq!(shape.to_string(), "one-way [0] -x-> [1]");
}
