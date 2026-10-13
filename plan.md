# Plan: Building the DST Curriculum

This file describes the task of producing the curriculum in `CURRICULUM.md`. `CURRICULUM.md` is the syllabus that a learner reads. This file is the work plan for the person who writes the modules. When you complete a module, update the status table at the bottom.

## Goal

Produce a complete, self-paced course on Deterministic Simulation Testing in Rust. The course has fourteen modules (0 to 13), as outlined in `CURRICULUM.md`. Every module ends in a coding task that extends one shared Cargo workspace. A learner who finishes the course has a replicated KV store and a VOPR-style simulator that they built themselves.

## Deliverables

1. `CURRICULUM.md`, the syllabus. It is written. When a module reveals a mistake in the plan, revise it.
2. `modules/NN-slug/README.md`, one full lesson per module. The template is below.
3. `project/`, the shared Cargo workspace that the learner builds. It ships as a skeleton for Module 1. Later modules add crates and files.
4. `solutions/NN-slug/`, a reference implementation of each module's task, kept out of the learner's path. It proves that each task is feasible. It also gives later modules a known starting point.
5. `README.md`, the entry point. It says what the course is, how to start, and how to find your way.

## Repository layout

```
dst_learnings/
  README.md
  CURRICULUM.md
  plan.md
  mise.toml           # tasks: check, check:NN, check:project, sweep, hunt
  modules/
    00-why-dst/README.md
    01-determinism/README.md
    02-architecture/README.md
    03-rust-hazards/README.md
    04-virtual-time/README.md
    05-sim-network/README.md
    06-sim-disk/README.md
    07-fault-injection/README.md
    08-workloads-checking/README.md
    09-vopr/README.md
    10-consensus/README.md
    11-debugging-shrinking/README.md
    12-ci/README.md
    13-landscape/README.md
  # each module directory also holds tests/, and scripts/, starter/,
  # clippy.toml or workflows/ when its task needs them
  project/            # learner's workspace, skeleton only
    Cargo.toml
    rust-toolchain.toml
    rustfmt.toml
    crates/
  solutions/
    01-determinism/
    02-architecture/
    ...
    13-landscape/     # also RETROSPECTIVE.md
```

## Per-module lesson template

Each `modules/NN-slug/README.md` has these sections, in this order:

1. Why this module exists. Two or three paragraphs that connect the module to the previous module and to the end goal.
2. Objectives. The list from `CURRICULUM.md`, refined.
3. Lesson. The teaching content. It gives the learner enough to do the task without reading every linked source. Use ASCII or Mermaid diagrams where a picture helps (event queue, network delivery, disk sectors).
4. Readings. Each reading has a working URL, a one-line reason to read it, and a marker for required or optional. When you write a module, make sure that every link works.
5. Coding task, with these sub-sections:
   - Starting point. The solution directory or prior task that the learner begins from.
   - Requirements. Numbered, specific, testable.
   - Acceptance criteria. What `cargo test`, `cargo clippy`, and any scripted checks must show. Prefer checks that the learner can run locally and that CI runs too.
   - Hints. Collapsed or at the end, so that the learner opts in.
   - Stretch goals. Optional extensions.
6. Check your understanding. A few questions, with the answers at the bottom.
7. What's next. One paragraph that hands off to the following module.

## Conventions for the coding tasks

- Rust stable, pinned in `rust-toolchain.toml`. `Cargo.lock` is committed.
- The workspace profile sets `overflow-checks = true` and `debug-assertions = true` for every profile that the simulator uses.
- From Module 3 onward, the core crate is `#![no_std]` + `alloc` and `#![forbid(unsafe_code)]`. `clippy.toml` bans the nondeterministic methods and types that Module 3 lists.
- The Module 1 determinism check (run a seed twice, compare the trace hashes) is a test in the acceptance criteria of every later module.
- Every task is cumulative. A learner who completes Module N has exactly the starting point for Module N+1. If a later module needs something that earlier tasks did not produce, fix the earlier task. Do not hand-wave.
- When a seed finds a real bug in the reference solution, that seed becomes a regression test in the solution. The lesson mentions it as an example.
- Dependencies are minimal and justified. The simulator crate can use `rand_pcg`/`rand_xoshiro`, `clap`, and a serialization crate. The core crate uses nothing that touches the OS.

## Workflow for writing each module

1. Re-read the module's entry in `CURRICULUM.md`, and the lesson and solution of the previous module.
2. Gather the readings and make sure that each link works. Replace anything dead or weak. If a source contradicts a claim in the syllabus, fix the syllabus.
3. Build the reference solution first, from the previous solution. This shows what the task requires and how long it takes.
4. Write the task specification from the solution: requirements, acceptance criteria, and tests. The tests must pass against the solution and fail against the starting point.
5. Write the lesson so that it teaches exactly what the task needs, and no more.
6. Write the questions and the hand-off paragraph.
7. Review the module against the template and against `CURRICULUM.md`. If the module drifted, update the syllabus. Update the status table below.

## Order of work

Write the modules in numeric order, because each task depends on the last. There are two exceptions:

- Module 0 has no code dependency. Write it at any time.
- Module 13 depends on everything. Write it last, after a real port to `turmoil` or `madsim`, so that the comparison is honest.

Within Part I, write Modules 1, 2, and 3 together before you polish any of them. The Module 3 workspace restructure changes what Modules 1 and 2 must look like.

## Open decisions

Resolve each decision before or during the module that needs it. Record the decision in that module's lesson.

- Consensus protocol for Module 10. DECIDED: Viewstamped Replication, with the primary of view `v` as `v mod n`. Raft's persisted term is a superblock, and Raft's consistency check is a parent checksum. Module 9's views, runs and newest-log rule were the shared core. A vote-based election is a stretch goal.
- Event loop ownership (Module 2). DECIDED: a hand-rolled single-threaded loop with explicit state machines (`kv_core::env::Driver`). The Module 2 lesson records it. Two related decisions were made there. The core defines its own `Instant(u64)`, because a fake clock cannot construct `std::time::Instant`. The driver is generic over the environment traits and does not use `dyn`.
- Tick model versus pure event queue (Module 4). DECIDED: a pure event queue, `sim::Scheduler`, with `(Instant, seq)` as the heap key. `World::run_for` in a loop gives the tick model, and a test proves that both drivers produce the same trace. The Module 4 lesson records the trade-offs. Two related decisions: the `sim` crate is `no_std` and depends on `kv-core` for `Instant`, and the fakes hold no handle to the scheduler. The world mediates.
- Linearizability checker depth (Module 8). DECIDED: Wing and Gong's search with Lowe's cache of `(placed set, state)` pairs, partitioned by key, under a budget that returns `Unknown`. That is Porcupine's algorithm without the parallelism. The Module 8 swarm costs about 190 candidates per seed. If Module 10's histories are too long, add the stretch goal that tightens pending writes with the client's next return, and revisit the budget.
- Pending operations (Module 8). DECIDED: an operation the client gave up on is allowed to take effect at any point after its call, or never. That is Lowe's Definition 1 and Knossos's `:info`. A pending read is dropped. Precedence between operations is decided by recording order, not by instants. A client's return and next call share an instant.
- Planted bugs (Module 8). DECIDED: behind a `bugs` cargo feature on the fault-point pattern, planted through `WorldConfig::bugs`. The configuration line shows them only for a non-empty list. The lost update is the write submitted behind another, because skipping the write in front was invisible once both completed. Module 9 added `Bug::StaleReadAfterStepDown`, a primary that steps down and keeps answering reads, and the VOPR catches it under forced partitions.
- Workload in the configuration (Module 8). DECIDED: the workload is not a fault family and `WorldConfig::swarm` does not draw it. `vopr::RunConfig::from_seed` draws the swarm, then the workload, then the workload's dice, from one `Prng`. The configuration line prints all three.
- Where solutions live. DECIDED: a separate directory per module. The repository became a jj and git repository with one commit per module. The directories stayed, because a learner without git can still use them.
- Panic strategy (Module 3). DECIDED: `panic = "abort"` in the dev and release profiles. Module 11 kept it. The hunt and the shrinker run child processes, and the tests use `catch_unwind` under the test profile, which unwinds. Closed.
- Heartbeat placement (Module 5). DECIDED: Module 9's `kv_core::replica::Replica` owns the `Heartbeat` and consumes its outputs. Beats carry the view and the log's runs. The `Network` trait carries peer messages through `send_peer` and `poll`. The real `TcpNetwork` does not dial peers: `kv serve` is a cluster of one, `send_peer` prints and drops, and dialling peers is a Module 9 stretch goal.
- Commit rule (Module 9). DECIDED: every replica, not a majority. A write is answered once every disk has it. Safety follows without an election, and the cluster blocks while any replica is down or unreachable. Module 10 replaced it with `f + 1` of `2f + 1`, counting only peers that report themselves backups of the view. It paid with the view change, the superblock and the fault atlas.
- Log repair (Module 9). DECIDED: a replica with a hole keeps the prefix and salvages every valid record beyond the hole. It fetches the missing record from any peer, and erases from the hole on only once every peer says it has no copy. The first version erased at once and seed 36 lost fourteen committed writes. A cluster of one still halts, as Module 6 decided. The superblock is still not needed, because the erasure zeros everything ever written beyond the log.
- Record identity (Module 9). DECIDED: records carry the view they were prepared in, in the header field Module 6 reserved. Beats carry the log as runs of `(view, last seq)`. Two logs are compared at their first disagreement, higher view wins, a record beats no record. The gathering primary adopts the newest log and truncates backups that disagree. Seed 402 forced this. Module 10's log matching builds on it.
- Boot ids (Module 9). DECIDED: the environment hands every replica a boot id at start, the host's incarnation in the fake world and the wall clock in the real one. Beats and acknowledgements carry it with the heartbeat's sequence number, and a replica applies only beats newer than the last applied. Seed 4668 forced this. A restarted node waits for a beat before taking a role. If the role is primary, it moves to `view + n`.
- Network queue (Module 5). DECIDED: the network holds no queue. `Net::send` returns delivery instants and the world schedules them. Partitions are checked at send and at delivery. Module 7 did not need `turmoil`-style `hold`/`release`. A pause is a property of the host, not of the link, and the host already has queues. Closed.
- Tail ambiguity and the superblock (Module 6). DECIDED in Module 10: four one-sector superblock copies ahead of the log hold the view promise, the last normal view and the log's length at that claim. A restarted replica still cannot tell a record it never had from one its disk lost. So an offer carries `whole`, and a view starts with `f + 1` whole logs or every log. The fault atlas keeps any one record intact on at least one disk. The chaos oracle's "fate unknown" stays for a single disk.
- Crash tearing (Module 6). DECIDED: in-flight writes land in order at a crash, and everything after the first torn write is lost. That matches the real environment's one `fsync` per append. It is what makes "a valid record after a bad one" mean "an acknowledged write is gone". TigerBeetle tears each in-flight write independently and needs its redundant headers to cope. If Module 10's journal gets a header ring, revisit.
- Client ids as sessions (Module 6). DECIDED: records carry the client id and request id, recovery rebuilds sessions, and a client id therefore names a durable session. A new client process takes a new id. Sessions never expire. If Module 8's workloads create many clients, add session expiry.
- Resume order (Module 7). DECIDED: on resume the world pumps the node, ticks the heartbeat at local time, then feeds the held beats. The heartbeat's invariant "next beat is in the future" forces the tick first. The paused node therefore suspects every peer for one instant on waking. A stretch goal flips the order and asks what the invariant needs.
- Clock faults on the monotonic clock (Module 7). DECIDED: `HostClock` skews the host's only clock, which `Instant` models as monotonic. Offset, drift up to 500 000 ppm, forward jumps only. TigerBeetle skews only the wall clock. Module 10 has no lease: reads wait for a majority's beats, and the planted lease is caught under this drift. No second clock was needed.
- Where `Chance` lives (Module 7). DECIDED: `determinism::Chance`, re-exported by `sim`. The core's fault points need it and cannot depend on `sim`. `roll` spends no randomness on `NEVER` or `ALWAYS`, which shifted every fingerprint once.
- Delay fault points (Module 7). DECIDED: none in the core. A sans-I/O state machine has no delay to inject. The disk and network latency ranges and the slow host are the delays. If Module 10's protocol wants a "slow step" inside the replica, add an `Output::Yield` and revisit.
- HashMap policy (Module 3). DECIDED: `clippy.toml` bans the `std` types outright, because the lint cannot see the hasher of a defaulted `HashMap<K, V>`. `BTreeMap` is the default. If ordered maps are too slow, a `hashbrown` + `FxBuildHasher` wrapper is the escape hatch.

## Notes from writing Part I

- The acceptance tests for each task are copied into `modules/NN-slug/tests/`, so that a learner never has to open `solutions/`. Keep the two copies in sync.
- The Module 2 `known_seed_fingerprint` test pins a trace hash. Any change to a trace line in the node, the client, or the fake environment changes it. When a later module touches those, update the constant on purpose and say why in the commit.
- A plain-English hook lints the lessons (short sentences, no modal verbs, no semicolons). Write new lessons to pass it from the start.
- The readings were checked against their sources on 2026-10-10. Two syllabus claims did not survive. The matklad "Deterministic Walk" is a P99 CONF 2023 talk, not a blog post. TigerBeetle no longer has a standalone VOPR docs page. Both are fixed in `CURRICULUM.md`.
- Module 4 changed the fingerprint on purpose. Every trace line now carries the scheduler's timestamp, and the workload draws jitter from the seed. The acceptance tests for Module 4 live in two sub-directories, `tests/sim/` and `tests/kv-env/`, because they belong to two crates.
- The one-year test does not assert on wall-clock time. A CI machine under load makes that flaky. It prints the speed and the lesson states the expected range.
- The line "thousands of cluster-years per day on commodity hardware" in Module 0 was too strong. TigerBeetle's own figure is about 700-1000x real time per core, which is about two simulated years per core-day. The thousands come from a fleet. `CURRICULUM.md` now says so.

## Notes from writing Module 5

- The `sim` crate now depends on `determinism` with `default-features = false`, because the network model needs `Prng`. The Module 4 lesson's "sim and kv-core, nothing else" was true for Module 4 and the Module 5 lesson records the change. The bare-metal check still passes.
- The syllabus reading "FoundationDB simulation network documentation" did not survive. The testing page has one sentence on the network. `fdbrpc/sim2.cpp` (`Sim2Conn`, `SimClogging`, `halfLatency`) is the real source, and it is a connection-level model with no reordering. `CURRICULUM.md` now points there and adds Kingsbury and Bailis, "The network is reliable". TigerBeetle's network wrapper is `src/testing/cluster/network.zig`, not in `vopr.zig`.
- The world is multi-host from Module 5 on. The Module 4 tests were ported: `w.driver` is `w.hosts[0].driver`, `add_client` takes a target node, `partitioned` is `net.cut`, and `drop_replies` is `net.drop_next`. Both fingerprints changed on purpose. The trace lines carry host ids and message ids, and the workload runs under `NetConfig::random`.
- `run_until_idle` no longer means "queue empty". Heartbeats never stop, so idle means no client traffic in flight and no append pending. A test that asserts on `scheduler.is_empty()` only works in a one-host world.
- The heartbeat is a separate state machine per host. Folding it into `Node` means extending the `Network` trait with peer sends, which also means teaching the real `TcpNetwork` to dial peers. When the node becomes a replica in Module 9, that changes. Recorded as an open decision above.
- The acceptance tests for Module 5 live in three sub-directories, `tests/sim/`, `tests/kv-core/` and `tests/kv-env/`. `scripts/check.sh` did not change.
- A first version of the false-suspicion test used a 100 ms reordering delay and saw no stragglers at all. A beat delayed by 100 ms cannot overtake one sent 100 ms later. The test now uses 200 ms, and the lesson shows the arithmetic. This is the kind of thing the model's counters are for.

## Notes from writing Module 6

- Two bugs in the store surfaced the moment a node restarted under a crash schedule. Sessions lived in memory, so a retry across a restart was applied twice. The fix puts the client id and request id in every log record. That made client ids durable sessions, which broke every test that rebuilt a world over an old disk with client 1 again. Both are in the lesson as found bugs.
- The syllabus reading "`StorageFaultAtlas` and simulated storage" needed a path. `ClusterFaultAtlas` is defined in `src/testing/storage.zig`, and `src/testing/cluster.zig` re-exports it as `StorageFaultAtlas`. `CURRICULUM.md` now says so and adds Dan Luu, Bairavasundaram, Rebello and FoundationDB's `AsyncFileNonDurable`. The FoundationDB file moved to `fdbrpc/include/fdbrpc/AsyncFileNonDurable.h`. The Bairavasundaram paper's modern USENIX page is a 404. The legacy HTML page works.
- The chaos oracle went through three versions. The first compared node state to a model and failed on writes the client gave up on. The second allowed one unknown write and failed on seed 14. There, a transient read fault at the tail made an acknowledged write vanish at one restart and come back at the next. The third keeps the client's issued writes with a certainty flag and matches them against `Node::recovered_records`, which was added for it. The seed 14 anomaly is in the lesson as the reason for a superblock.
- The disk fault rates in `DiskConfig::random` are per mille. Per cent rates ended every run in its first second.
- The torn-tail test from Module 2 cut a record at half its length. Half of a one-sector record is padding, and a record that lost only padding is whole. The test now cuts inside the payload.
- All three fingerprints changed on purpose: the log read at start is a disk event, records are whole sectors, and completions carry fates.
- The acceptance tests for Module 6 live in `tests/sim/disk.rs`, `tests/kv-core/log.rs` and four files under `tests/kv-env/`. The log tests moved out of `log.rs` into an integration test so the module can ship them. `scripts/check.sh` did not change.

## Notes from writing Module 7

- The `buggify!` macro is sugar for a method call on purpose. A `#[cfg(feature)]` in a `macro_rules!` body is evaluated in the crate that expands the macro. The gate is on the method, and `Buggify` has two definitions selected by the feature. `check.sh` now builds `kv-core` bare-metal with the feature on as well as off.
- `kv-env` depends on `kv-core` with `features = ["buggify"]`. Feature unification means the `kv` binary in the same crate carries the sites too. It constructs `Buggify::off`, so none are enabled. Both layers are in the lesson.
- The heartbeat's "next beat is in the future" invariant failed the first resume. Held beats fed before the tick arrive at a time past the scheduled beat. Tick first. The side effect, a paused node that suspects everyone on waking, is in the lesson as the view from the inside.
- `Skew.offset` started as a `Duration`. Then `set_drift_ppm` sped up a clock late in a run and needed a negative offset to stay continuous. The offset became an `i64`, and `local` clamps at zero.
- Drift alone, even at the model's 50% limit, never trips the heartbeat's 3.5-interval margin. The lesson has the arithmetic. The jump does trip it, and a jump is a pause from the inside. The acceptance test for drift asserts zero false suspicions at 20%, 50% and 50% against minus 50%.
- The two `timer node=` and `resume` lines carry the host's local time. Node input and output lines carry true time, from a `Clock` the observer owns. The first version used the driver's local time and the trace was unreadable with skew on.
- `Chance::roll` now spends no randomness on `NEVER`/`ALWAYS`. This changed every fingerprint along with the two configuration lines at the head of the trace. Both changes are intended and recorded in the tests.
- The swarm chaos test checks liveness and determinism only. The Module 6 oracle stays in `sim_disk.rs` unchanged. Checking answers under many concurrent clients is Module 8's job.
- The readings: TigerBeetle's simulator is `src/vopr.zig`, not `simulator.zig`, and its simulated clock is `TimeSim` in `src/stdx/time.zig`, which skews only the wall clock. `docs/internals/vopr.md` exists again and is an optional reading. FoundationDB's macro lives in `flow/include/flow/Buggify.h` and the per-site probabilities are `P_GENERAL_BUGGIFIED_SECTION_ACTIVATED` and `_FIRES`, both 0.25. `CURRICULUM.md` now says so.
- The acceptance tests for Module 7 live in `tests/sim/clock.rs`, `tests/kv-core/buggify.rs` and five files under `tests/kv-env/`. `scripts/check.sh` changed and ships with the module.

## Notes from writing Module 8

- The first `History` decided precedence by instants. One client with no think time then had a non-sequential history. Its return and its next call share an instant. The checker was also free to swap a client's write with its next read. Every call and return now carries a sequence number in recording order. Hand-built histories order calls before returns at equal instants, as Porcupine's `byTime` does.
- The first lost update skipped the write in front of a later pending write to the key. Once both completed, the later write covered it, and neither the history checker nor the replay saw anything. The bug is now the write behind, which is logged and never applied. The history checker still accepts it for two concurrent puts, because either order is legal. It catches it only through a delete that reports `existed: true`. The replay catches it always. Both facts are in the lesson.
- `WorldStats` gained a `thinks` field and three fingerprints changed, because three tests print `w.stats` into their traces. The field was removed. The think count is a trace line and a `WorkloadStats` field. All four Module 7 fingerprints are unchanged in Module 8.
- The lost-update hunt found one seed in twelve under a random network and eleven under a reliable one. A random network spreads the clients out and they stop colliding. The hunt uses a reliable network for that bug and says why.
- The disk completes writes in submission order, and a reliable disk with constant latency completes writes submitted at one instant at one instant. Several first attempts at a lost update relied on a window between two completions and never saw one.
- The readings: Jepsen's consistency overview page does not define linearizability. The one-page `models/linearizable` entry does. Porcupine has no handling for an operation without a return. Its `checker.go` assumes a return for every call. TigerBeetle's state checker is `src/testing/cluster/state_checker.zig`, not `src/testing/state_checker.zig`. The workload and auditor are under `src/state_machine/`. FoundationDB's `Cycle.actor.cpp` path moved and the lesson describes the workload without a link. `CURRICULUM.md` now says so.
- The acceptance tests for Module 8 live in `tests/checker/linearize.rs`, `tests/kv-core/bugs.rs` and `tests/kv-env/workload.rs`. The five Module 7 tests in `kv-env` do not change. `scripts/check.sh` gained a bare-metal check of `checker` and the `bugs` feature on the `kv-core` check.

## Notes from writing Module 9

- The replica wraps the node and gates its `AppendComplete` on every replica's acknowledgement. The node's constructor still issues a `Read`, which the replica answers from memory with the log it already read and repaired. The node gained one method, `set_view`.
- `env::Driver` drives a `Replica`. It feeds `Input::Tick` at the start of any step whose deadline passed, so neither environment needs a timer of its own. The fake world's `NodeTimer` event only pumps the driver, and the resume order of Module 7, tick before held messages, falls out of that.
- The sweep found three protocol bugs and one checker bug, all kept as regression seeds in `crates/vopr/tests/vopr.rs`. Seed 36: erasing beyond a hole threw away the only in-memory copies of committed records, three restarts in a row. Seed 4668: a straggling beat with an old log length made a restarted primary gather too early and prepare a second record 25. Seed 402: a stray copy of a lost record, salvaged from beyond a hole, was written back over the record that replaced it. Seed 12784: the white-box check compared the map with the disk, and a misdirected write had made a hole the primary had not restarted to find. The lesson tells the first three.
- The fault model needs every piece. Lost writes make latent holes, misdirected writes make strays, reordering makes stragglers, and crashes make restarts find them. Turning any one family off hides a class of bug.
- Twenty thousand seeds of 150 requests ran clean in release at about a hundred seeds per second. `scripts/check.sh` runs two hundred. The swarm test over forty seeds counts promotions, pulls, repairs and step-downs, and every seed failed over at least once.
- All five fingerprints of Modules 4 to 8 changed. The node is a replica, and its ticks, peer messages and status changes are in the trace. The Module 6 storage chaos runs on one host now, because its oracle is a single-disk oracle. `World::history_of` is gone, because the three hosts are one store, and the workload chaos checks one history.
- The readings: TigerBeetle's simulator is `src/vopr.zig`, and `docs/internals/vopr.md` is a short overview without command lines. The HACKING document the syllabus relied on is gone from the main repository. The 2023 liveness post and the 2026 protocol-aware post are the blog readings. The primary-backup paper is Budhiraja, Marzullo, Schneider and Toueg, "Optimal Primary-Backup Protocols", on a course mirror at Wisconsin. Raft section 5.3 is the reading for log matching. `CURRICULUM.md` now says so.
- The acceptance tests for Module 9 live in `tests/kv-env/replication.rs` and `tests/vopr/vopr.rs`, plus the five ported tests of Modules 4 to 8 in `tests/kv-env/`. `scripts/check.sh` gained the sweep.

## Notes from writing Module 10

- The protocol was written against the simulator and the sweep found fifteen bugs, told in the lesson's journal. Ten of them are kept as regression seeds. Eleven were the protocol's, two were the guard's (seeds 13 and 19), and two were the model's (seeds 619 and 353). Two of the protocol's were found by hand: a restarted primary's first beat, sent before its disk was read, emptied the backups' logs, and the restart rule fired on a primary's first beat.
- The superblock the plan deferred from Module 6 became necessary at seed 562: a replica voted for a view, crashed, forgot, and rejoined the old view, so two views had majorities. Four copies, written in turn, no quorum read. The log begins at sector four, and every test that read the medium directly now reads past the superblock through `World::log_on_disk`.
- The fault atlas the plan foresaw became necessary at seed 619: two disks lost the same committed record and the cluster erased it. `sim::Atlas` is asked after every roll and vetoes a fault that puts the same sector at fault on more than `f` disks. One or two disks get no atlas, so Modules 6 to 8 keep their tests. The dice roll either way, so a seed means the same thing.
- Seed 353 was a livelock of 1271 views in 950 seconds. A `Missing` counted only from a whole peer, and nobody was able to become whole without a primary. Under the atlas a `Missing` from any peer that read its disk is the truth about that disk.
- The planted lease was not caught in a hundred seeds of a hundred and twenty requests. The chaos phase was half a second long, one partition. The hunt runs six hundred requests per seed and catches it. The lesson says so, because a hunt that finds nothing is the usual first result.
- The seed draws five nodes one time in four and a core one time in two, after every other draw. The regression seeds are run under three nodes and no core, because they were found before those draws existed. Twenty thousand seeds take three minutes and forty-six seconds in release, about ninety a second.
- Every fingerprint of Modules 4 to 9 changed: the record header grew by four bytes, the superblock moved the log, and the view change put messages on the first beat. Module 5's cut-link test now measures after view 0 started, because the first round carries `StartViewChange` as well as beats.
- The readings: the VSR Revisited PDF at MIT's PMG site refused connections and DSpace returns 405 to a fetch, so the lesson links the Princeton course mirror. The protocol-aware recovery paper is on the USENIX FAST 18 page with a PDF. TigerBeetle's `ClusterFaultAtlas` is defined near the end of `src/testing/storage.zig`. Its `superblock.zig` keeps `view` and `log_view` with four, six or eight copies read as a quorum. `CURRICULUM.md` now says so.
- The acceptance tests for Module 10 live in `tests/kv-env/consensus.rs` and `tests/vopr/vopr.rs`. The ported tests are `tests/kv-core/log.rs` and the six of Modules 4 to 9 in `tests/kv-env/`. `scripts/check.sh` did not change.

## Notes from writing Module 11

- The bug was planted three times before it was caught. The vote kept in memory with the superblock written at the next claim never failed in twenty thousand seeds, because `note_caught_up` writes `log_view` and the restart reads it back. The superblock written with `view = log_view` never failed either, for the same reason. The restart that ignores the superblock's views, which is the state before seed 562, failed on two seeds in twenty thousand under three nodes and no core: 9916 and 16345. The hunt is what found them, and that is the lesson's point about hunts.
- The signature started as the whole first line with numbers blanked. The stale read's message carries the history, so every candidate looked like another bug and the shrink stopped at eighty-five requests. Cut at the first bracket, it reaches thirty. The lesson tells both.
- The first shrink loop searched the requests in every pass and retried every knob, so a shrink ended with a pass that did nothing but repeat. A pass now returns whether a knob was kept, and only then is another pass due.
- Seed 9916 shrinks to fifty-seven requests and keeps every family. Turning any family off reshuffles the later families' dice, so the shrinker cannot tell a needed family from needed dice. Per-family streams are the stretch goal, and the lesson says why.
- The inspector reads only the trace. The first version of its test looked for the replica with the higher `log_view` as the one that forgot, and it was the other one: the primary of view 19 crashed and came back in view 18. The test now finds the primary of the view and checks that its first status after the restart is lower.
- A line's node: `timer node=1` is about node 1 but does not contain `n1`. The binary's test checks for either.
- The hunt runs about seventy seeds a second in release as child processes, against ninety in one process. Twenty thousand seeds take about five minutes.
- The readings: the Saarland host of the Zeller paper refused connections, so the lesson links the Purdue course mirror. The Hypothesis paper's real title is "Test-Case Reduction via Test-Case Generation: Insights from the Hypothesis Reducer", and the Imperial PDF is open. TigerBeetle's `vopr.zig` has `--lite`, `--packet-loss-ratio`, `--requests-max` and `--replica-missing`, and no shrinker. The blog post's URL has no trailing slash. `CURRICULUM.md` now says so.
- The acceptance tests for Module 11 live in `tests/vopr/shrink.rs` and `tests/vopr/inspect.rs`. No earlier test changed, and no fingerprint changed. `scripts/check.sh` gained the shrink of seed 9916.

## Notes from writing Module 12

- The syllabus reading "TigerBeetle's CI and `vopr` fuzzing setup (`.github/workflows`, DevHub)" did not survive. The repository's `.github/workflows` holds only the release workflows now, `src/scripts/cfo.zig` was removed on 2026-08-27 ("cfo: remove from repo"), and devhub.tigerbeetle.com sits behind a Google login. The lesson links the orchestrator at the last commit before the removal, `e544c610`, and the two blog posts that survive. FoundationDB's testing page has no section on the test runner, so the Joshua and TestHarness2 READMEs are the readings. `CURRICULUM.md` now says so.
- The first check-step ceiling was 200 000, and four seeds of twenty thousand went over it. The maximum is 358 481 at seed 4888, and the median is 216. The distribution is in the lesson, and both ceilings are about twice the maximum. A ceiling set from the median catches nothing but the tail.
- `std::time::Instant` is a disallowed type as well as a disallowed method. The batch wraps it in `batch::Stopwatch` with two `#[expect]` attributes, the way `RealTime` does. That is the one wall clock in the binary.
- The pool spawns children and waits on the oldest, because threads are banned in the workspace. Two hundred seeds take 2.2 s in one process and 2.8 s as children one at a time. On eight cores they take 1.3 s at four jobs and 1.0 s at eight. Twenty thousand seeds take 96 s at eight jobs, about 208 a second.
- The batch folds the passing hashes the way `--seeds` does, so `vopr --seeds 0..200` and `vopr --batch 200 --from 0` print one number, and `scripts/check.sh` relies on that.
- The first `--commit` flag both recorded the commit and named the seeds, and it was exclusive with `--from`. The nightly's records therefore had no commit in them. Now `--from` wins for the seeds and the commit is recorded either way.
- The workflows parse as YAML, and every command in them was run locally. They have not run on GitHub, because the course repository is not a GitHub repository. The lesson says to push to a fork.
- The acceptance test for Module 12 is `tests/vopr/batch.rs`. `scripts/check.sh` gained `--locked`, a debug-against-release hash comparison, and a batch of the first two hundred seeds. The workflows ship in `modules/12-ci/workflows/`. No fingerprint changed, because the `cost:` line is in the report and not in the trace.

## Notes from writing Module 13

- The port is a new crate, `kv-turmoil`, and `kv-core` did not change. Three trait implementations and a wire format for `replica::Message`. A host loop, a client loop, a fault schedule applied between `sim.step()` calls, and `run`. About 1 400 lines with the tests. turmoil 0.7.2 and tokio 1.53.2 entered the lockfile.
- turmoil seeds tokio's own random number generator only under `--cfg tokio_unstable`, so a `select!` in host code is nondeterministic without the flag. The host loop uses `timeout` instead. Seed 2 agrees with itself in one process and across two.
- The first `SharedStorage` copied `FileStorage` and cut the file after every write. The superblock is at offset zero, so every promise erased the log, and seeds 34 and 42 lost records to a bounce. `FileStorage` had the same bug since Module 10 and nothing ran it: the fake disk writes in place and the real server is a cluster of one. `real.rs` lost the `set_len`, and `real_server.rs` gained a test. The lesson calls this the first finding.
- Seed 3 of the port shows five view changes with no fault: a first beat delayed past the `StartView` reads as a restart. The VOPR does the same on nine of the first forty seeds with every family off but the network, and on six with reordering off. No oracle names it. Left as it is, because the fix moves every fingerprint since Module 9. The oracle and the fix are stretch goals.
- Numbers, one core, release: the VOPR runs one hundred seeds of three hundred requests in 2.6 s, the port in 37.4 s. About 270 against 73 virtual seconds per wall second. The tick is the difference. In debug a port seed takes 2.4 s to 6 s, so the acceptance tests run eight seeds in all.
- No fingerprint test for the port. turmoil's latency draws go through `rand_distr::Exp`, which uses `ln`, and the sweep hash across two operating systems is untested. The VOPR fingerprint `0xcc45_30a4_8c93_496b` is unchanged.
- `scripts/check.sh` gained twenty seeds of the port in release, about eight seconds. The three workflows did not change.
- The syllabus title changed, because the lesson lint bans the old title's second word. The directory keeps its name.

## Definition of done

- All fourteen lessons follow the template, and every link works.
- A new learner who follows the modules in order can complete every task. The reference solutions pass every acceptance criterion.
- The final reference solution runs tens of thousands of seeds clean on the consensus implementation. The bug journal from Module 10 is published as part of the lesson.
- `CURRICULUM.md` matches what the modules teach.

## Status

| Module | Lesson | Task spec | Reference solution | Notes |
|---|---|---|---|---|
| 00 Why DST | reviewed | reviewed | n/a | Reference program lives in the lesson and `modules/00-why-dst/starter/`. Measured 68% failure rate at the defaults on an M1, and 52% to 80% across batches. |
| 01 Determinism | reviewed | reviewed | done | `solutions/01-determinism`. Skeleton with stubs, tests and `scripts/check.sh` in `project/`. |
| 02 Architecture | reviewed | reviewed | done | `solutions/02-architecture`. Decided: hand-rolled event loop, own `Instant` type, generics over `dyn`. `scripts/check.sh` scripts the restart test on a free port. |
| 03 Rust hazards | reviewed | reviewed | done | `solutions/03-rust-hazards`. Decided: ban `HashMap` type, `panic = "abort"`, `#[expect]` markers. `scripts/check.sh` is the full suite. |
| 04 Virtual time | reviewed | reviewed | done | `solutions/04-virtual-time`. New crate `sim` (scheduler, clock, timer). Decided: event queue, not ticks. Fingerprint changed to `0xd982_fd72_6e9e_453a`. Two simulated years in 0.4 s release, 2 s debug. |
| 05 Sim network | reviewed | reviewed | done | `solutions/05-sim-network`. `sim::net` (fate decisions, ledger, cut links, shapes) and `kv_core::heartbeat`. World has N hosts and one address space. Decided: no queue in the network, cuts checked at send and delivery, reordering is a delay. Fingerprints `0x2fcf_d1bb_e1bb_c8ef` and `0x5dae_82b3_b044_4863`. |
| 06 Sim disk | reviewed | reviewed | done | `solutions/06-sim-disk`. New `sim::disk` with five faults and a crash. New `kv_core::log` with whole sectors, CRC-32C and sequence numbers. `Output::Halt` on damage. A host is a disk plus an optional process, with a crash schedule. Decided: in-order landing at a crash, no superblock yet, client ids are durable sessions. Fingerprints `0x0965_0d64_15e6_a7b3`, `0x0bee_545c_7d3c_dcd4` and `0x3dec_d7ad_dae6_8e98`. |
| 07 Fault injection | reviewed | reviewed | done | `solutions/07-fault-injection`. `sim::HostClock` (skew, drift, jumps), host pauses, `kv_core::buggify` with four sites and a feature flag, `WorldConfig::swarm` over eight families, `stop_faults`. `Chance` moved to `determinism`. Config heads every trace. Decided: tick before held beats on resume, skew the monotonic clock, no delay sites in the core. Fingerprints `0x2f50_c6ad_b6ea_c983`, `0x8562_404d_a09c_be58`, `0xea7a_6198_ba24_4e5d`, `0x7ded_4ea7_b0c9_6c9e`. |
| 08 Workloads & checking | reviewed | reviewed | done | `solutions/08-workloads-checking`. New `no_std` crate `checker`: a history with recording-order precedence, `Model` and `check_sequential`, and a seed-driven `Workload`. Its search is Wing-Gong with Lowe's cache and a budget, `kv::partition` by key, and `kv::replay`. `kv_core::bugs` behind a `bugs` feature with three planted bugs. `World` records a `History`, drives `run_workload` with think time, plants bugs. Decided: precedence by sequence not instant, pending ops are allowed to happen or not, `WorldStats` unchanged so the four Module 7 fingerprints hold. New fingerprint `0x7cda_7998_33d5_689c`. |
| 09 VOPR | reviewed | reviewed | done | `solutions/09-vopr`. `kv_core::replica` wraps the node and the heartbeat: views, primary `v mod n`, forwarding, all-replica commit, the gather, repair by salvage and fetch, records stamped with their view, runs in beats, truncation, boot ids. `env::Driver` drives a replica. The world speaks peer messages and has `converged`, `check_logs`, `run_until_converged`. New crate `vopr`: `RunConfig::from_seed`, `run`, the binary with `--seed`, `--seeds`, `--trace`. Four regression seeds. Every fingerprint changed. New fingerprint `0xc85d_a241_4993_66e8`. |
| 10 Consensus | reviewed | reviewed | done | `solutions/10-consensus`. Viewstamped Replication: majority commit, the three view-change messages, `log_view` and `whole`, a hash-chained log. Also the superblock with four copies, reads that wait for a majority's beats, the fault atlas, and TigerBeetle's core. `Bug::LeaseRead` planted. Fifteen journal entries, ten regression seeds. Every fingerprint changed. New fingerprint `0xcc45_30a4_8c93_496b`. Twenty thousand seeds clean in 3 m 46 s. |
| 11 Debugging & shrinking | reviewed | reviewed | done | `solutions/11-debugging-shrinking`. `Bug::ForgottenPromise` as the sixth planted bug. `vopr::overrides` with twenty knobs as flags, `vopr::shrink` with the greedy loop, signatures and two oracles, `vopr::inspect` with snapshots and slices. The binary gained `--hunt`, `--shrink` and `--inspect`. Decided: children, not unwinding. Seed 9916 shrinks from 150 to 57 requests in 26 runs. No fingerprint changed. |
| 12 CI | reviewed | reviewed | done | `solutions/12-ci`. `vopr::batch`: seeds from the commit hash and a `Children` pool. Every failure is rerun twice with its trace. `Record` and `BatchReport` are files. A `Ceiling` in events per request and checker steps, 1 000 and 800 000. The binary gained `--batch`. Three workflows. `--locked` in `check.sh`. Decided: nothing is retried into a pass, wall-clock time is reported only, `Stopwatch` is the one wall clock. No fingerprint changed. Twenty thousand seeds clean in 96 s at eight jobs. |
| 13 Other simulators | reviewed | reviewed | done | `solutions/13-landscape`. New crate `kv-turmoil`: `TokioTime`, `SharedStorage`, `UdpNetwork`, `wire::Datagram`, `replica_host`, `client_host`, `Schedule`, `run`, and the binary. `FileStorage` no longer cuts the file. `RETROSPECTIVE.md` is the reference retrospective. Decided: `timeout` not `select!`, faults from outside, no fingerprint for the port. VOPR fingerprint unchanged. |

Status values: `not started`, `draft`, `reviewed`, `done`.
