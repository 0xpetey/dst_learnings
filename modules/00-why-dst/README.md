# Module 0: Why deterministic simulation?

## Why this module exists

At the end of this course you own two things. One is a replicated key-value
store. The other is a simulator that runs that store for simulated years
under network and disk faults. It hands you a seed for every bug that it
finds.

This module is the "before" picture. The interesting bugs of a distributed
system live in timing windows that are microseconds wide. Conventional tests
either cannot reach these windows, or cannot reach them twice.

In this module you write a small Rust program with a race in it. You watch
the program fail some fraction of the time. Then you write down everything
that you need to control to make one specific failure happen on demand. That
list is the specification for the next thirteen modules.

## Objectives

After this module you can:

- Explain, with a concrete example, why distributed systems are hard to test:
  too many interleavings, rare timing windows, and failures that cannot be
  reproduced.
- Place unit tests, integration tests, Jepsen-style chaos testing, formal
  methods, and deterministic simulation testing (DST) on one map. Say what
  each approach finds and what each misses.
- State the three pillars of DST and why the approach fails without any one
  of them. The pillars are determinism (same seed, same run), simulation (the
  whole cluster in one process), and fault injection (a hostile
  environment).
- Do the rough arithmetic behind "simulated time is free", and name the cases
  where it is not free.
- Say in one sentence what FoundationDB, TigerBeetle's VOPR, and Antithesis
  each added to the idea.
- State the limits. DST finds bugs only in behaviors that the simulator
  models, and only in the cases where the checks notice them.

## Lesson

### Why distributed systems resist testing

A concurrent system has one execution per interleaving of its steps. The
network can delay, reorder, duplicate, or drop any message, and the disk can
lie about what it wrote. For practical purposes the state space is infinite.

Most of that space is boring. The same few orderings happen almost every
time, because the scheduler, the network stack, and the disk have typical
behavior. Bugs hide in the atypical orderings.

A reply arrives after its timeout fired. The OS pauses a leader for 300 ms
in the middle of an election. A write reaches the disk cache, but not the
platter, before the power goes out. Each of these is a rare timing window.

In production, "rare" times a billion operations a day is daily. In a test
suite, "rare" times a thousand runs is never.

Suppose that one of these windows fires in production. The timing that
produced it depended on the kernel scheduler, the load on the machine, the
order of packet delivery, and the clock. None of those are inputs that you
control, so no procedure produces the failure again.

This is the core difficulty. The bug itself is often not subtle.

### Comparing the testing approaches

Each established approach attacks part of this problem. None of them attacks
the whole of it.

| Approach | What it exercises | Reproducible? | Reaches rare timing windows? | Checks | Main blind spot |
|---|---|---|---|---|---|
| Unit tests | One component, one input, one ordering | Yes | No | Hand-written assertions | Interaction between components. Anything timing-dependent. |
| Integration tests | Real components wired together, usually the common path | Mostly | Rarely, by accident, as flakes | Hand-written assertions | The rare paths. Flakes are usually ignored instead of investigated. |
| Jepsen-style chaos testing | A real cluster on real machines. Partitions, clock skew, and process kills injected from outside. | No. A found bug comes with logs, not a replay. | Yes, slowly, and only the windows that the nemesis (the external fault injector) can hit | History-based consistency checking against linearizability (every operation appears instant) and weaker models | Cannot reproduce what it finds. Wall-clock bound, so coverage per hour is low. Disk faults are hard to inject from outside. |
| Formal methods (TLA+, model checking) | An abstract model of the protocol, by exhaustive or bounded search | Yes | Yes, in the model | Invariants and temporal properties | The model is not the code. Implementation bugs, storage bugs, and anything the modeler abstracted away are invisible. |
| Deterministic simulation testing | The real code, whole cluster in one process, against a simulated clock, network, and disk, driven by a seed | Yes, exactly, from the seed | Yes. The simulator chooses the timing and runs millions of variations. | Inline assertions, reference models (simple models the system must match), consistency checkers | Only the behaviors you built into the simulator. Cannot test the boundary with systems outside the simulation. |

Two observations about the table follow. First, the "reproducible" and
"reaches rare windows" columns are usually in tension. The approaches that
reach real-world timing (Jepsen) cannot replay it. The approaches that replay
perfectly (unit tests, TLA+) do not run the real code under real-world
timing. DST is the entry that has both, and the price is the last column.

Second, these approaches complement each other. TigerBeetle runs the VOPR,
and commissioned a Jepsen report, and writes assertions everywhere. Module
13 returns to how they fit together.

### The three pillars

DST rests on three properties. Each is necessary.

The first pillar is determinism. Given the same seed, the program makes
exactly the same sequence of decisions and produces exactly the same output,
bit for bit. Every source of nondeterminism is either removed or derived
from the seed. That includes clock reads, random numbers, thread scheduling,
I/O completion order, and hash iteration order.

The payoff is that a failing seed is a complete bug report. Run it again and
the bug happens again, in the same place, with as much logging as you like.
Without determinism you have a fuzzer that fails now and then and cannot
tell you why.

The second pillar is simulation. The whole cluster, all replicas and all
clients, runs inside one ordinary process. The clock is a counter that the
simulator advances. The network is a priority queue of messages keyed by
delivery time. The disk is a byte array with a fault model.

Nothing in the simulated system touches the operating system. Simulation is
what makes determinism achievable, because there is one thread and one
source of randomness. It also makes time free (see below). Without
simulation you are back to real machines and real time.

The third pillar is fault injection. The simulated environment is hostile.
The simulator delays messages by hours, reorders them, duplicates them, and
drops them. Partitions form and heal. Processes crash at the worst moment
and restart with whatever reached the disk. Disk sectors return garbage.

The seed chooses the faults, so they are reproducible too. Without fault
injection you have a fast integration test of the happy path.

Remove determinism and you cannot debug what you find. Remove simulation and
you cannot run enough variations to find anything. Remove fault injection and
there is nothing to find.

Here is the shift in one picture:

```
  Jepsen-style: real cluster + external chaos tool

   +--------+   real    +--------+   real    +--------+
   | node 1 |<--------->| node 2 |<--------->| node 3 |
   | OS     |  network  | OS     |  network  | OS     |
   | disk   |           | disk   |           | disk   |
   +--------+           +--------+           +--------+
        ^                    ^                    ^
        |  iptables, kill -9, clock skew, tc netem
        +--------------------+--------------------+
                             |
                      +-------------+       wall clock: 1 hour = 1 hour
                      | chaos tool  |       replay: impossible
                      +-------------+       checks: history, after the fact


  DST: one process, simulated environment

   +------------------------------------------------------------+
   | simulator process                      seed = 0x5EED       |
   |                                                            |
   |   +--------+     +--------+     +--------+                 |
   |   | node 1 |     | node 2 |     | node 3 |   real code,    |
   |   +---+----+     +---+----+     +---+----+   no OS access  |
   |       |              |              |                      |
   |   +---+--------------+--------------+----+                 |
   |   | simulated network (priority queue)   |  delay, drop,   |
   |   | simulated disks  (byte arrays)       |  reorder, tear  |
   |   | simulated clock  (a counter)         |  crash, skew    |
   |   +--------------------------------------+                 |
   |                                                            |
   |   PRNG(seed) -> every fault, every delay, every tie-break  |
   +------------------------------------------------------------+
        wall clock: idle time costs nothing
        replay: rerun with the same seed
        checks: assertions fire the instant an invariant breaks
```

### Simulated time is free (mostly)

A discrete-event simulator runs one queued event at a time, and its clock
does not tick. The simulator keeps a queue of future events sorted by due
time. It pops the earliest event, sets the clock to that event's time, and
runs it. Suppose that the next event is a heartbeat timer three seconds
away. The clock jumps three seconds, and the cost was one pop from the
queue. Simulated idle time costs nothing. Only events cost.

So the question "how many simulated years per wall-clock hour" has no fixed
answer. The answer depends on event density, the number of events per
simulated second. Three worked cases follow.

The first case is a quiet cluster. Three replicas each send a heartbeat every
100 ms, and nothing else happens. That is 30 events per simulated second, or
about 950 million events per simulated year. At one microsecond per event, a
simulated year costs about 16 minutes. At 100 ns per event, it costs under
two minutes.

The second case is a retry loop. One client retries against a dead server
with exponential backoff, capped at one minute. That is roughly half a
million events per simulated year. A year costs well under a second. Module
4 asks you to build exactly this and run it for a simulated year in under a
second.

The third case is a busy cluster. Joran Greef gave the VOPR's numbers in his
2023 QCon talk. An average simulation runs about 235,000 ticks of 10 ms
each. That is about 39 minutes of simulated time, and it takes about 3.3
seconds of wall time. The speedup is roughly 700x. The TigerBeetle docs
round this to "1000x".

At 700x to 1000x, a simulated year of a busy cluster costs 9 to 12 hours of
one core. The 2023 TigerBeetle blog post puts it this way, per simulator.
One hour of simulation is about a month. One day is about two years.

The headline numbers that you see quoted ("thousands of cluster-years per
day") come from a fleet, not a laptop. TigerBeetle's docs say that the VOPR
runs 24/7 on 1024 cores. At two simulated years per core-day, that is about
two thousand cluster-years per day. On your 8-core machine it is closer to
sixteen. A bug that shows up once per cluster-year in production shows up a
dozen times in one morning.

The arithmetic points to one control. In the real world you do not choose
the event density. In the simulator you do. If leader elections under
partition are the window that you want to explore, you can make the cluster
spend a simulated year on nothing else.

### Who contributed what

FoundationDB (2010 onward) is the project that worked out this approach. The
team built a deterministic, event-based network simulator before they wrote
the database. Then they wrote the database in Flow. Flow is a C++ extension
that compiles actor-style code into a single-threaded, deterministic
runtime. The same runtime runs in production and in simulation.

One random number generator drives everything, and one seed replays
everything. The team named and refined fault patterns such as "swizzle
clogging". Swizzle clogging clogs the connections of a random subset of
nodes, one at a time, then unclogs them in random order. Their documentation
estimates roughly a trillion CPU-hours of simulated testing over the
project's life. Will Wilson said that the company saw only one or two
customer-reported bugs in its history. His 2014 Strange Loop talk is the
canonical statement of the idea, and the first required reading.

TigerBeetle's VOPR (2020 onward) took the FoundationDB design and extended
it in two ways. The first way is the fault model. The VOPR injects network
and process faults, and also storage faults. Corrupted reads and writes are
injected at double-digit percentages on every replica. The primary is not
exempt. The check is that no committed transaction is ever lost.

The storage faults forced the consensus protocol itself to become aware of
storage faults. That idea is Protocol-Aware Recovery, which Module 6 and
Module 10 cover.

The second way is the discipline around the simulator. Tiger Style,
assertions everywhere, static allocation of all memory at startup, and a
single-threaded control plane were all designed from the start. The design
lets the simulator drive the code. The VOPR is also the one that you can
watch. sim.tigerbeetle.com compiles the actual simulator to WebAssembly and
shows a cluster that survives three escalating fault levels in your browser.

Antithesis (2018, public 2024) asked a different question. What about
software that was not written to be deterministic? FoundationDB and
TigerBeetle rewrote their systems around determinism. Most codebases cannot.
Antithesis built a hypervisor that emulates a deterministic computer.

Any software run inside it becomes replayable, threads and clock reads and
random-number system calls included. Fault injection and state-space
exploration sit on top. It trades the architectural discipline of hand-built
DST for a platform. For this course, Antithesis proves that the three
pillars matter more than any one way to get them.

### The limits

DST is a search over a model of the world. It finds bugs in behaviors that
the model exhibits, and nowhere else. Three consequences follow.

First, a wrong or weak fault model finds nothing. A torn write is a write
stored only in part. Suppose that your simulated disk never produces one.
Then you never find the torn-write bug, and your simulator reports ten
thousand clean seeds anyway. If your network model never delivers a message
after the sender's timeout fired, the whole class of "stale reply" bugs is
invisible.

Phil Eaton quotes Will Wilson on this. Wilson calls it "terrifyingly easy to
build a DST system that appears to be doing a ton of testing". Such a system
explores almost none of the interesting state space. Module 7 (fault
injection) and Module 9 (measuring what the simulator exercises) are about
this.

Second, weak checks find nothing. The simulator can only report what an
assertion or a checker notices. If no invariant mentions a lost update, a
lost update is a clean run.

The oracle is the part that decides whether a run was correct. The quality
of a DST setup is the quality of its oracle: inline assertions, a reference
model, a consistency checker. Module 8 is about this.

Third, the boundary is untested. Everything outside the simulation is
replaced by a model. That includes the real kernel, the real disk firmware,
the real network, and any third-party service. Bugs in the gap between the
model and the real thing are exactly the bugs that DST cannot see.
Reproducibility is also tied to the code. A seed replays a bug on the commit
that found it.

After any code change, the same seed can do something entirely different.
DST does not find performance regressions, hardware faults, compiler bugs, or
kernel bugs. Module 13 returns to all of this, and to how Jepsen-style
real-cluster testing fills some of the gap.

These limits are not an argument against DST. They are an argument for a
precise definition of a clean run. A clean run means that the behaviors we
modeled, under the faults we injected, violated none of the properties that
our checks covered. Every later module widens one of those three clauses.

## Readings

Required readings are the minimum to do the task well. Optional ones add
depth. We made sure that every link worked at the time of writing.

The required readings are:

1. Will Wilson, "Testing Distributed Systems w/ Deterministic Simulation",
   Strange Loop 2014.
   URL: https://www.youtube.com/watch?v=4fFDFbi3toc
   Required. The origin of the approach, from the FoundationDB team, in 45
   minutes. Watch for the three pillars, and for the bug that only a
   simulator found.

2. Phil Eaton, "What's the big deal about Deterministic Simulation Testing?",
   August 2024.
   URL: https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html
   Required. The best short survey of what DST is and who does it. Its
   section on limits is frank. It quotes Wilson on how easy it is to build a
   simulator that tests nothing.

3. TigerBeetle docs, "Safety" (the section on the VOPR and fault injection),
   plus the simulator itself.
   URL: https://docs.tigerbeetle.com/concepts/safety/
   URL: https://sim.tigerbeetle.com
   Required. The docs page states what the VOPR covers and gives the "1000x
   speed, 1024 cores" figures. Then spend ten minutes with the browser
   simulator at sim.tigerbeetle.com. Watch the "Radioactive" level. Note
   which faults you can see and which you cannot.

4. Will Wilson, "Is something bugging you?", Antithesis blog, February 2024.
   URL: https://antithesis.com/blog/is_something_bugging_you/
   Required. The FoundationDB story retold ten years later. It explains why
   Antithesis built a deterministic hypervisor instead of asking everyone to
   rewrite their code.

5. Joran Dirk Greef, "A New Era for Database Design with TigerBeetle", QCon
   London 2023 (video, 50 minutes).
   URL: https://www.infoq.com/presentations/tigerbeetle/
   Required. The talk with the VOPR numbers used in this lesson: 3.3 seconds,
   235,000 ticks, 700x. It also states why TigerBeetle was designed for the
   simulator from the start and not retrofitted.

The optional readings are:

6. Phil Eaton and Joran Dirk Greef, "We Put a Distributed Database In the
   Browser, And Made a Game of It!", TigerBeetle blog, July 2023.
   URL: https://tigerbeetle.com/blog/2023-07-11-we-put-a-distributed-database-in-the-browser/
   Optional. It explains what sim.tigerbeetle.com runs, and what each level
   injects (up to 8% read corruption and 9% write corruption per replica).

7. FoundationDB documentation, "Simulation and Testing".
   URL: https://apple.github.io/foundationdb/testing.html
   Optional. A short primary source. It covers Flow, the single-threaded
   simulator, swizzle clogging, and the "trillion CPU-hours" figure.

8. Joran Dirk Greef, "TigerBeetle: Magical Memory Tour!", CMU Database Group
   seminar, November 2022 (video).
   URL: https://www.youtube.com/watch?v=FyGukn77gqA
   Optional. An earlier talk that focuses on design. It gives the static
   allocation and Tiger Style context before Module 2.

9. Dominik Tornow and Joran Dirk Greef, "Resonate Vibrations: Deterministic
   Simulation Testing" (video conversation).
   URL: https://www.youtube.com/watch?v=el-LqUTv00M
   Optional. Two people who each built a simulator compare notes. It shows
   where the designs diverge.

## Coding task

Write a small multi-threaded Rust program, standard library only, with a
deliberate timing bug. A counter loses updates under a retry-with-timeout
pattern. Run the program a thousand times and measure how often it fails.
Try to reproduce one specific failure. Then write down what you need to
control to make that failure happen on demand.

This task does not touch the course workspace. It is a standalone crate.

### Starting point

Copy `modules/00-why-dst/starter/` from this repository to a working
directory of your choice. The course runner has no check for this module.
When `NOTES.md` is written, `dst next` opens Module 1. The starter contains:

```
starter/
  Cargo.toml       # no dependencies; release profile keeps overflow checks on
  src/main.rs      # types, config parsing, thread setup, and the final check
                   # are written; store, call, and client bodies are TODO
  run.sh           # runs the binary N times and reports the failure rate
```

The skeleton compiles. It reports `FAIL` because the stubs never increment
anything. The design that it fixes in place is:

- One `store` thread owns a `Vec<u64>` of counters and receives `Request`s
  over an `mpsc` channel. `Get { key }` replies with the current value.
  `Put { key, value }` stores the value and replies with it.
- There are `clients` client threads. Each has its own reply channel, which
  it reuses for every request that it sends. Each performs
  `increments_per_client` increments as read-modify-write: `Get`, then
  `Put(value + 1)`. Every `shared_every`th increment goes to key 0, which all
  clients share. The rest go to the client's private key.
- Every request waits for its reply with `recv_timeout(timeout)`. If the
  timeout fires, the client resends the same request and counts a retry.
- `main` joins every thread, then makes sure that each counter is correct.
  Key 0 must equal the total number of shared increments. Each private key
  must equal the number of private increments. The program prints `OK`, or
  one `FAIL` line per wrong key, and exits non-zero on failure.

Defaults: 2 clients, 2000 increments each, every 500th shared, 50 µs
timeout. You can override all four as positional arguments:
`lost_update [clients] [increments] [shared_every] [timeout_us]`.

### Requirements

1. Fill in `store`, `call`, and `client` so that the program implements the
   design above. Use only `std`. `Cargo.toml` must have no `[dependencies]`.
2. The read-modify-write must be two separate requests (`Get`, then `Put`)
   with no locking between them. Do not "fix" the race.
3. On `RecvTimeoutError::Timeout`, `call` must resend the request on the same
   reply channel and increment `stats.retries`. It must not drain or discard
   stale replies. Do not "fix" that race either.
4. Run `cargo build --release` in the crate directory. It must produce no
   warnings. Remove the `#![allow(dead_code)]` line from the starter. Then
   run `cargo clippy --release`. It must produce no warnings.
5. With the default parameters, a single run must finish in well under a
   second.
6. On success the process must print `OK` and exit 0. On failure it must
   print a `FAIL` line that names each wrong key, and exit non-zero.
7. `run.sh N` must run the binary N times and print `F of N runs failed (P%)`.
   With `VERBOSE=1` set in the environment, it must also print the `FAIL`
   lines of each failing run.
8. Write `NOTES.md` next to `Cargo.toml` with three sections:
   - A section "Observed". Give your machine and the failure rate from at
     least one 1000-run batch at the defaults. Also give the rates from at
     least three other parameter sets. Vary the client count,
     `shared_every`, and the timeout. Report how many failures were on the
     shared key and how many on a private key.
   - A section "One failure". Pick a single failing run and try to reproduce
     that failure (same key, same number lost). Report how many runs it
     took, or that you gave up, and why.
   - A section "What has to be controlled". List every decision made outside
     your code that affected the outcome. For each one, state what you need
     to set it to a chosen value.

### Acceptance criteria

```sh
cargo build --release          # no warnings
cargo clippy --release         # no warnings (after removing allow(dead_code))
./run.sh 1000                  # prints "F of 1000 runs failed (P%)" with 0 < F < 1000
VERBOSE=1 ./run.sh 200         # shows FAIL lines; at least one names "shared key 0"
```

`NOTES.md` exists. Its third section names at least these five items:

- The OS thread scheduler (which thread runs, on which core, at what moment).
- The clock behind `recv_timeout`.
- The order in which the store receives requests from different clients.
- The order in which replies land in each client's reply channel relative to
  the timeout.
- The load on the machine from other processes.

If your list is shorter than that, you missed something. A longer list is
fine.

If you report a failure rate of 100%, also report a second parameter set
with a rate below 100%. The point is to see the rate move.

### Hints

<details>
<summary>Hint 1: the store loop</summary>

When every `Sender<Request>` is dropped, `for req in rx` ends. `main`
already drops its own clone after it spawns the clients. When a client
returns, its clone drops too. Match on `req.op`, update `values[key]`, and
`let _ = req.reply.send(result);`.

</details>

<details>
<summary>Hint 2: the call loop</summary>

```rust
loop {
    tx.send(Request { op, reply: reply_tx.clone() }).expect("store gone");
    match reply_rx.recv_timeout(timeout) {
        Ok(v) => return v,
        Err(RecvTimeoutError::Timeout) => stats.retries += 1,
        Err(RecvTimeoutError::Disconnected) => panic!("store gone"),
    }
}
```

`Op` is `Copy`, so a resend is another send of `op`.

</details>

<details>
<summary>Hint 3: why the private keys can fail</summary>

Only one client ever touches a private key, so a lost update is impossible
there. If a private key is wrong, a retry happened. After a timeout the
original reply is still on its way. The client resends, then receives the
first reply and treats it as the answer to the second request.

From then on, every reply that it reads belongs to the previous request. Now
suppose that the client's next request is a `Get` on key 0. Trace what
happens. It reads the reply to its own `Put` on a private key instead.

</details>

<details>
<summary>Hint 4: moving the failure rate</summary>

More clients and a smaller `shared_every` make the lost update more likely.
A smaller timeout makes retries more likely, and so the stale-reply failure
too. Fewer increments make everything rarer. Try the arguments
`2 200 200 50`, then `4 2000 50 50`, then `2 2000 500 2`.

</details>

<details>
<summary>Reference program. Opt in only after you wrote your own.</summary>

This is the program used to produce the numbers below. It was built with
`cargo build --release` under rustc 1.96.1. It passes `cargo clippy --release`
with no warnings. The only differences from the starter are the three
function bodies.

The numbers below were observed on an Apple M1 (8 cores), macOS, release
build. Default parameters: 2 clients, 2000 increments, every 500th shared,
50 µs timeout.

- 160 of 200 runs failed (80.0%). In those 200 runs, 3 also had a private-key
  failure. In those 3 a timeout fired and the client misattributed a stale
  reply.
- 683 of 1000 runs failed (68.3%).
- An earlier 100-run batch of the same configuration failed 52 times. Other
  batches were running on the machine at the time. Same binary, same
  parameters, different rate. That variance is itself the lesson.
- Every shared-key failure line was `got 7 expected 8 (1 lost)` or
  `got 6 expected 8 (2 lost)`. The output does not show which of the eight
  shared increments was lost. It was different each time.
- Retries per run at the defaults were 0 in roughly 7 of 10 runs, and 1 in
  the rest. One retry is enough to shift every later reply by one request.

Other parameter sets, 100 or 200 runs each, same machine:

| clients | increments | shared_every | timeout µs | failed |
|---|---|---|---|---|
| 2 | 200 | 200 | 50 | 16% |
| 2 | 500 | 500 | 50 | 42% |
| 2 | 2000 | 500 | 50 | 52% to 80% across batches |
| 2 | 2000 | 500 | 10 | 97%, most with a stale-reply failure |
| 2 | 2000 | 500 | 2 | 100%, all with a stale-reply failure |
| 3 | 2000 | 1000 | 50 | 92% |
| 4 | 2000 | 50 | 50 | 100% |
| 8 | 2000 | 1000 | 50 | 100% |

```rust
//! Reference program for Module 0: a counter with a lost update under a
//! retry-with-timeout pattern.
//!
//! One `store` thread owns an array of counters and answers `Get` and `Put`
//! requests over an mpsc channel. Several client threads increment counters
//! by reading a value and writing back value + 1. Each client waits for a
//! reply with a timeout and resends the request if the timeout fires.
//!
//! Key 0 is shared by every client; key `id + 1` is private to client `id`.
//! Two bugs hide here, both of them timing-dependent:
//!
//! 1. Lost update: two clients both read the shared counter, both write
//!    back value + 1, and one increment disappears.
//! 2. Stale reply: when a timeout fires and the client resends, the reply to
//!    the first attempt is still coming. The client later treats it as the
//!    reply to a different request.
//!
//! The program checks the final counters against what they must be and
//! exits non-zero if they differ.

use std::env;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// Run parameters. Defaults can be overridden on the command line:
/// `lost_update [clients] [increments_per_client] [shared_every] [timeout_us]`.
#[derive(Clone, Copy, Debug)]
struct Config {
    /// Number of client threads. Each also owns one private key.
    clients: usize,
    /// Increments each client performs.
    increments_per_client: u64,
    /// Every Nth increment goes to the shared key 0; the rest go to the private key.
    shared_every: u64,
    /// How long a client waits for a reply before resending.
    timeout: Duration,
}

const DEFAULT: Config = Config {
    clients: 2,
    increments_per_client: 2_000,
    shared_every: 500,
    timeout: Duration::from_micros(50),
};

#[derive(Clone, Copy, Debug)]
enum Op {
    Get { key: usize },
    Put { key: usize, value: u64 },
}

struct Request {
    op: Op,
    reply: Sender<u64>,
}

/// The store thread. Processes requests in arrival order. `Put` replies with
/// the value written so every request has a reply.
fn store(rx: Receiver<Request>, keys: usize) -> Vec<u64> {
    let mut values = vec![0u64; keys];
    for req in rx {
        let result = match req.op {
            Op::Get { key } => values[key],
            Op::Put { key, value } => {
                values[key] = value;
                value
            }
        };
        // A client that timed out is still holding its reply channel open,
        // so this send normally succeeds and the late reply sits in the
        // channel. If the client has exited, the reply is discarded.
        let _ = req.reply.send(result);
    }
    values
}

struct ClientStats {
    retries: u64,
}

/// Send one request and wait for its reply, resending on timeout.
fn call(
    tx: &Sender<Request>,
    reply_tx: &Sender<u64>,
    reply_rx: &Receiver<u64>,
    op: Op,
    timeout: Duration,
    stats: &mut ClientStats,
) -> u64 {
    loop {
        tx.send(Request { op, reply: reply_tx.clone() })
            .expect("store thread gone");
        match reply_rx.recv_timeout(timeout) {
            Ok(value) => return value,
            Err(RecvTimeoutError::Timeout) => {
                stats.retries += 1;
                // Fall through and resend. The original request may still be
                // in the store's queue, and its reply will arrive later.
            }
            Err(RecvTimeoutError::Disconnected) => panic!("store thread gone"),
        }
    }
}

fn client(id: usize, cfg: Config, tx: Sender<Request>) -> ClientStats {
    let (reply_tx, reply_rx) = mpsc::channel::<u64>();
    let mut stats = ClientStats { retries: 0 };
    let private_key = id + 1;
    for i in 0..cfg.increments_per_client {
        let key = if i % cfg.shared_every == 0 { 0 } else { private_key };
        let current = call(&tx, &reply_tx, &reply_rx, Op::Get { key }, cfg.timeout, &mut stats);
        call(
            &tx,
            &reply_tx,
            &reply_rx,
            Op::Put { key, value: current + 1 },
            cfg.timeout,
            &mut stats,
        );
    }
    stats
}

fn parse_config() -> Config {
    let mut cfg = DEFAULT;
    let args: Vec<String> = env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    let num = |i: usize| -> Option<u64> { args.get(i).and_then(|a| a.parse().ok()) };
    if let Some(v) = num(0) {
        cfg.clients = v as usize;
    }
    if let Some(v) = num(1) {
        cfg.increments_per_client = v;
    }
    if let Some(v) = num(2) {
        cfg.shared_every = v.max(1);
    }
    if let Some(v) = num(3) {
        cfg.timeout = Duration::from_micros(v);
    }
    cfg
}

fn main() -> ExitCode {
    let verbose = env::args().any(|a| a == "-v" || a == "--verbose");
    let cfg = parse_config();
    let started = Instant::now();

    // Key 0 is shared; keys 1..=clients are private.
    let keys = cfg.clients + 1;
    let (tx, rx) = mpsc::channel::<Request>();
    let store_handle = thread::spawn(move || store(rx, keys));

    let clients: Vec<_> = (0..cfg.clients)
        .map(|id| {
            let tx = tx.clone();
            thread::spawn(move || client(id, cfg, tx))
        })
        .collect();
    // Drop the main thread's sender so the store exits once clients finish.
    drop(tx);

    let mut total_retries = 0;
    for handle in clients {
        let stats = handle.join().expect("client panicked");
        total_retries += stats.retries;
    }
    let values = store_handle.join().expect("store panicked");

    // Expected values.
    let shared_per_client = cfg.increments_per_client.div_ceil(cfg.shared_every);
    let expected_shared = shared_per_client * cfg.clients as u64;
    let expected_private = cfg.increments_per_client - shared_per_client;

    let mut failures = Vec::new();
    if values[0] != expected_shared {
        failures.push(format!(
            "shared key 0: got {} expected {} ({} lost)",
            values[0],
            expected_shared,
            expected_shared as i64 - values[0] as i64
        ));
    }
    for id in 0..cfg.clients {
        let key = id + 1;
        if values[key] != expected_private {
            failures.push(format!(
                "private key {}: got {} expected {} (diff {})",
                key,
                values[key],
                expected_private,
                values[key] as i64 - expected_private as i64
            ));
        }
    }

    let elapsed = started.elapsed();
    if verbose || !failures.is_empty() {
        eprintln!("config={cfg:?}");
        eprintln!("values={values:?} retries={total_retries} elapsed={elapsed:?}");
    }
    if failures.is_empty() {
        println!("OK   retries={total_retries}");
        ExitCode::SUCCESS
    } else {
        for f in &failures {
            println!("FAIL {f}");
        }
        ExitCode::FAILURE
    }
}
```

</details>

### Stretch goals

- Force it. Insert a `thread::sleep` of a few microseconds between the
  `Get` and the `Put` in `client`. Insert another in `store` before it sends
  the reply. Watch the shared-key failure rate go to 100%, and watch the
  stale-reply failure appear in most runs. Then ask whether this made a
  specific failure reproducible, or only made failure in general certain.
  The number lost still varies. This is the difference between "flaky on
  purpose" and "deterministic".
- Order the world. Replace both `mpsc` channels with a single `Vec<Event>`
  that you process in a loop that you wrote. Replace `recv_timeout` with a
  comparison against a `u64` counter that you advance by hand. The race is
  still in the logic, but now you pick the interleaving. Can you write down
  an exact sequence of events that loses exactly one update on key 0? This
  is Module 4 in miniature.
- Try loom. Port the program to `loom`, a separate crate outside this task's
  no-dependency rule. Note what loom can model (threads, `std::sync`
  primitives) and what it cannot. There is no clock, so `recv_timeout` has
  no meaning. A model checker runs every possible schedule of a small
  program. The gap shows the split. Model checkers explore scheduling.
  Simulators explore time and faults.
- Count the interleavings. Take 2 clients, each doing 1 shared increment.
  That is 4 requests total, 2 per client, in a fixed order per client. List
  the possible arrival orders at the store and count how many lose an
  update. Then do 2 shared increments each. Notice how fast the count grows.

## Check your understanding

1. Your program failed 683 times out of 1000. A colleague says: "So the bug
   reproduces 68% of the time. That is easy to debug." What is wrong with
   that statement?

2. For each of the three pillars, name one thing that the task's program
   needs to have it. Which pillar is hardest to add to this program as
   written, and why?

3. A team adds a simulated network that can delay and drop messages. They run
   a million seeds and find nothing. Give two different explanations that do
   not involve the system being correct.

4. The lesson says that simulated time is "mostly" free. A simulated
   cluster-year took 16 minutes in one example and 12 hours in another. What
   determines the difference? What does that imply for the design of a
   simulation workload meant to find leader-election bugs?

5. Jepsen found a bug in a system that is not deterministic. Both Jepsen bugs
   and DST bugs are real bugs in the same code. Why is the Jepsen one harder
   to fix?

6. Only one thread ever writes each private key. Why did the private-key
   counters still fail in some runs?

7. FoundationDB and TigerBeetle rewrote their systems around determinism.
   Antithesis did not require that. What did Antithesis control instead, and
   what does that say about what determinism requires?

<details>
<summary>Answers to "Check your understanding"</summary>

1. The rate is reproducible. No individual failure is. Each failing run lost
   a different increment at a different moment under a different
   interleaving. Nothing in the output lets you re-run that specific
   interleaving. Debugging needs the same failure twice: once to see it, and
   again with more logging. A 68% chance of some failure gives you that for
   no particular failure.

   Also, the rate moved between batches (52% to 80%) with no code change. So
   even the rate is not a stable property of the program.

2. Determinism: a single thread (or a scheduler that you control), and a
   clock that is a counter rather than the real deadline behind
   `recv_timeout`. Simulation: the store and the clients run as state
   machines driven by one event loop, and the channels become a queue that
   the loop owns. Fault injection: the loop chooses, from a seed, the moments
   at which a reply is delayed past the timeout.

   Determinism is hardest here, because the nondeterminism is not in one
   place. It is in `thread::spawn`, in both `mpsc` channels, and in
   `recv_timeout`. Removing it means restructuring the program, not patching
   it. That is why Module 2 is about architecture.

3. (a) The fault model does not contain the triggering fault. It has delays
   and drops, but never a delivery after the sender's timeout fired, a
   crash mid-write, or a partition that heals asymmetrically.
   (b) The checks cannot see the failure. Nothing asserts the property that
   the bug violates, so the bug happens in many of those million runs and
   every one reports clean. A third explanation: the workload never reaches
   the state where the bug lives (one client, no contention, no retries).

4. Event density. The simulator pays per event, not per simulated second. A
   quiet cluster is cheap and a busy one is not. For leader-election bugs you
   want many elections per simulated second. That means short election
   timeouts, frequent partitions and crashes, and little client traffic.

   You tune the workload to spend its event budget in the window that you
   care about. No real cluster lets you do that.

5. Jepsen gives you logs and a history from one run. The system's own
   nondeterminism (scheduler, real network, real clocks) means that a re-run
   can miss the same interleaving forever. You are back to reasoning from
   symptoms. A DST failure gives you a seed that re-creates the exact
   interleaving on every run. You can add logging, bisect commits, and make
   sure that a fix works by running the seed again.

6. A timeout fired. The client resent the request while the original reply
   was still in flight. It then received the original reply and treated it
   as the answer to the resend. From then on, every reply that it read
   belonged to the previous request. That was harmless while both requests
   touched the same private key. Then that client issued a `Get` on key 0.

   It read the reply to its own earlier `Put` on the private key, and wrote
   that value plus one to key 0. Later it wrote a key 0 value back to its
   private key. One late reply corrupted two counters. Request IDs fix it.
   The task forbids the fix because the point is to see the bug.

7. Antithesis controls the machine rather than the program. Its hypervisor
   makes every input that the program can observe (clock, scheduling,
   randomness, I/O) a function of the simulation's own state. The lesson is
   that determinism is a property of the whole system and its environment.

   You can obtain it in two ways. Write the program so that it observes
   nothing uncontrolled (FoundationDB, TigerBeetle, this course). Or control
   everything that it is able to observe (Antithesis). Both reach the same
   three pillars.

</details>

## What's next

You now have a program that fails for reasons that you cannot set. You also
have a list in `NOTES.md` of exactly which reasons those are. They are the
scheduler, the clock, the order of channel delivery, and the load on the
machine. Module 1 turns that list into a catalog. It names every source of
nondeterminism that a program can have, in general. Module 3 shows where
each one hides in Rust. Module 1 introduces the seed as the single root from
which every decision must descend. Module 1 also has you build the first
piece of the course workspace. You build a seeded PRNG, a trace log, and a
check that runs the same seed twice and fails on any divergence. That check
runs in CI for the rest of the course.
