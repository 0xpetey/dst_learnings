# Module 5: The simulated network

## Why this module exists

The network in Module 4 was polite. Every message took one millisecond, arrived in order, and was never lost unless a test flipped a flag. Real networks are not like that. Kyle Kingsbury and Peter Bailis collected the public record in "The network is reliable". A switch firmware bug blocks traffic for ninety seconds. A network card drops inbound packets and keeps sending heartbeats, so the hot spare never takes over. A primary loses contact with its two secondaries while every other link stays up. None of these shapes appear in a model where delivery is a constant.

The point of simulation testing is an adversarial environment. This module builds the adversary for the network. It is one component that decides the fate of every message: when each copy arrives, or that none does. The seed drives every decision, so a run that finds a bug can be run again.

To give the faults something to act on, the world grows from one host to three. Each host sends heartbeats to the others and keeps a view of who is alive. That view is the first distributed state in the course, and it is wrong in instructive ways. A partition looks like a crash. A slow link looks like a crash. A one-way cut makes two nodes disagree about a third. Module 9 builds failover on this view. Module 10 builds consensus on it. The network model does not change again after this module. Module 7 only adds knobs to it.

## Objectives

By the end of this module you can:

- Model a network as a set of decisions about delivery events, with the scheduler as the only queue.
- Name the faults in the model and implement each one in a few lines: latency, loss, duplication, reordering, and symmetric, asymmetric and partial partitions.
- Explain why reordering is a delay and why constant latency gives in-order delivery.
- Keep a ledger of messages in flight, so that the network never delivers what nobody sent.
- Decide whether a partition applies at send time or at delivery time, and defend the choice.
- Build a heartbeat failure detector and read its mistakes as information about the network.
- Draw a fault configuration from the seed and show that the same seed gives the same run.
- Say where realism stops paying for itself.

## Lesson

### The network is another priority queue

Module 4 turned every delay into an event with a timestamp. A message is a delay with a destination. The network model therefore owns no queue. It owns a decision:

```text
   world.transmit(from, to, message)
        │
        ▼
   net.send(now, from, to, prng) ──► Err(Dropped)      trace it, no event
        │
        └──► Ok([Delivery { id, at, copy }, ...])      one scheduler event per copy
                                       │
                                       ▼  (clock reaches `at`)
   net.deliver(id) ──► Err(Partition)  the link was cut in the meantime
        │
        └──► Ok((from, to))            hand the message to its destination
```

`send` rolls the dice and returns a list of arrival instants. The world schedules one `Deliver` event per instant. When an event fires, the world asks `deliver` whether the copy still arrives. The scheduler remains the only place where time passes.

The other simulators differ here. TigerBeetle's `PacketSimulator` keeps a queue per path, with a `ready_at` on each packet. Its `step` function walks every path and pops what is due. That fits a tick loop. FoundationDB has no network queue. A simulated connection calls `delay()`, and the delay lands on the main scheduler. `turmoil` keeps an in-flight deque per link and steps it once per tick. The design here follows FoundationDB. One queue means one timeline. The send line in the trace already names the arrival instant of each copy.

### One address space

The world now has nodes and clients on the same network. `Addr::Node(id)` and `Addr::Client(id)` share one type, and a request, a reply and a heartbeat are all a `Message` from one address to another. The `partitioned` flag and the `drop_replies` list from Module 4 are gone. A test cuts a link or asks the network to lose the next message on it, with the same calls that the seed uses. TigerBeetle's cluster test code makes the same choice. Its `Network` maps each replica and client to a `u8` address, with replicas in the lowest slots.

### The fault model, one fault at a time

| Fault | What it looks like in production | The model |
|---|---|---|
| Latency | Queues in switches, route changes | Uniform draw from `latency_min..=latency_max` |
| Loss | Full buffers, a bad cable, a firewall | Per message, with probability `loss`, no event is scheduled |
| Duplication | TCP retransmits after a late ack, replayed UDP | Per message, with probability `duplicate`, a second copy with its own latency draw and the same id |
| Reordering | Two paths through the fabric, a retransmit | Per copy, with probability `reorder`, an extra delay in `0..=reorder_delay_max` |
| Partition | A cut cable, a misconfigured switch, a dead NIC | A set of cut directed links. A send on a cut link is dropped. A copy in flight at the moment of the cut is dropped too |

Each one is a few lines. The whole model is under four hundred, with its documentation. Three of the rows need a comment.

Latency has a distribution, and the shape of the distribution matters less than its tail. TigerBeetle draws from an exponential distribution with a minimum. FoundationDB's `halfLatency` gives one connection in a thousand a slow-tail latency. The model here is uniform, which is the simplest thing with a tail, and the reordering fault supplies a longer one. A stretch goal replaces the draw.

Reordering is a delay. The Linux `tc netem` manual says it plainly in its limitations section: "For any method of reordering to work, some delay is necessary". If the jitter exceeds the gap between sends, jitter alone reorders messages. An explicit knob gives the model a tail that jitter does not, and lets a test turn it on alone.

In-order delivery is not a knob. It follows from constant latency. If every message on a link takes exactly `d`, the one sent first arrives first. `NetConfig::reliable` sets `latency_min` equal to `latency_max` and every probability to zero. That is the configuration in which the acceptance test demands order and no loss.

### Partition shapes

A partition in the model is a set of cut directed links. One mechanism covers every shape in the catalogue:

```text
   symmetric            asymmetric           partial (bridge)
   n0 ─x─ n1            n0 ──► n1 cut        n0 ─── n1 ─── n2
   n0 ─x─ n2            n1 ──► n0 open       n0 ─x─ n2
   n1 ─── n2            n1 ─── n2            (n1 sees everyone)
```

`Net::cut(from, to)` cuts one direction. `partition(a, b)` cuts both. `split(side_a, side_b)` cuts every link between two sides. The `Shape` enum names the ones Jepsen found worth testing. Its `nemesis` module has `partition-random-node`, `partition-random-halves` and `bridge`. It also has `partition-majorities-ring`, in which "every node can see a majority, but no node sees the *same* majority as any other". `Shape::OneWay` adds the asymmetric case, which Jepsen builds from a one-directional grudge and Kingsbury's survey records as a dead network card that kept sending heartbeats.

### Send time or delivery time?

The cable is cut while a message is in flight. Does it arrive? In the model it does not. `send` checks the link at the moment the message leaves. `deliver` checks it again at the moment the copy is due. Either check drops the copy and counts it under `partitioned`. TigerBeetle checks only at delivery time, in `submit_packet_finish`. Checking at both ends costs nothing and keeps the ledger honest, because a message on a cut link is never scheduled at all.

This choice makes the second assertion of the module simple. Partitioned nodes never exchange messages, and the invariant is checked at the moment of delivery, in one place.

### The ledger

The network keeps a map from message id to the endpoints and the number of copies still in flight. `send` inserts. `deliver` decrements and removes at zero. A `deliver` for an id that is not in the map panics with a message that contains `never sent`.

The ledger does not guard against an attacker. It guards against the simulator. A world that clones an event, handles a stale one, or routes a message to the wrong host has a bug. From inside a node, that bug looks exactly like a bug in the system under test. The ledger turns it into a panic at the right place. The acceptance test takes a real delivery event out of one world and drops it into another world's queue. The second world panics. A second test schedules the same copy twice. The second arrival panics.

### Heartbeats: the network seen from inside a node

Each host runs a second state machine next to its KV node. `Heartbeat` sends `Beat { seq }` to every peer once per `interval`. It marks a peer `Up` on any beat and `Down` after `suspect_after` of silence. Silence is measured from the last beat heard. Before the first beat, it is measured from the node's start. Like the client, it never reads a clock. Every input carries `now`. The `next_deadline` method tells the world the instant of the next tick. The `sim::Timer` helper from Module 4 keeps the two in step without a change.

```text
   n0 ──Beat{1}──► n1      t=0       n1: n0 Unknown ──► Up
   n0 ──Beat{2}──► n1      t=100ms
   n0 ─x─ cut
   n0 ──Beat{3}──► n1      t=200ms   dropped
   n0 ──Beat{4}──► n1      t=300ms   dropped
                           t=451ms   n1: n0 Up ──► Down   (101ms + 350ms)
```

The receiver counts two things that only a faulty network produces. A beat whose `seq` it has already seen is a duplicate. A beat whose `seq` is below one it has seen is a straggler. Both still count as evidence that the peer is alive. The counters appear in the stats, so a test can show that the network did what the seed asked, from the inside.

Chandra and Toueg named the two properties of a failure detector. Completeness says that a crashed process is eventually suspected. Accuracy says that a correct process is not. This detector is complete and not accurate. A partitioned peer is suspected after `suspect_after`. A peer behind a slow link is suspected too, and that is a false suspicion. A slow message and a dead sender look the same, so perfect accuracy is impossible. The detector does not try. The constraint it does need is `suspect_after > interval`, which the constructor asserts. The acceptance test with heavy duplication and reordering shows the arithmetic. A beat is at most 205 ms late, the next one is at least 101 ms behind it, so the longest silence is 304 ms. That is under 350 ms, and no node flaps.

A node keeps beating at a peer it suspects. If it stopped, that side never notices a healed partition.

### Partitions from the seed

Two things in the world come from the seed. `NetConfig::random` draws a latency range, a loss rate, a duplication rate and a reordering rate, in a fixed order. `PartitionSchedule` adds random partitions over time. After each heal, the next partition starts within `gap_max`. It applies a random `Shape` over the hosts and lasts up to `hold_max`. The world heals everything before it applies the next shape, so one shape is in force at a time.

`World::stop_partitions` cancels the schedule and heals every link. The chaos test calls it and sets the network to reliable. Then it runs for one suspicion window plus one beat. Every node must then see every other node `Up`. This is the quiet period. Module 7 turns it into the liveness check that ends every run.

### Reading a trace

Seed 3, three hosts, one client, duplication, reordering and loss on. The client puts a key at time zero. The world cuts `n0` off at 120 ms.

```text
t=0ns net dropped n0->n1 beat seq=1 loss
t=0ns net send n0->n2 m0 beat seq=1 at=t=3601043ns
t=0ns net send n1->n0 m1 beat seq=1 at=t=1540558ns
t=0ns net send c1->n0 m3 request client=1 id=1 at=t=1597604ns
t=1540558ns net deliver n1->n0 m1 beat seq=1
t=1540558ns n0 peer up n1
t=1597604ns net deliver c1->n0 m3 request client=1 id=1
t=1597604ns n0 in  Request client=1 id=1 Put { key: [107], value: [118] }
t=1597604ns n0 out Append op=1 offset=0 bytes=15
t=6597604ns n0 disk complete
t=6597604ns n0 out Reply client=1 id=1 PutOk
t=6597604ns net send n0->c1 m4 reply client=1 id=1 at=t=8643211ns dup=t=7796068ns
t=7796068ns net deliver n0->c1 m4 reply client=1 id=1
t=7796068ns client=1 done id=1 PutOk
t=8643211ns net deliver n0->c1 m4 reply client=1 id=1
...
t=200000000ns net dropped n0->n1 beat seq=3 partition
t=351597868ns n0 peer down n2
```

Three things to notice. The first beat from `n0` to `n1` is lost at time zero, and no event exists for it. The reply `m4` is duplicated, and the duplicate lands first, at 7.8 ms, because each copy draws its own latency. The client answers once and ignores the second copy. The last line is a suspicion. `n0` heard `n2` at 1.597868 ms. The beat at 100 ms was lost and the beat at 200 ms hit the cut. 350 ms after the last beat, `n0` gives up on `n2`.

The send line carries the planned arrival of every copy. When a seed fails, the trace says what the network did to each message without a debugger.

### Realism versus reproducibility

The model leaves things out, on purpose.

- Bandwidth and capacity. When a path holds more than `path_maximum_capacity`, TigerBeetle drops a random packet. `turmoil` has `tcp_capacity`. The model here has no limit. No code in this course depends on back-pressure yet.
- Connections. FoundationDB simulates TCP connections, not packets. Its `Sim2Conn` closes a connection at random, clogs a pair of hosts for a while, and never reorders bytes within a connection. The model here is packet-shaped, because the KV protocol is request and response and the heartbeat is one message.
- Bytes. The real environment sends frames over TCP. The simulated one sends `Message` values. The frame codec is tested in Module 2 on its own. A simulator that encoded and decoded every message finds codec bugs, and every seed runs slower for it.
- Clogs. TigerBeetle holds a path for a random duration. `turmoil` holds a link until `release`. Both are a delay applied to a link instead of a message. The reordering fault approximates one, and a stretch goal adds the real thing.

The rule is the one from the syllabus. When a class of bug needs more fidelity, add it. Record what the model does not cover. Module 13 returns to that list.

### Decisions recorded

- The `sim` crate now depends on `determinism` with `default-features = false`, for `Prng`. It is still `no_std` and still builds for `thumbv7em-none-eabihf`. `cargo tree -p sim` shows three crates.
- The network decides and the scheduler queues. `Net` holds no message and no queue, only configuration, link state, the ledger and counters.
- A cut link is checked at send time and at delivery time. A message in flight at the moment of the cut is lost.
- Reordering is an extra delay with a probability. In-order delivery is a consequence of constant latency, not a flag.
- One trace serves every host. Each driver's observer holds an `Rc<RefCell<Trace>>` and tags its lines with the host id.
- The heartbeat is a state machine of its own on each host. The KV node does not see it until Module 9.
- `World::run_until_idle` runs until no client has a request in flight, no request or reply is on the wire, and no append waits for a disk. Heartbeats do not count as work, because they never stop.
- The `known_seed_fingerprint` hash is `0x2fcf_d1bb_e1bb_c8ef` for seed 1. Trace lines carry host ids and message ids, and the workload runs under random faults. The new `known_cluster_fingerprint` is `0x5dae_82b3_b044_4863`.

## Readings

Required:

- [Kyle Kingsbury and Peter Bailis, "The network is reliable"](https://aphyr.com/posts/288-the-network-is-reliable). The catalogue of real partitions. Read it for the shapes, and for the network card that kept sending heartbeats.
- TigerBeetle: [`src/testing/packet_simulator.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/testing/packet_simulator.zig). Read `PacketSimulatorOptions`, `PartitionMode`, `submit_packet` and `submit_packet_finish`. Then [`src/testing/cluster/network.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/testing/cluster/network.zig) for `process_to_address` and `transition_to_liveness_mode`.
- [`turmoil::Sim`](https://docs.rs/turmoil/latest/turmoil/struct.Sim.html), the `partition`, `partition_oneway`, `repair`, `hold` and `release` methods. Then [`turmoil::Builder`](https://docs.rs/turmoil/latest/turmoil/struct.Builder.html) for `min_message_latency`, `max_message_latency` and `fail_rate`. A Rust reference design for the same API.
- Jepsen: [`jepsen/src/jepsen/nemesis.clj`](https://github.com/jepsen-io/jepsen/blob/main/jepsen/src/jepsen/nemesis.clj). Search for `partition-random-halves`, `partition-random-node`, `bridge` and `majorities-ring`. The docstrings are the catalogue of shapes.
- [`tc-netem(8)`](https://manpages.debian.org/testing/iproute2/tc-netem.8.en.html). Read the `reorder`, `duplicate` and `loss` options and the limitations section. This is how a real kernel reorders packets on request.

Optional:

- FoundationDB: [`fdbrpc/sim2.cpp`](https://github.com/apple/foundationdb/blob/main/fdbrpc/sim2.cpp). Search for `Sim2Conn`, `rollRandomClose`, `SimClogging` and `halfLatency`. A connection-level model, for contrast. The [testing page](https://apple.github.io/foundationdb/testing.html) adds one paragraph on "swizzle-clogging".
- [`madsim::net::Config`](https://docs.rs/madsim/latest/madsim/net/struct.Config.html) and [`madsim::net::NetSim`](https://docs.rs/madsim/latest/madsim/net/struct.NetSim.html). A two-field fault model and a `clog_link` API.
- [Chandra and Toueg, "Unreliable Failure Detectors for Reliable Distributed Systems"](https://www.cs.utexas.edu/~lorenzo/corsi/cs380d/papers/p225-chandra.pdf). Read section 2 for completeness and accuracy. The rest is for Module 10.
- [Jepsen tutorial, "Introducing Faults"](https://github.com/jepsen-io/jepsen/blob/main/doc/tutorial/05-nemesis.md). How a nemesis schedule looks in a real Jepsen test.

## Coding task

### Starting point

Start from your Module 4 workspace, or from `solutions/04-virtual-time/`. The acceptance tests are in `modules/05-sim-network/tests/`. `sim/net.rs` goes into `crates/sim/tests/`. `kv-core/heartbeat.rs` goes into `crates/kv-core/tests/`. The three files under `kv-env/` replace `crates/kv-env/tests/fake_world.rs` and `virtual_time.rs` and add `sim_network.rs`. `scripts/check.sh` does not change.

### Requirements

#### The network model, in `sim::net`

1. Add `determinism` to `sim` with `default-features = false`. The crate stays `no_std` and still builds for `thumbv7em-none-eabihf`.
2. `Chance` is a probability as `num / den`. It has `new`, `percent`, `NEVER`, `ALWAYS`, `is_never`, `roll(&mut Prng) -> bool` and a `Display` of the form `1/4`.
3. `NetConfig` has public fields `latency_min`, `latency_max`, `loss`, `duplicate`, `reorder` and `reorder_delay_max`. `reliable(latency)` is constant latency and no faults. `random(&mut Prng)` draws a configuration in a fixed order. `latency_min` is 0 to 4 ms and `latency_max` is at most 50 ms above it. `loss` and `reorder` are below 20 percent, `duplicate` is below 10 percent, and `reorder_delay_max` is 10 to 199 ms. `validate` panics with a message that contains `latency_min` on an inverted range. `Default` is `reliable(1 ms)`.
4. `MessageId` displays as `m{n}`. `Delivery` has public `id`, `at` and `copy`. `Dropped::{Partition, Loss}` display as `partition` and `loss`.
5. `Net<A>` for any `A: Copy + Ord + Debug`:
   - `new(NetConfig)`, `config()`, `set_config(NetConfig)`, `stats() -> NetStats`, `in_flight() -> usize`.
   - Link state: `is_connected`, `cut`, `restore`, `partition` and `heal`. Also `split`, `isolate`, `heal_all`, `cuts()` and `apply(&Shape<A>)`. The test knob is `drop_next(from, to)`.
   - `send(now, from, to, &mut Prng) -> Result<Vec<Delivery>, Dropped>`. The order of checks is cut link, `drop_next`, loss dice, then one latency draw per copy, with the reordering dice rolled once per copy. A duplicate shares the original's id and has `copy = 1`. Sending to yourself panics with a message that contains `talk to itself`.
   - `deliver(MessageId) -> Result<(A, A), Dropped>`. It decrements the copies in flight. If the link is cut now, it returns `Err(Partition)`. An id that is not in the ledger panics with a message that contains `never sent`.
6. `NetStats` counts `sent`, `delivered`, `duplicated`, `reordered`, `lost` and `partitioned`. A loss at send time and a `drop_next` both count under `lost`. A cut at send time and a cut at delivery time both count under `partitioned`.
7. `Shape<A>` has `Isolate { node, rest }`, `Halves { a, b }`, `Bridge { bridge, a, b }`, `OneWay { from, to }` and `Ring { order }`. `Shape::random(&mut Prng, &[A])` shuffles the nodes and draws a kind. Two nodes never draw `Bridge` or `Ring`, and three never draw `Ring`. `Display` writes `halves [0 1] | [2]`, `one-way [0] -x-> [1]`, `isolate 0`, `bridge 1 [0] | [2]` and `ring [0 1 2 3]`. `Ring` cuts every pair that is not adjacent in `order`.

#### The heartbeat, in `kv_core::heartbeat`

8. `NodeId` is `u8`. `Config` has `interval` and `suspect_after`, with defaults of 100 ms and 350 ms. `Message::Beat { seq }`, `Input::{Message { from, message }, Tick}`, `Output::{Send { to, message }, PeerUp, PeerDown}` and `Liveness::{Unknown, Up, Down}` are as in the lesson.
9. `Heartbeat::new(me, peers, config, now, out)` asserts that `interval` is positive and below `suspect_after`, that `me` is not a peer, and that no peer repeats. It pushes the first round of beats into `out`. If `now` goes backwards, or a beat comes from a non-peer, `on_input(now, input, out)` panics. `next_deadline()` is the earlier of the next beat and the earliest suspicion, or `None` with no peers. A late tick sends one beat. `liveness(peer)`, `up_peers()`, `peers()` and `stats()` are queries. `Stats` counts `sent`, `received`, `duplicates`, `reordered`, `ups` and `downs`.

#### The world, in `kv_env::fake`

10. `Addr::{Node, Client}` displays as `n0` and `c1`. `Message::{Request, Reply, Heartbeat}` displays as `request client=1 id=2`, `reply client=1 id=2` and `beat seq=3`. `Event` has `ClientTimer`, `NodeTimer`, `Deliver { id, from, to, message }`, `DiskComplete { node }`, `Partition` and `Heal`.
11. `WorldConfig` has `seed`, `net`, `heartbeat`, `disk_latency`, `disk_jitter`, `keep_trace_lines` and `partitions: Option<PartitionSchedule>`. `PartitionSchedule` has `gap_max` and `hold_max`. `WorldStats` counts `client_timers_fired`, `node_timers_fired`, `requests_delivered`, `replies_delivered`, `beats_delivered`, `appends_completed`, `partitions` and `heals`.
12. `World` has public fields `hosts`, `net`, `scheduler`, `prng`, `config`, `clients`, `completed`, `gave_up` and `stats`. `Host` has public `id`, `driver` and `heartbeat`. Constructors are `new(FakeStorage)`, `cluster(n, WorldConfig)` and `with_storages(Vec<FakeStorage>, WorldConfig)`. `add_client(id, config, node)` names the node the client talks to. `trace()` returns the shared trace. `is_idle()`, `node_addrs()` and `stop_partitions()` are new. `submit`, `step`, `run_until`, `run_for`, `run_while`, `call` and `take_completed` keep their Module 4 meaning. `run_until_idle` runs until `is_idle()`.
13. Every message goes through `net.send` and `net.deliver`. A dropped message is traced and never scheduled. A copy that `deliver` rejects is traced as dropped. A delivered request goes to the host's `FakeNetwork`, a delivered reply to the client, and a delivered beat to the host's `Heartbeat`. Any other pairing panics.
14. With a `PartitionSchedule`, the first `Partition` event is scheduled within `gap_max` of the start. On `Partition`, the world heals everything, applies `Shape::random` over the hosts, and schedules `Heal` within `hold_max`. On `Heal`, it heals everything and schedules the next `Partition`. `stop_partitions` cancels the pending event and heals everything.
15. Trace lines, where `{now}` is the `Display` form of the current `Instant`:
    - Driver lines gain a host tag: `{now} n{id} in  Request ...`, `{now} n{id} out Reply ...`.
    - `{now} net send {from}->{to} {mid} {message} at={t}` with ` dup={t}` appended for a second copy.
    - `{now} net dropped {from}->{to} {message} {reason}` at send time, and `{now} net dropped {from}->{to} {mid} {message} {reason}` at delivery time.
    - `{now} net deliver {from}->{to} {mid} {message}`.
    - `{now} timer client={id}`, `{now} timer node={id}`, `{now} n{id} disk complete`.
    - `{now} n{id} peer up n{peer}`, `{now} n{id} peer down n{peer}`.
    - `{now} net partition {shape}`, `{now} net heal`, and `{now} net heal (partitions stopped)`.
16. The seeded workload in `fake_world.rs` draws its `NetConfig` from the test's `Prng` with `NetConfig::random`. The fingerprint for seed 1 is `0x2fcf_d1bb_e1bb_c8ef`. The chaos fingerprint in `sim_network.rs` is `0x5dae_82b3_b044_4863`.

### Acceptance criteria

```sh
scripts/check.sh                                # ends with "all checks passed"
cargo tree -p sim                               # sim, determinism and kv-core, nothing else
cargo tree -p kv-core                           # kv-core alone, plus determinism as a dev-dependency
cargo test --release -p kv-env --test virtual_time one_simulated_year -- --nocapture
```

The last command still prints `two simulated years in` under one second in a release build. The reference solution takes about 0.6 s, a little more than Module 4, because every dropped request now goes through the network model. The chaos test runs twenty seeds three times each in about a second in release.

### Hints

<details>
<summary>The ledger</summary>

A `BTreeMap<u64, InFlight>` where `InFlight` holds `from`, `to` and a `copies` count. `send` inserts with `copies` equal to the length of the deliveries. `deliver` looks the id up, decrements, and removes the entry at zero. Do the lookup before the cut check, so that a dropped copy still leaves the ledger.

</details>

<details>
<summary>One trace for several drivers</summary>

The `Driver` owns its observer, and there are now several drivers. Give each `TraceObserver` an `Rc<RefCell<Trace>>` and a `node` field. The world keeps one more clone and exposes it through `trace()` as a `Ref<'_, Trace>`. A `for line in w.trace().lines()` loop works, because the temporary lives for the whole loop. Write a small `log(&self, args: fmt::Arguments)` helper on the world that borrows the cell mutably for one line.

</details>

<details>
<summary>The borrow checker in <code>pump</code></summary>

The outbox lives inside a host and `transmit` needs `&mut self`. Drain the outbox into a `Vec` first, and read the number of new appends into a local. Then loop over the `Vec` and call `transmit`. The host borrow is over by then.

</details>

<details>
<summary>Idle means no client work</summary>

Heartbeats never stop, so "until the queue is empty" never returns in a cluster. Count the scheduled `Deliver` events that carry a request or a reply. Increment the count at scheduling time. Decrement it at firing time, whether or not `deliver` accepts the copy. When that count is zero, every client is idle and no host has an append pending, the world is idle.

</details>

<details>
<summary>Which event is the real one</summary>

The two ledger tests pop events out of `w.scheduler` by hand. The first `Deliver` event in a fresh cluster is a beat due at 1 ms. The other five beats of that round are due at the same instant. If you re-schedule a copy and then call `step` twice, you handle two of the other five. Run for a millisecond instead, so that every event at that instant is handled.

</details>

<details>
<summary>Finding your fingerprints</summary>

Set each constant to zero, run the test, and read the hash from the assertion message. Make sure that `seeded_workload_matches_model_and_is_deterministic` and `chaos_is_a_pure_function_of_the_seed` pass first. A fingerprint only means something once the trace is stable.

</details>

### Stretch goals

- Replace the uniform latency with TigerBeetle's exponential distribution and a minimum. Keep every test passing, and say which assertions had to change.
- Add a clog. `Net::clog(from, to, until)` holds every message on the link until `until`, and the seed schedules clogs the way it schedules partitions. Decide whether a clog needs a queue inside the network, or whether a delay computed at send time is enough.
- Add `path_maximum_capacity`. When more than N copies are in flight on a link, a new send drops one of them at random. Which assertion in the module does this interact with?
- Run five hosts under `Shape::Ring`. Every node sees a majority. Write down what a leader election built on `up_peers()` does in that shape, before Module 10 shows you.
- Give the heartbeat a `Ping`/`Pong` round trip and estimate the latency of each link. Use the estimate to set `suspect_after` per peer, in the spirit of the phi accrual detector.

## Check your understanding

1. The network model holds no queue. Where do messages wait, and what does the model lose by this choice?
2. Why does `NetConfig::reliable` deliver in order with no flag that says so?
3. A partition starts while a beat is in flight. What happens to the beat, where is that decided, and what happens to the ledger?
4. The ledger panics on an id that was never sent. What class of bug does this catch, given that nothing in the simulator is an attacker?
5. The link from `n0` to `n1` is cut, and the link from `n1` to `n0` is not. After a second, who suspects whom?
6. The false-suspicion test uses `reorder_delay_max` of 200 ms with `suspect_after` of 350 ms. Why 304 ms, and what is the largest `reorder_delay_max` that cannot produce a false suspicion?
7. The Module 4 fingerprint changed in this module and the lesson says that is fine. When is a fingerprint change not fine?

<details>
<summary>Answers</summary>

1. They wait in the scheduler, as `Deliver` events with a timestamp. The model gives up the ability to change a message's fate after it was sent, with one exception. The cut check at delivery time can still drop it. A clog that holds a link for an unknown duration needs a queue, or a delay computed at the time of the clog.
2. Every message on a link takes exactly `latency_min`, which equals `latency_max`. The one sent earlier is due earlier. The scheduler breaks ties by schedule order, so two messages sent at the same instant also arrive in send order.
3. The copy fires as a `Deliver` event. The world calls `net.deliver`, which finds the id, decrements the copies, and sees that the link is cut now. It returns `Err(Partition)`, the world traces a drop, and the destination never sees the beat. The ledger no longer holds the copy. Once every copy of the round is resolved, `in_flight` is zero.
4. Bugs in the world. An event cloned by mistake, a stale event that survived a cancellation, or a message routed to the wrong place. Those look like network bugs from inside a node. The ledger turns them into a panic at the network boundary, which says "the simulator is wrong" instead of "the system is wrong".
5. `n1` suspects `n0`, because no beat from `n0` arrives. `n0` does not suspect `n1`, because `n1`'s beats arrive. `n2` suspects nobody. Two nodes hold different views of a third, and nothing in the heartbeat protocol can reconcile them. That is the asymmetric partition from Kingsbury's survey.
6. A beat sent at `t` arrives at `t + 1 ms` at the earliest and `t + 5 ms + 200 ms` at the latest. The next beat is sent at `t + 100 ms` and arrives at `t + 101 ms` at the earliest. The longest silence is from `t + 1 ms` to `t + 305 ms`, which is 304 ms. For no false suspicion you need `latency_max + reorder_delay_max + interval - latency_min < suspect_after`, so `reorder_delay_max` below 246 ms with the other defaults.
7. When a change to a trace line or to the model was intended, and the determinism check still passes, a fingerprint change is fine. When nothing in the trace format or the model was meant to change, it is not fine. Then something nondeterministic leaked, or a behaviour changed by accident. The commit that changes the constant says which one it was.

</details>

## What's next

The network now lies, loses, repeats and delays, and the heartbeats show what that looks like from inside a node. The disk is still honest. Every append becomes durable after a fixed delay, in order, with every byte intact. Module 6 replaces it with a disk that tears writes, loses them after a successful `fsync`, flips bits and misdirects sectors. The node gets a checksummed log and a recovery procedure, and the world gets crash and restart. The scheduler and the network do not change.
