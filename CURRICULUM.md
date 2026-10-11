# Deterministic Simulation Testing: A Learning Curriculum

This is a self-paced course on Deterministic Simulation Testing (DST).
TigerBeetle (the VOPR), FoundationDB, Antithesis, and others practice DST. The
course has one project that grows with each module. By the end, you will write
a small replicated key-value store and the simulator that tortures it. This is
the same spirit as TigerBeetle's VOPR.

All coding is in Rust. In some modules, TigerBeetle's Zig source is the best
reference for an idea. The module says so, but you do not need to write any
Zig.

This document is the high-level plan. Each module will later get its own
directory with detailed notes, readings, and a fully specified coding task.

---

## Who this is for

You can write non-trivial Rust. You know some concurrency and networking.
Flaky tests or unreproducible production bugs burned you in the past. You do
not need prior distributed-systems or consensus experience. The course
introduces what it needs.

## What you will be able to do at the end

1. Explain precisely what makes a system *deterministic* and *simulable*.
   Identify every source of nondeterminism in a codebase.
2. Know exactly where Rust and its standard library smuggle nondeterminism
   into a program. Know how to make those paths unreachable from core logic.
3. Architect a system so that time, randomness, network, and storage sit
   behind interfaces. You can swap each interface for a simulated version.
4. Build a discrete-event simulator with virtual time, a simulated network, and
   a simulated disk. Each has a realistic fault model.
5. Design fault injection (FoundationDB-style "buggify", swarm testing) and
   workload generation with model-based and invariant-based checking.
6. Assemble these into a VOPR-style simulator that runs a cluster for
   simulated years in seconds. Use it to find, reproduce, shrink, and fix
   real bugs in a consensus protocol.
7. Run the simulator continuously in CI. Know the limits of what DST can
   and cannot find.

## The running project

Every module's coding task extends the same Cargo workspace:

| Stage | What exists after this stage |
|---|---|
| Modules 1-3 | A single-node KV store written as a pure state machine with pluggable I/O. It lives in a core crate that *cannot* reach the OS, and lints enforce this |
| Modules 4-6 | A simulator with virtual time, simulated network, and simulated disk |
| Modules 7-8 | Fault injection and a workload generator + checker |
| Module 9 | A VOPR for a primary-backup replicated KV store |
| Module 10 | A consensus protocol (Raft or Viewstamped Replication) under the VOPR |
| Modules 11-12 | Shrinking, trace tooling, and continuous seed runs in CI |
| Module 13 | A port onto an existing Rust DST framework and a retrospective |

### Why Rust, and what that implies

Rust has the richest DST ecosystem to learn from and compare against
(`madsim`, `turmoil`, `shuttle`, `loom`). Its type system and module privacy
make it possible to *enforce* the "no hidden I/O" discipline rather than only
ask for it. The flip side is that Rust's standard library and the async
ecosystem do not have determinism as a design goal. Several common idioms
quietly break it. Module 3 is devoted to this.

---

## Course structure

The course has fourteen modules in five parts. Each module has learning
objectives and key readings, and ends with a coding task. The estimated effort
assumes roughly 5-8 focused hours per week.

```
Part I   Foundations            Modules 0-3    ~4 weeks
Part II  Building the simulator Modules 4-6    ~4 weeks
Part III Finding bugs           Modules 7-9    ~4 weeks
Part IV  The real thing         Module 10      ~3 weeks
Part V   Operating DST          Modules 11-13  ~3 weeks
```

---

## Part I: Foundations

### Module 0: Why deterministic simulation?

#### Objectives

- Understand why distributed systems are hard to test: combinatorial state
  space, rare timing windows, unreproducible failures.
- Compare the testing approaches: unit tests, integration tests, Jepsen-style
  black-box chaos testing, formal methods (TLA+), and DST.
- Learn the three pillars of DST. *Determinism* means the same seed gives the
  same run. *Simulation* means the whole cluster runs in one process with a
  fake environment. *Fault injection* means the environment is adversarial.
- Learn the "simulated time is free" argument. Simulation runs at roughly
  1000x real time per core. That is about two simulated years per core-day,
  and thousands of cluster-years per day from a modest fleet.

#### Key readings / viewing

- Will Wilson, "Testing Distributed Systems w/ Deterministic Simulation"
  (Strange Loop 2014). This is the canonical FoundationDB talk.
- Phil Eaton, "What's the big deal about Deterministic Simulation Testing?"
- TigerBeetle docs: the "Safety" page (the VOPR section). The old standalone
  VOPR page redirects here. Play with sim.tigerbeetle.com.
- Antithesis, "Is something bugging you?"
- Joran Dirk Greef talks on TigerBeetle's testing philosophy.

#### Coding task (sketch)

Write a small multi-threaded Rust program with a deliberate race or timing
bug. For example, write a counter with a lost update under a retry-with-timeout
that uses `std::thread` and `std::sync::mpsc`. Run it many times and record
how rarely it fails. Try to reproduce a specific failure. Write up what you
must control to make the failure reproducible on demand. This is the "before"
picture for the whole course.

### Module 1: Determinism fundamentals

#### Objectives

- Catalogue sources of nondeterminism in general. Examples are the wall clock,
  the monotonic clock, OS thread scheduling, I/O completion order, random
  number generators, and hash-map iteration order. Other examples are memory
  addresses, environment variables, uninitialized memory, and floating-point
  behavior across platforms.
- Understand the seed as the single root of all randomness: `seed -> PRNG ->
  every decision`.
- Understand why determinism must hold at the *bit* level for reproduction and
  for comparing traces across runs.
- Learn to test determinism itself: run twice with the same seed, hash the
  trace, compare.

#### Key readings

- TigerBeetle "Tiger Style" (sections on determinism and assertions).
- FoundationDB docs, "Simulation and Testing", on the single-threaded
  deterministic model. Alex Miller's transactional.blog has only the BUGGIFY
  post on simulation. That post is a Module 7 reading.
- The `rand_pcg` or `rand_xoshiro` crate docs. The `rand` book's section on
  reproducibility, and why `StdRng` is *not* a stable algorithm across
  versions.

#### Coding task (sketch)

Create the Cargo workspace. Build a seedable PRNG wrapper with a pinned
algorithm and a trace log. Build a test runner that runs a program under a
seed and emits a hash of its trace. Write a "determinism check" that runs the
same seed twice and fails on any divergence. Then deliberately introduce each
class of nondeterminism from the catalogue. Make sure that the determinism
check catches each one. Keep this check. It runs in CI for the rest of the
course.

### Module 2: Architecting for simulation

#### Objectives

- Dependency inversion of the environment. Everything that touches the outside
  world (clock, random, network, disk, process control) lives behind an
  interface. Each interface has a *real* and a *simulated* implementation.
- The "sans-I/O" / state-machine style. The core logic consumes events and
  produces commands. It never blocks or performs I/O itself.
- Single-threaded, event-driven control flow as the simplest path to
  determinism. Why TigerBeetle avoids threads in the replica.
- Completion-based I/O (io_uring-style) as a natural fit: submit, then handle
  a completion event later.
- Rust-specific design choices. Traits with generic parameters versus
  `dyn Trait` for the environment. Owning the event loop versus adopting an
  async runtime. A context struct instead of globals.
- Assertions as a first-class design tool. The simulator is only as good as
  the invariants the system asserts about itself.

#### Key readings

- Alex Kladov (matklad), "A Deterministic Walk Down TigerBeetle's main()
  Street" (P99 CONF 2023 talk and slides).
- Brett Cannon's "sans-io" pattern documentation (Python, but the idea is
  universal). Firezone's "sans-IO: The secret to effective Rust for network
  services". `quinn-proto` and `str0m` as production examples of the pattern
  in Rust. Polar Signals' "A Theater of State Machines" on choosing this
  architecture for DST.
- TigerBeetle source: `src/io/` and how `Replica` is parameterized over its
  I/O, storage, and time.
- Tiger Style on assertions and static allocation.

#### Coding task (sketch)

Write a single-node KV store (`get`, `put`, `delete`, with a simple
request/response protocol). Write it as a pure state machine parameterized
over `Time`, `Network`, and `Storage` traits. Provide a real implementation
(TCP + files + system clock) and a trivial in-memory fake. The same core logic
must run against both. Add internal assertions for every invariant you can
state.

### Module 3: How Rust and `std` fight determinism

This module is the Rust-specific counterpart to Module 1. The general
catalogue said *what* kinds of nondeterminism exist. This module shows where
each one hides in idiomatic Rust and why it is there. It also shows how to
make each one unreachable from the core crate rather than relying on
discipline.

#### Objectives

##### Hazards in the standard library

- `HashMap` and `HashSet` use `RandomState`, which seeds SipHash from OS
  randomness per process. Iteration order differs run to run. Any logic that
  iterates a map is nondeterministic, for example choosing a peer, draining a
  queue, or formatting a log line that is then hashed. Fixes: `BTreeMap`, or
  `HashMap` with a fixed `BuildHasher` (`BuildHasherDefault<FxHasher>` or a
  fixed-key `ahash`), and a lint that bans the default constructors.
- `std::time::{Instant, SystemTime}` and `std::thread::sleep` are direct
  syscalls. Any `Instant::now()` in core logic is a hidden dependency on the
  real clock. The only fix is an injected clock and a ban on the originals.
- `std::thread`, `Mutex`, `RwLock`, atomics, `mpsc` with multiple producers,
  `Once`/`OnceLock`/`LazyLock` initialization races, and `rayon` all expose
  the OS scheduler's choices. A single-threaded core is the baseline. `loom`
  and `shuttle` are the tools for the few places where concurrency is
  unavoidable.
- `std::fs::read_dir` order is filesystem-dependent. `std::env`, `std::net`
  DNS resolution, and `std::process` all read the outside world. These belong
  behind the I/O trait and nowhere else.
- Memory addresses: `{:p}` formatting, `Arc::as_ptr`, `ptr::hash`, sorting or
  hashing by pointer, and `std::ptr::addr_of!` leaking into data. ASLR makes
  every one of these differ per run.
- Debug versus release behavior. Integer overflow panics in debug and wraps in
  release. `debug_assert!` vanishes in release. A simulator must not have two
  behaviors, so the profile must pin `overflow-checks = true` and
  `debug-assertions = true`. TigerBeetle always runs with assertions on.
- Floating point is deterministic on one platform and toolchain. But
  `f64::sin`, `exp`, and friends come from the platform `libm` and differ
  across OSes. Keep transcendentals out of core, or use a pure-Rust
  deterministic `libm`.
- Unstable sorts (`sort_unstable`, `select_nth_unstable`) are deterministic
  for a given toolchain. After a toolchain upgrade, they can reorder equal
  elements differently. Pin the toolchain with `rust-toolchain.toml` and
  commit `Cargo.lock` so that "same seed, same result" survives across
  machines.
- `unsafe` and undefined behavior can make anything nondeterministic. Keep
  `unsafe` out of the core (`#![forbid(unsafe_code)]`), and run Miri on
  anything that needs it.

##### Hazards in the async ecosystem

- Tokio's default multi-threaded work-stealing runtime is a nondeterministic
  scheduler. `current_thread` helps, but you still do not control task wake
  order.
- `tokio::select!` and `futures::select!` pick a ready branch *at random* by
  design. `biased;` and `select_biased!` restore a fixed order. This single
  macro breaks more "deterministic" tests than anything else.
- `FuturesUnordered`, `JoinSet`, and `tokio::spawn` completion order depend on
  wake order. `tokio::time` depends on the real clock unless the runtime
  starts with `start_paused` and `test-util`.
- Why `madsim` and `turmoil` exist: they replace the runtime, the network, and
  the clock underneath unchanged application code. `madsim` does this by
  swapping the `tokio` crate under `--cfg madsim`. `turmoil` provides a
  simulated `net` module and runs each host on a deterministic executor.
  These approaches explain the trade-off the course makes by owning its own
  event loop.

##### Hazards from running a whole cluster in one process

- Simulated replicas believe they are separate machines, but they run in one
  process. Every `static`, `thread_local!`, `OnceLock`, global metrics
  registry, global `log`/`tracing` subscriber, or singleton is now common to
  all of them. State leaks silently between nodes. The fix is structural: no
  mutable globals, all state hangs off a per-replica struct, and the simulator
  tells the tracing subscriber which simulated node is "current".
- Panics: a replica panic must be the simulator's assertion failure, not an
  unwind that leaves half a cluster in an unknown state. Decide on
  `panic = "abort"` versus catching at the replica boundary.

##### Hazards from dependencies

- Third-party crates call `Instant::now()`, `thread_rng()`, spawn threads, and
  use `HashMap` freely. `uuid::new_v4()`, `fastrand`, `getrandom`, and any
  crate built on `tokio` are all suspects. A dependency the core crate pulls
  in is a dependency on the real world.
- Auditing: `cargo tree` for the core crate, grepping vendored sources for the
  banned calls, `cargo deny` to ban crates outright.

##### Making the hazards unreachable

- Split the workspace into three crates. The core crate is `#![no_std]` +
  `alloc`, so `std::time`, `std::thread`, `std::fs`, `std::net`, and
  `std::env` do not exist. The `env` crate has the real and simulated
  environments. The third is a `sim` crate.
- Where `no_std` is too strict, use clippy's `disallowed_methods` and
  `disallowed_types` in `clippy.toml` with `#![deny(clippy::disallowed_methods,
  clippy::disallowed_types)]`. Use them to ban `Instant::now`,
  `SystemTime::now`, `thread::spawn`, `thread_rng`, `HashMap::new`, and
  friends.
- The Module 1 determinism check in CI is the last line of defense.

#### Key readings

- `std::collections::HashMap` docs on `RandomState`. The `rustc-hash` and
  `ahash` READMEs on fixed hashing.
- `tokio::select!` documentation, specifically the "Fairness" section.
- `madsim` README and the `--cfg madsim` design. `turmoil` README and its
  `net` module docs.
- `loom` and `shuttle` READMEs for the schedule-exploration alternative in
  the cases where threads are unavoidable (neither models a clock).
- Clippy docs for `disallowed_methods` and `disallowed_types`. `cargo deny`
  docs.
- The `rand` book's reproducibility chapter.
- Tyler Neely (sled), the sled simulation guide (sled.rs/simulation.html).
  S2's "Deterministic simulation testing for async Rust" on retrofitting DST
  onto a Tokio codebase.

#### Coding task (sketch)

This is a nondeterminism hunt in three parts. First, write a set of small
programs. Each program uses one hazard from the lists above. Examples:

- A `HashMap` peer selector.
- A `tokio::select!` loop.
- A `thread_rng()` backoff.
- A pointer-keyed sort.
- A `static` counter shared by two "nodes".
- A crate that hides an `Instant::now()`.

Run each through the Module 1 determinism check, and record which ones the
check catches and how. Second, fix each one with the recommended replacement
and show that the check passes. Third, restructure the Module 2 workspace so
the core crate is `no_std` + `alloc`. Add `clippy.toml` bans, a pinned
toolchain, and a pinned profile. Then reintroduce each hazard into the core
crate and make sure that it fails to compile or fails lint.

---

## Part II: Building the simulator

### Module 4: Virtual time and the discrete-event scheduler

#### Objectives

- Discrete-event simulation: a priority queue of events keyed by virtual time.
  The simulator jumps the clock to the next event rather than sleeping.
- Timers, timeouts, and retries under virtual time.
- Tie-breaking rules so that the simulator processes events at the same
  virtual time in a deterministic order (`BinaryHeap` ordering on equal keys
  is a trap here).
- The "tick" model that TigerBeetle uses versus a pure event-queue model, and
  the trade-offs.
- Running simulated decades in seconds, and why this matters for finding
  rare timing bugs.

#### Key readings

- The Wikipedia article on discrete-event simulation, and the SimPy "Basic
  Concepts" page as a complete example in another language.
- TigerBeetle source: `src/vopr.zig` (the tick loop), `src/testing/cluster.zig`
  (`tick()`), and `src/stdx/time.zig` (`TimeSim`).
- FoundationDB `fdbrpc/sim2.cpp` (how it implements `delay()` and the run
  loop) and `flow/TaskQueue.h` (how it breaks ties).
- `tokio::time::pause` and `tokio::time::advance` docs, as an example of
  virtual time retrofitted onto a runtime.

#### Coding task (sketch)

Implement the virtual clock and event scheduler. Port the KV store's timeouts
and retries onto it. Write a test that runs a client with an exponential
backoff retry loop for one simulated year. The test must complete in under a
second in a release build. Its trace must show every timer firing in a deterministic order.

### Module 5: The simulated network

#### Objectives

- Model a network as a scheduler of message delivery events between in-process
  nodes.
- Fault model: latency (with distributions), reordering, duplication, loss,
  symmetric and asymmetric partitions, and partial partitions.
- Why "the network is just another priority queue" and how to keep it
  deterministic.
- Realism versus reproducibility: enough fidelity to find bugs, no more.

#### Key readings

- Kyle Kingsbury and Peter Bailis, "The network is reliable", for the
  catalogue of real partitions.
- TigerBeetle source: `src/testing/packet_simulator.zig` (`PacketSimulator`
  and its options) and `src/testing/cluster/network.zig` (how replicas and
  clients get addresses).
- FoundationDB `fdbrpc/sim2.cpp`: `Sim2Conn` and `SimClogging`, a
  connection-level model for contrast. The FoundationDB testing docs have
  only one paragraph on the network.
- `turmoil`'s network model and its `partition`/`hold`/`repair` API, as a
  Rust reference design.
- Jepsen's `nemesis.clj`, for the catalogue of partition shapes worth
  simulating, and the `tc-netem` manual for how a kernel reorders packets.

#### Coding task (sketch)

Implement the simulated network with every fault from the model above. The
seed configures the faults, including random partitions over time. Run three
KV nodes that pass heartbeats and keep a liveness view of each other. Make
sure with assertions that:

- The network never delivers a message that no node sent.
- Partitioned nodes never exchange messages.
- With faults disabled, delivery is in-order and lossless.

### Module 6: The simulated disk

#### Objectives

- The storage fault model: torn writes, misdirected reads and writes, latent
  sector errors, bit rot, lost writes from an `fsync` that lies, and crash
  before or after durability.
- Crash-and-restart: simulate process death and recovery from the persisted
  state only.
- Checksums, write-ahead logging, and why "storage faults are different from
  network faults" (they persist, and corruption can hit a majority).
- How TigerBeetle's storage fault model goes beyond FoundationDB's.

#### Key readings

- Alagappan et al., "Protocol-Aware Recovery for Consensus-Based Storage"
  (FAST '18). This paper drives TigerBeetle's storage fault model.
- Pillai et al., "All File Systems Are Not Created Equal" (OSDI '14) on
  crash-consistency bugs. Dan Luu's "Files are hard" is the short tour.
- TigerBeetle source: `src/testing/storage.zig` (the simulated storage and
  `ClusterFaultAtlas`, which `cluster.zig` imports as `StorageFaultAtlas`)
  and the recovery decision table in `src/vsr/journal.zig`.
- Optional: Bairavasundaram et al. (FAST '08) for the corruption rates,
  Rebello et al. (ATC '20) for failed `fsync`, and FoundationDB's
  `AsyncFileNonDurable` for a crash-only storage model.

#### Coding task (sketch)

Implement an in-memory simulated disk with sector-level faults driven by the
seed. Give the KV store a sector-aligned, checksummed write-ahead log. Give
it a recovery procedure that tells a torn tail from a hole and halts on the
hole. Simulate crash/restart at random points. Assert that acknowledged writes
survive on a disk that cannot lose them, and count the ones a lying disk
loses. Assert that the store always detects corruption and never silently
returns it.

---

## Part III: Finding bugs

### Module 7: Fault injection, buggify, and swarm testing

#### Objectives

- Process-level faults: pause (long GC / VM stall), clock skew, drift and
  forward jumps, and a slow disk on one host. Crash and restart came with
  Module 6.
- FoundationDB's `BUGGIFY`: in-code fault points that fire with some
  probability only in simulation, enabled per site per run. In Rust this is
  a macro that expands to a method call. Outside the simulator feature the
  method is a constant `false` and the branch compiles to nothing.
- Swarm testing: each seed chooses a random *subset* of fault families and
  their probabilities, so no single configuration dominates.
- Fault scheduling in phases: chaos, then a quiet period where the system must
  recover (the liveness check).

#### Key readings

- Alex Miller, "BUGGIFY" (transactional.blog).
- Groce et al., "Swarm Testing" (ISSTA '12).
- Martin Kleppmann, "How to do distributed locking", for process pauses and
  clock jumps.
- TigerBeetle source: `src/vopr.zig` (`options_swarm`, the pause
  probabilities, `transition_to_liveness_mode`). FoundationDB source:
  `flow/include/flow/Buggify.h` for the two probabilities.
- Optional: Kyle Kingsbury, "The trouble with timestamps". TigerBeetle
  `src/stdx/time.zig` (`TimeSim`, which skews only the wall clock) and
  `src/testing/cluster.zig` (`ReplicaHealth` with its `paused` flag).

#### Coding task (sketch)

Add process pauses and per-host clocks with skew, drift and forward jumps to
the simulator. Implement a `buggify!` macro and put fault points through the
KV store and the heartbeat: drop a request, drop a reply, duplicate a reply,
skip a beat. Implement swarm-style option generation so that each seed
produces a distinct fault configuration. Add one call that turns every fault
off and starts the quiet period. Log the configuration at the top of every
run.

### Module 8: Workloads and checking (the oracle problem)

#### Objectives

- Generating realistic random workloads from the seed: mixes of operations,
  hot keys, concurrent clients, retries after timeout.
- The oracle problem: how do you know the result was *wrong*?
- Three families of checks. Inline assertions in the system. A reference model
  (a trivially correct in-memory map) compared against observed results.
  History-based consistency checking (linearizability, with Porcupine and
  Knossos as reference implementations).
- Safety versus liveness: the cluster must never return a wrong answer, and
  must eventually make progress once faults stop.

#### Key readings

- Kyle Kingsbury, Jepsen consistency model overview.
- Herlihy & Wing on linearizability (read for the definition, not the proofs).
- Porcupine (Go) README and its history format.
- TigerBeetle VOPR source: `StateChecker` and the workload/auditor.

#### Coding task (sketch)

Write a seed-driven workload generator with multiple simulated clients. Build
a reference-model checker for the single-node KV store and a simple
linearizability checker for small histories. Plant three bugs in the KV store:
a lost update, a stale read after restart, and a duplicate apply on retry.
Make sure that the checker finds each one.

### Module 9: Assembling the VOPR

#### Objectives

- Compose clock, network, disk, fault injection, workload, and checker into
  one simulator loop driven by a single seed.
- The structure of a run: configure from seed, run the chaos phase for N
  events, stop faults, run until convergence or timeout, report.
- Replication as the first real target: primary-backup with explicit failover,
  so there are genuine distributed bugs to find.
- Reading a failing run: seed, configuration, trace, and the assertion that
  fired.

#### Key readings

- TigerBeetle VOPR source: `simulator.zig` top to bottom.
- TigerBeetle blog and talks on the bugs the VOPR caught.

#### Coding task (sketch)

Turn the KV store into a replicated cluster (primary-backup with a simple
failover rule). Build the simulator binary. `sim --seed N` runs one complete
simulation and either passes or prints the seed, configuration, and failing
assertion. Run a few thousand seeds. You will find bugs. Fix them and keep
the seeds as regression tests.

---

## Part IV: The real thing

### Module 10: Consensus under simulation

#### Objectives

- Implement a real consensus protocol: Raft, or Viewstamped Replication (VSR)
  as TigerBeetle uses it.
- The classic bug classes DST finds: split votes with log divergence,
  stale leaders committing, recovery with a corrupted log, membership and
  view-change edge cases.
- Using the simulator as the primary development tool, not an afterthought:
  write the protocol against the simulated environment first.
- Measuring what the simulator exercises: fault coverage and state coverage.

#### Key readings

- Ongaro & Ousterhout, "In Search of an Understandable Consensus Algorithm"
  (Raft), or Liskov & Cowling, "Viewstamped Replication Revisited".
- "Protocol-Aware Recovery for Consensus-Based Storage" again, now for the
  recovery protocol itself.
- TigerBeetle `replica.zig` for how VSR integrates with the storage fault
  model.

#### Coding task (sketch)

Replace primary-backup with Raft or VSR. Run it through the VOPR with full
network and storage faults. Keep a bug journal. For each failing seed, record
the symptom, root cause, and fix. Target a clean run across tens of thousands
of seeds.

---

## Part V: Operating DST

### Module 11: Debugging, shrinking, and tooling

#### Objectives

- Reproduction: a seed plus a commit hash is a complete bug report.
- Shrinking a failing run: reduce the fault schedule, shorten the run, or
  search nearby seeds for a minimal failing case.
- Trace tooling: structured event logs, per-replica state dumps, and
  time-travel ("show me the cluster at tick T").
- Visualization: what sim.tigerbeetle.com shows and why it is useful.

#### Key readings

- Hypothesis / QuickCheck literature on shrinking. The `proptest` crate's
  shrinking strategy as the Rust reference.
- TigerBeetle's SimTigerBeetle writeup.
- Antithesis writing on multiverse debugging.

#### Coding task (sketch)

Build a shrinker that takes a failing seed and produces a shorter or
less-faulty reproduction. Build a trace viewer that reconstructs each
replica's state at any virtual time. Use both on a real failure from
Module 10.

### Module 12: Continuous simulation in CI

#### Objectives

- Running the simulator continuously: nightly swarms, per-commit seed batches,
  and a fleet of long-running fuzzers.
- Reporting: failing seeds as artifacts, flake-free by construction.
- Deterministic builds and why reproduction requires the same binary: pinned
  toolchain, committed lockfile, and a fixed profile.
- Performance budgets for the simulator itself so it stays fast enough to be
  useful.

#### Key readings

- TigerBeetle's CI and `vopr` fuzzing setup (`.github/workflows`, DevHub).
- FoundationDB's "correctness" test runner documentation.

#### Coding task (sketch)

Add a CI job that runs a batch of seeds on every push. Add a scheduled job
that runs continuously, collects failing seeds, and makes sure that each one
reproduces. Add the Module 1 determinism check to CI so that it runs each
failing seed twice and compares the trace hashes.

### Module 13: The wider landscape and the limits of DST

#### Objectives

- Survey the ecosystem: FoundationDB Flow, Antithesis (deterministic
  hypervisor for nondeterministic programs), `madsim`, `turmoil`, `shuttle`,
  `loom`, Go `synctest`, sled's simulation testing, WarpStream, Resonate,
  Polar Signals' FrostDB.
- What DST cannot find: performance regressions, hardware behavior, compiler
  and kernel bugs, and mismatches between the simulated and real environment
  (the fidelity gap).
- Combining DST with Jepsen-style real-cluster testing, fuzzing, and formal
  specification.

#### Key readings

- Antithesis documentation and blog.
- `madsim` and `turmoil` READMEs and examples.
- WarpStream, "Deterministic Simulation Testing for Our Entire SaaS".
- Tyler Neely (sled), "Reliable Systems Series".

#### Coding task (sketch)

Port your Module 10 consensus implementation onto `turmoil` or `madsim`.
Compare them: what was easier, what you lost, and what each one found that
the other did not. Write a short retrospective on what your simulator does
and does not model.

---

## Capstone

By the end, your repository contains a replicated, consensus-based KV store
and a VOPR-style simulator. The simulator ran the store for millions of
simulated hours under network and storage faults. You recorded every
discovered bug as a reproducible seed. The retrospective from Module 13 is
the written deliverable.

## Next steps for building out this curriculum

1. Expand Module 0 and Module 1 into full lessons with concrete, graded coding
   tasks and acceptance criteria.
2. Continue module by module. Adjust later modules based on how the running
   project actually shapes up.
