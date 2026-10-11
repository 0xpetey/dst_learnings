//! Module 4 acceptance tests: timers, timeouts and retries under virtual
//! time, and the speed that comes from never sleeping. Ported to the
//! Module 5 world, where `partitioned` is a cut link, and to the Module 6
//! world, where the log read at start is a disk event of its own.

use determinism::check_determinism;
use kv_core::client::Config;
use kv_core::{Duration, Instant, Request};
use kv_env::fake::{Addr, World, WorldConfig};
use sim::{Disk, DiskConfig, NetConfig};

const CLIENT: Addr = Addr::Client(1);
const NODE: Addr = Addr::Node(0);

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn at(millis: u64) -> Instant {
    Instant::ZERO + ms(millis)
}
fn get() -> Request {
    Request::Get { key: b"k".into() }
}

/// The deadlines a client with `cfg` hits when nothing ever answers,
/// starting from a submit at time zero. Mirrors the client's backoff rule.
fn expected_deadlines(cfg: &Config, until: Instant) -> Vec<Instant> {
    let mut out = Vec::new();
    let mut timeout = cfg.initial_timeout;
    let mut t = Instant::ZERO + timeout;
    // The timer fires once per retry and once more to give up.
    while t <= until && out.len() < cfg.max_attempts as usize {
        out.push(t);
        timeout = timeout
            .checked_mul(2)
            .unwrap_or(cfg.max_timeout)
            .min(cfg.max_timeout);
        t = t + timeout;
    }
    out
}

#[test]
fn retry_timers_fire_at_exact_backoff_deadlines() {
    let cfg = Config {
        initial_timeout: ms(100),
        max_timeout: ms(1000),
        max_attempts: 5,
    };
    let mut w = World::new(Disk::default());
    w.add_client(1, cfg, 0);
    w.net.cut(CLIENT, NODE);
    let rid = w.submit(1, get());
    w.run_until_idle();

    assert_eq!(w.gave_up, vec![(1, rid)]);
    assert_eq!(w.net.stats().partitioned, 5, "one send per attempt");
    assert_eq!(w.stats.client_timers_fired, 5);
    // 100, 300, 700, 1500, 2500 ms: doubling, then the cap.
    let want = vec![at(100), at(300), at(700), at(1500), at(2500)];
    assert_eq!(expected_deadlines(&cfg, at(10_000)), want);
    let fired: Vec<String> = w
        .trace()
        .lines()
        .iter()
        .filter(|l| l.contains("timer client=1"))
        .cloned()
        .collect();
    let want_lines: Vec<String> = want.iter().map(|t| format!("{t} timer client=1")).collect();
    assert_eq!(fired, want_lines);
    assert_eq!(w.now(), at(2500));
}

#[test]
fn an_answer_cancels_the_timer() {
    let mut w = World::new(Disk::default());
    w.add_client(1, Config::default(), 0);
    w.call(1, get());
    assert_eq!(w.stats.client_timers_fired, 0);
    assert_eq!(
        w.scheduler.stats().cancelled,
        1,
        "the armed timeout was cancelled"
    );
    assert!(w.scheduler.is_empty(), "nothing left in the queue");
    // Idle time does not exist. Running for a year with nothing scheduled
    // moves the clock and nothing else.
    w.run_for(Duration::from_secs(365 * 24 * 3600));
    assert_eq!(
        w.scheduler.stats().fired,
        3,
        "the log read, request and reply delivery, nothing more"
    );
}

#[test]
fn idle_gap_between_events_is_free() {
    // Two clients, one of which retries every 10 minutes for an hour. The
    // other does nothing. The number of events is the number of retries,
    // not the number of milliseconds.
    let mut w = World::new(Disk::default());
    w.add_client(
        1,
        Config {
            initial_timeout: Duration::from_secs(600),
            max_timeout: Duration::from_secs(600),
            max_attempts: 6,
        },
        0,
    );
    w.add_client(2, Config::default(), 0);
    w.net.cut(CLIENT, NODE);
    w.submit(1, get());
    w.run_for(Duration::from_secs(3600));
    assert_eq!(w.stats.client_timers_fired, 6);
    assert_eq!(w.scheduler.stats().fired, 6 + 1, "six timers and the log read");
    assert_eq!(w.now(), Instant::ZERO + Duration::from_secs(3600));
}

#[test]
fn tick_driver_and_event_driver_produce_the_same_trace() {
    // The tick model runs the queue in fixed slices. The event model runs
    // it in one go. Both must handle the same events in the same order, so
    // the traces are identical. What differs is the number of times the
    // driver was called: ticks cost time even when nothing happens.
    fn scenario(w: &mut World) {
        w.add_client(1, Config::default(), 0);
        w.add_client(2, Config::default(), 0);
        w.net.drop_next(NODE, CLIENT);
        w.submit(
            1,
            Request::Put {
                key: b"a".into(),
                value: b"1".into(),
            },
        );
        w.submit(2, get());
    }
    let config = WorldConfig {
        seed: 7,
        net: NetConfig {
            latency_min: ms(1),
            latency_max: ms(4),
            ..NetConfig::default()
        },
        disk: DiskConfig {
            write_latency_max: ms(15),
            ..DiskConfig::default()
        },
        ..WorldConfig::default()
    };
    let end = Duration::from_secs(2);

    let mut event_model = World::cluster(1, config.clone());
    scenario(&mut event_model);
    event_model.run_for(end);

    let mut tick_model = World::cluster(1, config);
    scenario(&mut tick_model);
    let tick = ms(10);
    let mut ticks = 0;
    while tick_model.now() < Instant::ZERO + end {
        tick_model.run_for(tick);
        ticks += 1;
    }
    assert_eq!(ticks, 200);
    assert_eq!(tick_model.trace().hash(), event_model.trace().hash());
    assert_eq!(tick_model.completed, event_model.completed);
    assert!(event_model.completed.len() == 2, "{:?}", event_model.completed);
}

#[test]
fn the_driver_reads_the_scheduler_clock() {
    let mut w = World::new(Disk::default());
    w.add_client(1, Config::default(), 0);
    w.call(1, get());
    assert_eq!(w.hosts[0].process().driver.time.now(), w.now());
    assert_eq!(w.now(), at(2));
}

/// One simulated year of a client that never gets an answer and never
/// gives up. Backoff caps at a minute, so that is about half a million
/// timer events. The assertion is on the count and on determinism; the
/// wall-clock time is printed, not asserted, because a CI machine under
/// load would make a wall-clock assertion flaky. Run with `--nocapture`
/// to see it. In a release build it is well under a second.
#[test]
#[expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "measuring the simulator's own wall-clock speed"
)]
fn one_simulated_year_of_retries() {
    const YEAR: Duration = Duration::from_secs(365 * 24 * 3600);
    let cfg = Config {
        initial_timeout: ms(100),
        max_timeout: Duration::from_secs(60),
        max_attempts: u32::MAX,
    };
    let want = expected_deadlines(&cfg, Instant::ZERO + YEAR);
    assert_eq!(want.len(), 525_608);

    let started = std::time::Instant::now();
    let hash = check_determinism(42, 2, |_prng, trace| {
        let config = WorldConfig {
            keep_trace_lines: false,
            ..WorldConfig::default()
        };
        let mut w = World::cluster(1, config);
        w.add_client(1, cfg, 0);
        w.net.cut(CLIENT, NODE);
        w.submit(1, get());
        w.run_for(YEAR);
        assert_eq!(w.now(), Instant::ZERO + YEAR);
        assert_eq!(w.stats.client_timers_fired, want.len() as u64);
        assert_eq!(w.scheduler.stats().fired, want.len() as u64 + 1);
        assert!(w.gave_up.is_empty());
        // The world's trace has every timer line in it. Fold its hash into
        // the outer trace so the determinism check compares them.
        determinism::trace!(trace, "year hash={:#018x} {:?}", w.trace().hash(), w.stats);
    })
    .unwrap_or_else(|d| panic!("{d}"));
    let elapsed = started.elapsed();
    eprintln!(
        "two simulated years in {elapsed:?}: {:.0}x real time, hash {hash:#018x}",
        2.0 * YEAR.as_secs_f64() / elapsed.as_secs_f64()
    );
}
