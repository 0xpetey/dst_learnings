# Module 4: Virtual time and the discrete-event scheduler

## Why this module exists

The Module 2 world had a clock that tests advanced by hand. The test author chose each advance of the clock. That worked for one client and a handful of operations. It does not scale. Suppose a test wants a disk completion at 7 ms, a timeout at 100 ms and a retry at 300 ms. It must step through all three by hand, in order, without a mistake.

A simulator turns this around. Every effect that takes time becomes an event with a timestamp. A scheduler keeps the events in a queue sorted by time. It pops the earliest event, sets the clock to its timestamp, and handles it. Nothing sleeps. The idle time between two events costs nothing. A year of retries takes as many steps as there are retries.

This module builds that scheduler. It then moves the timeouts, retries, network delivery and disk completion of the KV store onto it. Modules 5 and 6 replace the simple delivery and completion rules with adversarial ones. The scheduler does not change again.

## Objectives

By the end of this module you can:

- Describe a discrete-event simulation as a clock, an event list and a loop that jumps the clock to the next event.
- Build a scheduler whose order is a function of the seed and nothing else, including the order of events at the same instant.
- Say why a `BinaryHeap` with the payload in its key is a trap, and what the fix is.
- Drive the deadline of a state machine from the scheduler with a timer that cancels and re-arms.
- Compare the tick model with the event model and name the case that fits each one.
- Run a simulated year in under a second and say where the speed comes from.

## Lesson

### A simulation is a clock and a queue

A discrete-event simulation has three parts. The clock holds the current virtual time. The event list holds every event that is due in the future, each with a timestamp. A pseudorandom number generator supplies every random choice, so that a run can be repeated. The simulation loop is four lines:

```text
loop:
    (t, event) = queue.pop_min()    // the earliest event
    clock = t                       // jump, never sleep
    handle(event)                   // can push new events at time >= t
```

The textbooks call this next-event time progression. The other option is fixed-increment time progression, where the clock moves in slices of equal size and the loop looks for work in each slice. Next-event progression skips the slices where nothing happens, so it is faster whenever events are sparse. The section on the tick model returns to this choice.

Three rules keep the loop honest:

1. Time moves forward only.
2. Only a pop moves time, and it moves time to the timestamp of the popped event.
3. Nothing is scheduled in the past. `schedule_at` asserts this. A clamp to "now" hides a bug in the caller, and an assertion reports it.

A fourth rule follows from the first three. Idle time has no cost. If the next event is ten years away, the loop takes one step to get there. That is the whole source of the speed that this module measures.

### Ties, and the `BinaryHeap` trap

Two events can share an instant. A client's timeout lands at 100 ms. The reply to that request also lands at 100 ms. If the timeout runs first, the client retransmits and the node sees a duplicate. If the reply runs first, the client is done. Both orders are legal behavior for a real system. The simulator must pick the same one on every run of the same seed.

`BinaryHeap` makes no promise about the order of elements that compare equal. If the heap key is `(Instant, Event)` and `Event` derives `Ord`, the payload decides the tie. A change to the order of the enum variants then changes the schedule of every seed. If the key compares on time alone, the heap decides. Its choice depends on the shape of the heap at that moment, so an unrelated event scheduled an hour earlier can flip it. The result is deterministic for one build, and it is not specified by anything you wrote. Deterministic is not the same as specified.

The fix is a sequence number. Each call to `schedule` takes the next number from a counter, and the heap key is `(at, seq)`. Equal instants then pop in schedule order, first in, first out, and the payload takes no part in it. FoundationDB does the same for its ready queue. Its `TaskQueue` folds a counter into the priority of each task, so tasks of equal priority run first in, first out. Its timer heap compares on time alone and leaves equal times to the C++ `priority_queue`. Both choices are deterministic for one build. Only the first is specified.

Tokio's paused clock shows the same trap from the runtime side. The documentation for `tokio::time::advance` says that every sleep with a deadline inside the advance completes "at the same time". The runtime then polls them in any order.

### Cancellation

A client sends a request and arms a timeout. The reply usually arrives first. The timeout must then go away, or it fires later against a request that is already done. A `BinaryHeap` cannot remove an element from its middle.

There are two ways around this. The first is a generation check. The timer fires, and the handler asks the owner whether it still wants a timer at that instant. If not, the handler drops the event. The second is a live set. The scheduler keeps the set of ids that are scheduled and not cancelled. `cancel` removes an id from the set. `pop` discards any heap entry whose id is not in the set. The reference solution uses the live set, because it keeps the scheduler's statistics honest. A cancelled timer is counted as cancelled, not as fired and ignored.

### Timers that follow a state machine

The client from Module 2 does not own a timer. It reports the instant of its next wake-up through `next_deadline()`. That value changes at three points: a request starts, a retry goes out, or a reply arrives. The simulator keeps one scheduled event in step with it:

```text
client event          next_deadline()      timer action
submit at 0           Some(100 ms)         schedule ClientTimer @ 100 ms
reply at 7 ms         None                 cancel it
timer fires @ 100 ms  Some(300 ms)         schedule ClientTimer @ 300 ms
```

The `sim::Timer` helper does exactly this. After every input to the client, the world calls `reconcile` with the client's current deadline. If the deadline is the one that is armed, nothing happens. If it changed, the helper cancels the old event and schedules the new one. This is better than "schedule on submit, cancel on reply", because the backoff rule lives in the client and nowhere else. The simulator asks. It does not guess. The same helper serves a node's heartbeat in Module 5 and a view-change timeout in Module 10.

### Everything is an event

The world in this module has four kinds of event:

```text
client.submit ──► Send ──► DeliverRequest @ now + net ──► node
node ─────────► Append ──► DiskComplete   @ now + disk ──► node ──► Reply
node ─────────► Reply ───► DeliverReply   @ now + net ──► client
client.next_deadline() ──► ClientTimer    @ deadline  ──► client.on_tick
```

Network latency is a constant plus optional jitter. Jitter is a uniform random addition drawn from the world's own `Prng`, which is seeded from the configuration. Disk latency works the same way with one extra rule. Appends must complete in submission order, because the node asserts it. The world therefore schedules each completion at `max(now + latency, previous completion)`.

The fake storage and the fake network hold no handle to the scheduler. The driver runs the node until it has nothing to do. The world then reads the network's outbox and the storage's count of submitted appends, and schedules an event for each new item. The alternative is to give each fake a shared handle, which is what `turmoil` does with a thread-local. The handle is more convenient and hides the control flow. The explicit version keeps every call to `schedule` in one file. When a seed misbehaves, that is where you want it.

### One clock, many readers

The scheduler owns the current instant. Everything else holds a `Clock`, a cheap handle onto that instant. The driver's `Time` implementation is a `Clock`. The handle is an `Rc<Cell<Instant>>`, so it cannot cross a thread. That is a feature. A clock that two threads can read is a clock that two threads can race on.

### The tick model and the event model

TigerBeetle's simulator does not have an event queue. Its main loop in `src/vopr.zig` calls `simulator.tick()` up to `ticks_max_requests` times, which defaults to forty million. Each tick steps the network, each client, each replica's storage and each replica, in a fixed order. Each replica has a `TimeSim` whose monotonic clock is `ticks * resolution`, where the resolution is `constants.tick_ms`. Timeouts inside the replica count ticks.

The tick model has real advantages. Time inside a replica is a counter, so there is no scheduler to get wrong. Every component acts in every tick in a fixed order, so there are no ties to break. The cost of a tick does not depend on what happens in it. The costs are the mirror image. Idle time is not free, because an idle tick still runs every component. Nothing happens between two ticks, so the resolution of every timeout is one tick. A simulated year at 10 ms per tick is three billion ticks.

FoundationDB uses the event model. When its run loop has no ready task, it adds the time until the next timer to the clock and continues. It also adds a tiny random extra, so that the jump lands a little after the timer rather than exactly on it. Its `delay()` function adds a random extra to one call in four. Both extras come from the seed. The simulator perturbs its own timing on purpose, so that code which depends on exact timing fails under simulation. `turmoil` sits between the two models. Its `Sim::step` runs each host for one `tick_duration` and then steps the network.

This project uses the event model. The decision is recorded at the end of this lesson. The client already exposes `next_deadline`, so the scheduler has what it needs. The run time is proportional to the number of events, not to the span of virtual time. That is what makes a one-year test cheap. The tick model is available on top of it for free. `World::run_for(tick)` in a loop is the tick model. One of the acceptance tests runs the same seed both ways and asserts that the two traces are identical.

### Why a year matters

Some bugs need many expirations before they show. A backoff that reaches its cap only after ten retries. A counter of milliseconds in a `u32` that wraps after 49.7 days. A lease that expires in the same instant as the heartbeat that renews it. Under real time, a test that waits a year is not a test. Under virtual time it is half a million events.

The reference solution runs one simulated year of a retrying client twice, because the determinism check needs two runs. The two years take about two seconds in a debug build. A release build takes under half a second. That is more than one hundred million times real time. The number is not comparable to the figures in Module 0, which come from a full cluster with a storage model and a consensus protocol. In this world, each event is one heap pop and a few trace lines. The point is the shape of the cost, not the constant. A simulator that never sleeps is bounded by the number of events, and you choose how many events a year contains.

The trace is where the time goes. With `keep_trace_lines` on, the world stores a `String` per line. The one-year test turns it off and keeps only the running hash, which is what the determinism check compares anyway.

### Decisions recorded

- The project uses a pure event queue, not a tick loop. If a later module needs the tick model, `World::run_for` in a loop gives it.
- Events at the same instant run in schedule order. The heap key is `(Instant, seq)`. The payload never takes part in ordering.
- The `sim` crate depends on `kv-core` for `Instant` and `Duration`, and on nothing else. It is `no_std`.
- The fakes do not hold a handle to the scheduler. The world mediates between the driver and the queue.
- The `known_seed_fingerprint` hash changed in this module, because every trace line now carries a scheduler timestamp. It is `0xd982_fd72_6e9e_453a` for seed 1.

## Readings

Required:

- [Discrete-event simulation](https://en.wikipedia.org/wiki/Discrete-event_simulation) on Wikipedia. Read the sections on the clock, the event list and the two kinds of time progression.
- [SimPy: Basic Concepts](https://simpy.readthedocs.io/en/latest/simpy_intro/basic_concepts.html). This is a complete discrete-event simulation in twenty lines of Python. Note `env.timeout`, `env.run(until=15)` and `env.now`.
- [`std::collections::BinaryHeap`](https://doc.rust-lang.org/std/collections/struct.BinaryHeap.html). Read it for what is missing. It makes no promise about elements that compare equal.
- [`tokio::time::pause`](https://docs.rs/tokio/latest/tokio/time/fn.pause.html) and [`tokio::time::advance`](https://docs.rs/tokio/latest/tokio/time/fn.advance.html). The second page documents the tie problem from the runtime side.
- TigerBeetle: [`src/vopr.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vopr.zig) has the main loop and `ticks_max_requests`. [`src/testing/cluster.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/testing/cluster.zig) has `tick()`. [`src/stdx/time.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/stdx/time.zig) has `TimeSim`.
- FoundationDB: [`fdbrpc/sim2.cpp`](https://github.com/apple/foundationdb/blob/main/fdbrpc/sim2.cpp), search for `delay(` and `runLoop`, and [`flow/TaskQueue.h`](https://github.com/apple/foundationdb/blob/main/flow/include/flow/TaskQueue.h) for `getFIFOPriority` and `DelayedTask`.
- [`turmoil::Sim`](https://docs.rs/turmoil/latest/turmoil/struct.Sim.html), the `step` method. This is the hybrid of the two models.

Optional:

- [Will Wilson, Testing Distributed Systems w/ Deterministic Simulation](https://www.youtube.com/watch?v=4fFDFbi3toc). The FoundationDB talk from Strange Loop 2014. The simulator section explains `delay()` and the speedup.
- [`madsim::time`](https://docs.rs/madsim/latest/madsim/time/index.html). A drop-in `tokio::time` on a virtual clock.
- [sled: simulation guide](https://sled.rs/simulation.html), from Module 3, now that you build the loop it describes.

## Coding task

### Starting point

Start from your Module 3 workspace, or from `solutions/03-rust-hazards/`. The acceptance tests are in `modules/04-virtual-time/tests/`: `sim/scheduler.rs` and `sim/timer.rs` go into `crates/sim/tests/`, and `kv-env/fake_world.rs` and `kv-env/virtual_time.rs` replace and extend `crates/kv-env/tests/`. The updated `scripts/check.sh` is in `modules/04-virtual-time/scripts/`.

### Requirements

#### The `sim` crate

1. Add a crate `sim`. It is `#![no_std]` with `extern crate alloc`, `#![forbid(unsafe_code)]`, and the same lint attributes as `kv-core`. It depends on `kv-core` and on nothing else. It builds for `thumbv7em-none-eabihf`.
2. Implement `Scheduler<E>` with this surface:
   - `new()`, `starting_at(Instant)`, `clock() -> Clock`, `now()`, `len()`, `is_empty()`, `stats()`.
   - `schedule_at(Instant, E) -> EventId` and `schedule_after(Duration, E) -> EventId`. Scheduling in the past panics with a message that contains `scheduled in the past`. Scheduling at `now` is allowed.
   - `pop() -> Option<Fired<E>>`. It returns the earliest event, moves the clock to its instant, and counts it as fired. `Fired` has public fields `at`, `id` and `event`.
   - Events at the same instant pop in the order they were scheduled, whatever their payload.
   - `peek() -> Option<Instant>` and `pop_until(Instant) -> Option<Fired<E>>`. The second pops only an event due at or before the bound. It does not move the clock to the bound.
   - `advance_to(Instant)`. If no event is due before the target, it moves the clock. If one is, it panics with a message that contains `would skip an event`.
   - `cancel(EventId) -> bool`. It returns true once for a live event and false for an event that fired, was cancelled, or never existed. `len()` does not count cancelled events.
   - `SchedulerStats { scheduled, fired, cancelled }`.
3. Implement `Clock`. It is `Clone`, it implements `kv_core::env::Time`, and every clone of one scheduler's clock reads the same instant. Only the scheduler can move it.
4. Implement `Timer` with `new()`, `deadline()`, `reconcile(&mut Scheduler<E>, Option<Instant>, impl FnOnce() -> E)` and `fired(EventId) -> bool`. If the wanted deadline is already armed, `reconcile` schedules nothing. It cancels the old event before it schedules a new one.

#### The simulated environment

5. Delete `FakeTime`. The driver's time type is `sim::Clock`.
6. `FakeStorage` loses `auto_complete`. It has `new()`, `with_log(Vec<u8>)`, `complete_next()`, `pending()` and `submitted()`. The last one counts every append ever submitted.
7. Add `Event` with the four variants from the lesson. Add `WorldConfig` with `seed`, `net_latency`, `net_jitter`, `disk_latency`, `disk_jitter` and `keep_trace_lines`. Add `WorldStats` with `timers_fired`, `requests_delivered`, `requests_dropped`, `replies_delivered`, `replies_dropped` and `appends_completed`. The defaults are seed 0, 1 ms of network latency, 5 ms of disk latency, no jitter, and lines kept.
8. Rebuild `World` on the scheduler.
   - Public fields: `driver`, `scheduler`, `prng`, `config`, `clients`, `completed`, `gave_up`, `drop_replies`, `partitioned` and `stats`.
   - Setup and query methods: `new(FakeStorage)`, `with_config(FakeStorage, WorldConfig)`, `add_client`, `now`, `trace`, `submit`, `call` and `take_completed`.
   - Run methods: `step`, `run_until_idle`, `run_until(Instant)`, `run_for(Duration)` and `run_while(impl FnMut(&World) -> bool) -> bool`.
   - `with_config` runs the driver once, so that the node finishes recovery before the first event.
   - `submit` sends the request, reconciles the client's timer, and returns the request id.
   - `step` pops one event and handles it. If the queue is empty, it returns false.
   - `run_until` handles every event due at or before the bound, then calls `advance_to(bound)`.
   - `call` submits, runs until that request completes or gives up, and returns the response. If the client gave up or the world went idle first, it panics.
   - While `partitioned` is true, every client request is dropped on its way to the node. `drop_replies` works as in Module 2.
   - Disk completions are scheduled in submission order, at `max(now + latency, previous completion)`.
   - Every client input is followed by a `reconcile` of that client's timer.
9. Write these trace lines, in addition to the Module 2 lines, where `{now}` is the `Display` form of the current `Instant`:
   - When a client timer fires: `{now} timer client={id}`.
   - When a request is scheduled for delivery: `{now} net send client={id} id={rid}`.
   - When `partitioned` drops a request: `{now} net dropped request client={id} id={rid}`.
   - On delivery: `{now} net deliver request client={id} id={rid}` and `{now} net deliver reply client={id} id={rid}`.
   - When `drop_replies` drops a reply: `{now} net dropped reply client={id} id={rid}`.
   - When an append becomes durable: `{now} disk complete`.
10. Port the Module 2 tests to the new API as `fake_world.rs`. The seeded workload now draws its world seed from the test's `Prng`, with 5 ms of network jitter and 20 ms of disk jitter. The fingerprint for seed 1 is `0xd982_fd72_6e9e_453a`.
11. Add the `sim` crate to the bare-metal check in `scripts/check.sh`.

### Acceptance criteria

```sh
scripts/check.sh                                # ends with "all checks passed"
cargo tree -p sim                               # sim and kv-core, nothing else
cargo test --release -p kv-env --test virtual_time one_simulated_year -- --nocapture
```

The last command prints a line that starts with `two simulated years in`. In a release build the time is under one second. In a debug build it is a few seconds, because overflow checks and debug assertions stay on. The `fake_world.rs` tests pass unchanged, with the new fingerprint.

### Hints

<details>
<summary>The heap entry</summary>

Wrap each entry in a struct with `at`, `seq` and `event`, and write `Ord` by hand on `(at, seq)`. Do not derive it. A derived `Ord` includes `event` and requires `E: Ord`. Push `Reverse(entry)` so that `BinaryHeap` becomes a min-heap. `PartialEq` and `Eq` must agree with `Ord`, so write them on the same pair.

</details>

<details>
<summary>Cancel without removal</summary>

Keep a `BTreeSet<u64>` of live sequence numbers next to the heap. `schedule` inserts, `cancel` removes, and `pop` removes. Before `peek` and `pop` look at the top of the heap, discard entries whose number is not in the set. Everything is logarithmic and nothing scans the heap.

</details>

<details>
<summary>The borrow checker in the routing loop</summary>

The `drain` loop borrows `self` for its whole body. The body also needs `&mut self` to schedule. Replace the drain with `std::mem::take(&mut self.client_out)` and iterate the owned `Vec`.

</details>

<details>
<summary>Where the node's recovery runs</summary>

The driver issues the log read at construction, and `FakeStorage` completes reads at once. Nothing in the scheduler represents that. Run the driver once at the end of `with_config`, and after every event that feeds it. The reference solution has one `pump` function. It runs the driver until idle. Then it schedules a `DeliverReply` for each reply in the outbox and a `DiskComplete` for each new append.

</details>

<details>
<summary>Finding your fingerprint</summary>

Set the constant to zero, run the test, and read the hash from the assertion message. Before you adopt it, make sure that `seeded_workload_matches_model_and_is_deterministic` passes. A fingerprint only means something once the trace is stable.

</details>

### Stretch goals

- Add a seeded tie-break. Among events at the same instant, pop in an order chosen by the `Prng` instead of schedule order. Keep every test passing. Then say which trace lines changed and why.
- Replace the heap with a `BTreeMap<(Instant, u64), E>`. Compare the two on one million events in a release build.
- Add FoundationDB's run-loop jitter. Under a configuration flag, add a tiny random extra to every `schedule_after`. The fingerprint changes. The model test must still pass.
- Give each client its own `TimeSim`-style offset, so that `Tick { now }` carries a skewed clock, and see which client code notices.

## Check your understanding

1. Why must the event payload stay out of the heap key?
2. A timer and a reply land at the same instant. Which one runs first in your scheduler, and why is "either, as long as it is always the same" the right answer?
3. `run_until`, and so `run_for`, ends with `advance_to`. Suppose it did not. What breaks?
4. The one-year test runs at more than one hundred million times real time. Module 0 quoted about a thousand. Which is right?
5. The disk model schedules each completion at `max(now + latency, previous)`. What invariant does that protect, and which code asserts it?
6. In the tick model, what happens to an event that is due 3 ms into a 10 ms tick?
7. `schedule_at` panics on an instant in the past. Why is a clamp to "now" the wrong fix?

<details>
<summary>Answers</summary>

1. The payload's `Ord` is an accident of how it is written. A change to a field, a variant order or a `Debug` string then changes the schedule of every seed. The key must name only what the simulation means by "earlier", which is the instant and the order of scheduling.
2. In the reference solution the one that was scheduled first runs first. Both orders are legal for a real system, so the model is not wrong either way. What matters is that the same seed makes the same choice on every run. The choice is written down, not left to a data structure.
3. The clock stays at the last event, not at the bound. A later `schedule_after` then computes its instant from the wrong base, and `now()` lies to the test. The tick-model test also fails, because the two drivers no longer agree on the time at the end of each tick.
4. Both are right about different worlds. This world has one node, one client and a constant-latency network, so an event is a heap pop and a few trace lines. A full cluster simulation does real work per event. The ratio is a property of the model, not of the scheduler.
5. The node asserts that appends complete in submission order, because it computes log offsets under that assumption. With jitter, a later append can draw a shorter latency than an earlier one. The `max` keeps completion order equal to submission order. Module 6 gives the disk model faults and revisits this.
6. That depends on the loop. A loop that only ticks runs it at the end of the tick, with the clock at 10 ms. A loop that pops due events at their own instant runs it at 3 ms. TigerBeetle's replicas see the first behavior, since time inside a replica is a count of ticks. The `World::run_for` loop sees the second, because the scheduler pops each event at its own instant and only moves to the bound afterwards.
7. An event in the past has one of two causes. A component computed a deadline from a stale clock, or the world handled events in the wrong order. A clamp runs the event late and hides the cause. The assertion fails on the seed that found the bug, which is the whole point of the simulator.

</details>

## What's next

The world now has a clock that only the scheduler moves, and every delay in it is an event. The network in it is still polite. Messages arrive in order, after a fixed latency, and are never lost unless a test asks. Module 5 replaces it with a network model where the seed decides latency, reordering, duplication, loss and partitions. Three nodes pass heartbeats through it. The scheduler you built here is the only thing it needs.
