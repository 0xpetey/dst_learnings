# Module 9: Assembling the VOPR

## Why this module exists

After Module 8 every piece of a simulator exists. A scheduler owns time. A network model and a disk model decide the fate of every message and every write. A swarm configuration turns fault families on and off from the seed. A workload drives many clients at once, a history records what they saw, and a checker says whether that was right. What does not exist is the program. A test function draws a configuration, builds a world, runs the workload, stops the faults and checks, and each test does it a little differently. This module makes it one program with one argument. `vopr --seed N` draws everything from the seed and prints the configuration. It runs the chaos phase, stops the faults, waits for the cluster to converge, runs a quiet phase and checks. A seed that fails prints the configuration and the assertion that fired.

The second thing that does not exist is a target worth the trouble. Three hosts were three stores. Each node answered its own clients from its own disk, and nothing one node did reached another. The bugs that a simulator earns its keep on are the ones between nodes. So this module makes the three hosts one store. The node becomes a replica. One replica is the primary and answers clients, the others are backups, and a write is answered only once it is on every disk. There is an explicit failover rule, a procedure for a new primary to take over, and a repair for a disk that lost a record. That is primary-backup replication, the simplest replication there is, and it is enough to have real distributed bugs.

The simulator found four while the module was written, three in the protocol and one in its own checker. Each one is in this lesson as a story, because that is what a VOPR produces: a seed, a configuration, a trace, and an assertion, which together say exactly what went wrong. Module 10 replaces the protocol with a real consensus protocol. The VOPR built here is the tool it is developed under.

## Objectives

By the end of this module you can:

- Describe the structure of one simulator run, and say what each phase is for.
- Draw every parameter of a run from one seed in a fixed order, and say why the order matters.
- Turn the store into a primary-backup cluster: views, the failover rule, the gather, the commit rule.
- State the two invariants of the protocol and say why they hold.
- Repair a log with a hole in it from a peer, and say what must not be repaired that way.
- Explain why a record carries the view it was prepared in, and what two logs are compared by.
- Read a failing run from its seed, configuration, trace and assertion to the bug.
- Say what this protocol cannot do, and what Module 10 pays for liveness with.

## Lesson

### The shape of a run

```text
  seed ──► RunConfig::from_seed ──► WorldConfig::swarm, WorkloadConfig::random, the workload's dice
                 │
                 ▼
  chaos phase    run_workload(requests) under every fault family the seed turned on
                 │
                 ▼
  stop_faults    schedules cancelled, links healed, hosts up, disks and network reliable
                 │
                 ▼
  convergence    run until every replica is in one view with one log, or time out
                 │
                 ▼
  quiet phase    run_workload(quiet_requests): every request must be answered
                 │
                 ▼
  checks         the history is linearizable, the logs are prefixes of one another,
                 the primary's map is its log replayed
                 │
                 ▼
  report         the configuration line, the counts, the trace hash
```

Every simulator of this kind has this shape. TigerBeetle's `vopr.zig` runs ticks until every request is answered. Then it enters a liveness mode in which faults stop, and runs until the cluster has nothing pending. FoundationDB's simulation runs a workload, then a "quiescence" check. The phases exist because the two properties a run checks need different conditions.

Safety is checked everywhere. A wrong answer is a bug whatever the faults were, so the history is recorded through the chaos and checked at the end. Liveness is checked only in the quiet phase. During the chaos a client is allowed to give up, because its primary is allowed to be partitioned away or down. Once the faults stop, nothing excuses an unanswered request. The convergence timeout is the liveness assertion. A cluster that has not settled some seconds after the last fault is stuck, and the report says in what state each replica was stuck.

The chaos phase is measured in requests, not in events or ticks. A run with a slow disk and a flapping network handles many more events per request than a quiet one. A budget in events ends the first before anything happened. The quiet phase is a fixed number of requests too, and every one must complete.

### Everything from the seed

`RunConfig::from_seed(seed, requests)` makes one `Prng` from the seed and draws from it in a fixed order: the swarm configuration, the workload configuration, then one number that seeds the workload's own dice. The order is a contract. A draw added at the end changes no earlier seed's meaning, and a draw added in the middle changes every seed after it. The configuration line prints the draws:

```text
seed=1 nodes=3 clients=6 requests=150 quiet=30 seed=14971601782005023387 net=[reliable latency=1ms] disk=[read=746µs..1.669ms write=2.993ms..22.73ms lost=10/1000 torn=5/100 misdirect=1/1000 rot=9/1000 read_fault=3/1000] heartbeat=[interval=100ms suspect_after=350ms] partitions=[every<=1.244s for<=179ms] crashes=[every<=1.759s down<=404ms] pauses=none clocks=none buggify=[enable=50/100 fire=9/100] slow_host=[disk x4] workload=[keys=20 hot=3@89/100 mix=get:4 put:4 delete:2 value<=42 think=25/100@<=187ms]
```

The second `seed=` is the world's own, drawn by the swarm and printed as Module 7 prints it. The workload's seed is the last draw and is not printed, because the outer seed fixes it. The line is printed before the run starts. A run that aborts at an assertion has already said what it was. The workload is drawn here because Module 8 left it out of `WorldConfig::swarm` on purpose. The workload is not a fault, and the swarm is the fault families. The run's configuration holds both.

### The replica

```text
  clients ──► any replica ──► forward ──► primary ──► Prepare ──► backups
                                            │                        │
                                            │◄──── PrepareOk ────────┘
                                            │
                              commit once every disk has it, apply, reply to the client
```

`kv_core::replica::Replica` wraps the `Node` of Modules 2 to 8 and owns the `Heartbeat` of Module 5. The node still does what it did. It takes a request, appends a record, and applies the record and answers once the environment reports the append durable. What changed is the meaning of durable. The replica appends the record to its own disk and sends it to every backup as a `Prepare`. A backup appends it at the same offset and answers `PrepareOk` once its disk reports the write durable. The replica feeds the node its `AppendComplete` only once every replica has the record on disk. The node applies and replies. Replication is durability across machines, and the node never learns that anything is different.

One replica per view is the primary. The primary of view `v` is node `v mod n`. Only the primary runs a node, and only the primary answers clients. A backup forwards every request it receives to the primary, and the primary replies to the client directly. Clients do not know which replica is the primary and do not need to. Reads are answered by the primary from its map, which holds every committed write and nothing else, so they are linearizable with no further work.

Every beat now carries the sender's view and a summary of its log. The failover rule uses the first. A backup whose failure detector marks the primary `Down` moves to the next view whose primary it believes alive, and says so in its beats. Views only go up. A replica that hears a higher view adopts it at once, and a primary that hears one steps down. The old primary's node is dropped, and with it every write it had not yet committed. The clients retry.

### The gather

The new primary serves nothing until it gathered. It needs four things:

1. Every replica reported in the new view. Until then another primary is allowed to be preparing.
2. No replica is still repairing its log. A repairing replica holds records its beat cannot vouch for yet.
3. Its own log is the newest. If a peer's log is newer, the primary drops what disagrees and pulls the rest from that peer.
4. Every disk has its log, exactly, and durably. A peer that disagrees is told to truncate, and a peer that is behind is pushed to.

Then it builds its node from the log, which recovers sessions and state the way a restart does, and answers the requests that queued meanwhile. Step 4 is the re-commit. A record the old primary prepared but never committed, which a backup still holds, is committed by the new primary before it serves. A read answered after that reflects it, and no later failover can lose it, because it is on every disk.

### Two invariants

The protocol keeps two facts true at every instant, and the acceptance tests check both.

Every committed write is on every disk. That is the commit rule, read backwards. A write is answered only once every replica reported it durable.

The logs are prefixes of one another, except for a tail that the next gather removes. A backup accepts record `s` only at the offset where its own log ends. So `World::check_logs` compares every pair of logs byte for byte up to the shorter. Only the newest log, and no other, is ever pushed, so no backup is ever asked to overwrite a committed record.

Together they answer the question of two primaries. There are never two that commit. The new primary commits nothing until every replica reported in its view, and a replica that reported never acknowledges the old primary again. The old primary keeps answering reads until it hears the new view. Those reads are not stale, because nothing newer was committed anywhere.

### What it costs

A write needs every replica. A gather needs every replica. A cluster with one replica down or unreachable commits nothing until it comes back. In the swarm that is most of the chaos phase, and many requests give up. In the quiet phase every replica is up, and the cluster converges.

This is the honest price of primary-backup, and it is the whole reason for Module 10. Consensus commits on a majority. Then the longest log is no longer the right log. A replica is allowed to hold a committed record that another replica never saw. The new primary has to be chosen so that it holds every committed record anyway. That is an election. Everything in this module that works because every replica reported is replaced there by something that works because a majority did.

### Repairing a log

A replica that restarts reads its disk. Module 6 decided what a node does with a hole in the middle of its log: it halts, because serving on top of a missing record answers reads with state that lost an acknowledged write. A cluster of one still halts. A replica with peers need not, because every committed record is on every other disk.

The first version of the repair reported the prefix before the hole, erased everything after it, and let the primary push the rest. It is wrong, and seed 36 found out how.

```text
seed=36 ... disk=[... lost=19/1000 ... misdirect=9/1000 rot=7/1000 ...] crashes=[every<=1.488s down<=80ms] ...
safety: key "k0" is not linearizable: after 12 operations [0, 1, ..., 10, 12], none of these fit: op 11
    8 c4 id=6  Put k0=c4-53    ret=t=8.466s -> PutOk
    9 c5 id=4  Put k0=c5-54    ret=t=9.200s -> PutOk
   10 c4 id=8  Put k0=c4-58    ret=t=9.480s -> PutOk
   11 c4 id=12 Get k0          ret=t=20.911s -> Value(c4-53)
```

A read at twenty seconds returned a value that two acknowledged writes had replaced at nine. The trace shows the log lengths in the beats. At eleven seconds two replicas had 42 records. At twelve and a half one of them reported 21. At fourteen the third reported 16. At seventeen a new primary gathered on the longest log in sight, and served. That log was 28 records, all the other 42-record replica kept after its own restart.

A lost write is a hole that nobody sees until the next restart. At nineteen lost writes in a thousand and a hundred writes per replica, every replica has a hole somewhere. Each replica's hole is in a different place. Every restart found a hole, erased everything after it, and reported the short prefix. The records after the hole were real, and on that replica's disk, and the only copies left in anyone's memory. Three restarts in a row threw away fourteen committed writes.

The repair now keeps what it finds. Recovery stops at the first hole, and `log::salvage` collects every valid record beyond it. The replica asks every peer for the missing record. The first answer fills the hole, and the replica writes out the records it kept. If the next record is another hole, it asks again. A replica that is repairing reports in its beats the highest record it holds, so a gathering primary knows to wait for it. Only once every peer answered that it does not have the record does the replica erase from the hole on. A record that no peer has was never anywhere else, so it was never acknowledged, and what follows it was prepared on top of it.

The erasure is zeros from the hole to the end of everything ever written. Without it a stale record beyond the hole reads back as current after the next restart, whenever a shorter record lands in front of it. A repair must erase what it cannot vouch for.

### Records carry their view

Seed 402 found the next one. Node 1, primary of view 4, prepared record 96 and crashed before anyone had it. Node 2 became the primary of view 5, gathered on 95 records, and prepared a different record 96. Later node 1 restarted with a hole and salvaged a stray copy of its own record 96 from beyond the hole. It wrote that copy back in place over the right one. Both records were record 96. Nothing in the record said which one the cluster kept.

Now the header does. The field Module 6 reserved holds the view the record was prepared in. A log is summed up as runs of views, `[0..30 4..95 5..96]`, and that is what beats carry instead of a length. Two logs are compared by their first disagreement. At the first record on which they differ, the record from the higher view is the one the cluster kept. A record beats no record. A gathering primary adopts the newest log by this order, not the longest. A primary that sees a backup's runs disagree with its own sends `Truncate { at }`, and the backup erases from there and is pushed the rest. That is Raft's log matching property, without Raft's elections, and the reading says where it comes from.

A restarted node that finds itself the primary of the view it hears moves to a view of its own, `view + n`. It is allowed to hold records it prepared in the old view before it crashed, and to miss some of them. A new view keeps their numbers from being reused, which is the same reason a Raft term changes on every election.

### Stragglers

Seed 4668 found the third one, and it is the oldest kind of distributed bug there is. Node 0's beat number 4 said its log had 25 records. Its beat number 3, delayed by the network's reordering, arrived after it and said 24. Node 1 was a primary that had just restarted and was gathering. It took the later arrival for the truth, decided its own 24 records were as long as anyone's, served, and prepared a second record 25.

Beats and acknowledgements now carry a boot id and the heartbeat's own sequence number. The environment hands the replica its boot id at start. It is the host's incarnation in the fake world and the wall clock in the real one, so a restart gets a larger one. A replica applies a beat only with a `(boot, seq)` above the last one it applied from that peer. The heartbeat still counts the straggler as a sign of life, which it is. Only the claim about the log is ignored.

### The checker's own bug

Seed 12784 found the fourth, and it was not in the protocol. The white-box check compared the primary's map with its disk. A misdirected write had put a hole in that disk, and the primary had not restarted, so it had not found the hole yet. Its map was right and its disk was wrong, and the check blamed the map. The check now compares the map with the log the replica holds in memory. Only a disk that cannot damage its medium is also compared with that log. A checker that is wrong about what the system must hold is a bug like any other. The seed that found it is a regression test like any other.

### Reading a failing run

The four stories above were read the same way, and the lesson spells it out once:

1. The seed. `vopr --seed N` reproduces the run bit for bit, in this process and in another.
2. The configuration line. Which families were on tells you which faults to suspect. Seed 36 had lost writes and crashes. Seed 4668 had reordering. Seed 402 had misdirected writes.
3. The assertion. A safety failure names the key, prints the operations on it, and says which operation no order explains. A liveness failure prints each replica's status, view and log, and the first record on which two logs disagree.
4. The trace. `vopr --seed N --trace` prints every line as it is recorded, so a run that aborts has shown everything up to the assertion. Find the operation named by the assertion and the replica that answered it. Walk back through its status lines to the view change or restart that put it in that state.

Every trace line carries true time, the host, and what happened. The lines you want first are `status view=`, `crash`, `restart`, `net partition` and `net heal`. Then the beats, which say what each replica believed about its log.

### What the swarm reaches

A protocol that is never driven into its hard paths is not tested there. The acceptance test counts, over forty seeds of one hundred requests each, how often the swarm reached each path. Every seed failed over at least once. A new primary pulled a log it did not have on most seeds. Damaged logs were repaired on a handful. A primary stepped down on a majority of seeds. The counts are printed by `the_swarm_reaches_the_replication_code`, and the lesson does not pin them, because every change to the protocol moves them.

Two hundred seeds run in `scripts/check.sh`. Twenty thousand ran clean before the module was finished, at about a hundred seeds per second in release. The four seeds that found bugs are regression tests, and the test that runs them says what each one found.

### How TigerBeetle does it

`src/vopr.zig` is the reading, and its main is the shape above with real numbers. The seed either comes from the command line or is drawn fresh. A fresh seed requires a release build, so that a failure found by chance is found at speed. `options_swarm` draws replica and client counts, packet loss, partition mode, read and write fault probabilities, crash and restart probabilities. The run ticks until every request is answered. Then `transition_to_liveness_mode` picks a core of replicas, restarts the core's downed replicas and heals the partitions between them. Everything outside the core stays broken. Then it ticks until `pending()` returns nothing, or fails with "no state convergence". The post on liveness mode, in the readings, says why the core is the trick. A cluster has to make progress while the replicas outside the core never recover. A majority protocol passes that liveness check and the protocol of this module cannot. It is the first test Module 10 adds.

### Decisions recorded

- The replica wraps the node and gates its `AppendComplete` on every replica's acknowledgement. The node is unchanged except for `set_view`, which stamps the view into every record it appends.
- The heartbeat lives inside the replica, which consumes its outputs. The `Network` trait carries peer messages, and the real `TcpNetwork` drops them with a message, because `kv serve` is a cluster of one. Dialling peers over TCP is a stretch goal.
- The commit rule is every replica, not a majority. Safety follows from it without an election, and liveness is the price. Module 10 pays it.
- Clients talk to whichever replica they were given, and backups forward. The primary replies to the client directly. The history is one history for the cluster, and `World::history_of` is gone.
- A damaged log is repaired by keeping what is found beyond the hole and fetching the missing record from a peer. Only what no peer has is erased. A cluster of one still halts.
- Records carry their view in the header's reserved field. Beats carry the log as runs of views. The newest log wins, and a backup whose log disagrees is truncated by an erasure from the first disagreement.
- Beats and acknowledgements carry a boot id and a sequence number, and a replica applies only beats newer than the last it applied. The environment hands the replica its boot id.
- A restarted node waits for a beat before taking up a role, and a restarted node that finds itself the primary moves to `view + n`.
- The trace can be printed as it is recorded. `Trace::set_echo` is behind the `std` feature, and `vopr --trace` uses it, so a run that aborts has shown its trace.
- Every fingerprint from Modules 4 to 8 changed. The node became a replica, and its ticks, peer messages and status changes are in the trace. The Module 6 storage chaos runs on one host, because its oracle is about one disk.

## Readings

Required:

- TigerBeetle: [`src/vopr.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vopr.zig). Read `main` top to bottom: the seed, `options_swarm`, the request loop, `transition_to_liveness_mode`, the convergence loop, and what is printed on failure. Then read `cluster_recoverable`, which is the liveness assertion naming what is stuck.
- matklad, ["Simulation Testing For Liveness"](https://tigerbeetle.com/blog/2023-07-06-simulation-testing-for-liveness/) (TigerBeetle, 2023). The view-change livelock that a safety-only simulator never saw, and why the liveness mode keeps the replicas outside the core broken.
- Ongaro and Ousterhout, ["In Search of an Understandable Consensus Algorithm"](https://raft.github.io/raft.pdf) (2014). Only section 5.3 this time. The Log Matching Property, and how a leader brings a follower's log in line with its own. Module 10 reads the rest.
- Budhiraja, Marzullo, Schneider and Toueg, ["Optimal Primary-Backup Protocols"](https://pages.cs.wisc.edu/~remzi/Classes/739/Fall2015/Papers/92-1265.pdf) (1992). Read the introduction and section 2. The definition of the primary-backup approach, failover, and the bounds on how long a service is without a primary.

Optional:

- TigerBeetle: [`docs/internals/vopr.md`](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vopr.md). The overview, and the note that a seed plus a commit hash reproduces a run.
- Eaton and Greef, ["We Put a Distributed Database In the Browser"](https://tigerbeetle.com/blog/2023-07-11-we-put-a-distributed-database-in-the-browser/) (TigerBeetle, 2023). The simulator as a game, at sim.tigerbeetle.com, with the faults of this module turned up by hand.
- TigerBeetle, ["Protocol-Aware Deterministic Simulation Testing"](https://tigerbeetle.com/blog/2026-08-20-protocol-aware-dst/) (2026). Checks that run inside each replica, including the one this module's `check_logs` is a small version of: every replica committed the same record at the same place.

## Coding task

### Starting point

Start from your Module 8 workspace with `dst start 09`. If you did not finish Module 8, `dst start 09 --from-solution` starts from `solutions/08-workloads-checking/` instead. The command adds `replication.rs` to `crates/kv-env/tests/` and `vopr.rs` to `crates/vopr/tests/`. It replaces five tests of Modules 4 to 8 in `crates/kv-env/tests/`, and `virtual_time.rs` stays as it is. They were ported to the replicated store, and the lesson's last decision says how. It replaces `scripts/check.sh`, which gains a two-hundred-seed sweep at the end.

### Requirements

#### The replica, in `kv_core::replica`

1. `Replica::new(me, boot, peers, heartbeat::Config, now, out)` and `with_buggify` with three `Buggify` values, for the replica, its node and its heartbeat. Node ids are `0..n`. The constructor issues `Output::Read` and the heartbeat's first beats.
2. `Input` is `Request`, `Peer { from, message }`, `ReadComplete`, `AppendComplete` and `Tick`. `on_input(now, input, out)`. `Output` is `Read`, `Append`, `Reply`, `Send { to, message }`, `Halt`, `PeerUp`, `PeerDown` and `Status { view, status }`.
3. `Message` is `Beat { boot, seq, view, runs, top, durable }`, `Prepare { view, seq, offset: Option<u64>, bytes }` and `PrepareOk { boot, view, durable }`. Then `Pull { view, from }`, `Forward { view, envelope }`, `Fetch { view, seq }`, `Missing { view, seq }` and `Truncate { view, at }`. `Display` is what the trace shows.
4. `Status` is `Reading`, `Joining`, `Repairing`, `Erasing`, `Backup`, `Gathering`, `Primary` and `Halted`. The primary of view `v` is `v mod n`. The queries are `view`, `status`, `primary`, `is_primary`, `heartbeat` and `node() -> Option<&Node>`. Then `len`, `durable`, `top`, `runs`, `log_bytes`, `records`, `recovery`, `damage`, `uncommitted`, `next_deadline` and `stats`.
5. The commit rule. The primary appends locally and sends `Prepare` to every peer. A backup accepts `seq == len + 1` from the primary of its view, at the offset where its log ends, and asserts the offset. It answers `PrepareOk` with its durable count once the disk reports the write durable. The primary feeds the node `AppendComplete` once every replica's durable count, its own included, reaches the record.
6. Forwarding. A backup sends a client request to the primary of its view as `Forward`. If the primary is `Down`, it drops the request. A primary that is not ready queues up to `QUEUE_MAX`.
7. The failover rule. On `PeerDown` of the primary, a backup moves to the next view whose primary it believes alive, or itself. Any message with a higher view is adopted. A primary that adopts a view steps down, dropping its node and its uncommitted writes.
8. The gather, in the four steps of the lesson. The newest log is decided by `(view, seq)` runs, with the first disagreement deciding and a record beating no record. Pulls are answered with `Prepare`, a batch of `BATCH` at a time. A peer whose runs disagree is sent `Truncate`, and a peer that is behind is pushed. The primary is ready once every peer's runs equal its own and every durable count, its own included, reaches its length.
9. Repair. On `Tail::Damaged` with peers, keep the prefix and salvage every valid record beyond the hole with `log::salvage`, keeping the higher view per number. Ask every peer with `Fetch`. Write kept records back at their place. Erase from the hole on only once every peer answered `Missing`. A cluster of one halts.
10. `log::Record::encode_in_view(seq, view, out)` puts the view in the reserved field. `log::Recovered` gains `views`. `log::record_header(bytes) -> Option<(seq, view)>`. `log::salvage(log, from) -> Vec<(seq, view, bytes)>`. `Node::set_view`.
11. Stragglers. A beat with `(boot, seq)` at or below the last applied from that peer updates the heartbeat and nothing else. A `PrepareOk` from an older boot than the last beat is ignored.
12. A restarted node, `boot > 0` with peers, waits in `Joining` for a beat before taking a role. If that role is primary, it moves to `view + n`.
13. `Site::DropPrepare` as a fifth fault point, in the primary's send. `Bug::StaleReadAfterStepDown` as a fourth planted bug: a primary that steps down keeps its node, applies its uncommitted tail to it without replying, and answers reads from it. Both without the features are as before.
14. Invariants after every input. Offsets and views match the log. The durable count never exceeds the length. Pending appends of the current generation are numbered in order. A primary's node is at its length less its uncommitted writes. A backup has no node.

#### The environment

15. `env::Storage::poll_completions` and `env::Network::poll` fill `Vec<replica::Input>`. `Network::send_peer(to, message)`. `Driver` owns a `Replica`, takes `me`, `boot` and `peers`, and feeds `Input::Tick` at the start of a step whose deadline passed. `Observer` sees replica inputs and outputs.
16. `real.rs` runs a cluster of one with a boot id from the wall clock. `kv serve` behaves as in Module 2, and the real server test passes unchanged.
17. `kv_env::fake`: `Message::Peer(replica::Message)` on the wire. The inbox holds requests and peer messages in arrival order, and a paused host holds both. `Process` is the driver alone, and the world's timer wakes the driver. `World::replica(n)`, `node(n)` for the primary only, `primary()`, `converged()`, `check_logs()`, `run_until_converged(timeout)`. `WorldStats` renames `beats_delivered` and `held_beats` to `peer_messages_delivered` and `held_messages`. `stop_faults` disables every fault point in every replica.
18. `WorldConfig::echo_trace`, and `Trace::set_echo` behind the `std` feature of `determinism`.

#### The simulator, in a new crate `vopr`

19. `vopr::RunConfig::from_seed(seed, requests)` draws the world, the workload and the workload's seed in that order. It fixes three nodes, two clients per node, thirty quiet requests, a ten-second convergence timeout, and the client configuration of the Module 8 chaos test. `Display` is the configuration line of the lesson.
20. `vopr::run(&RunConfig) -> Report`, with the phases of the lesson in order. It panics at the first check that fails, with a message that starts `safety:` or `liveness:`. A liveness message lists every replica and the first record two logs disagree on. The white-box check compares the primary's map with the log it holds. On a disk that cannot damage its medium, it also compares that log with the disk. `Report` has the counts the binary prints and `hash`, the trace's hash.
21. The binary. `vopr --seed N [--requests N] [--trace]` and `vopr --seeds A..B [--requests N]`. It prints the configuration before the run and ends with a `trace_hash=` line, which for a sweep folds every seed's hash into one.
22. The fingerprints. Seed 1 of `RunConfig::from_seed(1, 150)` is `0xc85d_a241_4993_66e8`. The five ported `kv-env` tests changed too. The seeded workload is `0x7f7e_3433_c031_8cdd`. The network chaos run is `0x1e66_f57c_f2f3_a4a2`. The storage chaos run is `0x48cc_d1a5_c747_609a`. The swarm run is `0xcf70_50fa_25da_f2f4`. The workload chaos run is `0xe688_4dda_2f79_6c8c`.

### Acceptance criteria

```sh
scripts/check.sh                                              # ends with "all checks passed"
cargo test -p vopr -- --nocapture the_swarm_reaches           # prints the counts
cargo run --release -p vopr -- --seeds 0..2000 | tail -1      # one trace_hash line, no abort
cargo run --release -p vopr -- --seed 36 --trace | tail -5    # the first regression seed, passing
```

`check.sh` runs the whole suite and then two hundred seeds. The two thousand seeds take about twenty seconds in release. A seed that fails aborts the sweep at its assertion, with its configuration line above it.

### Hints

<details>
<summary>Where to start</summary>

Make a cluster of one work first. A replica with no peers is the primary of view 0 as soon as its disk is read. Every test of Modules 4 to 8 on one host passes before you write a single peer message. Then three hosts with a reliable network and no faults. Then the swarm.

</details>

<details>
<summary>The node's op ids</summary>

The node numbers its own operations from one, and a new node starts over. The replica has its own storage ops and keeps a queue of `(seq, node op)` for the writes awaiting commit. Feed the node `AppendComplete` in sequence order, from the head of that queue, because the node asserts the order.

</details>

<details>
<summary>Building the node from memory</summary>

The node's constructor issues a `Read`. Answer it at once with the log the replica holds, and never send it to the disk. The replica read the disk already, checked it, and repaired it. The node sees a clean log every time.

</details>

<details>
<summary>The pending queue after a truncation</summary>

An append submitted before a truncation completes after it. Its sequence number is beyond the log, and counting it durable is a lie. Tag every append with a generation, bump the generation on truncation, and ignore completions from an older one. The erasure precedes every later append in the queue, so the disk is consistent even though the memory moved first.

</details>

<details>
<summary>Comparing runs</summary>

Write `view_at(runs, seq)`. The divergence of two logs is the first sequence number at which it differs. If it never differs, the divergence is one past the longer log. `newer` compares `view_at` at the divergence, with `None` below every view. Test it by hand on three-element lists before you trust it under the swarm.

</details>

<details>
<summary>Who answers a fetch</summary>

Anyone. A fetch goes to every peer, and the first `Prepare` back fills the hole. It comes from the primary's log, a backup's log, or a record a repairing peer kept beyond its own hole. A record kept beyond a hole is sent with `offset: None`, because its sender does not know where it belongs.

</details>

<details>
<summary>Why your gather never completes</summary>

Print each peer's runs and durable count next to your own on every beat. The usual causes: a peer still repairing, which you must wait for. A peer whose beat you ignored as a straggler, because its boot id did not change on restart. A peer whose view is one behind, because its beat with the new view did not arrive yet.

</details>

<details>
<summary>Finding your fingerprint</summary>

Set the constant to zero and read the hash from the assertion. Then run the binary twice and compare, because the cross-process test is the one that catches a hazard a single process cannot see.

</details>

### Stretch goals

- Dial peers over TCP. `TcpNetwork` learns the addresses of the other replicas and keeps one connection to each. Run three `kv serve` processes and kill the primary.
- Make the liveness mode TigerBeetle's. Pick two of the three replicas as the core after the chaos phase, leave the third down, and see which assertion fires. Keep the test, because Module 10 must pass it.
- Replace the all-replicas commit rule with a majority, change nothing else, and run a thousand seeds. Keep the first failing seed and its trace. That is the bug Module 10 exists to fix properly.
- Shorten the trace. A beat line every hundred milliseconds per host is most of it. Keep a count instead, and print only the beats that changed the receiver's view of the sender.
- Add the view to the client's reply and let the client send its next request to the primary it last heard from. Measure how many fewer forwards the swarm does.

## Check your understanding

1. Name the phases of a run and say which property each one is for.
2. Why is the chaos phase measured in requests rather than in simulated time or events?
3. State the commit rule and say what follows from it for a new primary.
4. Two primaries exist at one instant, the old one partitioned away. Why does the old one's read not violate linearizability?
5. A backup's log is longer than the primary's. In which case does the primary pull from it, and in which case does it tell the backup to truncate?
6. What did the first repair throw away, and why were the records it threw away the only copies?
7. A record is salvaged from beyond a hole with the right sequence number. What tells the replica whether it is the record the cluster kept?
8. Why does a restarted primary move to `view + n` instead of gathering in the view it finds?
9. What does a straggling beat get to say, and what does it not?
10. What does this protocol need every replica for, and what does Module 10 replace it with?

<details>
<summary>Answers</summary>

1. Configure from the seed, chaos, stop faults, converge, quiet, check, report. Safety is checked over the whole history, so it belongs to every phase. Liveness is the convergence timeout and the quiet phase, where nothing excuses an unanswered request.
2. A run under a slow disk and a flapping network handles many more events per request than a quiet one. A budget in events ends it before anything happened. A budget in requests gives every configuration the same amount of work to answer.
3. A write is answered only once every replica has it on disk. So every committed record is on every disk. A new primary that repaired its own log and gathered every replica's report holds every committed record, without an election.
4. The new primary commits nothing until every replica reported in its view, and a replica that reported never acknowledges the old primary again. Nothing newer was committed, so the old primary's map is still the latest committed state.
5. The primary compares runs. Suppose the backup's log agrees with the primary's up to the primary's end and goes on. Then the backup's tail is newer, because a record beats no record. A gathering primary pulls it. If the two disagree at some record within both, the one from the higher view wins. A serving primary is the newest, so it sends `Truncate`.
6. Every record beyond the hole. A lost write is a hole nobody sees until the next restart, so every replica had one somewhere, in a different place. Each restart erased what came after its own hole, and after three restarts no replica's memory held the records beyond the last hole.
7. Its view. Two records carry one number only in one case: a primary lost the first before anyone saw it, and a later primary numbered a new write the same. The later primary's view is higher, so the record from the higher view is the one the cluster kept.
8. It is allowed to hold records it prepared in that view before it crashed, and to miss some of them. A gather in the same view numbers new writes the same as the lost ones. A fresh view means every record has one view and one number.
9. That its sender is alive, which the heartbeat records. Not what its sender's log looks like, because a later beat already said, and the straggler's claim is older. Seed 4668 took the older claim and prepared a second record 25.
10. Every commit and every gather. Module 10 brings a majority commit rule and an election that chooses a primary holding every committed record. A view change copies the chosen log to the majority. Then the cluster makes progress with one replica down. That is the liveness check this module cannot pass.

</details>

## What's next

The simulator is a program now. It has caught three bugs in a protocol that fits in one file, and one in its own checker. The protocol is also the simplest one there is, and it blocks whenever a replica is away. Module 10 replaces it with a consensus protocol, Viewstamped Replication, written against this simulator from the first line. The commit rule becomes a majority, the gather becomes a view change, and the repair becomes a protocol of its own. The VOPR gains TigerBeetle's liveness mode, in which the replicas outside the core can stay broken for the whole chaos phase. The bug journal starts. The target is tens of thousands of seeds clean, with every seed that ever failed kept as a test.
