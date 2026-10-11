# Module 3: How Rust and `std` fight determinism

## Why this module exists

Module 1 cataloged the kinds of nondeterminism that exist. Module 2 built a store whose core, by discipline, touches none of them. Discipline does not survive contact with a second contributor, a deadline, or a dependency upgrade. This module replaces discipline with the compiler.

The module has two halves. The first half is a hunt. You write idiomatic Rust that looks deterministic and is not. You run it through the Module 1 check, and you watch the check catch each hazard and pass each fix.

The second half is structural. The core crate becomes `no_std`, and the workspace gets lint bans and a pinned toolchain. You then prove that any hazard pasted into the core fails to build.

The result is the workspace that every later module extends. From here on, "the node reads the clock" is not a code review comment. It is a compile error.

## Objectives

By the end of this module you can:

- Point to the exact `std` APIs that smuggle the OS into a program and say what each one reads.
- Explain why `tokio::select!` breaks determinism by design and how `biased;` restores it.
- Explain why a `static` is a cluster-wide shared variable once a cluster runs in one process.
- Audit a dependency tree for hidden clocks and random number generators.
- Split a workspace into a `no_std` core and a `std` environment, and say what `no_std` does and does not rule out.
- Use clippy's `disallowed_methods` and `disallowed_types` with `#[expect]` markers at the few legitimate call sites.
- Pin a toolchain and a profile so one seed means one run on every machine.

## Lesson

### Hazards in the standard library

Each item below is a hidden input. The fix always has the same shape. Move the read to the environment, and pass the value in.

#### `HashMap` and `HashSet`

The default hasher is `RandomState`. A hasher turns each key into a slot number. `RandomState` takes a seed from OS randomness once per process and then perturbs it per map. Two maps with the same contents iterate in different orders. Any code that walks a map is nondeterministic. Examples are code that picks the first healthy peer or drains a set of timers. Another example formats a map into a log line that is later hashed.

The fix is `BTreeMap`, or a `HashMap` with a fixed `BuildHasher` such as `FxBuildHasher` from `rustc-hash`. This course bans the `std` types outright. The lint cannot see which hasher a `HashMap<K, V>` with a defaulted third parameter uses. The `ahash` crate is not a fix either. Its README says that each map hashes differently unless you disable the `runtime-rng` feature.

#### `Instant`, `SystemTime`, `thread::sleep`

These three are direct syscalls. A syscall is a call from the program into the operating system. `Instant::now()` in core logic is a dependency on the real clock. There is no local fix. The caller must inject the clock. Module 2 already did this with `kv_core::Instant`.

#### Threads and everything that protects shared state

The hazards are `thread::spawn`, `Mutex`, `RwLock`, atomics, a channel with several senders, and `rayon`. Each one exposes the choices of the scheduler. A single-threaded core is the baseline. For the rare place where concurrency is unavoidable, `loom` and `shuttle` explore schedules systematically. That is a different tool for a different question.

#### `OnceLock`, `LazyLock`, `Once`

These are initialize-once primitives. The scheduler decides which thread wins. More important, the value lives for the whole process. See the section on the one-process cluster below.

#### `std::env`, `std::fs::read_dir`, `std::net`, `std::process`

All of these read the outside world. The order of `read_dir` depends on the filesystem. DNS resolution depends on the network. These calls belong in the environment crate and nowhere else.

#### Memory addresses

The hazards are `{:p}` formatting, `Arc::as_ptr`, `ptr::hash`, a map keyed by pointer, and a pointer mixed into an id. ASLR (address space layout randomization) moves the heap on every process start. The subtle part is that a sort by raw address survives ASLR. A slide of the whole heap preserves relative order. A hash of an address does not survive, because the slide changes every hashed value. The `pointer_order` hazard in this module shows both cases.

#### Debug versus release

Integer overflow panics in debug and wraps in release. `debug_assert!` disappears in release. A simulator with two behaviors is two simulators. The seed that fails in one passes in the other. The workspace pins `overflow-checks = true` and `debug-assertions = true` for every profile. TigerBeetle runs with assertions on in production for the same reason.

#### Floating point

Floating point is deterministic on one machine and toolchain. `f64::sin`, `exp` and `powf` come from the platform `libm` and differ across operating systems. These are transcendental functions, which means functions such as sine, exponent and power. Keep transcendental functions out of core, or depend on a pure-Rust `libm`. The `no_std` core in this module has no `f64::sin` at all, because those methods live in `std`.

#### Unstable sorts and the toolchain

`sort_unstable` is deterministic for a given compiler. It can reorder equal elements after a compiler upgrade. Pin the toolchain in `rust-toolchain.toml` and commit `Cargo.lock`. "Same seed, same result" must hold across machines, and that means the same binary.

`unsafe` code opens the door to undefined behavior, and undefined behavior can make anything nondeterministic. Put `#![forbid(unsafe_code)]` on the core, and run Miri on any code elsewhere that needs `unsafe`.

### Hazards in the async ecosystem

The default Tokio runtime is multi-threaded and work-stealing, which means idle threads take tasks from busy threads. `current_thread` removes the threads, but task wake order is still not under your control.

#### `tokio::select!`

By design, the macro picks a random ready branch to poll first, for fairness. Two channels that are both ready produce a coin flip per iteration. `biased;` makes the macro poll branches from top to bottom. The hazard crate in this module runs both versions through the determinism check. This one macro broke more "deterministic" tests than any other single construct.

In `FuturesUnordered`, `JoinSet` and `tokio::spawn`, completion order follows wake order. `tokio::time` follows the real clock unless the runtime starts paused with the `test-util` feature. Paused time is a limited form of the virtual time that Module 4 builds.

#### Why `madsim` and `turmoil` exist

Both replace the runtime, the network and the clock underneath unchanged application code. `madsim` swaps the `tokio` crate itself under `--cfg madsim`. `turmoil` provides a simulated `net` module and runs each host on a deterministic executor. They are the right choice for an existing Tokio codebase. This course owns its event loop instead, so there is nothing to swap. Module 13 ports the project onto one of them for an honest comparison.

### Hazards from running a cluster in one process

Every `static`, `thread_local!`, `OnceLock`, global metrics registry, global `log` or `tracing` subscriber, and singleton is now shared. The simulated replicas that share it believe that they are separate machines. Node B reads a counter that Node A incremented. The log attributes a line from Node A to Node B. State leaks silently.

The fix is structural. Use no mutable globals. All state hangs off a per-replica struct that the simulator creates. If you must have a global subscriber, tell it which simulated node is "current" before each step.

Panics need a decision too. A replica panic is an assertion failure of the simulator. An unwind runs the `Drop` impls of every frame that it passes. An unwind through half a cluster leaves the state in an unknown condition and can hide the evidence. This workspace sets `panic = "abort"` in the `dev` and `release` profiles. Cargo ignores that configuration for the `test` profile, so `#[should_panic]` tests keep working.

### Hazards from dependencies

Third-party crates call `Instant::now()` and `rand::random()`, spawn threads, and use `HashMap` freely. `uuid::new_v4`, `fastrand`, `getrandom`, and anything built on `tokio` are all suspects. A dependency that the core crate pulls in is a dependency on the real world.

Audit with three tools. `cargo tree -p kv-core` lists what the core depends on. After this module, the answer is nothing. A `grep` over vendored sources for the banned calls finds the hidden clock in a crate that calls itself pure. `cargo deny` bans crates by name, so a future `cargo add` cannot reintroduce one.

### Making the hazards unreachable

There are four mechanisms, from strongest to weakest.

#### `no_std` + `alloc`

The core crate declares `#![no_std]` and `extern crate alloc;`. `Vec`, `String`, `BTreeMap` and `VecDeque` come from `alloc`. `std::time`, `std::thread`, `std::fs`, `std::net`, `std::env`, `HashMap`, `Mutex`, `OnceLock`, `println!` and `f64::sin` do not exist. This is not about embedded targets. It is the strongest statement that Rust can make about what a crate cannot reach.

The proof is `cargo check --target thumbv7em-none-eabihf`. A bare-metal target has no `std` to fall back on. If the crate compiles there, nothing in it touches the OS.

The `determinism` crate gets the same treatment with a `std` feature. `Prng` and `Trace` are `no_std`, so the core can use them later. The `harness` module spawns processes, so it sits behind the feature.

The core also declares `#![forbid(unsafe_code)]`. An inner `allow` cannot override `forbid`. One line, and the core cannot contain undefined behavior.

#### Clippy bans

`no_std` is too strict everywhere outside the core. There, `clippy.toml` lists `disallowed-methods` and `disallowed-types` with a reason for each, and the workspace sets both lints to `deny`. The list bans the clocks, `thread::spawn` and `sleep`, `std::env`, `read_dir`, `rand::random` and `rand::rng`. It also bans `HashMap`, `HashSet`, `RandomState`, `Mutex`, `RwLock`, `OnceLock`, `LazyLock`, and pointer-to-integer helpers.

The real environment legitimately reads the clock and sleeps. Each such site carries `#[expect(clippy::disallowed_methods, reason = "this is the real clock")]`, not `#[allow]`. If the lint stops firing, `#[expect]` fails the build, so the markers cannot rot. A `grep` for `#[expect(clippy::disallowed` lists every place where the program touches the OS.

Three more lints close the pointer hole that `no_std` leaves open. `borrow_as_ptr`, `ref_as_ptr` and `ptr_as_ptr` catch a reference that turns into a pointer. That cast is the first step toward an address-derived value.

#### The determinism check

Lints are a net, not a wall. `ptr::from_ref(x).addr()` still compiles. A dependency can still hide a clock behind a feature flag. The Module 1 check runs on every change in CI. It is the last line of defense and the only one that catches what nobody thought to ban.

### Decisions recorded

- The workspace bans `HashMap` and `HashSet` from `std` as types, not only their constructors. `BTreeMap` is the default. If ordered iteration is too slow, a fixed-hasher map from `rustc-hash` is the escape hatch.
- `panic = "abort"` applies in `dev` and `release`. A panic is a found bug.
- `clippy.toml` lives at the workspace root and applies to every crate. Crates that exist to demonstrate hazards opt out with a crate-level `#![allow]` and say so in their docs.

## Readings

Required:

- [`std::collections::HashMap`](https://doc.rust-lang.org/std/collections/struct.HashMap.html), the paragraphs on `RandomState` and `with_hasher`.
- [`rustc-hash`](https://docs.rs/rustc-hash/latest/rustc_hash/) and the [aHash README](https://github.com/tkaitchuck/aHash). One is a fixed hasher. The other is random by default and says so.
- [`tokio::select!`](https://docs.rs/tokio/latest/tokio/macro.select.html), the Fairness section.
- [`tokio::time::pause`](https://docs.rs/tokio/latest/tokio/time/fn.pause.html) and [`#[tokio::test(start_paused = true)]`](https://docs.rs/tokio/latest/tokio/attr.test.html). These retrofit virtual time onto a runtime.
- [madsim](https://github.com/madsim-rs/madsim) and [turmoil](https://github.com/tokio-rs/turmoil) READMEs, plus the [turmoil docs](https://docs.rs/turmoil/latest/turmoil/). These are two ways to swap the world under unchanged code.
- [Clippy: `disallowed_methods`](https://rust-lang.github.io/rust-clippy/master/index.html#disallowed_methods) and [`disallowed_types`](https://rust-lang.github.io/rust-clippy/master/index.html#disallowed_types). This is the configuration format that this module uses.
- [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html). The page lists the defaults of `overflow-checks`, `debug-assertions` and `panic`, and says why those defaults are a problem.
- [sled: simulation guide](https://sled.rs/simulation.html). The Rust community uses this recipe. It pairs deterministic state machines with a discrete-event simulator, which jumps from event to event.
- [S2, Deterministic simulation testing for async Rust](https://s2.dev/blog/dst). The post retrofits DST (deterministic simulation testing) onto a Tokio codebase. A team that lived it tells the "`std` fights you" story.

Optional:

- [loom](https://github.com/tokio-rs/loom) and [shuttle](https://github.com/awslabs/shuttle). These explore schedules for the places where threads are unavoidable. Neither models a clock.
- [cargo-deny](https://embarkstudios.github.io/cargo-deny/). It bans crates by name.
- [The `#[expect]` attribute](https://doc.rust-lang.org/reference/attributes/diagnostics.html#the-expect-attribute).
- [RisingWave, Deterministic Simulation: A New Era of Distributed System Testing](https://risingwave.com/blog/deterministic-simulation-a-new-era-of-distributed-system-testing/). The authors of `madsim` give its design rationale.
- [Stevan Andjelkovic, Introduction to simulation testing](https://stevana.github.io/introduction_to_simulation_testing.html). This is a language-neutral framing.

## Coding task

### Starting point

Start from your Module 2 workspace, or from `solutions/02-architecture/`. The acceptance tests and scripts are in `modules/03-rust-hazards/`: `tests/hunt.rs`, `scripts/check.sh`, `scripts/hazard-check.sh` and a reference `clippy.toml`.

### Requirements

#### Part 1 and 2: the hunt

Add a crate `hazards` with `rand` and `tokio` (`rt`, `macros`, `sync`, `time`) as dependencies. It is the one crate that can use banned APIs. It says so with a crate-level `#![allow(clippy::disallowed_methods, clippy::disallowed_types)]`.

1. Implement these six pairs. Each function is a `fn(&mut Prng, &mut Trace)`:
   - `hazard_hashmap_peer` / `fixed_hashmap_peer`: pick the first healthy peer from a map.
   - `hazard_select_loop` / `fixed_select_loop`: a `tokio::select!` over two always-ready channels, 64 iterations, with and without `biased;`.
   - `hazard_thread_rng_backoff` / `fixed_thread_rng_backoff`: jitter from `rand::random` versus from the `Prng`.
   - `hazard_pointer_order` / `fixed_pointer_order`: a `BTreeMap` keyed by a hash of each replica's address versus of its id.
   - `hazard_static_counter` / `fixed_static_counter`: two "nodes" that share a `static AtomicU64` versus two that each own a field.
   - `hazard_hidden_clock` / `fixed_hidden_clock`: a helper module that reads `SystemTime` internally versus one that takes `now` as a parameter.
2. Add a binary `pointer_order`. It prints the trace hash of the pointer hazard in the Module 1 `trace_hash=` format. With `--fixed`, it prints the hash of the fix.
3. Copy `tests/hunt.rs` into the crate. It asserts three things. `check_determinism` with eight runs catches every hazard. Every fix passes. The cross-process check catches the pointer hazard whether or not the in-process check saw it.

#### Part 3: make it unreachable

4. Convert `kv-core` to `#![no_std]` with `extern crate alloc;`. Add `#![forbid(unsafe_code)]`, `#![deny(clippy::disallowed_methods, clippy::disallowed_types)]`, and `#![deny(clippy::borrow_as_ptr, clippy::ref_as_ptr, clippy::ptr_as_ptr, clippy::fn_to_numeric_cast_any)]`. Change every `std::` path in the crate to `core::` or `alloc::`. Make test modules import `alloc::vec`.
5. Convert `determinism` to `#![cfg_attr(not(feature = "std"), no_std)]` with a default `std` feature. Make `harness` and the `trace_hash` binary require the feature. Make `Prng` and `Trace` build without it.
6. Put a `clippy.toml` at the workspace root that bans at least everything in the reference file. In the workspace `Cargo.toml`, set `disallowed_methods` and `disallowed_types` to `deny` under `[workspace.lints.clippy]`. Give every crate `[lints] workspace = true`.
7. Mark every legitimate use of a banned API in `kv-env` with `#[expect(..., reason = "...")]`. Use exactly as many markers as there are OS touch points. Do not use `#[allow]`.
8. Pin a channel in `rust-toolchain.toml` and commit `Cargo.lock`. Set `overflow-checks = true`, `debug-assertions = true` and `panic = "abort"` in the `dev` and `release` profiles.
9. Make sure that `scripts/check.sh` passes end to end. It runs `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo check -p kv-core --target thumbv7em-none-eabihf`, the same for `determinism --no-default-features`, `cargo test --workspace`, and `scripts/hazard-check.sh`.
10. Make sure that `scripts/hazard-check.sh` passes. It pastes sixteen snippets one at a time into `kv-core` and requires `cargo clippy` to fail for each. The snippets are:
    - `Instant::now`, `SystemTime::now`, `thread::spawn` and `thread::sleep`.
    - `HashMap`, `env::var`, `read_dir` and `TcpStream`.
    - `Mutex`, `OnceLock`, `static mut` and `unsafe`.
    - A reference cast to an integer, `f64::sin` and `println!`.
    - A `HashMap` reached through `alloc`.

### Acceptance criteria

```sh
rustup target add thumbv7em-none-eabihf     # once
scripts/check.sh                            # ends with "all checks passed"
cargo tree -p kv-core                       # kv-core alone, no dependencies
grep -rn "#\[expect(clippy::disallowed" crates/kv-env   # one line per OS touch point
```

The Module 2 tests still pass unchanged, including `known_seed_fingerprint`. The restructured crates must not change a single trace line.

### Hints

<details>
<summary>Convert `kv-core` to `no_std`</summary>

Add the attributes. Then let the compiler list what is missing. `Vec` and `String` need `use alloc::vec::Vec;` and `use alloc::string::String;`. `BTreeMap` and `VecDeque` come from `alloc::collections`. `fmt` comes from `core`.

The `vec!` macro needs `use alloc::vec;` in each module that uses it. `format!` is `alloc::format!`. `Duration` was always in `core::time`.

</details>

<details>
<summary>`#[expect]` placement</summary>

`disallowed_types` fires where the type is written. A struct with a `std::time::Instant` field needs the marker on the struct. Each function that names the type in a signature or body needs its own marker. `disallowed_methods` fires at the call. If you put a marker where nothing fires, the build fails with `unfulfilled_lint_expectations`. That is the point.

</details>

<details>
<summary>Why `HashMap` reached through `alloc` is in the probe list</summary>

`alloc::collections::HashMap` does not exist. So the probe fails to compile. The probe documents a gap. The `hashbrown` crate underlies the `std` map and is `no_std`. If you add it as a dependency, it compiles fine. `cargo deny` or a `cargo tree` check in CI is the guard for that.

</details>

<details>
<summary>The check does not catch the `select!` hazard</summary>

The test makes eight runs of 64 coin flips each. If the check ever passes, the channels were not both ready. Fill both channels completely before the loop, so that neither is ever empty while the loop runs.

</details>

### Stretch goals

- Add `cargo deny` with a `bans` list for the `kv-core` crate. The list contains `rand`, `getrandom`, `uuid`, `fastrand`, `tokio` and `parking_lot`. Wire it into `check.sh`.
- Write a lint-free fixed-hasher map wrapper over `hashbrown` with `FxBuildHasher`. Prove that it iterates deterministically under the check. Benchmark it against `BTreeMap` for a thousand keys.
- Add a `tracing` subscriber to the fake environment that prefixes every event with the current simulated node. Then demonstrate the leak without it, with two nodes and one global subscriber.

## Check your understanding

1. `HashMap::new` is banned, but so is the type. Why is a ban on the constructor not enough?
2. A `no_std` crate still has `core::ptr`. What address-derived hazard remains possible, and what catches it?
3. Why does the real clock carry `#[expect]` and not `#[allow]`?
4. The in-process check catches the `static` counter hazard but not always the pointer hazard. What is different about the two?
5. Why does the workspace set `panic = "abort"`, and why does that not break `#[should_panic]` tests?
6. `madsim` makes an unchanged Tokio program deterministic. Why does this course not use it from the start?
7. The `kv-core` crate compiles for `thumbv7em-none-eabihf`. What exactly does that prove, and what does it not prove?

<details>
<summary>Answers</summary>

1. A `HashMap<K, V>` with its third parameter defaulted still uses `RandomState`. Code can create it through `Default::default()`, `collect()`, or `FromIterator` without the name `new`. The lint sees the type path. It does not see the hasher.
2. Code can cast a reference to a pointer, cast the pointer to an integer, and then hash or compare the integer. `no_std` does not stop that. The `ref_as_ptr` and `borrow_as_ptr` lints catch the common forms. The cross-process determinism check catches whatever gets through.
3. `#[allow]` silences the lint forever, even after someone deletes or moves the code that it guards. If the lint stops firing, `#[expect]` fails the build. Therefore every marker sits on a real OS touch point.
4. The `static` keeps its value across the two runs, so run two starts from the end state of run one. The allocator hands out addresses. Within one process, it tends to reuse the same addresses after a free. Only a new process gets a new heap layout.
5. A panic in a simulated replica is a found bug. An unwind through the rest of the cluster runs `Drop` impls and leaves the state undefined. Cargo ignores the `panic` configuration for the `test` profile, so tests still unwind.
6. The course wants you to own the event loop, so that you understand what the runtime swap replaces. An owned loop also means that the core has no runtime at all. That is simpler to reason about than a runtime with a replaced clock. Module 13 ports to `madsim` or `turmoil`, so you make the comparison with real experience.
7. It proves that nothing in the crate reaches `std`, because `std` does not exist for that target. It does not prove that the crate is deterministic. Pointer casts, floating point in `core`, and logic bugs all compile fine. The determinism check covers what the type system cannot.

</details>

## What's next

Part I is complete. You have a seed and a check, a store written as state machines, and a workspace where the core cannot reach the OS. Part II builds the simulator that those state machines run inside. Module 4 replaces the hand-advanced fake clock with a discrete-event scheduler and virtual time. It runs a year of client retries in under a second.
