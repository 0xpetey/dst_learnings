# Module 7: Fault injection, buggify, and swarm testing

## Why this module exists

The network, the disk and the process each have a fault model. Three gaps remain, and this module closes them.

The first is inside the process. A real process stops for reasons outside the network and the disk: a garbage collection, a VM migration, a `SIGSTOP`. Kleppmann's essay on distributed locking lists these and shows that each one breaks any protocol that measures time with a local clock. The clock itself is a component with faults. The second gap is in the code under test. The network model cannot make the node drop a reply, and the disk model cannot make it skip a heartbeat. FoundationDB's answer is a macro at the point of decision, `BUGGIFY`, and this module writes the Rust version. The third gap is in how a run picks its faults. Modules 5 and 6 drew every probability from a wide range on every run, and swarm testing replaces that search.

Every run from here on has one shape: a configuration drawn from the seed, a chaos phase, and a quiet period. The quiet period carries the liveness check, which is the one that the consensus protocol in Module 10 fails most often.

## Objectives

By the end of this module you can:

- Model a process pause and say what it looks like from the inside and from the outside.
- Give each host a clock with an offset, a rate error and forward jumps, and convert a local deadline back to simulator time.
- Say which of these faults the heartbeat tolerates and which one defeats it, with the arithmetic.
- Write a `buggify!` fault point, choose where it belongs, and say why a `#[cfg]` inside the macro is wrong.
- Explain the two probabilities behind `BUGGIFY` and why sites are enabled per run.
- Explain swarm testing and say why omitting a fault family finds bugs that enabling it hides.
- Draw a whole fault configuration from the seed and put it at the head of the trace.
- Split a run into a chaos phase and a quiet period, and state the liveness check.

## Lesson

### The faults so far, and the ones that are missing

| Where the fault lives | Module | Faults |
|---|---|---|
| Between machines | 5 | Latency, loss, duplication, reordering, partitions |
| On the platter | 6 | Torn, lost and misdirected writes, bit rot, read faults, crash |
| In the process | 7 | Pause, clock skew and drift, clock jumps, a slow disk |
| In the code | 7 | `buggify!` sites: dropped request, dropped reply, duplicate reply, skipped beat |

The environment causes rows one and two, the machine row three, and the program row four.

### A pause is a crash that keeps its memory

A paused process handles nothing. Its memory is intact. Everything addressed to it waits, because the kernel and the device keep working:

```text
   pause(n0)                                        resume(n0)
   ───┬──────────────────────────────────────────────┬────────────►
      │  request ──► inbox (unread)                  │  pump: reads every request at once
      │  beat    ──► held_beats                      │  tick: 460 ms of silence on its own clock
      │  disk write completes ──► completions queue  │  held beats: peers are back
      │  heartbeat timer: disarmed                   │  timer: armed again
```

The world implements this with one flag. `Host::paused` makes `pump` return without stepping the driver, makes `receive` hold a beat instead of delivering it, and makes `arm_host_timer` want no timer. Requests still land in `FakeNetwork`'s inbox and disk completions in `FakeStorage`'s queue. Nothing is dropped, and the node has no idea.

`resume` does three things in a fixed order. It pumps the driver, so the node reads every request that queued. It ticks the heartbeat at the time the host's clock reads now, so the heartbeat notices that time passed. Then it feeds the held beats. The order is a modelling decision. A real event loop wakes up with expired timers and readable sockets at the same time, and which it handles first depends on the loop. The lesson's order gives the paused node a moment in which it believes every peer went silent. `a_pause_looks_like_a_crash_to_the_peers_and_like_nothing_to_the_host` pins both views.

A crash during a pause ends the pause, because the process that was paused is gone. The heartbeat's sequence numbers stay dense across a pause, because beats that were never sent have no numbers. The peers therefore see no straggler, where a restart in Module 6 produced several. That is how a protocol can tell a pause from a restart, and Module 9 uses it.

TigerBeetle's cluster has the same fault. Its `ReplicaHealth` is `up` with a `paused` flag, `down`, or `reformatting`. A paused replica skips its `tick`. The comment on the loop says that time keeps ticking for a replica that is down or paused. The VOPR pauses a replica with probability `8 / 10_000_000` per tick and crashes one with `2 / 10_000_000`, so pauses are four times as common as crashes. That ratio is a statement about production.

### The clock is a component

Every host now reads its own clock, `sim::HostClock`. It is the scheduler's clock seen through a `Skew`:

```text
   local(t) = offset + t * (1_000_000 + drift_ppm) / 1_000_000

   true  ──────────────────────────────────►
   n0    ──────────────────────────────────►   drift_ppm = 0
   n1    ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─►   drift_ppm > 0, runs fast
   n2    ────────────╱──────────────────────►  a jump: offset grows
```

The offset is the skew. Two hosts that start at different instants read different times, and nothing in the system can compare a local instant with a remote one. Nothing does yet. Module 10's planted lease is the first thing that wants to, and the skew is there to catch it. The drift is the rate error in ppm. Real oscillators are within a few hundred ppm, and the model allows up to half a million, which is 50%. The jump is a step, as after a hypervisor pause or a missed interrupt, and it is only forward. `Instant` models a monotonic clock, and a monotonic clock that steps backwards is a hardware bug the core is allowed to assert against.

The process reads the host's clock, and nothing else does. The disk and the network keep true time, because a device does not care what the kernel believes. The world converts in one place. The heartbeat reports its next deadline on its own clock, and `arm_host_timer` schedules the event at `clock.to_true(deadline)`. That function returns the earliest true instant at which the clock reads the deadline or later. A jump moves deadlines into the past. `jump_clock` therefore ticks the heartbeat at once, and every deadline inside the leap comes due together.

TigerBeetle's `TimeSim` has four offset types, `linear`, `periodic`, `step` and `non_ideal`, and applies them to the wall clock only. Its monotonic clock is `ticks * resolution`, ideal. This model skews the monotonic clock too, because the heartbeat's timeouts are measured on it, and two monotonic clocks on two machines do drift apart. A clock that is read by nothing cannot find a bug.

The trace keeps one timeline. The world's lines and the node's input and output lines carry true time. `TraceObserver` stamps the node's lines from its own `Clock` and ignores the local reading the driver passes in. The local reading appears on the `timer node=`, `resume` and `clock jump` lines.

### What the heartbeat tolerates

The heartbeat has a margin. It beats every 100 ms and suspects a peer after 350 ms of silence, so three and a half beats have to go missing. The acceptance tests pin which faults of this module cross that margin.

| Fault | What the paused or skewed host sees | What its peers see |
|---|---|---|
| Pause of 500 ms | On waking, everyone silent for 500 ms, so two `Down` and two `Up` in one instant | One `Down` after 350 ms, one `Up` on the first beat after the resume |
| Jump of 400 ms | Everyone silent for 400 ms, two `Down` at once, two `Up` within a round | Nothing. The jump is inside one host |
| Jump of 200 ms | Nothing. The margin is 350 ms | Nothing |
| Drift of 20% | Nothing. 350 ms of its time is 292 ms of wall time, and beats are 100 ms apart | Its beats arrive every 83 ms, which is early and harmless |
| Drift of 50% against a peer at minus 50% | Nothing. Its margin is 233 ms of wall time, and the slow peer beats every 200 ms | Nothing |
| Half of all beats skipped | Three skipped in a row is one chance in eight, so some runs of silence cross the margin | `Down`, then `Up` on the next beat, every time |

Drift alone cannot eat a margin of three and a half intervals. The jump finds something at once, because a jump is a pause from the inside. The process reads the clock before and after and sees the same gap, whether the world moved or not. The realistic faults are the pause and the jump. The drift is there for the protocols of Modules 9 and 10, which compare durations across hosts.

### BUGGIFY: faults the code does to itself

FoundationDB has a macro, `BUGGIFY`, that is false in production and sometimes true in simulation. The fault code therefore ships in the production binary. It sits at the points where the author knows the code is supposed to cope. Drop a message the protocol must retry. Delay a step the protocol must not depend on. Take the error branch that a real error takes once a year.

There are two probabilities. The first time a run reaches a `BUGGIFY` site, the site is enabled or disabled for the whole run. FoundationDB's `Buggify.h` keeps a map from file and line to that decision, and `P_GENERAL_BUGGIFIED_SECTION_ACTIVATED` is 25%. An enabled site then fires with `P_GENERAL_BUGGIFIED_SECTION_FIRES`, also 25%, each time the code reaches it. Two runs with different enabled sets exercise different unusual paths, and no site fires often enough to stop the system making progress. That is swarm testing applied to the code's own decisions.

The Rust version is `kv_core::buggify`:

```rust
if crate::buggify!(self.buggify, Site::DropReply) {
    return; // the outbound buffer was full and the reply fell on the floor
}
```

`Site` is an enum with one variant per fault point. The trace can name the one that fired, and a test can enable exactly one. `Buggify` holds the enabled set, the firing probability, a `Prng` forked from the world's, and a count per site. The macro expands to a method call on purpose. Rust evaluates a `#[cfg]` inside a `macro_rules!` body in the crate that expands the macro, against that crate's features. A `cfg(feature = "buggify")` in the macro body asks whether `kv-env` has the feature, not whether `kv-core` does. The gate therefore sits on the method. Without the `buggify` cargo feature, `Buggify` is a zero-sized type and `fire` is a `const fn` that returns `false`. The compiler removes the branch and the production binary carries no dice. With the feature, the production server still constructs `Buggify::off`, under which no site is enabled. Both layers are on purpose, because a fault point that fires in production is a bug.

This module has four sites. `DropRequest` sits at the top of `Node::on_request`, before anything is counted or recorded, so a dropped request leaves no trace in the node. `DropReply` and `DuplicateReply` sit in one `reply` function that every reply goes through. A dropped reply is answered from the session on the retry. A duplicate reply has to be ignored by the client. `SkipBeat` sits in the heartbeat's tick, and a skipped round sends nothing and takes no sequence number. The protocol is supposed to survive each one, and the site makes the surviving code run on half the seeds instead of never.

One site Alex Miller's "BUGGIFY" article suggests is missing on purpose. "Delay an fsync" is a `wait(delay(5))` in FoundationDB, and there is no delay inside a sans-I/O state machine. The disk model's latency range is the fsync delay, and the slow host of the next section is the fsync that is always slow. A fault point in the core decides, and the environment delays.

`grep -rn 'crate::buggify!' crates/kv-core/src` lists every fault point. That is the whole discovery mechanism, and it is the same one FoundationDB has. The names are the documentation, and a site that nobody can explain in one line is a site to remove.

### Swarm testing

Groce, Zhang, Eide, Chen and Regehr published "Swarm Testing" in 2012. In their words, random testing uses "a large 'swarm' of randomly generated configurations, each of which omits some features", "with configurations receiving equal resources". In a week of testing, the swarm "found 42% more distinct ways to crash a collection of C compilers" than the hand-tuned default configuration did.

The paper gives two reasons. The first is in their stack example. A stack with a capacity bug fails above 32 items. A tester that pushes and pops with equal probability almost never gets there, because the pops keep the stack short. A configuration that omits `pop` finds the bug in seconds. In their words, some features "actively prevent the system from executing interesting behaviors". Second, features compete for room in a test of fixed length, so a run with fewer of them goes deeper into each.

Both reasons apply to faults. A crash schedule that resets every host every second hides a bug that needs ten seconds of uptime under a flaky network. Network loss makes clients retry, and the retries hide a node that drops replies on its own. Module 6's chaos run had every fault on in every seed, with its rate drawn from a range. That configuration is one point in the space, and the paper says that it is a poor one.

`WorldConfig::swarm` draws the configuration the paper asks for. A coin turns each of eight fault families on or off, and each one that is on draws its own parameters:

```text
   net          NetConfig::random        or reliable
   disk         DiskConfig::random       or reliable
   partitions   PartitionSchedule        or none
   crashes      CrashSchedule            or none
   pauses       PauseSchedule            or none
   clocks       ClockConfig::random      or ideal
   buggify      enable 10..60%, fire 5..30%   or off
   slow_host    one disk, 2x to 10x slower    or none
```

The draws happen in a fixed order, and a family that is off spends no randomness. Adding a ninth family at the end keeps the meaning of every earlier draw for every existing seed. `every_family_is_on_in_some_runs_and_off_in_others` checks the swarm over 64 seeds. Each family is on in between a quarter and three quarters of them, and at least 40 of the 64 subsets are distinct.

TigerBeetle's `options_swarm` in `src/vopr.zig` is the production version. It draws the replica count, the client count, every network and storage probability, and the partition mode from the seed. Its crash and pause probabilities are fixed, with a comment that says "TODO Swarm testing: Test long+few crashes and short+many crashes separately". The lesson's version takes that comment as a requirement.

### Chaos, then quiet

In the chaos phase every enabled fault runs and the workload runs under it. Then `World::stop_faults` turns everything off at once. It cancels every schedule, heals every link, restarts every host that is down, and resumes every host that is paused. It makes the network and every disk reliable at their minimum latencies, and it disables every fault point. What follows is the quiet period, which asks whether the cluster comes back.

Liveness can be checked only in the quiet period. During the chaos a partitioned node is allowed to be out of touch, and a client is allowed to give up. Once the faults stop, nothing excuses a host that is down or a peer that stays `Down`. The chaos test in this module asserts three things after `stop_faults`. Every host that is up sees every other host that is up. Every host that is down halted over a damaged medium, and only a disk that can damage its medium produced one. Every host that is up answers a fresh client. The last check needs a new client id, because the old ids name sessions that are in the log.

FoundationDB and TigerBeetle both have this shape. FoundationDB sets `speedUpSimulation` after 300 simulated seconds. The destructive `BUGGIFY` sites guard on it, so a test shifts from causing damage to letting the system recover. TigerBeetle's `transition_to_liveness_mode` restarts and unpauses a core set of replicas and sets the crash, restart, pause and reformat probabilities to zero. The run then has `ticks_max_convergence` to reach a converged state. A run that does not is reported as `no state convergence`, with the seed to reproduce it. Module 9 builds that report.

### The configuration heads the trace

The first two lines of every trace are the configuration:

```text
t=0ns config seed=... net=[latency=2ms..31ms loss=7/100 dup=3/100 reorder=12/100+88ms] disk=[read=...] heartbeat=[...] partitions=none crashes=[every<=2.1s down<=312ms] pauses=[every<=1.4s for<=210ms] clocks=none buggify=[enable=40/100 fire=12/100] slow_host=[disk x7]
t=0ns config drawn: slow_host=n2 buggify_sites=[drop_reply skip_beat]
```

The first line is what the run was given. The second is what the seed drew from it: which host has the slow disk, each host's clock skew, and which sites are enabled. A failing seed is therefore readable before its first event. This is the syllabus requirement, and it is also TigerBeetle's practice. The VOPR prints every option it drew, under a `SEED=` header, before the first tick.

Every fingerprint changed on purpose, because of these two lines and because `Chance::roll` now spends no randomness on `NEVER` and `ALWAYS`.

### Reading a trace

Two hosts and one client. The test pauses host 0 at 305 ms, submits a write during the pause, and resumes the host at 765 ms. Beat sends and deliveries between the hosts are cut from the listing. Timestamps are shortened.

```text
t=305ms    n0 pause
t=306ms    net deliver c1->n0 m8 request client=1 id=1
t=401ms    n0 paused, held beat seq=5
t=405ms    timer client=1
t=406ms    net deliver c1->n0 m10 request client=1 id=1
t=501ms    n0 paused, held beat seq=6
t=601ms    n0 paused, held beat seq=7
t=605ms    timer client=1
t=606ms    net deliver c1->n0 m13 request client=1 id=1
t=651ms    n1 peer down n0
t=701ms    n0 paused, held beat seq=8
t=765ms    n0 resume held_beats=4 local=t=765ms
t=765ms    n0 in  Request client=1 id=1 Put { key: [97], value: [49] }
t=765ms    n0 out Append op=1 offset=0 bytes=512
t=765ms    n0 in  Request client=1 id=1 Put { key: [97], value: [49] }
t=765ms    n0 in  Request client=1 id=1 Put { key: [97], value: [49] }
t=765ms    n0 peer down n1
t=765ms    n0 peer up n1
t=766ms    n1 peer up n0
t=770ms    n0 disk write complete op=1 landed
t=770ms    n0 out Reply client=1 id=1 PutOk
t=771ms    client=1 done id=1 PutOk
```

The request is delivered at 306 ms and waits in the inbox. The client retries at 405 ms and 605 ms, and both retries queue behind the first. Host 1 gives up on host 0 at 651 ms, which is 350 ms after the last beat it heard at 301 ms. On resume, the node reads all three requests in one step, appends once, and absorbs two duplicates. In the same instant the heartbeat suspects host 1 and hears from it again.

### How the other simulators do it

TigerBeetle's VOPR runs until the requests are done and then calls `transition_to_liveness_mode`. The code under test has no `BUGGIFY`. TigerBeetle puts its faults in the environment and its assertions in the code.

FoundationDB has `BUGGIFY` in the code and fewer knobs in the environment. Its simulated network and disk are models of a fairly ordinary data centre, and the thousands of `BUGGIFY` sites supply the variety. `BUGGIFY_WITH_PROB` lets a site lower its own rate.

`turmoil` has `hold` and `release` on a link, which is a pause of the network between two hosts rather than a pause of a host. `madsim` can kill and restart a node and skews time per node. Neither has a fault-point macro, and neither draws a configuration from the seed. The test author writes the schedule by hand, which is a reasonable choice for a test and the wrong one for a search.

### Decisions recorded

- A pause is a flag on the host. The process is not stepped, requests and disk completions queue where they already queue, beats are held, and the timer is disarmed. On resume the order is pump, tick, held beats. One host is paused at a time by the schedule.
- Each host has a `HostClock` with a signed offset, a rate error in ppm up to 500 000, and forward jumps only. The process reads it. The disk, the network and the trace observer read true time. Timers are armed at `to_true(deadline)`, clamped to now.
- A jump ticks the heartbeat at once, so every deadline inside the leap is handled at the moment of the jump.
- `Chance` moved to `determinism`, and `roll` spends no randomness on `NEVER` or `ALWAYS`. `sim::Chance` is a re-export.
- `buggify!` expands to a method call. The feature gate is on the method, not in the macro, because a `cfg` in a macro body is evaluated in the calling crate. `Buggify::off` is the production value under both configurations.
- Sites are enabled per run from the world's `Prng`, and each state machine gets its own `Buggify` with a forked `Prng`. The firing rate is per world. Fires are drained into the trace after every pump.
- There is no delay site in the core. Time belongs to the environment.
- The swarm is eight families, each on by a coin, drawn in a fixed order. A family that is off spends no randomness.
- `stop_faults` is the one call that starts the quiet period. Drift and offsets stay, because they are not faults a cluster recovers from.
- The configuration and the per-run draws are the first two trace lines. The three older fingerprints changed on purpose, and the swarm run adds a fourth.

## Readings

Required:

- [Alex Miller, "BUGGIFY"](https://transactional.blog/simulation/buggify). The best account of FoundationDB's fault points: the two probabilities, where to put a site, and the 300 second damage control. Read all of it.
- [Groce, Zhang, Eide, Chen and Regehr, "Swarm Testing" (ISSTA '12)](https://agroce.github.io/issta12.pdf). Read the abstract, section 1 for the stack example, and the compiler results. The two reasons in section 1 are the argument for `WorldConfig::swarm`.
- [Martin Kleppmann, "How to do distributed locking"](https://martin.kleppmann.com/2016/02/08/how-to-do-distributed-locking.html). Read the sections on process pauses and on clocks. Every cause on his list is a `pause` or a `jump_clock` in this module.
- TigerBeetle: [`src/vopr.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vopr.zig). Read `options_swarm`, the `replica_pause_probability` lines of the options, the `SEED=` print, and `transition_to_liveness_mode`.
- FoundationDB: [`flow/include/flow/Buggify.h`](https://github.com/apple/foundationdb/blob/main/flow/include/flow/Buggify.h). Short. Read the two `P_GENERAL_` constants and the map that memoises each site's activation.

Optional:

- [Kyle Kingsbury, "The trouble with timestamps"](https://aphyr.com/posts/299-the-trouble-with-timestamps). Why a timestamp from another node is not comparable with yours. The argument for the offset in `Skew`.
- TigerBeetle: [`src/stdx/time.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/stdx/time.zig). `TimeSim` and its four `OffsetType`s. Note that `monotonic` is ideal and `realtime` carries the offset.
- TigerBeetle: [`src/testing/cluster.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/testing/cluster.zig). `ReplicaHealth`, `replica_pause` and the comment on `tick`.
- [TigerBeetle, "VOPR" internals page](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vopr.md). The project's own summary of what the seed drives.

## Coding task

### Starting point

Start from your Module 6 workspace with `dst start 07`. If you did not finish Module 6, `dst start 07 --from-solution` starts from `solutions/06-sim-disk/` instead. The command adds `clock.rs` to `crates/sim/tests/` and `buggify.rs` to `crates/kv-core/tests/`. In `crates/kv-env/tests/` it replaces the four existing integration tests and adds `fault_injection.rs`. It replaces `scripts/check.sh`, which gains one line, a bare-metal check of `kv-core` with the `buggify` feature on.

### Requirements

#### A shared probability, in `determinism`

1. `determinism::Chance` is the type that was `sim::net::Chance`. It gains `per_mille`, `is_always`, `numerator` and `denominator`. `roll` returns without touching the `Prng` on `NEVER` and on `ALWAYS`. `sim::Chance` and `sim::net::Chance` re-export it.

#### Fault points, in `kv_core::buggify`

2. `kv-core` has a cargo feature `buggify` that enables an optional `no_std` dependency on `determinism`. `cargo tree -p kv-core` shows no dependency without it.
3. `Site` is an enum with `DropRequest`, `DropReply`, `DuplicateReply` and `SkipBeat`. `Site::ALL`, `Site::COUNT`, `name()`, `index()` and `Display` as the snake-case name.
4. With the feature, `Buggify` has `off()`, `new(&[Site], Chance, Prng)` and `draw_sites(Chance, &mut Prng) -> Vec<Site>`. Its queries are `is_enabled`, `enabled_sites`, `fire_chance`, `fired(Site) -> u64` and `total_fired`. Its mutators are `set_fire`, `disable_all`, `fire(Site) -> bool` and `take_log() -> Vec<Site>`. `fire` spends randomness only on an enabled site. Without the feature, `Buggify` is zero sized and `fire` is a `const fn` returning `false`. `off`, `fired`, `total_fired` and `take_log` exist in both.
5. `buggify!(b, site)` is a `#[macro_export]` macro that expands to `$crate::buggify::Buggify::fire(&mut b, site)` under both configurations.
6. `Node::with_buggify(Buggify, &mut Vec<Output>)`, `Node::buggify()` and `buggify_mut()`. `Node::new` is `with_buggify(Buggify::off(), ..)`. `DropRequest` is checked at the top of `on_request` before `stats.requests` is counted. Every reply goes through one path that checks `DropReply` and then `DuplicateReply`. `Driver::with_buggify(Buggify, T, S, N, O)`, and `Driver::new` uses `off`.
7. `Heartbeat::with_buggify(Buggify, me, peers, config, now, out)`, `buggify()` and `buggify_mut()`. On `SkipBeat` a round sends nothing and does not advance `seq`, and `next_beat` still advances.

#### Host clocks, in `sim::clock`

8. `Skew { offset_nanos: i64, drift_ppm: i64 }` with `IDEAL`, `ahead(Duration)`, `validate` and `Display` as `offset=+1.5s drift=10ppm`. `DRIFT_PPM_MAX` is 500 000 and `validate` panics beyond it with a message that contains `outside the model`.
9. `HostClock::new(Clock, Skew)`, `ideal(Clock)`, `skew()`, `now()`, `local(Instant)`, `to_true(Instant)`, `jump(Duration)` and `set_drift_ppm(i64)`. `HostClock` implements `Time`. Clones share the skew. `local` never returns below zero. `to_true(l)` is the earliest true instant `t` with `local(t) >= l`. `set_drift_ppm` keeps `now()` unchanged.
10. `ClockConfig { offset_max, drift_ppm_max, jump_gap_max, jump_max }` with `IDEAL`, `random`, `has_jumps`, `validate`, `draw_skew` and `Display`. `draw_skew` draws a non-negative offset in `0..=offset_max` and a drift in `-drift_ppm_max..=drift_ppm_max`, spending no randomness on a zero range.
11. `NetConfig` and `DiskConfig` implement `Display` on one line. A reliable configuration has `reliable` in the text.

#### The world, in `kv_env::fake`

12. `WorldConfig` gains `pauses: Option<PauseSchedule { gap_max, hold_max }>`, `clocks: Option<ClockConfig>`, `buggify: Option<BuggifyConfig { enable, fire }>` and `slow_host: Option<SlowHost { factor }>`. `BuggifyConfig::default()` is 25% and 25%. `WorldConfig::swarm(&mut Prng)` draws eight families by coin in the order of the lesson. `families() -> [bool; 8]` reports them. `WorldConfig` implements `Display`.
13. `Host` gains `clock: HostClock`, `is_paused()` and `local_now()`. `FakeDriver` is `Driver<HostClock, FakeStorage, FakeNetwork, TraceObserver>`. `TraceObserver` has a `clock: Clock` and stamps lines with true time. `FakeStorage::completions_waiting()` and `FakeNetwork::inbox_len()` are queries.
14. `World::with_disks` draws three things in this order: one `Skew` per host, the slow host, and the enabled sites with `Buggify::draw_sites`. A family that is off draws nothing. The slow host's disk latencies are multiplied by `factor`. Then it logs the two configuration lines, then it starts the hosts and the schedules.
15. Every process gets two `Buggify` values, one for the node and one for the heartbeat. Each has this run's sites, the configured `fire` and a `Prng` forked from the world's. With `buggify: None` nothing is forked.
16. `Event` gains `Pause`, `Resume { node }` and `ClockJump`. `World::pause(node)`, `resume(node)`, `jump_clock(node, by)`, `is_paused(node)`, `local_now(node)`, `buggify_sites()` and `slow_host()`.
17. A paused host is not pumped, holds beats in arrival order, and has no timer. Requests wait in the inbox and disk completions in the storage queue. `resume` pumps, ticks the heartbeat at local time, feeds the held beats, routes and arms. `crash` ends a pause and drops the held beats. The pause schedule pauses one running host at a time and `Resume` skips a host that crashed meanwhile.
18. Heartbeat inputs carry the host's local time. `arm_host_timer` schedules at `clock.to_true(deadline).max(now)` and wants no timer for a paused host. `jump_clock` ticks the heartbeat of a running host at once.
19. `stop_pauses`, `stop_clock_jumps` and `stop_faults`. `stop_faults` logs `faults stopped` and calls the four `stop_` methods. It sets the network to `reliable(latency_min)` and every disk to `reliable(read_latency_min, write_latency_min)`. It clears the enabled sites and disables every live `Buggify`.
20. `WorldStats` gains `held_beats`, `pauses`, `resumes`, `clock_jumps` and `buggify_fires`. Every fire is one trace line and one count.
21. Trace lines, where `{now}` is true time and `{local}` the host's reading:
    - `{now} config {config}` and `{now} config drawn: n0=[{skew}] ... slow_host=n{k} buggify_sites=[a b]`. A family that is off has no part in the second line.
    - `{now} n{id} pause`, `{now} n{id} paused, held beat seq={s}`, `{now} n{id} resume held_beats={k} local={local}`.
    - `{now} n{id} clock jump by={d:?} local={local}`, `{now} timer node={id} local={local}`.
    - `{now} n{id} buggify {site}`.
    - `{now} pauses stopped`, `{now} clock jumps stopped`, `{now} faults stopped`.
22. The fingerprints. The seeded workload in `fake_world.rs` is `0x2f50_c6ad_b6ea_c983`. The network chaos run is `0x8562_404d_a09c_be58`. The storage chaos run is `0xea7a_6198_ba24_4e5d`. The swarm run in `fault_injection.rs` is `0x7ded_4ea7_b0c9_6c9e`.

### Acceptance criteria

```sh
scripts/check.sh                                # ends with "all checks passed"
cargo tree -p kv-core                           # kv-core alone, plus determinism as a dev-dependency
cargo tree -p kv-core --features buggify -e no-dev   # kv-core and determinism
cargo tree -p sim                               # sim, determinism and kv-core, nothing else
grep -rn 'crate::buggify!' crates/kv-core/src   # exactly four sites: three in node.rs, one in heartbeat.rs
cargo test --release -p kv-env --test virtual_time one_simulated_year -- --nocapture
cargo test --release -p kv-env --test fault_injection
```

The year test still prints `two simulated years in` under one second in a release build. The reference solution takes about 0.6 s. The swarm run tests 24 seeds three times each, with a different fault configuration per seed. It takes about 0.15 s in release and 0.5 s in debug.

### Hints

<details>
<summary>The pause flag</summary>

Three functions read the flag. `pump` returns at once, `receive` pushes a beat onto `held_beats`, and `arm_host_timer` wants `None`. Nothing else changes.

</details>

<details>
<summary>The heartbeat invariant on resume</summary>

The heartbeat asserts that its next beat is in the future after every input. A held beat fed before the tick breaks it, because the pause carried the clock past the next beat. Tick first. The tick's loop advances `next_beat` past now, and the held beats then arrive at a time the heartbeat already knows about.

</details>

<details>
<summary>Converting a deadline</summary>

Write `to_true` as a ceiling division and then check it. Loop while `local(t)` is still below the deadline, and loop back while `local(t - 1)` already reaches it. With the arithmetic right, both loops run zero times. The test with two thousand random skews proves it. Clamp the result to `now` in the world, because a jump moves deadlines into the past.

</details>

<details>
<summary>A signed offset</summary>

`set_drift_ppm` keeps the current reading fixed and changes the slope. To speed up a clock late in a run, it needs a negative offset. The offset is therefore an `i64` in nanoseconds, and `local` clamps at zero.

</details>

<details>
<summary>Where the `cfg` goes</summary>

Put `#[cfg(feature = "buggify")]` on two private modules that each define a `Buggify`, and re-export one of them. The macro body has no `cfg` in it. With one there, a test that enables the feature on `kv-core` alone sees sites that never fire.

</details>

<details>
<summary>The off-by-one in a skipped beat</summary>

A heartbeat's `next_deadline` is also the moment a silent peer becomes suspect, which is not a round. A test that counts rounds by stepping to `next_deadline` counts one too many. Tick on the beat instead.

</details>

<details>
<summary>Finding your fingerprints</summary>

Find them as in the earlier modules, after the four determinism tests pass. Any difference in a `Display` impl changes the configuration line and so the hash.

</details>

### Stretch goals

- Feed the held beats before the tick on resume. Which assertion in the module breaks, and what does the heartbeat need so that both orders are legal?
- Add a periodic offset to `Skew`, as TigerBeetle's `TimeSim` has. When `local` is not monotonic in `t`, what does `to_true` need?
- Give `Buggify` a per-site firing probability, as `BUGGIFY_WITH_PROB` does, and lower `DropRequest` to 1%. Which test changes?
- Add a `SlowReply` site that holds a reply until the next input. Say why it belongs in the environment instead, or show that it does not.
- Make the pause schedule draw a mode per run, many short pauses or few long ones, as the TigerBeetle comment asks for crashes. Compare the `downs` counts.
- Pause a client instead of a host. What does the node see, and what does the session table do with the burst of retries?

## Check your understanding

1. A paused host and a crashed host look the same to their peers for the first 350 ms. Name two ways the protocol can tell them apart afterwards.
2. Why does `resume` tick the heartbeat before it feeds the held beats, and what does the paused node believe in between?
3. The model allows 50% drift and the heartbeat never notices. Why is the drift still worth having?
4. A clock jump of 400 ms and a pause of 400 ms produce the same reading on the host's clock. What differs, and which trace line shows it?
5. State the two `BUGGIFY` probabilities. If the first one is removed and every site is always enabled, what goes wrong?
6. Why is a `#[cfg(feature = "buggify")]` inside the `buggify!` macro body wrong, and where does the gate go instead?
7. Give the stack example from the swarm testing paper and translate it into two fault families of this module.
8. Why does the quiet period keep the clock drift and offsets but turn off the jumps?
9. All four fingerprints changed in this module. Name the two changes to the trace that did it.

<details>
<summary>Answers</summary>

1. A restarted process counts beats from one again, so its first beats are stragglers to its peers. A resumed process continues its sequence, so there are none. A restarted node read its disk and rebuilt its sessions. A resumed node still has its in-memory state, including requests it read and did not answer yet.
2. After every input, the heartbeat asserts that its next beat is in the future. A beat fed at a time past the next beat breaks that. The tick advances the schedule. In between, the paused node marks every peer `Down`. Its own clock says that they were silent for the length of the pause.
3. Nothing in the system compares a duration measured on one host with a duration measured on another. Module 10's view-change timeouts and its planted lease do. The drift is the test for them, and it costs nothing until they arrive.
4. In a pause the world moved on without the process. Peers suspected it, requests queued, and disk writes completed. In a jump only the host's clock moved, and the peers saw nothing. The `resume` line carries `held_beats=`, and the `clock jump` line carries only `by=` and `local=`.
5. Each site is enabled with 25% probability once per run, and an enabled site fires with 25% probability each time it is reached. If every site is always enabled, every run exercises the same mix of unusual paths. A site that masks another's bug masks it on every seed, which is the problem swarm testing exists to fix.
6. A `cfg` in a macro body is evaluated in the crate that expands the macro, against that crate's feature set. The gate goes on the `fire` method in `kv-core`, with two definitions of `Buggify` selected by the feature.
7. A tester that pops as often as it pushes never fills a stack with a capacity bug. A configuration without `pop` finds the bug. Crashes are the pops. A schedule that restarts a host every second prevents any bug that needs ten seconds of uptime. Network loss is another. Retries hide a node that drops its own replies.
8. Drift and offsets are properties of the machines. No cluster recovers from them, and a cluster that only works with perfect clocks is broken. A jump is an event, and an event in the quiet period is a fault.
9. The two configuration lines at the head of every trace, and `Chance::roll` spending no randomness on `NEVER` and `ALWAYS`. The second one shifts every draw after a fault that is off.

</details>

## What's next

The simulator cannot yet say whether an answer the store gave was right. Module 8 builds a workload generator with many concurrent clients, a reference model, and a small linearizability checker for the histories the model cannot judge. It then plants three bugs in the store and makes sure that the checker finds each one.
