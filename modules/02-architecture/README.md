# Module 2: Architecting for simulation

## Why this module exists

Module 1 gave you a way to tell whether two runs of a program were the same. It said nothing about how to write a program that passes that test and also does real work. Real work means talking to clients over a network, writing to a disk, and waiting for timeouts. This module covers that.

The idea is old and has several names: dependency inversion of the environment, "sans-I/O", and the state-machine style. A state machine is code that holds state and reacts to one input at a time. The core of the system never touches the outside world. It consumes events and emits commands.

The environment is the code that talks to the outside world. It turns commands into real socket writes and file syncs. It turns what comes back into events. Swap the environment for an in-memory fake, and the same core runs in a simulator. The swap needs no `cfg` flags and no mocking framework.

At the end of this module your workspace has four parts:

- A single-node key-value store with durable writes. A durable write survives a crash.
- A client with timeouts and retries.
- A real environment that serves the store over TCP.
- A fake environment that runs the store under the Module 1 determinism check.

Every later module adds to this workspace. Nothing in it is thrown away.

## Objectives

By the end of this module you can:

- Draw the boundary between core logic and environment, and name every crossing of it.
- Write a component as a state machine that takes inputs and emits outputs, and explain why it never blocks.
- Explain why a single-threaded event loop, one loop that handles one input at a time, is the simplest route to determinism.
- Explain why the course owns its loop instead of adopting an async runtime.
- Explain completion-based I/O (submit, then handle a completion later) and why it fits simulation.
- Choose between generic parameters and `dyn Trait` for the environment, and defend the choice.
- Explain why `std::time::Instant` cannot be the notion of time in the core, even in a crate that can use `std`.
- Treat assertions as design, and state the invariants (conditions that must always hold) of a node and a client.

## Lesson

### The boundary

Draw a line through your program. On one side is everything that is a pure function of its inputs. A pure function returns the same output for the same inputs. This side parses a request, decides to append to the log, updates the map, and chooses a reply. On the other side is everything that touches the world: sockets, files, the clock, threads. The design question of this module is how to keep that line sharp.

```text
            core (kv-core)                        environment (kv-env)
   ┌──────────────────────────────┐        ┌──────────────────────────────┐
   │  Node: state machine         │ Output │  real: TcpNetwork, FileStorage│
   │  Client: state machine       │ ─────► │        RealTime               │
   │  protocol: wire codec        │        │  fake: FakeNetwork, FakeStorage│
   │  log: WAL format             │ ◄───── │        FakeTime, TraceObserver│
   │  env: Time/Storage/Network   │ Input  │                               │
   │       traits + Driver        │        │  (the only code allowed to    │
   └──────────────────────────────┘        │   touch the OS)               │
                                           └──────────────────────────────┘
```

The core crate defines the vocabulary: `Input`, `Output`, and the `Time`, `Storage`, `Network` traits. The environment crate implements the traits twice. The `Driver` is the only event loop that the server has. It is generic over the traits. It lives in the core because it does no I/O itself. It calls methods on things that do.

### State machines in, state machines out

The `Node` in this module has this shape:

```rust
pub enum Input {
    Request(RequestEnvelope),
    ReadComplete { op: OpId, data: Vec<u8> },
    AppendComplete { op: OpId },
}

pub enum Output {
    Read { op: OpId },
    Append { op: OpId, offset: u64, bytes: Vec<u8> },
    Reply(ResponseEnvelope),
}

impl Node {
    pub fn new(out: &mut Vec<Output>) -> Node;
    pub fn on_input(&mut self, input: Input, out: &mut Vec<Output>);
}
```

That is the whole interface. The node never calls `write`, never calls `now`, and never waits. When it wants something from the world, it pushes an `Output` and returns. When the world has something for the node, the world calls `on_input`. The function is synchronous and finite. Every call ends.

This has three consequences that you will feel for the rest of the course:

- Testing is trivial. Construct a node, feed it inputs, and assert on outputs. The test needs no sockets, no temp files, and no sleeps.
- Simulation is the same as testing. A simulator is a loop that feeds inputs in an order of its choosing. To delay, drop, or reorder an input is a `VecDeque` operation.
- Determinism is structural. The node has no way to reach the clock or the scheduler. The only possible nondeterminism is in the environment, where you can see it and fence it.

The client has the same shape with time added. It takes `Tick { now }` and `Response`. It emits `Send`, `Done` and `GaveUp`. It exposes `next_deadline()` so that the environment knows the time to wake it. It never sleeps. Module 4 will drive those ticks from a virtual clock and run a year of retries in a second.

### Life cycle of the node

```text
   Node::new ──► Output::Read{op:0}
        │
   [Recovering]  requests arriving now are queued
        │
   Input::ReadComplete ──► decode log, apply entries, drain queue
        │
   [Running]
        │
   Request(Get)  ──► Reply immediately from the map
   Request(Put)  ──► Output::Append{op, offset, bytes}; remember it as pending
   AppendComplete{op} ──► pop oldest pending (assert op matches), apply, Reply
```

Two rules give the store its correctness:

- A write is applied only after it is durable. The node does not touch the map until the environment reports the append complete. Readers therefore see only acknowledged writes. A crash between the append and the completion loses an unacknowledged write. That loss is allowed.
- Storage completes appends in order. The node computes the offset of each append from the log length plus every pending append before it. That offset is right only under one condition. Completions must arrive in submission order. The node asserts this order. An environment that completes out of order broke the contract. The assertion says so at once, instead of letting offsets drift.

### Sessions, or why retries are safe

The client retries on timeout with the same request id. The node keeps one session per client. A session is the last request id of the client and its response.

A duplicate of a finished request gets the cached reply and is not applied again. A duplicate of a request that still waits for the disk is absorbed. The node sends the reply after the append completes. A request older than the last one is a reordered duplicate, and the node drops it.

This is a small amount of code. It is the difference between a store that is safe under retries and one that applies a delete twice. Module 8 will plant exactly that bug and watch the checker find it.

### The write-ahead log and the torn tail

The write-ahead log is the file where the node records each write before it applies it. Each log entry is `[u32 length][tag][key][value]`. Recovery reads entries front to back and stops at the first entry that is cut short.

The torn tail is that short last entry. It is what a crash between `write` and `fsync` looks like, and it is expected. The node remembers the length of the valid prefix. The next append lands at that length and overwrites the garbage.

A malformed entry that is not a plain truncation is different. The format has no way to produce one. If recovery sees one, something is wrong beyond a crash. The node refuses to start instead of skipping the entry. To skip a corrupt entry silently is silent data loss. Module 6 adds checksums and a sector model, and revisits this.

### Time without `Instant`

This module allows `std`, so the obvious type for "now" is `std::time::Instant`. It does not work. Understand why before Module 3 makes it a compile error.

There is no way to construct a `std::time::Instant` except to ask the OS for the current time. A fake clock cannot say "it is now the year 2031".

So the core defines its own `Instant`. It is a `u64` of nanoseconds since an epoch, a zero point for time that the environment defines. Its arithmetic is checked and panics on overflow in every profile. The real environment converts from the monotonic clock. The fake environment increments a counter. The core never knows the difference.

### Completion-based I/O

The `Storage` trait has `read_all(op)`, `append(op, offset, bytes)` and `poll_completions(&mut Vec<Input>)`. You submit an operation with an id. Later, you poll, and the completion for that id arrives as an `Input`. This is the shape of `io_uring` and of TigerBeetle's I/O layer. It is the right shape for simulation.

The real `FileStorage` is synchronous underneath. `append` writes, syncs, and queues `AppendComplete` for the next poll. From the point of view of the node, the completion still arrives on a later turn of the loop. That is exactly what happens under the fake. The fake can hold a completion back as long as it likes. That is how the tests make sure that a write is invisible until it is durable.

### The event loop decision

`plan.md`, the course's work plan, listed an open decision: a hand-rolled single-threaded loop with explicit state machines, or a minimal `async` executor written for the course. This module decides on the hand-rolled loop and keeps it.

The reasons are:

- The state machines are already explicit. An executor adds a second way to express "wait for a completion" on top of the one we have.
- `async` in Rust pulls toward a runtime, and the wake order of a runtime is the thing we try not to depend on. Module 3 shows how `tokio::select!` alone breaks determinism.
- The loop is twelve lines. It is a `Driver::step` that polls storage, polls the network, feeds each input to the node, and dispatches the outputs.

The cost is that long-lived flows, for example a client that waits for a reply, are written as state plus a `match`. They are not straight-line code with `.await`. That is the trade. It is the same trade that `quinn-proto`, `str0m` and TigerBeetle make. It is what makes the fake environment an ordinary struct.

### Generics or `dyn`?

The `Driver` is `Driver<T: Time, S: Storage, N: Network, O: Observer>`. It uses generic parameters, not trait objects. Monomorphization means that the compiler makes one copy of the code per environment type. So the fake environment costs nothing per event. That matters for a simulator that processes millions of events.

A mismatched environment is a compile error. The cost is a long type name, and a type alias hides it. If you ever need to choose the environment at runtime, `dyn` is a local change inside the environment crate, not a change to the core.

### Assertions are design

Every state machine in this module ends `on_input` with `check_invariants()`. The node asserts four invariants:

- Pending op ids strictly increase.
- Every in-flight session has exactly one pending write.
- Nothing is pending during recovery.
- The request queue is empty once the node is running.

The client asserts two invariants. A response is for a request that the client sent. A new request is never submitted over one in flight.

These are not debug aids. They are the specification, written where the compiler can run it. A simulator is only as good as the invariants that the system states about itself. The simulator supplies the adversarial inputs, and the assertions supply the definition of "wrong". The workspace keeps `debug-assertions` on in every profile so that there is one behavior, not two.

## Readings

Required:

- [matklad, A Deterministic Walk Down TigerBeetle's main() Street](https://www.p99conf.io/session/a-deterministic-walk-down-tigerbeetles-main-street/) (P99 CONF 2023). The talk puts I/O in a struct, runs a tick loop, and allocates at startup. This is the architecture that this module imitates.
- [Brett Cannon, Network protocols, sans I/O](https://sans-io.readthedocs.io/) and the [how-to](https://sans-io.readthedocs.io/how-to-sans-io.html). This is the original statement of the pattern. It is written for Python, but every word applies here.
- [Thomas Eizinger, sans-IO: The secret to effective Rust for network services](https://www.firezone.dev/blog/sans-io). This is the best Rust-specific case for the pattern. It argues for state machines plus an external event loop, and it states the costs honestly.
- [`quinn-proto`](https://docs.rs/quinn-proto/latest/quinn_proto/). This production QUIC implementation describes itself as fully deterministic and containing no networking code. Read the crate-level docs.
- [Polar Signals, Deterministic Simulation Testing in Rust: A Theater of State Machines](https://blog.polarsignals.com/blog/posts/2025/07/08/dst-rust). A Rust team makes the same choice as this module. It compares that choice against approaches that swap the runtime.
- [TigerStyle](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md). Read the Safety section again, this time for the assertion rules.

Optional:

- [str0m](https://github.com/algesten/str0m) README. str0m is a sans-I/O WebRTC stack. The README is the canonical text.
- [TigerBeetle `src/io/`](https://github.com/tigerbeetle/tigerbeetle/tree/main/src/io) and [`src/vsr/replica.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/replica.zig). These are the I/O abstraction and the replica that it feeds. Skim for shape, not detail.
- [matklad, Simulation Testing For Liveness](https://tigerbeetle.com/blog/2023-07-06-simulation-testing-for-liveness). This post lists the invariants that a simulator tests, and the split between safety and liveness.
- [The Geek Narrator, Assertions in production? with matklad](https://www.youtube.com/watch?v=W7ZpVfnLMP4). A long-form talk on the assertion philosophy.

## Coding task

### Starting point

Start from your Module 1 workspace with `dst start 02`. If you did not finish Module 1, `dst start 02 --from-solution` starts from `solutions/01-determinism/` instead. The command adds `fake_world.rs` and `real_server.rs` to `crates/kv-env/tests/` and replaces `scripts/check.sh`. Add two crates: `kv-core` and `kv-env`. The tests fix the public API that they use. The requirements list that API.

### Requirements

1. `kv-core` has no I/O. It uses `std` collections and formatting only. It never names `std::time`, `std::thread`, `std::fs`, `std::net` or `std::env`. For now, run `grep` to make sure that the crate names none of them. Module 3 makes it a compiler error.
2. `kv_core::time` defines `Instant` as nanoseconds in a `u64`. It provides `from_nanos`, `nanos`, `checked_add`, `saturating_duration_since`, and `Add<Duration>` that panics on overflow. It re-exports `core::time::Duration`.
3. `kv_core::protocol` defines `Request::{Get, Put, Delete}` and `Response::{Value, PutOk, DeleteOk, Rejected}`. It defines `RequestEnvelope` and `ResponseEnvelope` with `client: u64` and `request_id: u64`. It defines a length-prefixed frame codec (`encode_frame`, `decode_frame`). It defines the limits `MAX_KEY_LEN = 1024` and `MAX_VALUE_LEN = 1 MiB`. Decoding never panics on hostile bytes. A short buffer is `DecodeError::Incomplete`, not an error.
4. `kv_core::log` defines `Entry::{Put, Delete}`, `Entry::encode`, and `recover(&[u8]) -> Result<Recovered, DecodeError>`. `Recovered` has `entries` and `valid_len`. Recovery drops a truncated last entry, and `valid_len` excludes it. A malformed entry is an error.
5. `kv_core::node::Node` has the `Input`, `Output`, `new`, `on_input` shape from the lesson. It also has `is_running`, `stats`, `pending_writes`, `get`, `len`. The node applies writes only on `AppendComplete`. Appends carry the offset at which they land. Completions out of order panic. The node queues requests during recovery and serves them in arrival order once running. Sessions deduplicate retries as the lesson describes. Requests that violate a limit get `Response::Rejected` and never reach storage. The node asserts its invariants after every input with `assert!`, not `debug_assert!`.
6. `kv_core::client::Client` has `new(id, Config)`, `submit(now, request, out) -> request_id`, `on_input(Input::{Response, Tick{now}}, out)`, `next_deadline()`, `is_idle()`, `retries()`. On timeout, it resends the same envelope with the timeout doubled, up to `max_timeout`. It emits `GaveUp` after `max_attempts` sends. It ignores a reply for anything but the in-flight request. A reply for a request that the client never sent panics.
7. `kv_core::env` defines the `Time`, `Storage`, `Network`, `Observer` traits as in the lesson. It defines a generic `Driver` with `new`, `step() -> usize`, `run_until_idle()`.
8. `kv_env::fake` provides `FakeTime`, `FakeStorage`, `FakeNetwork`, `TraceObserver`, and `World`. `FakeTime` advances by hand. `FakeStorage` is a `Vec<u8>` log with explicit or automatic completion, `with_log` for restart, and `pending()`. `FakeNetwork` has two queues. `TraceObserver` records every input and output into a Module 1 `Trace`. `World` wires one node and any number of clients together. It provides `add_client`, `submit`, `advance`, `settle`, and `call`. It also provides `take_completed`, `drop_replies`, `completed`, `gave_up`, `trace()`.
9. `kv_env::real` provides `RealTime`, `FileStorage`, `TcpNetwork`, `serve(addr, data_dir, verbose, on_ready)` and `call(addr, client_id, request, config)`. `FileStorage` writes at the offset from the node, truncates, and calls `sync_data`. `TcpNetwork` does non-blocking accept and read, keeps per-connection buffers, and routes replies by client id. `call` drives a `Client` with the real clock and a read timeout.
10. The `kv` binary supports `serve <addr> <dir> [--verbose]`, `get`, `put`, `delete`. It prints `listening on <addr>` once bound. It exits non-zero on a rejected request.
11. The determinism check is a test. `seeded_workload_matches_model_and_is_deterministic` runs a seeded random workload against the fake environment. The workload has up to four clients, lost replies, and delayed completions. The test compares every reply against a reference `BTreeMap`. It runs the whole thing through `check_determinism` for twenty seeds. It must pass. `known_seed_fingerprint` pins the trace hash of seed 1. Your hash differs from the hash of the solution unless your trace lines are byte-identical. Replace the constant with yours after the workload test passes. Keep it pinned from then on.

### Acceptance criteria

```sh
cargo test --workspace                 # all green, including the real TCP test
cargo clippy --workspace --all-targets # no warnings
grep -rn "std::time\|std::thread\|std::fs\|std::net\|std::env" crates/kv-core/src | grep -v "//"
                                            # no output: comments do not count, code does
```

Then, by hand, in two terminals:

```sh
cargo run --bin kv -- serve 127.0.0.1:7000 /tmp/kv-data --verbose
cargo run --bin kv -- put 127.0.0.1:7000 greeting hello
cargo run --bin kv -- get 127.0.0.1:7000 greeting     # hello
# kill the server, start it again on the same directory
cargo run --bin kv -- get 127.0.0.1:7000 greeting     # still hello
```

`scripts/check.sh` runs `cargo fmt --check`, the three commands and then the restart, on a port the OS picks and in a temporary directory. It ends with "all checks passed".

### Hints

<details>
<summary>Order of work</summary>

Write `time` first. Then write `protocol` with its round-trip tests. Then write `log`. Then write `node` with unit tests that feed inputs by hand. Then write `client`. Only then write `env` and the fakes. Write the real environment last. It is the least interesting part. If you write it first, you will design the core around sockets.

</details>

<details>
<summary>Offsets with several pending appends</summary>

The offset of a new append has two parts. It is the durable log length plus the encoded length of every pending append. Keep `log_len` as "bytes known durable". Add to it only on `AppendComplete`. The fake storage asserts that the offset is not past the end of its log. It truncates to the offset before it writes. That truncation is also how a torn tail gets overwritten.

</details>

<details>
<summary>Routing client outputs in `World`</summary>

`client::Output::Done` does not name the client. Route outputs right after each call into a specific client, while you still know which client it was. Do not drain a shared buffer later.

</details>

<details>
<summary>The non-blocking server</summary>

Set the listener and every accepted stream to non-blocking. On each `poll_requests`, do four things in order. Accept until `WouldBlock`. Read each connection until `WouldBlock`. Decode as many complete frames as the buffer holds. Then write out the pending bytes of each connection until `WouldBlock`. Remove closed connections and rebuild the client-to-connection map. When `step()` processes nothing, sleep one millisecond. A production server blocks in `poll(2)`. This one is for learning.

</details>

<details>
<summary>The real-server test</summary>

The test spawns the `kv` binary with `env!("CARGO_BIN_EXE_kv")`. It waits for the `listening on` line. It runs the client binary against the server. It kills the server and starts it again on the same directory. To pick a free port, bind to port 0 first and read the port back.

</details>

### Stretch goals

- Add a `Get` fast path for a key that is absent from the recovered log. The fast path answers from the map during recovery. Explain in a comment why that is wrong, then remove it. It is wrong. Say precisely why.
- Make `World::settle` detect a client that waits on a reply that the network dropped, and report it. Do not rely on the test to advance time.
- Replace the one-millisecond sleep in the real server with `poll(2)` through the `libc` crate. Keep the sleep behind a feature flag for platforms that you did not test.

## Check your understanding

1. The node receives `AppendComplete { op: 2 }` while `op: 1` is the oldest pending write. What does it do, and why is that the right behavior?
2. A client times out and retries. Both the original and the retry reach the node. The append of the original is still pending. How many appends happen, and how many replies go out?
3. Why can the fake clock not produce a `std::time::Instant`? Why does that matter even in a crate that can use `std`?
4. The `Driver` is generic over four traits. Give one reason for generics and one situation where `dyn Trait` is the better choice.
5. The real `FileStorage` completes an append synchronously. Why does it still report through `poll_completions` instead of returning a result from `append`?
6. The workload test only ever has one operation in flight at a time. Suppose two clients overlap. What can the reference model no longer say, and which module addresses that?

<details>
<summary>Answers</summary>

1. It panics. The offsets of the node assume in-order completion. An out-of-order completion means that the environment broke its contract, and every later offset is wrong. To fail loudly at the first violation is better than to write at the wrong offset and find out at recovery.
2. One append and one reply. The retry sees a session whose last request id matches and whose response is `None`, so the node absorbs it. The reply goes out once, after the append completes.
3. `std::time::Instant` has no public constructor from a number, only `now()`. A fake clock must be able to say "it is now T" for any T. The core needs its own time type whether or not `std` is available.
4. Generics give zero-cost dispatch in a loop that runs millions of times, and a mismatched environment is a compile error. `dyn` lets you choose the environment at runtime, for example a binary that serves over TCP or over a Unix socket depending on a flag.
5. So that the node sees one shape of event under both environments. If `append` returned the result, the node applies the write in the same call under the real environment. Under the fake, it applies the write on a later turn. The two then behave differently. Completion-based I/O keeps the core identical.
6. The model assumes that operations take effect in the order it saw them. With overlap, a `Get` can legally return either the old or the new value, and the model cannot say which one is correct. That needs a linearizability checker, a tool that tests whether a history matches one sequential order. Module 8 builds it.

</details>

## What's next

The store works, and the fake environment runs it under the determinism check. But "works" currently depends on discipline. Nothing stops a contributor from adding `Instant::now()` to the node tomorrow. Module 3 catalogs every place where idiomatic Rust leaks nondeterminism. Then it removes the possibility. The core crate becomes `no_std`, the workspace gets lint bans, and any reintroduced hazard fails to compile.
