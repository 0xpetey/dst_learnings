//! Module 7 acceptance tests: each host's clock is the scheduler's clock
//! seen through a skew, and the conversion back is exact.

use determinism::Prng;
use kv_core::env::Time;
use kv_core::{Duration, Instant};
use sim::{ClockConfig, HostClock, Scheduler, Skew, DRIFT_PPM_MAX};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn at(nanos: u64) -> Instant {
    Instant::from_nanos(nanos)
}

#[test]
fn an_ideal_host_clock_is_the_scheduler_clock() {
    let mut s: Scheduler<()> = Scheduler::new();
    let host = HostClock::ideal(s.clock());
    assert_eq!(host.now(), Instant::ZERO);
    s.schedule_after(ms(250), ());
    s.pop();
    assert_eq!(host.now(), s.now());
    assert_eq!(Time::now(&host), s.now());
    assert_eq!(host.to_true(s.now()), s.now());
    assert_eq!(host.skew(), Skew::IDEAL);
}

#[test]
fn offset_shifts_and_drift_scales() {
    let mut s: Scheduler<()> = Scheduler::new();
    let host = HostClock::new(
        s.clock(),
        Skew {
            offset_nanos: 500_000_000,
            drift_ppm: 10_000, // 1% fast
        },
    );
    assert_eq!(host.now(), at(500_000_000), "offset at true zero");
    s.schedule_after(Duration::from_secs(1), ());
    s.pop();
    assert_eq!(host.now(), at(500_000_000 + 1_010_000_000));
    let slow = HostClock::new(
        s.clock(),
        Skew {
            offset_nanos: 0,
            drift_ppm: -10_000,
        },
    );
    assert_eq!(slow.now(), at(990_000_000), "a slow clock is still ahead of zero");
}

#[test]
fn to_true_is_the_earliest_true_instant_at_which_the_clock_reads_the_deadline() {
    let mut prng = Prng::from_seed(7);
    let s: Scheduler<()> = Scheduler::new();
    for _ in 0..2000 {
        let skew = Skew {
            offset_nanos: prng.below(6_000_000_000) as i64 - 3_000_000_000,
            drift_ppm: prng.below(2 * DRIFT_PPM_MAX as u64 + 1) as i64 - DRIFT_PPM_MAX,
        };
        let host = HostClock::new(s.clock(), skew);
        let local = at(prng.below(10_000_000_000_000));
        let t = host.to_true(local);
        assert!(host.local(t) >= local, "{skew}: local({t}) < {local}");
        if t > Instant::ZERO {
            let before = at(t.nanos() - 1);
            assert!(
                host.local(before) < local,
                "{skew}: {before} already reads {local}"
            );
        }
    }
}

#[test]
fn a_jump_moves_only_forwards_and_every_clone_sees_it() {
    let s: Scheduler<()> = Scheduler::new();
    let host = HostClock::ideal(s.clock());
    let process_view = host.clone();
    host.jump(ms(300));
    assert_eq!(process_view.now(), at(300_000_000));
    assert_eq!(host.skew(), Skew::ahead(ms(300)));
    assert_eq!(host.to_true(at(300_000_000)), Instant::ZERO);
    assert_eq!(host.to_true(at(300_000_001)), at(1));
}

#[test]
fn a_rate_change_does_not_move_the_clock() {
    let mut s: Scheduler<()> = Scheduler::new();
    let host = HostClock::new(
        s.clock(),
        Skew {
            offset_nanos: 100_000_000,
            drift_ppm: 50_000,
        },
    );
    s.schedule_after(Duration::from_secs(2), ());
    s.pop();
    let before = host.now();
    host.set_drift_ppm(-50_000);
    assert_eq!(host.now(), before);
    s.schedule_after(Duration::from_secs(1), ());
    s.pop();
    assert_eq!(host.now(), before + ms(950), "the new rate applies from here on");
    // Speeding up needs a negative offset to keep the clock continuous.
    host.set_drift_ppm(500_000);
    assert!(host.skew().offset_nanos < 0, "{}", host.skew());
    assert_eq!(host.now(), before + ms(950));
    assert_eq!(
        host.local(Instant::ZERO),
        Instant::ZERO,
        "and never reads below zero"
    );
}

#[test]
#[should_panic(expected = "outside the model")]
fn drift_beyond_the_model_is_rejected() {
    let s: Scheduler<()> = Scheduler::new();
    let _ = HostClock::new(
        s.clock(),
        Skew {
            offset_nanos: 0,
            drift_ppm: DRIFT_PPM_MAX + 1,
        },
    );
}

#[test]
fn a_random_config_draws_skews_inside_its_bounds() {
    let mut prng = Prng::from_seed(3);
    let mut jumps = 0;
    for _ in 0..50 {
        let config = ClockConfig::random(&mut prng);
        config.validate();
        jumps += u32::from(config.has_jumps());
        for _ in 0..20 {
            let skew = config.draw_skew(&mut prng);
            assert!(skew.offset_nanos >= 0);
            assert!(skew.offset_nanos as u128 <= config.offset_max.as_nanos());
            assert!(skew.drift_ppm.abs() <= config.drift_ppm_max);
        }
    }
    assert!(jumps > 10 && jumps < 40, "{jumps}");
    assert_eq!(
        ClockConfig::random(&mut Prng::from_seed(9)),
        ClockConfig::random(&mut Prng::from_seed(9))
    );
    let ideal = ClockConfig::IDEAL.draw_skew(&mut prng);
    assert_eq!(ideal, Skew::IDEAL);
}
