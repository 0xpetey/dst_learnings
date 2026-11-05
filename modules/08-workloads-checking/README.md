# Module 8: Workloads and checking (the oracle problem)

## Why this module exists

After Module 7 the simulator can hurt the store in every way the course has a model for. It draws a fault configuration from the seed, runs it, stops it, and checks that the cluster comes back. What it cannot do is say that an answer the store gave was wrong. Module 6 had an oracle of sorts, one client per node that remembered the writes it had issued. That was enough for one client at a time. It says nothing about two clients on one key, which is where the bugs that matter live.

This is the oracle problem. A test needs two things, a way to make the system do something and a way to know whether what it did was right. The first half is the workload, and a workload that touches each key once finds nothing. The second half is the checker, and there are three kinds. The code can check itself with assertions. A reference model can say what each answer is supposed to be. And for a history with concurrent operations, a search can decide whether any order of those operations explains the answers. That last one is linearizability, and this module builds a checker for it.

The checker is only worth anything once it caught something. So the module ends by planting three bugs in the store. A lost update, a stale read after a restart, and a duplicate apply on a retry. Each one has to be found. Module 9 then puts the workload and the checker under the simulator loop and calls the result a VOPR.

## Objectives

By the end of this module you can:

- Draw a workload from the seed with an operation mix, a hot set of keys, unique values and think time. Say why each of those matters.
- Name the three families of checks and say what each one catches that the others do not.
- State what a sequential history is and why the reference model is a complete check for one.
- State the definition of linearizability from Herlihy and Wing, and apply it to a three-operation history by hand.
- Say what a pending operation is and how the checker treats one.
- Explain the search, the cache that makes it feasible, and the partition by key that makes it fast.
- Write a white-box check that replays the log and compares it with the node's state.
- Plant a bug and show which workload and which check find it.
- Say what safety and liveness mean for the store and where each is checked.

## Lesson

### Three families of checks

| Family | Where it looks | What it catches | What it misses |
|---|---|---|---|
| Inline assertions | Inside the node, between inputs | A state the code itself knows is impossible | A wrong answer that is consistent with itself |
| Reference model | The replies, in order | Any wrong answer in a history without overlap | Everything in a history with overlap |
| History checker | The calls and replies, with their times | Any wrong answer, overlap or not | A wrong state that no reply reveals |
| State checker | The node's memory against its log | A wrong state, at once | A wrong reply from a right state |

The first three are the syllabus's three families. The fourth is the one TigerBeetle leans on hardest, and the lesson adds it because one of the planted bugs is invisible without it. The families overlap and that is the point. Each one is cheap, and together they cover more than any one of them.

### The workload

A workload is a stream of requests drawn from the seed. The crate `checker::workload` draws it from a `WorkloadConfig`:

```text
   keys=16 hot=2@50/100 mix=get:2 put:2 delete:1 value<=16 think=25/100@<=100ms
```

Four choices in that line are deliberate. The key space is small and has a hot set, two keys that take half the operations. Two clients collide only on a key they share, and the fewer keys there are, the more they collide. The mix has deletes in it, because a delete is the one operation whose reply says something about the previous state. The lesson comes back to that. Every value names the client and the operation that wrote them, `c2-11` padded to a random length. A read that returns the right bytes from the wrong write is a bug, and only unique values make it visible. And a client sometimes thinks, which is to say it does nothing for a while. With think time the number of operations in flight varies. It goes from one up to the number of clients, so the store sees both the quiet and the crowded case in one run.

`World::run_workload` drives it. Every client that is idle and not thinking gets its next step at once. A client is therefore always either waiting for a reply or thinking. The world records every call and every return in a `History`. A client that gives up leaves its operation open, and the next section says what the checker does with that.

TigerBeetle's `Workload` goes further than this. It plans every transfer from its index, so that the expected result of a request is a pure function of the request's number. Its `Auditor` keeps, for every request in flight, the *set* of results the request is allowed to return. That set is how it handles uncertainty, and it is the same idea as the pending operation below.

### Family one: the code checks itself

`Node::check_invariants` runs after every input. It asserts four things. Pending writes are numbered in order. Every session with no response has exactly one pending write. A halted node has a reason. The log is whole sectors. Those are facts the node knows about itself, and a violation is a bug in the node, not in the environment. The `Driver` and the `Client` assert too. An append completing out of order is an environment bug, and a reply to a request the client never sent is a routing bug.

Assertions are the cheapest check there is, and they run in production. Their limit is that they only know what the code knows. A node that answers a read from a map that is one write behind is consistent with itself. Nothing inside it is contradicted. That bug needs a view from outside.

### Family two: the reference model

The reference model is the simplest correct store there is, a `BTreeMap`. `checker::kv::KvModel` wraps one and implements `Model`, which is a state and one function, `apply(request) -> response`. For a history in which no two operations overlap, the check is a loop. Apply each request to the model in call order and compare the model's reply with the store's. That is what the tests of Modules 2 to 7 did with a map inline, and `check_sequential` is that loop with a name.

The model is a complete check for a sequential history, because a sequential history has exactly one order. The moment two operations overlap, there are two orders, and the model does not know which one the store took. The test `many_clients_overlap_and_only_the_search_can_judge_them` shows the loop refusing: `NotSequential { first, second }`, the first pair that overlaps.

### Family three: linearizability

Herlihy and Wing gave the definition in 1990, and it is one sentence. Each operation appears to take effect instantaneously at some point between its invocation and its response. The picture is the one every Jepsen report draws:

```text
   c1   ├──── put k=1 ────┤                     ├── get k ──┤
   c2           ├──── get k ────┤
   c3                                 ├── del k ──┤
        ──────────────────────────────────────────────────────► time
```

Each operation is a window from call to return. The store is linearizable under one condition. There is one point in each window, and one order of those points, under which the model agrees with every reply. In the picture, the first `get` is allowed to return `None` or `1`, because its window overlaps the `put`. The last `get` is not allowed to return `1`. The `del` returned before it was called, and the `put` returned before the `del` was called. Real time is not negotiable. An operation that returned before another was called comes first in every legal order, however the network reordered the packets.

Three small histories in `tests/checker/linearize.rs` pin the three facts that matter:

- A concurrent read sees either side of a write. Both `None` and `1` pass.
- A value cannot be unseen. A read of `1` followed by a read of `None`, with no delete anywhere, fails.
- Real time is not negotiable. Two puts in sequence and then a read of the first one's value fails.

Lowe's paper states the definition with events, and it adds one clause that this module needs. Take a history, append returns for some of the operations that have none, and linearize the result. For a linearizable history, some such extension exists. That clause is the pending operation. A client that gave up has an operation with a call and no return. The operation is allowed to take effect at any point after its call, or never. The store is correct under either choice that explains the replies. Knossos calls these `:info` operations and treats them this way. Porcupine does not treat them at all. Its users give such an operation a return at the end of time, which forces it to happen. The test `a_write_the_client_gave_up_on_may_or_may_not_have_happened` shows both choices passing on their own and failing together. A write cannot both happen and not happen.

A pending read constrains nothing, since nobody saw its answer. `kv::partition` drops those before the search.

### The search

The checker in `checker::linearize` is Wing and Gong's algorithm. Keep a set of operations placed so far and the model state after them. At each step the candidates are the operations not yet placed that no other unplaced operation precedes. One operation precedes another under one condition: it returned before the other was called. Try each candidate. Apply it to a copy of the model, and keep going only for a candidate whose model reply matches the store's. A pending operation matches any reply. Once every completed operation is placed, the history is linearizable. Once no candidate fits, back up.

```text
   placed: {}          candidates: every op that no unplaced op precedes
        │
        ├─ try a: model(a) == a.reply?  yes ─► placed: {a} ─► ...
        │                              no  ─► next candidate
        ├─ try b ...
        └─ nothing fits ─► remember (placed, state), back up
```

That search is exponential, and Lowe's change is the one that makes it usable. Remember every pair of placed set and model state from which the search already failed, and never enter one twice. For the store, the state after a set of writes to one key depends only on the last write. Eight concurrent writers therefore produce at most `8 * 2^7` distinct pairs where the plain search has `8!` orders. The test `the_cache_keeps_a_wide_history_cheap` pins both numbers. The cache does not change the answer, only the cost.

The second trick is older and bigger. Herlihy and Wing proved that linearizability is local. A history of several objects is linearizable exactly in the case that the history of each object is. For the store, each key is an object, so `kv::check` splits the history by key and searches each part alone. Porcupine calls this `Partition`. Horn and Kroening call it P-compositionality and report an order of magnitude from it. With the partition, the swarm runs of this module cost about 190 candidates per seed on average and 305 at worst. Without it, every key's operations compete in one search, and the cost grows with the whole history instead of the busiest key.

Linearizability checking is NP-complete, and nothing above changes that. A history with many writers on one key and no reads to pin them down is still exponential. So the search has a budget, and a search that spends it returns `Unknown`, never a verdict it did not earn. The budget is the reason the workload keeps the hot set small. It is also the reason Module 9's report prints the search statistics next to the seed.

### A detail found while writing: recording order, not instants

A history records instants, and the first version of `History` decided precedence by them. One client with no think time then had a history that was not sequential. The client receives its reply and submits its next request in the same event, so the return and the next call carry the same instant. By instants they overlap. In fact one happened before the other, because the simulator handled them in that order. Worse, a stale read by that client right after its own write was allowed, because the checker was free to swap them.

The `History` now gives every call and return a sequence number in recording order, and precedence is decided by the sequence. The instants are kept for people. A history built by hand has no recording order. For those, `from_ops` orders calls before returns at equal instants. That makes touching operations overlap, which is the conservative choice. Porcupine's `byTime` comparator makes the same one.

### Family four: the state checker

The history checker is a black box. It sees only what the clients saw, and that is its strength, because it can run against a system it cannot open. A simulator can open the system. The node's map is supposed to be the log replayed, at every moment the node is idle. `kv::replay` rebuilds a `KvModel` from log records, and `Node::entries` is the map. On a disk that does not lose or corrupt writes, the medium holds exactly the records the node applied. The two have to be equal.

This is the check TigerBeetle calls its `StateChecker`. Every replica that commits an operation reports the checksum of its state. The checker requires every replica at the same operation to report the same checksum, and the chain of checksums to be unbroken. Module 10 builds that version, with replicas to compare. Here there is one node and its log, and the comparison is just as sharp. The next section has the bug that only this check sees.

### The planted bugs

`kv_core::bugs` plants bugs the way Module 7 planted fault points. `Bug` is an enum with three variants. `Bugs` holds the planted set, and without the `bugs` cargo feature it is a zero-sized type whose `has` is a constant `false`. Each bug is one branch in the node, and `grep 'bugs.has'` lists them. The simulator plants them through `WorldConfig::bugs`. The configuration line of the trace shows them, so a trace with a bug in it says so in its first line.

| Bug | The mistake | What it takes to reach it | Which check finds it |
|---|---|---|---|
| `LostUpdate` | A write submitted while another write to the key is in flight is logged and acknowledged but never applied | Two clients on one key at the same moment. No fault at all | The state checker always. The history checker only through a delete |
| `StaleReadAfterRestart` | Recovery rebuilds the last record's session and skips its entry | A crash and a restart | The state checker at the restart. The history checker on the first read after it |
| `DuplicateApplyOnRetry` | A retry of a finished request runs again instead of being answered from the session | A lost reply, so that the client retries | The history checker, through a delete that is told the key was not there |

The lost update is the interesting row. Two clients put to one key at the same moment, and the second one's value never appears. The history checker accepts that. The two puts overlap, so the order in which they took effect is the store's to choose. "The second one took effect first" explains every later read. The history cannot tell a lost write from a write that lost the race. Only a third fact pins the order. A delete submitted behind a put is the write that is lost. It is answered `existed: true`, and a read after it returns the put's value. No order explains that, and the checker says so. The state checker needs no such luck. The log has the second record and the map does not, and the replay disagrees on the spot. The test `the_white_box_check_finds_a_bug_before_any_client_can` runs a workload of nothing but writes. The history checker passes it, and the replay fails it.

Each bug also hides from a workload that cannot reach it, and `each_bug_hides_from_a_workload_that_cannot_reach_it` pins that too. One client never has two writes in flight. No crash means no recovery. A reliable network means no retry. A checker that reports nothing told you nothing about the store. It told you about the workload.

### Safety and liveness

Safety is "nothing bad happens", and for the store it is linearizability. Every history the store produces is linearizable, under every fault. Liveness is "something good happens", and for the store it is that a request is eventually answered. Under faults that is not a promise the store can keep. A client whose node is partitioned away is allowed to give up, and the history records a pending operation. Liveness is therefore checked where Module 7 put it, in the quiet period. After `stop_faults`, the chaos run of this module submits thirty more requests and requires that every one of them completes. A request that is not answered after the faults stopped is a liveness bug, and Module 10 produces several.

One exception is deliberate. A host that halted over a damaged log stays down, and `run_workload` gives its clients nothing more to do. They are reported as stranded rather than failed. Module 9 repairs that host from its replicas, and the exception goes away.

### Reading a failure

The duplicate apply, caught on seed 1 of the hunt in `each_planted_bug_is_found_by_the_checker`. The report names the key, the longest order the model agreed with, and every candidate that was tried next:

```text
seed 1 key "k0": not linearizable: after 22 operations [0, 1, 2, 4, 5, 3, 6, 7, 8, 9,
   11, 12, 10, 13, 14, 15, 16, 17, 18, 19, 20, 21], none of these fit:
   op 22 (model says DeleteOk { existed: true })
   20 c3 id=16 Put k0 "c3-59..."   call=1221ms ret=1336ms -> PutOk
   21 c3 id=17 Put k0 "c3-60..."   call=1336ms ret=1351ms -> PutOk
   22 c3 id=18 Delete k0            call=1351ms ret=1465ms -> DeleteOk { existed: false }
```

Client 3 put a value, put another, and deleted the key, each one after the previous one returned. The delete was told the key was not there. No order of the twenty-three operations explains that. The put at 21 returned before the delete was called, and nothing else touched the key in between. The trace for the same seed shows why. The delete's first reply was lost and the client retried after 100 ms. The retry ran the delete again on a key that it already deleted. The pattern to learn is the one in Module 7's lesson. The verdict names the operation, the history names the client and the time, and the trace names the cause.

### How the other simulators do it

TigerBeetle has no history checker. Its `Workload` knows the expected result of every request before it sends it. Its `Auditor` holds the set of results a request in flight is allowed to return. Its `StateChecker` compares replicas. That design works because its state machine is deterministic and its workload is planned. It does not work for a store that lets clients race on a key, which is why this module has a search instead.

Jepsen has the history checker and nothing else, because it tests systems it cannot open. Knossos was first, and Porcupine is the fast one that most people use now. Both take the model as code and search for a linearization, and both are limited by the length of the history. Jepsen's newer checker, Elle, checks weaker models from the dependencies between transactions. It scales to long histories, which is why Jepsen reports now use it for databases. For a key-value store with single-key operations, linearizability per key is the right model and the search is the right tool.

FoundationDB's simulation tests are workloads that carry their own check. A workload writes a structure with an invariant, such as a ring of keys where each key names the next. A check phase at the end asserts the invariant. There is no general oracle, and there does not need to be. The invariant is chosen so that any lost or duplicated write breaks it. That is the reference model, specialised to the point where the check is a loop.

### Decisions recorded

- The oracles live in a new `no_std` crate, `checker`. It depends on `kv-core` for the request types and on `determinism` for the dice. The simulator depends on it. Nothing in it reads a clock.
- The history is per world, and `World::history_of(node)` filters it, because each node is a store of its own until Module 9 replicates.
- Precedence is decided by recording order, not by instants. Hand-built histories order calls before returns at equal instants.
- A pending operation is allowed to take effect at any point after its call, or never. A pending read is dropped before the search.
- The search is Wing and Gong's with Lowe's cache, on a partition by key, under a budget. Running out of budget is `Unknown`.
- The planted bugs are behind a cargo feature, on the fault-point pattern, and the configuration line names them. A run without bugs has the trace it had before this module.
- `WorldStats` did not gain a field, because three tests print it into their traces. The earlier fingerprints are unchanged. The think count is a trace line and a `WorkloadStats` field instead.
- The workload is not a fault family. `WorldConfig::swarm` does not draw it. The chaos test draws a `WorkloadConfig` from the same seed, and Module 9 folds it into the run's configuration.
- The lost update is the write submitted behind another, never the one in front. The version that skipped the write in front was invisible to every check once both writes completed. The second write covered it.

## Readings

Required:

- [Herlihy and Wing, "Linearizability: A Correctness Condition for Concurrent Objects" (TOPLAS 1990)](https://cs.brown.edu/~mph/HerlihyW90/p463-herlihy.pdf). Read section 1 for the motivation and section 2 for the definition. Skip the proofs. Section 3's locality theorem is the reason `kv::check` can split by key.
- [Jepsen, "Linearizability"](https://jepsen.io/consistency/models/linearizable). One page. The definition in plain words, what counts as an object, and how it relates to strict serializability.
- [Anish Athalye, "Testing Distributed Systems for Linearizability"](https://anishathalye.com/testing-distributed-systems-for-linearizability/). The two example histories with pictures, and why ad-hoc tests miss bugs that a history check catches. Read the NP-completeness argument at the end.
- [Porcupine, README](https://github.com/anishathalye/porcupine). The `Model` interface is the one this module's `Model` trait copies, and `Partition` is `kv::partition`. Then read [`checker.go`](https://github.com/anishathalye/porcupine/blob/master/checker.go), which is the search of this module in Go, with the cache keyed by bitset and state.
- [Gavin Lowe, "Testing for Linearizability" (Concurrency and Computation 2017)](https://www.cs.ox.ac.uk/people/gavin.lowe/LinearizabilityTesting/paper.pdf). Read section 2 for Definition 1, which handles pending operations by appending returns. Read section 3 for the cache.
- TigerBeetle: [`src/testing/cluster/state_checker.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/testing/cluster/state_checker.zig). The state checker: every replica at one commit reports the same checksum. And [`src/state_machine/auditor.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/state_machine/auditor.zig), the file-level comment and the set of allowed results per request in flight.

Optional:

- [Horn and Kroening, "Faster linearizability checking via P-compositionality" (2015)](https://arxiv.org/abs/1504.00204). The partition idea, generalised beyond objects.
- [Knossos, README](https://github.com/jepsen-io/knossos). The `:invoke`, `:ok`, `:fail`, `:info` history format. The `:info` example is the pending write of this module.
- [Kyle Kingsbury, "Strong consistency models"](https://aphyr.com/posts/313-strong-consistency-models). The pictures that explain why real time is the thing linearizability adds to sequential consistency.
- TigerBeetle: [`src/state_machine/workload.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/state_machine/workload.zig). The file-level comment, and how a transfer's expected result is a pure function of its index.

## Coding task

### Starting point

Start from your Module 7 workspace with `dst start 08`. If you did not finish Module 7, `dst start 08 --from-solution` starts from `solutions/07-fault-injection/` instead. The command adds `linearize.rs` to `crates/checker/tests/`, `bugs.rs` to `crates/kv-core/tests/`, and `workload.rs` to `crates/kv-env/tests/`. The five tests that you already have in `kv-env` do not change. It replaces `scripts/check.sh`. The new script adds one line, a bare-metal check of `checker`, and puts the `bugs` feature on the bare-metal check of `kv-core`.

### Requirements

#### The oracle crate, `checker`

1. A new workspace crate `checker`, `#![no_std]` with `alloc`, `#![forbid(unsafe_code)]`, with the same lint header as `sim`. It depends on `kv-core` and on `determinism` with `default-features = false`, and on nothing else. `cargo tree -p checker` shows those three.
2. `checker::history::Op<Req, Resp>` with public `client`, `request_id`, `request`, `call: Instant`, `ret: Option<Instant>` and `response: Option<Resp>`, plus `is_complete` and `is_pending`.
3. `History<Req, Resp>` with `new`, `from_ops(Vec<Op>)`, `call(client, request_id, request, now) -> usize`, `ret(client, request_id, response, now)` and `give_up(client, request_id)`. Its queries are `ops`, `len`, `is_empty`, `completed`, `pending`, `precedes(a, b)`, `overlaps(a, b)`, `is_sequential`, `first_overlap`, `max_concurrency` and `filter`. A client has at most one operation open, and a second call panics. Precedence is by recording order. `from_ops` sorts by call and orders calls before returns at equal instants.
4. `checker::model::Model`, a trait with `type Request`, `type Response: PartialEq + Clone + Debug`, `fn init() -> Self` and `fn apply(&mut self, &Request) -> Response`, bounded by `Clone + Ord`. `check_sequential::<M>(&History) -> Result<M, SequentialError<M::Response>>`. The error is `NotSequential { first, second }` or `Mismatch { index, expected, got }`. A pending last operation is applied without a comparison.
5. `checker::linearize::check::<M>(&History, limit) -> Verdict<M::Response>`. `Verdict` is `Linearizable { order, stats }`, `Illegal(Illegal)` or `Unknown(stats)`, with `is_linearizable` and `stats`. `Stats` has `ops`, `pending`, `steps`, `cached` and `depth`. `Illegal` has three fields. `linearized` is the longest order found. `stuck` is the candidates at its end with the model's reply for each. `stats` is the search statistics. It implements `Display` starting with `not linearizable: after`. The search is depth first over candidates in index order. A candidate is an unplaced operation that no unplaced operation precedes. The search applies the model to a copy and skips a candidate whose reply differs. It accepts any reply for a pending operation. It remembers every `(placed set, state)` it entered. `steps` counts candidates tried, and a search that reaches `limit` returns `Unknown`.
6. `checker::kv::KvModel`, a `BTreeMap` behind `Model`, with `get`, `len`, `is_empty`, `entries` and `apply_entry(&Entry)`. A request that fails `validate` is answered `Rejected` and changes nothing. `kv::replay(&[Record]) -> KvModel`. `kv::partition(&History) -> Vec<(key, History)>` in key order, with pending reads dropped. `kv::check(&History, limit) -> Report { keys, stats, failed: Option<Failure { key, verdict, history }> }` with `is_linearizable`, stopping at the first key that fails.
7. `checker::workload::WorkloadConfig` with the fields of the lesson's line: `keys`, `hot_keys`, `hot: Chance`, `gets`, `puts`, `deletes`, `value_len_max`, `think: Chance` and `think_max`. It has the defaults of the lesson, `random(&mut Prng)`, `validate`, `key(index) -> Vec<u8>` giving `k{index}`, and `Display` as the line in the lesson. `Workload::new(config, Prng)`, `config`, `stats` and `next(client) -> Step`, where `Step` is `Request(Request)` or `Think(Duration)`. A think is drawn first, then the hot coin, then the key, then the operation. A put's value is `c{client}-{n}` with `n` the request count, padded with `.` to a drawn length. `WorkloadStats` has `requests`, `gets`, `puts`, `deletes`, `hot` and `thinks`.

#### Planted bugs, in `kv_core::bugs`

8. A cargo feature `bugs` on `kv-core`, off by default. `Bug` is an enum with `LostUpdate`, `StaleReadAfterRestart` and `DuplicateApplyOnRetry`. It has `ALL`, `COUNT`, `name`, `index`, `from_name` and `Display` as the snake-case name. With the feature, `Bugs` has `none`, `plant`, `has` and `planted`. Without it, `Bugs` is zero sized, `has` is a `const fn` returning `false`, and `none` and `planted` exist. `Node::bugs` and `bugs_mut`, and `Node::new` plants nothing.
9. `Entry::key()` on the log entry, and `Node::entries()` over the map in key order.
10. The three branches, each guarded by `self.bugs.has(..)` and nothing else:
    - `LostUpdate`: a write submitted while another write to the same key is pending is marked, and on completion its entry is not applied. The reply is what the entry earns on its own.
    - `StaleReadAfterRestart`: recovery records the last record's session and does not apply its entry. `log_seq` and `log_len` still count it.
    - `DuplicateApplyOnRetry`: a retry of a request whose session has a response is executed again. A retry of a request still pending is still absorbed.

#### The world, in `kv_env::fake`

11. `kv-env` depends on `checker` and enables `bugs` on `kv-core`. `WorldConfig` gains `bugs: Vec<Bug>`, empty by default and not drawn by `swarm`. The configuration line shows ` bugs=[a b]` for a non-empty list and nothing for an empty one. `World::start` plants them in every new process.
12. `World` gains `pub history: History<Request, Response>`. `submit` records the call, a `Done` records the return, and a `GaveUp` records the give-up. `history_of(node)` is the history of the clients that target `node`. `target(client)`.
13. `Event::ClientWake { client }`, `World::think(client, d)`, `is_thinking(client)`. A thinking client is idle to `is_idle`, and `submit` panics for it. The think logs `{now} client={id} think for={d:?}`.
14. `World::run_workload(&mut Workload, requests) -> WorkloadRun { submitted, completed, gave_up, stranded }`. It logs `{now} workload clients={n} requests={requests} {config}` first. On every pass it offers a step to each client that is idle, not thinking, and not on a halted host, in client id order. It stops offering once `requests` are submitted. It then handles one event. It stops once no client is in flight or thinking. `completed` and `gave_up` count the operations submitted by this call.
15. `WorldStats` gains nothing. The four fingerprints of Module 7 are unchanged, and the tests that pin them pass as they are.
16. The fingerprint. The workload chaos run in `workload.rs` is `0x7cda_7998_33d5_689c`.

### Acceptance criteria

```sh
scripts/check.sh                                # ends with "all checks passed"
cargo tree -p checker                           # checker, kv-core and determinism, nothing else
cargo tree -p kv-core                           # kv-core alone, plus determinism as a dev-dependency
grep -rn 'self.bugs.has(' crates/kv-core/src/node.rs  # exactly three branches
cargo test -p kv-env --test workload -- --nocapture each_planted_bug_is_found
cargo test --release -p kv-env --test fault_injection
```

The bug hunt prints the seeds on which each bug was caught. The reference solution catches the lost update on eleven of twelve seeds, the stale read on nine, and the duplicate apply on five. The whole `workload.rs` suite takes about 0.4 s in debug. The swarm of Module 7 still takes 0.15 s in release, because nothing it runs changed.

### Hints

<details>
<summary>The candidates</summary>

Find the earliest return among the unplaced operations. Every unplaced operation called before that return is a candidate, and nothing else is. An unplaced operation with no return never lowers the minimum. That is exactly what "a pending operation blocks nothing" means.

</details>

<details>
<summary>The cache key</summary>

Use a set of pairs. A `BTreeSet<(Bits, M)>` with `Bits` a `Vec<u64>` is enough, which is why `Model` is `Ord`. Insert the pair on entering a configuration, not on leaving it. A configuration entered and not yet left is on the current path, and entering it again is a loop.

</details>

<details>
<summary>Equal instants</summary>

Give the history a counter. Stamp every call and return with it. A one-client history that is not sequential means you are comparing instants. The same mistake lets the checker swap a client's write with its next read.

</details>

<details>
<summary>Why your lost update is not found</summary>

Skip the write behind. The write in front is covered by the one behind it as soon as that one completes, and nothing is left to see. And hunt for it on a reliable network. A random one spreads the clients out and they stop colliding.

</details>

<details>
<summary>Which stats you print</summary>

Three existing tests print `w.stats` into their trace. A new field in `WorldStats` changes three fingerprints. Put the count somewhere else.

</details>

<details>
<summary>Stranded clients</summary>

Stop on an empty pass. With every idle client on a halted host and nothing thinking, `run_workload` has nothing to offer and the heartbeats keep the queue busy forever. Count those clients as stranded, and stop once nobody is in flight.

</details>

<details>
<summary>Finding your fingerprint</summary>

Set the constant to zero. Run the test, and read the hash from the assertion message. Make sure the two determinism runs agree first.

</details>

### Stretch goals

- Tighten the pending write. Suppose a client gave up on request `n` and then got request `n + 1` answered. Then any effect of `n` came before the effect of `n + 1`. Give the pending operation that return and measure how many fewer candidates the search tries.
- Report a partial linearization the way Porcupine's visualiser does, as one line per client with the chosen point marked. Use it on the failure in the lesson.
- Add a `Cas { key, expected, value }` request to the protocol, to the model, and to the workload. A compare-and-swap is the operation that makes a key-value store interesting to check, and the history checker needs no change.
- Replace the per-key `KvModel` in the search with `Option<Vec<u8>>`. Measure the cache.
- Plant a fourth bug, a read served during recovery from the queued requests before the log is applied. Say which family finds it and why the inline assertion in `check_invariants` does not.
- Run the hunt under `WorldConfig::swarm` instead of a fixed configuration and count how many seeds find each bug. Which families hide which bug?

## Check your understanding

1. State the definition of linearizability in one sentence, and say which word in it is the difference from sequential consistency.
2. Two clients put different values to one key at the same moment, and every later read returns the first client's value. Is the history linearizable? Which check finds the bug, and what does it compare?
3. Why is the reference model enough for a sequential history and useless for a concurrent one?
4. A client gives up on a delete. A later read returns the value. A still later read returns `None`. Is that linearizable? And the other way round?
5. What does the cache remember, and why does the state after a set of writes to one key make it small?
6. Why does `kv::check` split the history by key, and what theorem allows it?
7. Why is `Unknown` a verdict of its own, and what is the test author supposed to do with one?
8. A delete is the operation that catches two of the three bugs. What does its reply say that a put's reply does not?
9. Where is liveness checked, and why not during the chaos phase?
10. One client with no think time produced a history the first checker called concurrent. What was wrong, and what is the fix?

<details>
<summary>Answers</summary>

1. Each operation appears to take effect instantaneously at some point between its call and its return. The word is "between". Sequential consistency keeps each client's order and drops the real-time bound.
2. Yes, it is linearizable. The two puts overlap, so either order is legal. The order with the first client's put last explains every read. The state checker finds it, because the log has both records and the map has only one. It compares the map with the log replayed.
3. A sequential history has exactly one order, so the model has exactly one sequence to run. A concurrent history has several, and the model does not know which one the store took. Checking all of them is the search.
4. The first is linearizable. The delete took effect after the first read and before the second. A pending operation is allowed to take effect at any point after its call. The other way round, `None` and then the value with no put between, is not linearizable. A value cannot be unseen.
5. Every pair of placed set and model state the search entered. The state of one key after a set of writes is the last write's value. The number of distinct states is therefore the number of writers, and the pairs are bounded by writers times subsets instead of permutations.
6. Operations on different keys never read or write each other's state, so each key is an object of its own. Herlihy and Wing's locality theorem: a history is linearizable exactly in the case that the history of each object is.
7. The search ran out of budget before it finished, so neither "linearizable" nor "illegal" was shown. A checker that said either is lying. The author shortens the history, shrinks the hot set, or raises the budget and waits.
8. `existed`. A put's reply says only that the put happened. A delete's reply says whether a value was there, which pins the order of the delete against every put before it. The lost update and the duplicate apply are both caught by a delete that reports the wrong thing.
9. In the quiet period, after `stop_faults`. During the chaos a client is allowed to give up. Its node is allowed to be partitioned away or down, and a pending operation is not a bug. After the faults stop, nothing excuses an unanswered request.
10. Precedence was decided by instants. A client that receives a reply and sends its next request in the same event has two operations at one instant. The fix is a sequence number per recorded event, with precedence decided by the sequence.

</details>

## What's next

The simulator can run a swarm configuration, drive a workload across many clients, and say whether the store answered correctly. The pieces are all there and they are still assembled by hand. A test function draws a configuration, builds a world, runs the workload, stops the faults and checks. Module 9 makes that one program. `vopr --seed N` draws everything from the seed and prints the configuration. It runs the chaos phase for a number of requests, stops the faults, runs to convergence, and reports. Then it gets a real target. The store becomes a primary with backups and an explicit failover rule. That is the first version of it with genuine distributed bugs to find, and the first one where the liveness check has something to say.
