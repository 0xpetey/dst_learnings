# Module 13: Other simulators, and the limits of DST

## Why this module exists

Twelve modules built one simulator. It has a scheduler, a network, a disk, a fault swarm, an oracle, a shrinker, an inspector and a nightly. Every decision in it was yours. That is its strength, and it is also the reason you cannot yet judge it. A simulator you built is the only simulator you know.

This module gives you a second one. You take the Module 10 replica, unchanged, and run it on turmoil, the simulation framework from the tokio project. The port is small, because the replica is sans-I/O. Three traits, a wire format and a loop. Then you compare. What was easier, what you lost, what each one found that the other did not. The answers are in the numbers and in two findings, and both findings say something about the limits of the method.

The last part of the course steps back from the code. Deterministic simulation finds a class of bugs well, and finds other classes not at all. The retrospective is where you write down which is which, for your simulator, with evidence. It is the course's written deliverable.

## Objectives

- Place the simulators in four shapes. One is written around a sans-I/O core. One is a runtime swapped in under existing async code. One is a hypervisor that makes a whole machine deterministic. One is a model checker for thread interleavings.
- Port the replica onto turmoil with no change to the core, and say what the port needed: a disk turmoil does not have, a wire format the fake network never asked for, and a wait that does not use `select!`.
- Compare the two simulators with numbers from the same replica: seeds per second, virtual time per second, and what each one found.
- Name what DST cannot find: the fidelity gap, performance regressions, compiler and kernel behaviour, bugs in the real environment, and bugs in the oracle.
- Say what covers each gap: real-cluster testing, fuzzing, formal specification, assertions kept on in production, and a hypervisor.
- Write the retrospective.

## Lesson

### Four shapes

Every deterministic simulator answers one question: who owns the clock, the network and the disk, so that a seed decides everything? The answers fall into four shapes.

The first shape is the one this course built. The code under test is a state machine with no I/O, and the simulator is the environment around it. FoundationDB is the original. Its Flow language compiles actors to C++. The simulator swaps the network and the disk under them in one process, with `BUGGIFY` points in the code. TigerBeetle's VOPR is the same shape in Zig. WarpStream wrote one in Go for its whole service. The cost is that the code has to be written for it from the start. The reward is a simulator that models exactly what you need, at the speed of a function call.

The second shape is a runtime swapped in under existing code. turmoil runs each host on its own paused tokio runtime, and gives you `turmoil::net` in place of `tokio::net`. madsim goes further and replaces tokio itself, behind a `--cfg madsim` flag, with patched copies of the crates that reach the OS. sled's simulation guide and Go's `testing/synctest` are the same idea. The code keeps its shape, and the simulator takes the runtime. The cost is that the runtime decides what is modelled, and anything that reaches around it is not.

The third shape is a hypervisor. Antithesis runs your containers, unchanged, on a hypervisor that makes the whole machine deterministic, and explores the state space from snapshots. Nothing is ported. The cost is speed, a service, and a view of the system from outside: the system's own assertions, through an SDK, are the oracle.

The fourth shape is a model checker for threads. loom runs a block of concurrent Rust under every interleaving the C11 memory model allows. shuttle runs it under a random scheduler with a seed. Neither has a network or a disk. They check a lock-free queue, not a cluster. They are listed here because people confuse the two, and because a replica built on threads needs them before it needs anything in this course.

| Shape | Examples | You write | It owns | Speed |
|---|---|---|---|---|
| Environment around a sans-I/O core | FoundationDB, TigerBeetle, WarpStream, this course | the core and the simulator | everything | a function call |
| A runtime under existing code | turmoil, madsim, sled, synctest | the code, in the runtime's style | time and the network | a task switch |
| A deterministic hypervisor | Antithesis | assertions | the machine | a virtual machine |
| A thread model checker | loom, shuttle | a small concurrent block | the scheduler | every interleaving |

### The port

The replica is already sans-I/O. `kv_core::env` has three traits, `Time`, `Storage` and `Network`, and a `Driver` that moves events between them and the replica. The port is three implementations of those traits in a new crate, `kv-turmoil`, and nothing in `kv-core` changes.

`TokioTime` reads tokio's paused clock, relative to the moment the host started. turmoil advances every host's clock one tick at a time, one millisecond by default, so two events in a tick share an instant. A bounced host gets a fresh runtime and a clock that starts from zero again, which is what Module 7's hosts did.

`SharedStorage` is a `Vec<u8>` per host, kept outside the host in an `Rc`. turmoil has no disk. Its filesystem is behind an unstable feature, so the port keeps the bytes itself. A crash drops the host's runtime and keeps the bytes, and a bounce reads them back. When an append returns, it is durable, as one `fsync` per append makes it. Nothing is in flight at a crash, so nothing is ever torn. Module 6's disk had five faults and a crash model. This one has none.

`UdpNetwork` is one `turmoil::net::UdpSocket` per host, bound to a fixed port. A peer message goes to the host named `n{peer}`, and a reply goes to the host named `c{client}`. Every client is a turmoil host of its own, named for its id, so that a reply from any replica finds it. turmoil's DNS does the routing that the fake world's one address space did.

The network needed one thing the fake network never did: a wire format for the replica's messages. The fake network of Modules 5 to 12 carried `replica::Message` by value. The real environment never dialled a peer. So the protocol between replicas had no bytes until this module. `kv_turmoil::wire` gives it a tag byte per variant, in the style of `kv_core::protocol`, and a datagram envelope with the sender's id in it. A protocol with no wire format is a protocol that was never finished.

```text
                 turmoil::Sim
   ┌────────────────────────────────────────────────┐
   │  host n0     Driver<TokioTime, SharedStorage,  │
   │  host n1             UdpNetwork, TraceObserver>│      Disks: Rc<RefCell<Vec<Disk>>>
   │  host n2     ─── step(), then wait ───►        │ ◄──  one Vec<u8> per host, outside the hosts
   │  client c1   Client + Workload + History       │
   │  client c2   ...                               │
   └────────────────────────────────────────────────┘
          ▲ sim.step() in a loop, and between steps:
          │ partition / repair / hold / release / crash / bounce
          │ from a Schedule drawn from the seed
```

### The loop

The driver is synchronous. It steps on request and polls its environment. An async host has to wait instead, for a datagram or for the replica's next deadline, whichever comes first.

```rust
loop {
    driver.step();
    // snapshot the replica here, for the checks at the end of the run
    if driver.is_halted() {
        return Err(format!("n{node} halted: {damage}").into());
    }
    let wait = match driver.replica.next_deadline() {
        Some(deadline) => deadline.saturating_duration_since(driver.time.now()),
        None => Duration::from_millis(10),
    };
    let _ = tokio::time::timeout(wait, driver.network.socket.readable()).await;
}
```

The obvious tool is `tokio::select!`, and the port does not use it. `select!` polls its branches in a random order, from tokio's own random number generator. turmoil seeds that generator, but only under `--cfg tokio_unstable`, because the method that takes the seed is unstable. Without the flag, a `select!` in host code is a source of nondeterminism that no lint in Module 3 can see. `timeout` has one order, and the loop is deterministic without the flag. The acceptance test runs seed 2 twice and across two processes to prove it.

The clients are the same `kv_core::Client` state machine, driven the same way: submit, send the request as a datagram, wait on the socket or the client's deadline, feed a reply or a tick. The history is the same `checker::History`, and the check at the end is the same `checker::kv::check`. The oracle crossed over without a change, which is what keeping it `no_std` was for.

### Faults, from outside

turmoil has no scheduler to hand a fault schedule to. Its faults are method calls on the `Sim`: `partition`, `repair`, `hold`, `release`, `crash` and `bounce`. So the port draws a `Schedule` from the seed before the run. The schedule is a list of instants and actions, and each one is applied between steps of `sim.step()`. The run loop is the test code. That is turmoil's model: the test is the fault injector, and the simulation is what it injects into.

The three families map onto Module 7's like this. A partition is a cut link, with one difference: turmoil drops every message in flight on the link at the moment of the cut. Module 5 decided to check a cut at send and at delivery, which lets a message already sent arrive. A hold is a pause of a link, not of a host. Messages queue on the link and are delivered on release. Module 7's pause was a property of the host. A crash is a crash, and a bounce is a restart. There is no clock skew between hosts, because every host's clock advances by the same tick. There are no fault points in the code, no slow host, and no disk fault.

### What the port found

Two things, and neither is what you expect.

The first was in the real environment. Seeds 34 and 42 failed with a client that gave up in the quiet phase. The trace showed replicas going into `repairing` and `erasing` after every bounce. A bounce loses nothing in this port, so a hole in the log had no business being there. The port's first `SharedStorage` copied `FileStorage` from `kv_env::real`. That code writes at the replica's offset and then cuts the file at the end of the write. Module 10 put the superblock at offset zero. So every promise written to the superblock erased the whole log behind it. The fake disk of Module 6 never cut anything, so twenty thousand seeds of the VOPR never saw it. The real server runs a cluster of one. A cluster of one writes its superblock once, before any record, so the restart test of Module 2 never saw it either. A three-node cluster over real TCP, the Module 9 stretch goal, loses its log at its first view change. The fix is one line removed from `real.rs`. A test in `kv-env` now writes at a lower offset and reads the bytes past it back.

That bug was in the code the simulator does not run. `real.rs` is the least tested file in the workspace, and it is the file production runs. The port found the bug because its storage copied the real one. Nothing about turmoil found it. The lesson is about what a simulation covers, and the retrospective gives it a paragraph of its own in part 2.

The second was a view change with no fault. Seed 3 draws a latency of up to nine milliseconds and no partitions, holds or crashes, and its report says five view changes. The trace shows why. Node 0 sends its first beat at time zero, while it is still reading its disk. The beat takes six milliseconds to reach node 4. In those six milliseconds node 0 reads its empty disk, gathers the view, and starts it. Node 4 becomes a backup under the `StartView`, and then the old beat arrives, saying `reading`. Module 10's rule for a backup whose primary is reading its disk is that the primary restarted, and node 4 starts a view change. A first beat that arrives after the view it belongs to is a start read as a restart.

Is that turmoil's finding? Run the VOPR with every fault family off except the network, and read the `view_changes` field of its report. On seeds 7, 8, 9, 16, 21, 24, 32, 36 and 38 of the first forty it is three or more. With reordering off too it still happens on six seeds, because a uniform latency range is enough. The VOPR dealt this on thousands of seeds and nothing turned red. No oracle asks whether a run with no faults has a view change. turmoil did not find a bug the VOPR missed. A trace that someone read found a question the VOPR was never asked. The fix is a stretch goal, because it moves every fingerprint since Module 9. The new oracle is a stretch goal too.

The planted bugs behave as they did. `Bug::StaleReadAfterStepDown` is caught on nine of the first twelve seeds, by the same checker, with the same `safety:` message. Seed 3 catches it with no partition at all, through the view change above.

### The numbers

A seed's report has the shape of the VOPR's. Seed 2 draws three nodes, partitions only, and a latency of up to 116 milliseconds:

```text
seed=2 nodes=3 clients=6 requests=50 quiet=10 chaos=20s latency<=116ms faults=[partitions] workload=[keys=18 hot=4@40/100 mix=get:3 put:4 delete:1 value<=48 think=24/100@<=260ms]
chaos: submitted=300 completed=300 gave_up=0 in 20s
faults: partitions=9 holds=0 crashes=0
cluster: view_changes=25 promotions=9 pulls=68 forwarded=452 datagrams=5447 rejected=0
quiet: submitted=60 completed=60
check: history=360 keys=18 steps=411 cached=385 depth=56
cost: ticks=27358 virtual=27.358s trace_lines=0
trace_hash=c1065db4e0bc1e36
```

Nine partitions in twenty seconds gave twenty-five view changes and nine promotions, and every one of the three hundred and sixty requests was answered. The cost line is in ticks, because that is the unit turmoil has.

The same replica, one hundred seeds each, in release on one core. The VOPR runs three hundred requests per seed, with `--requests 300`. The port's `--requests` counts per client, so `--requests 50` with six clients is also three hundred, in a chaos phase of twenty virtual seconds. Then it settles for five seconds and runs sixty requests that must all complete.

| | VOPR | turmoil port |
|---|---|---|
| Wall time for one hundred seeds | 2.6 s | 37.4 s |
| Wall time per seed | 26 ms | 374 ms |
| Virtual time per seed | about 7 s | about 27 s |
| Virtual seconds per wall second | about 270 | about 73 |
| Cost unit | scheduler events, about 10 per request | ticks, about 90 per request |
| One seed in debug | under 50 ms | 2.4 s to 6 s |

The port is fourteen times slower per seed and four times slower per virtual second. The difference is the tick. The VOPR jumps from event to event, and an idle cluster costs nothing. turmoil advances every host by one millisecond, polls every task, and ticks the network, whether anything happened or not. Twenty-seven thousand ticks for three hundred requests is the price of a general runtime. It is still twenty-seven virtual seconds in a third of a wall second, which is faster than real time by a factor of seventy.

The port has what the VOPR does not: a real async runtime under the code, TCP with connection semantics, DNS, and the `tracing` output. It also has nothing of its own to maintain. The VOPR has what the port does not: a disk with five faults and a crash model, clock skew and jumps, and host pauses. It also has fault points in the code, a cost in the seed's units, the shrinker, the inspector and the batch. Every one of those is a module of this course, and every one has to be rebuilt on top of turmoil.

### Why not madsim

madsim replaces tokio under a `--cfg madsim` flag. Code written against tokio keeps compiling. The simulator takes the runtime, the clock, the network and the random number generator in one move. RisingWave tests its whole database that way. It is the right tool for a system that already lives in tokio.

The replica does not. Its core has no runtime in it, and the only tokio in the workspace is the one the port added. madsim buys the port nothing that turmoil did not. It costs the `--cfg` flag in every command and a patched copy of every crate that reaches the OS. The madsim port is a stretch goal, and the lesson is the comparison: a sans-I/O core makes the choice of simulator cheap, and a core written on a runtime makes the runtime's simulator the only choice.

### What DST cannot find

The simulator runs the core against a model of the world. Every bug it finds is a bug in the core against the model. Everything else is outside.

The fidelity gap is the distance between the model and the world. Module 6's disk tears writes in order and loses everything after the first torn write, because the real environment does one `fsync` per append. A real disk with a write cache lies about `fsync`. A real filesystem reorders metadata. Module 5's network drops, duplicates, reorders and delays. A real network also delivers a packet to the wrong process after a port is reused. It resets a TCP connection with data in both buffers, and it drops a packet larger than a path allows. Module 7's clock skews, drifts and jumps forward. When NTP corrects a real clock, the clock steps backward. The retrospective lists each assumption the model makes, and names the real behaviour it leaves out.

The real environment itself is outside. `real.rs` is a few hundred lines that the simulator never executes, and this module found a bug in it that lost every record. Whatever runs in production and not in the simulator is tested by nothing but the integration test of Module 2.

Performance is outside. The simulator has no wall clock. Module 12 put a ceiling on the simulator's own cost in events and steps. That catches a change that makes the cluster do more work. It does not catch a change that makes the same work slower: a lock held too long, a cache missed, a syscall added. Nothing in the trace changes, and nothing turns red.

The compiler and the kernel are outside. The same compiler builds the core for the simulator and for production, so a miscompilation is in both. If the oracle sees the wrong answer, the simulator catches it, and not otherwise. A kernel bug is in production only. Module 12's two operating systems and two architectures check that the trace is a function of the source. That is a check on the compiler's determinism, not on its correctness.

The oracle is outside. The checker says whether a history is linearizable against the KV model. Module 10's journal has two entries, seeds 13 and 19, where the guard was wrong and the protocol was right. An oracle that is wrong in the other direction passes a wrong answer, and no seed ever shows it. The planted bugs of Module 8 are the one check on the oracle, and they check only that it finds the bugs someone thought of.

Coverage is outside, too. Twenty thousand seeds explore what the swarm's distributions reach. The view change at a clean start was reachable, was reached, and was not a failure, because nothing asked. A simulator finds the bugs its oracles describe in the states its faults reach. It says nothing about the rest.

### What covers the gaps

Each gap has a tool, and none of them is another simulator.

Jepsen-style testing runs the real binary on real machines, with a real network and a real disk. It injects faults with the operating system's own tools. It is slow, a run is not reproducible, and it is the one thing that tests `real.rs`. Jepsen's checker, Elle, is the same oracle this course built, on a history taken from a real cluster.

Fuzzing covers the parsers. The wire format of this module decodes bytes that come from the network, which is to say from an adversary. A fuzzer feeds it bytes that no seed draws. The acceptance test has one hand-written lie in it, a beat that claims a billion runs. A fuzzer writes a million of those an hour.

Formal specification covers the protocol. A TLA+ model of the view change, checked by TLC, explores every state of a small cluster. It says whether the invariant holds in all of them. The simulator explores a random sample of a large cluster. The two are complementary, and TigerBeetle has both.

Assertions in production cover the fidelity gap from the other side. The replica's invariants are checked in every profile, because Module 3 kept `debug-assertions` on in release. A production crash on an assertion is a seed you did not have. TigerBeetle runs with its assertions on, and so does this replica.

Antithesis covers the system as deployed, environment and all. It is the hypervisor shape: the containers run unchanged, the whole machine is deterministic, and the SDK's `always` and `sometimes` assertions are the oracle. It finds the bugs in `real.rs` that the simulator cannot run, at the speed of a virtual machine and the price of a service.

### The retrospective

The written deliverable is a retrospective on your simulator. `solutions/13-landscape/RETROSPECTIVE.md` is the reference one, and yours is yours. It has six parts.

1. What the simulator models, in a table: the clock, the network, the disk, the hosts, the code's fault points, and the oracle. One row per model, with the decision it rests on and the module that made it.
2. What each model leaves out, next to the real behaviour it leaves out, and what covers the gap.
3. The journal: every bug a simulator found, from Module 6's first seed to what the port found in Module 13, and the three classes they fall into.
4. What the port found, and what that says about the simulator.
5. The numbers: seeds per second, virtual time per second, the cost of a seed in the seed's units, and the ceiling.
6. What you want to do differently next time, with the reason.

The retrospective is short, two to four pages. It is honest about the gaps, because a reader who trusts the simulator more than it deserves is the worst outcome of the course.

### Decisions recorded

- The port is a new crate, `kv-turmoil`, and `kv-core` does not change. The three traits were enough.
- The disk is a `Vec<u8>` per host, outside the host. A write lands at its offset and the bytes past it stay. `FileStorage` now does the same, and a test in `kv-env` holds it there.
- Peer messages have a wire format, one tag byte per variant, in `kv_turmoil::wire`. The client's requests and replies reuse `kv_core::protocol`.
- The host loop waits with `timeout`, not `select!`, so the run is deterministic without `--cfg tokio_unstable`.
- Faults are a `Schedule` drawn from the seed and applied between steps, from outside the simulation.
- The spurious view change at a clean start stays as it is. The fix moves every fingerprint since Module 9, and the oracle that catches it is a stretch goal.
- `scripts/check.sh` runs twenty seeds of the port in release. The port has no fingerprint test. turmoil's latency draws go through floating point and `ln`, and the course has not checked them across two operating systems.

## Readings

Required:

- turmoil, [the crate documentation](https://docs.rs/turmoil/latest/turmoil/) and [the README](https://github.com/tokio-rs/turmoil), for hosts, clients, `partition`, `hold` and the note on `tokio_unstable`. Then `src/rt.rs` and `src/top.rs` in the repository, for the paused runtime and the link that clears its queue on a partition.
- madsim, [the README](https://github.com/madsim-rs/madsim), for the `--cfg madsim` swap and the list of patched crates. Read it to see what a runtime swap costs.
- Antithesis, ["How Antithesis works"](https://antithesis.com/docs/introduction/how_antithesis_works/) in the documentation, and Will Wilson, ["Is something bugging you?"](https://antithesis.com/blog/is_something_bugging_you/) (2024). The hypervisor shape, and the case for it.
- WarpStream, ["Deterministic Simulation Testing for Our Entire SaaS"](https://www.warpstream.com/blog/deterministic-simulation-testing-for-our-entire-saas) (2024). The first shape, written for a whole service in Go, and what it found.
- Tyler Neely, [the sled simulation guide](https://sled.rs/simulation.html), subtitled "jepsen-proof engineering". The rules for code that a simulator can run, in one page.
- FoundationDB, ["Simulation and Testing"](https://apple.github.io/foundationdb/testing.html) and the [Flow README](https://github.com/apple/foundationdb/blob/main/flow/README.md), for the original of the first shape.

Optional:

- Will Wilson, ["Testing Distributed Systems w/ Deterministic Simulation"](https://www.youtube.com/watch?v=4fFDFbi3toc) (Strange Loop, 2014). The talk that started the field, and the "fidelity gap" in his words.
- Antithesis, ["So you think you want to write a deterministic hypervisor?"](https://antithesis.com/blog/deterministic_hypervisor/), for what the third shape costs. Then [the SDK documentation](https://antithesis.com/docs/using_antithesis/sdk/) for how its oracle looks.
- The Go blog, ["Testing concurrent code with testing/synctest"](https://go.dev/blog/synctest). The second shape in Go's standard library.
- Polar Signals, ["(Mostly) Deterministic Simulation Testing in Go"](https://www.polarsignals.com/blog/posts/2024/05/28/mostly-dst-in-go) (2024), and Resonate, [on simulating its server](https://journal.resonatehq.io/p/deterministic-simulation-testing). Two more teams, two more trade-offs.
- Phil Eaton's ["What's the big deal about Deterministic Simulation Testing?"](https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html) (2024) places the method among the other ways to test. Pierre Zemb's ["Diving into FoundationDB's Simulation Framework"](https://pierrezemb.fr/posts/diving-into-foundationdb-simulation/) tours the original.
- [loom](https://github.com/tokio-rs/loom) and [shuttle](https://github.com/awslabs/shuttle), the READMEs. The fourth shape, and why it is not this course.
- TigerBeetle, ["Simulation Testing For Liveness"](https://tigerbeetle.com/blog/2023-07-06-simulation-testing-for-liveness) (2023). A bug that only a core found, and what a liveness oracle takes.
- Jepsen, [the analyses](https://jepsen.io/analyses). What real-cluster testing finds, system by system, including systems with a simulator.

## Coding task

### Starting point

Start from your Module 12 workspace with `dst start 13`. If you did not finish Module 12, `dst start 13 --from-solution` starts from `solutions/12-ci/` instead. The command adds `port.rs` to `crates/kv-turmoil/tests/` and replaces `real_server.rs` in `crates/kv-env/tests/`, which gained one test. It replaces `scripts/check.sh`, which gained one step. Every earlier test stays as it is, because nothing in `kv-core` changes.

### Requirements

#### The crate

1. A new crate, `crates/kv-turmoil`, with `turmoil` and `tokio` as dependencies and the workspace lints. `kv-core` does not change.
2. `wire::Datagram`, with `Request`, `Reply` and `Peer { from, message }`, and `encode`, `decode` and `to_bytes`. Every variant of `replica::Message` round-trips. Trailing bytes, an unknown tag and a run count that does not fit in the datagram are errors, and no error allocates for a lie.

#### The three traits

3. `TokioTime`, a `Time` from tokio's paused clock, relative to the host's start.
4. `Disk { bytes, starts }`, `Disks`, `new_disks`, and `SharedStorage`, a `Storage` over one host's `Vec<u8>` outside the host. A write lands at its offset and keeps the bytes past it. If the vector is too short, the write grows it. A read returns the whole vector. Both complete at once.
5. `UdpNetwork`, a `Network` over one `turmoil::net::UdpSocket`. A reply goes to host `c{client}` on `CLIENT_PORT`, a peer message to host `n{peer}` on `NODE_PORT`. `poll` drains the socket and decodes every datagram. `NetStats` counts what was sent, received and rejected.
6. `TraceObserver`, an `Observer` that records every input and output into one shared `Trace` with the simulation's time.

#### Hosts and clients

7. `replica_host(node, cluster, shared)`, the software of host `n{node}`. It binds the socket, takes a boot id from the disk's `starts`, builds the `Driver`, plants the bugs, and loops: step, snapshot, halt check, then `timeout(wait, socket.readable())`. Not `select!`.
8. `client_host(id, target, plan, shared, stats)`, the software of host `c{id}`. It drives a `Client` through the `Workload` for `requests` steps, waits until `quiet_at`, and then for `quiet_requests` steps, every one of which must complete. Calls and returns go into the shared `History`.

#### Faults and the run

9. `Action`, `FaultConfig`, `Schedule::draw(prng, nodes, chaos, config)` and `apply`. Every fault is a start and an end inside the chaos phase, drawn from the seed, in time order. A family that is off draws nothing.
10. `RunConfig::from_seed(seed, requests)`, which draws the latency, the workload, its dice, the fault families and the cluster size from one `Prng`. Its `Display` is one line.
11. `run(&RunConfig) -> Report`. It builds the `Sim` with `rng_seed(seed)`, registers the hosts and the clients, steps the simulation and applies the schedule between steps. Then it checks the history with `checker::kv::check`, the committed prefixes of every pair of logs, and the quiet phase. A failure panics with `safety:`, `liveness:` or `halt:` first.
12. `Report`, with the client counts, the fault counts, a `Snapshot` per replica and the check's statistics. It also carries the trace hash, the tick count and the virtual time, and its `Display` has the form in the lesson.
13. The `kv-turmoil` binary: `--seed N [--trace] [--requests N] [--bugs a,b]` and `--seeds A..B`, printing the configuration, the report and a `trace_hash=` line, as `vopr` does.

#### The real environment and the script

14. `FileStorage::append` in `kv_env::real` no longer cuts the file after a write. The new test in `tests/kv-env/real_server.rs` passes.
15. `scripts/check.sh` runs `kv-turmoil --seeds 0..20 --requests 50` in release after the batch.
16. The retrospective, as a Markdown file at the root of your workspace, with the six parts the lesson lists.

### Acceptance criteria

```sh
scripts/check.sh                                   # ends with "all checks passed"
cargo test -p kv-turmoil --test port               # eight tests
cargo test -p kv-env --test real_server            # three tests
cargo run --release -p kv-turmoil -- --seeds 0..100 --requests 50
                                                   # clean, under a minute
```

The retrospective is read, not run.

From the repository root, `mise run check:13` runs this module's `scripts/check.sh` against the reference solution. `mise run sweep` runs the capstone's twenty thousand seeds against it and prints the sweep hash last.

### Hints

<details>
<summary>The host closure</summary>

`sim.host` takes a closure that builds the future, because turmoil calls it again on every bounce. Give it `move` copies of the `Rc` handles and clone them inside: `move || replica_host(node, cluster.clone(), shared.clone())`. The future has to be `'static`, so it cannot borrow from the test.

</details>

<details>
<summary>Where a reply goes</summary>

A backup forwards a request to the primary, and the primary replies. The primary never saw the client's address. Make every client a host named `c{id}`, bound to one fixed port, and send the reply to `(client_name(id), CLIENT_PORT)`. turmoil's `lookup` resolves the name.

</details>

<details>
<summary>Time on a host</summary>

`turmoil::sim_elapsed()` is the simulation's time and is the same on every host. `turmoil::elapsed()` is the host's own, and restarts on a bounce. Use the first for the trace and the history, so that lines from two hosts sort. `tokio::time::Instant::now()` is the host's paused clock, and is what `TokioTime` reads.

</details>

<details>
<summary>The simulation's duration</summary>

turmoil stops with an error after ten virtual seconds by default. A run here is thirty or more. Set `simulation_duration` to something you will not reach.

</details>

<details>
<summary>A beat that lies</summary>

`runs` is a count followed by twelve bytes per run. Compare the count with `r.remaining() / 12` before the loop. A count of four billion in a twenty-byte datagram is refused before any `Vec` is sized for it.

</details>

<details>
<summary>Reading a failure</summary>

Run the seed with `--trace` and grep the `status` and `fault` lines. A replica that goes `repairing` or `erasing` after a bounce has lost bytes from a disk nothing tore. Look at the storage. A view change with no fault line before it is the first beat arriving late. Look at the primary's first `Send` and the backup's matching `Peer` line.

</details>

### Stretch goals

- The oracle the VOPR lacks. Add a check to `vopr::run`: a run with no fault family on has no view change after the first `Primary` status. Then fix the rule in `replica.rs` so that a beat older than the view's start is not a restart. Beats carry the boot id and the sequence number. Every fingerprint since Module 9 moves, and the acceptance tests of Modules 9 to 12 need their new values. Say how you decided that the new values are right.
- Port to madsim as well. Measure the three simulators on the same seeds. Say what the `--cfg madsim` flag cost you in the workflows of Module 12.
- Put the port under `--cfg tokio_unstable`, change `timeout` back to `select!`, and show that the run is still deterministic. Then build without the flag and show the two hashes that differ.
- turmoil's filesystem, behind `unstable-fs`. Replace `SharedStorage` with it, turn on `sync_probability`, and see what a crash loses. Compare with Module 6's crash model.
- TCP. Replace the UDP socket with `turmoil::net::TcpStream` and a connection per peer. Say what a reconnect after a bounce needs that a datagram did not.
- Run the port's one hundred seeds on the two operating systems of Module 12's matrix, and compare the sweep hashes. If they differ, find the floating-point draw that differs.
- A Jepsen-style run. Three `kv serve` processes with real TCP between them, which is the Module 9 stretch goal. Run them under `tc` and `kill -9` for an hour, and check the history with the Module 8 checker. Say what it found that no seed did, and what it cost.
- Antithesis assertions. Add `always` and `sometimes` assertions from the Antithesis SDK to the replica's invariants and to the view change, behind a feature. Say which of the course's oracles each one replaces.
- A TLA+ specification of Module 10's view change, with the invariant that two primaries never serve the same view. Check it with TLC on three nodes, and compare what it explores with what twenty thousand seeds explore.

## Check your understanding

1. The replica moved onto turmoil with no change to `kv-core`. Which decision of Module 2 made that true, and what did the port still have to write?
2. Why does the host loop use `timeout` and not `select!`, and what flag changes the answer?
3. A partition in turmoil and a cut link in Module 5 differ in one way. Which, and which one is closer to a real network?
4. The port found a bug in `real.rs`. Why did twenty thousand seeds of the VOPR not find it, and what does that say about what a simulator tests?
5. Seed 3 of the port shows five view changes with no fault. Did turmoil find a bug the VOPR missed? Say what the VOPR shows under the same conditions.
6. The port runs fourteen times slower per seed. Where does the time go, and why is that the price of a general runtime?
7. Name five things DST cannot find, and the tool that covers each.
8. Why is the oracle outside what the simulator can check, and what is the one check on it that this course has?

<details>
<summary>Answers</summary>

1. The environment as traits, with a driver generic over them and a core that does no I/O. The port still had to write the three implementations, a wire format for the peer messages, and a loop that waits like an async program.
2. `select!` polls its branches in a random order from tokio's random number generator, which turmoil seeds only under `--cfg tokio_unstable`. Without the flag it is nondeterminism the lints cannot see. `timeout` has one order. With the flag, `select!` is seeded and deterministic too.
3. turmoil drops every message in flight on the link at the moment of the cut. Module 5 checks the cut at send and at delivery, so a message already sent still arrives. A real cut loses what is on the wire and in the switch's queue, which is closer to turmoil. It also delivers what was already in the receiver's buffer, which is closer to Module 5. Neither is the whole truth.
4. The simulator never runs `real.rs`. The fake disk writes in place, and the real file was cut after every write. The simulator tests the core against a model, and the environment that production runs is outside the model.
5. No. With every family off but the network, the VOPR shows view changes on nine of the first forty seeds. With reordering off too, it shows them on six. The behaviour was reached and dealt thousands of times. No oracle asked about it. Reading a trace found the question.
6. Into the tick. turmoil advances every host by one millisecond, polls every task and ticks the network on every step, idle or not. The VOPR jumps from event to event. A general runtime cannot know that nothing will happen in the next millisecond, so it has to look.
7. Bugs in the real environment: Jepsen-style runs on real machines, or a hypervisor. Parser bugs on hostile bytes: fuzzing. Protocol bugs in states the swarm does not reach: formal specification. The fidelity gap in production: assertions kept on in release. Performance regressions: measurement, which no trace hash sees.
8. The oracle decides what a wrong answer is, and the simulator can only report what the oracle calls wrong. An oracle that passes a wrong answer is never contradicted. The planted bugs of Module 8 are the one check, and they check only for the bugs someone thought to plant.

</details>

## What's next

Nothing in this course. You have a replicated store under consensus and a simulator that dealt it tens of thousands of faulty runs. You have a journal of every bug those runs found, the tools that read a failure, and a nightly that runs the seeds. You have a second simulator under the same replica, and a retrospective that says what all of it does not cover. The stretch goals of this module are the next course: a port to madsim, a run on real machines, a specification in TLA+, and the oracle the VOPR lacks. Pick the one your system needs. The one thing to carry forward is the habit. When a seed fails, read the trace. When nothing fails for a month, ask what nobody asked.
