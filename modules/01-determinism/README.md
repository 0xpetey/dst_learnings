# Module 1: Determinism fundamentals

## Why this module exists

Module 0 left you with a program that fails two runs in three and never in the same way twice. You wrote down what you need to control to make one failure repeat. This module builds the first piece of that control: a precise, mechanical way to say whether two runs of a program were the same.

Everything later in the course rests on this check. A simulator that is "mostly deterministic" is a flaky test with extra steps. When seed 8812 finds a bug on the CI machine, the same seed must find the same bug on your laptop. It must find the bug at the same virtual time, with the same trace.

Virtual time is the clock that the simulator keeps. A trace is the record of every decision in a run. Every decision in the run must flow from the seed and from nothing else.

This module does three things. It catalogs the sources of nondeterminism that you will meet. It gives you a seeded PRNG with a pinned algorithm, and a trace log with a stable hash. It also gives you the determinism check that the rest of the course runs on every change. The check runs a seed twice, compares the two hashes, and fails on any difference.

## Objectives

By the end of this module you can:

- Name each source of nondeterminism in a typical program, and say which kind of check detects it.
- Explain why the seed is the single root of randomness. Explain why derived streams, child generators forked from one parent, keep seeds stable as a simulator grows.
- Explain why determinism must hold at the bit level for traces to be comparable.
- Build a PRNG wrapper with a pinned algorithm, and explain why `rand::StdRng` is the wrong choice.
- Build a trace log with a running hash, and use it to find the first point where two runs diverge.
- Write the determinism check in two forms, in-process and cross-process. Say what each one catches and what each one misses.

## Lesson

### What "deterministic" means here

A deterministic program's outputs are a pure function of its inputs. The hard part is that the inputs include everything that the program reads from the outside world. Most of those reads are invisible in the source.

The clock is an input. The thread scheduler is an input. The OS entropy pool is an input too, because it seeds `HashMap`. Find every one of those hidden inputs. Then remove each one, or route it through a single place that you control.

For simulation, the one input that you keep is the seed. A 64-bit integer goes in, and a complete run comes out. Everything that the simulated cluster does is derived from the seed. That includes every message delay, every disk fault, and every client request. The picture is:

```text
seed ──► Prng ──► every decision ──► trace ──► hash
         │
         ├──► fork() ──► network's Prng
         ├──► fork() ──► disk's Prng
         └──► fork() ──► each client's Prng
```

Two runs with the same seed must produce the same hash. You can test this rule, and the test is the heart of the module.

### The catalog

Each row below is a hazard, a way for the outside world to leak into your program. The last column says which form of the determinism check sees it, in-process or cross-process. You build both forms in the coding task.

| Source | Where it hides | Caught by |
|---|---|---|
| Wall clock | `SystemTime::now()`, timestamps in logs | in-process |
| Monotonic clock | `Instant::now()`, measuring elapsed time, timeouts | in-process |
| Thread scheduling | any two threads touching shared state | in-process, with enough runs |
| I/O completion order | several outstanding reads or writes, channel receive order | in-process, with enough runs |
| OS randomness | `rand::random`, `getrandom`, `uuid::new_v4` | in-process |
| Hash map iteration | every `HashMap` and `HashSet` with the default hasher | in-process |
| Memory addresses | `{:p}`, pointer casts, sorting by `Arc::as_ptr` | cross-process only |
| Environment | `std::env::var`, hostname, locale, current directory | neither, unless the input changes between runs |
| Process-wide state | `static`, `OnceLock`, `thread_local!`, global registries | in-process, on the second run |
| Uninitialized memory | `MaybeUninit` misuse, `unsafe` | no check, use Miri |
| Floating point | `sin`, `exp`, `powf` from the platform `libm` | neither, differs across machines |
| Unstable sort | `sort_unstable` on equal keys | neither, differs across toolchains |

Two rows need a closer look.

The check catches thread scheduling only with enough runs. Two threads that each push to a shared vector usually interleave differently. But on a quiet machine they sometimes finish one after the other, and then the trace matches. That is why the check takes a `runs` count. It is also why the thread tests in this module use a barrier to force the threads to overlap.

Memory addresses are the reverse. Within one process, the allocator hands the second run the same addresses that the first run freed. The in-process check passes. Between processes, ASLR slides the heap, and the addresses change. Only a check that spawns the program twice sees this.

### Bit-level determinism

"Same result" is not enough. Two runs that both say "cluster converged" but took different paths are not reproducible. If the bug is on the second path, you see it only half the time.

The standard is bit-level determinism: the complete sequence of decisions is identical. To make that testable, write every decision into a trace and hash it. Equal hashes mean equal runs. Unequal hashes mean a leak, and the stored trace lines show you exactly where.

That makes the hash function part of your contract. It must be stable forever. This module uses FNV-1a, 64-bit, written out in about six lines. It is not cryptographic, and it does not need to be. It must never change under you, and that rules out any hash from a crate whose algorithm is an implementation detail.

### The PRNG

The same reasoning applies to the PRNG. `rand::StdRng` is good randomness, but its documentation says that the algorithm can change between versions. A seed recorded today with `StdRng` is a different run after `cargo update`. For a simulator, that is fatal.

The course pins xoshiro256++ (Blackman and Vigna, 2018). It is fast, it has 256 bits of state, and its reference implementation is short. SplitMix64, a second small generator, expands the 64-bit seed to 256 bits of state, as the authors recommend. You write both out by hand. A test then compares your output with the `rand_xoshiro` crate given identical state. That way you know that you implemented the real algorithm and not a lookalike.

Two details matter. The first is unbiased ranges. `next_u64() % n` is biased for most `n`. The bias is small, and that makes it worse. It is a subtle nondeterminism in the distribution, and it changes which bugs you find. Use Lemire's multiply-and-reject method or another unbiased scheme.

The second is derived streams. Suppose the network, the disk, and ten clients share one `Prng`. Then one new random decision in the disk model shifts every later decision in every other component. Every recorded seed changes meaning.

`fork()` derives a child generator from the parent. Each component then owns its own stream, and a change in one does not disturb the others. This does not make old seeds valid forever, but it limits the damage from a change.

### Two checks, not one

The in-process check runs a closure twice, with a fresh `Prng` and `Trace` each time, and compares the two runs. The check is fast, it reports the first differing line, and it catches most of the catalog.

Everything that the closure touches must be constructed inside it. If the closure reaches for anything outside, the second run sees the leftovers of the first run, and the check fails. That is a feature. It is exactly how two simulated nodes that share a `static` fail later in the course.

The cross-process check spawns a binary twice as separate OS processes and compares the hash that each one prints. It is slower, and it cannot show you the differing line. But it is the only way to see hazards that are fixed for the lifetime of a process. Examples are heap addresses, a `static` initialized once, and an environment variable read at startup.

A divergence report from the in-process check looks like this:

```text
seed 1 is not deterministic: trace hashes 0x8a3f... vs 0x11c0...
first divergence at line 0:
  run 1: peer order [41, 7, 23, 0, ...]
  run 2: peer order [3, 58, 19, 44, ...]
```

That one report tells you the hazard (a hash map iterated), the line that leaked it, and the seed to reproduce it. The whole course is about making every bug look like this.

### What the check cannot do

A passing check proves that the program is a function of its seed on this machine, with this toolchain, and in this environment. Floating point transcendentals differ between platforms. Unstable sorts differ between compiler versions. An environment variable read at startup is the same in both runs, so the check cannot see it.

The fixes for those are structural, not checks. Pin the toolchain. Commit the lockfile. Keep `libm` out of core logic. Pass configuration explicitly. Module 3 turns most of those into compile errors.

## Readings

Required:

- [TigerStyle](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md), the Safety section. It describes the design discipline that makes a simulator tractable: assertions everywhere, bounds on everything.
- [FoundationDB: Simulation and Testing](https://apple.github.io/foundationdb/testing.html). One page on the single-threaded deterministic model that started all of this.
- [The Rust Rand Book: Reproducibility](https://rust-random.github.io/book/crate-reprod.html). It says which `rand` items are stable across versions and which are not. Read it before you use `StdRng` in a simulator.
- [matklad, Random Fuzzy Thoughts](https://tigerbeetle.com/blog/2023-03-28-random-fuzzy-thoughts). Seeds versus saved inputs, and why a seed stops meaning the same thing as the code changes.
- [`rand_xoshiro::Xoshiro256PlusPlus`](https://docs.rs/rand_xoshiro/latest/rand_xoshiro/struct.Xoshiro256PlusPlus.html). The tests compare your implementation against this reference.

Optional:

- [Vigna, A PRNG shootout](https://prng.di.unimi.it/) with the reference C for [xoshiro256++](https://prng.di.unimi.it/xoshiro256plusplus.c) and [SplitMix64](https://prng.di.unimi.it/splitmix64.c). Public domain, and shorter than this paragraph.
- [The Rust Rand Book: Our RNGs](https://rust-random.github.io/book/guide-rngs.html). A comparison of the generator families.
- [Blackman and Vigna, Scrambled Linear Pseudorandom Number Generators](https://arxiv.org/abs/1805.01407). The theory behind xoshiro.
- [`rand_pcg`](https://docs.rs/rand_pcg/latest/rand_pcg/). The other small generator that people pin. Note its caveat about correlated streams.

## Coding task

### Starting point

Start in `project/` at the root of this repository. Run `dst init` once, then `dst start 01`. The directory is a Cargo workspace with one crate, `determinism`, and a `scripts/check.sh`. The crate has a written public API, and its bodies are `todo!()`.

The tests in `project/crates/determinism/tests/` are the acceptance tests. They compile against this skeleton and fail. Your job is to make them pass. Do not change their contents or the public signatures.

### Requirements

1. Make `Prng::from_seed(u64)` expand the seed with SplitMix64 into xoshiro256++ state. Make `Prng::next_u64` one xoshiro256++ step. The test in `tests/reference_prng.rs` compares 10 000 outputs for six seeds against `rand_xoshiro`. It must pass.
2. Make `Prng::below(n)` unbiased, and make it panic on `n == 0`. Build `shuffle` as Fisher-Yates on top of `below`. Make `fork` derive a child from `next_u64`.
3. Make `Trace::record` hash the formatted line plus a newline with FNV-1a 64-bit. Store the line unless the trace is `hash_only`. Write a known-answer test to pin the algorithm. The FNV-1a hash of the single byte `a` is `0xaf63dc4c8601ec8c`.
4. Make `Trace::first_divergence` return the index and both lines of the first difference. Past the end of the shorter trace, either line can be `None`.
5. Make `check_determinism(seed, runs, program)` run the closure `runs` times, with a fresh `Prng` and `Trace` each time. Compare each run to the first. Return the hash, or a `Divergence` that names the first differing line. Make `run_twice` the two-run case.
6. Make the `Display` of `Divergence` print the seed, both hashes, and both versions of the first differing line.
7. Make `run_process_twice(binary, args, seed)` spawn the binary twice with `--seed N` appended. Read the last `trace_hash=<hex>` line from each stdout, and compare the two. Treat a non-zero exit or a missing hash line as its own error, not as a divergence.
8. Make `seed_from_args` accept both `--seed N` and `--seed=N`.
9. The `trace_hash` binary in `src/bin/` is given. It uses your library to print a hash, with `--hazard address` and `--hazard env` variants. Make it run.
10. Make every test in `tests/hazards.rs` pass. Each one introduces one row of the catalog. Each one asserts that the in-process check catches the row, or documents that it does not. Make every test in `tests/cross_process.rs` pass.

### Acceptance criteria

Run these commands from `project/`:

```sh
cargo test                       # all tests pass
cargo clippy --all-targets       # no warnings
cargo run --bin trace_hash -- --seed 7           # prints trace_hash=<16 hex digits>
cargo run --bin trace_hash -- --seed 7           # prints the same line
cargo run --bin trace_hash -- --seed 7 --hazard address   # run twice: different lines
```

Run the hazard tests twenty times in a loop. They must pass every time. If a thread test passes 19 of 20 times, your barrier or workload is too small.

`scripts/check.sh` runs all of the above, including the twenty runs, and ends with "all checks passed". It adds `cargo fmt --check`. `dst check` runs the script and prints one line per step. When every step passes, `dst next` opens Module 2.

### Hints

<details>
<summary>SplitMix64 and xoshiro256++ in Rust</summary>

Both are direct transcriptions of the C reference. Use `wrapping_add` and `wrapping_mul`, never plain `+` and `*`. The workspace has overflow checks on in every profile, so plain arithmetic panics. `rotate_left` is the `rotl` helper.

SplitMix64 adds the golden-ratio constant `0x9E3779B97F4A7C15` to its state. Then it mixes the result with the two multipliers from the reference. Fill the four state words from four consecutive SplitMix64 outputs.

</details>

<details>
<summary>Unbiased `below`</summary>

Use Lemire's method. Multiply a 64-bit random `x` by `bound` as a 128-bit product. The high 64 bits are the result. The low 64 bits tell you whether this `x` fell in the small band that biases the result.

If `low < bound`, compute `threshold = bound.wrapping_neg() % bound`. Then reject while `low < threshold`. Powers of two never reject.

</details>

<details>
<summary>Comparing runs</summary>

Run once, and keep the `Trace`. For each later run, compare the hashes first. If they differ, call `first_divergence` to find the line. Build the `Divergence` from the seed, the index, the two optional lines, and both hashes.

</details>

<details>
<summary>The thread tests</summary>

If `thread_interleaving_is_caught` sometimes passes although it must fail, the threads ran one after another. The given test uses a `Barrier`, so all four threads start together and each pushes ten thousand times. If your machine still produces identical interleavings, raise the push count. Do not lower the run count in the test.

</details>

### Stretch goals

- Add `Prng::next_f64` that produces a double in `[0, 1)` from the top 53 bits. Write a test that its output is bit-identical to the `rand_xoshiro` route. Then note in a comment why the course still keeps floats out of core logic.
- Add a `Trace::diff(&self, other)` that prints a unified diff of two traces. Use it in the `Display` of `Divergence`.
- Make `run_process_twice` capture and compare the full trace, not only the hash. To do that, have the binary write lines to a file named on the command line.

## Check your understanding

1. The in-process check runs a program twice, and the trace hashes match. Name two hazards that can still be present.
2. A test depends on `rand_xoshiro` anyway. Why does the course write out xoshiro256++ instead of depending on `rand_xoshiro` directly?
3. A simulator has one `Prng` shared by the network model and the disk model. You add a new random decision to the disk model. What happens to seed 8812, which found a bug last week?
4. A colleague proposes to hash only the final state of the cluster instead of the whole trace, to save memory. What do you lose?
5. The bias of `x % n` is below one part in a million for small `n`. Why does `below` still need an unbiased algorithm?
6. The `static_state_is_caught` test fails on the second run, not the first. Why? What later situation in the course does this model?

<details>
<summary>Answers</summary>

1. Memory addresses, because the allocator reuses them within a process. Also anything read once per process, such as an environment variable. Floating point and unstable sorts can also be present, but they differ only across machines or toolchains.
2. A dependency is a dependency on the versioning policy of someone else. Writing the generator out makes the meaning of the seed part of the course's own source. The crate is used only in a test, as an independent witness that the implementation is the real algorithm.
3. The disk model now consumes one more number from the shared stream. Every decision after that point in the run, in both models, shifts. Seed 8812 is a different run, and it probably no longer finds the bug. With per-component `fork()` streams, only the decisions of the disk shift.
4. Two runs that reach the same final state by different paths look identical. The bug on the path that you did not see stays hidden. The trace hash is the fingerprint of how the run happened, not only of what it produced.
5. The bias itself is harmless. The problem is that the bias is a property of the implementation, not of the seed. If you replace the biased method with an unbiased one later, every recorded seed changes meaning. Pick the correct method once and never touch it.
6. The `static` survives from run one to run two, so run two starts with a counter that is already non-zero. Two simulated replicas in one process that share a global fail in the same way. Node B sees the state of node A and believes that it is its own.

</details>

## What's next

You now have a seed, a generator, a trace, and a check. Module 2 builds the thing worth checking: a key-value store written as a pure state machine. A pure state machine has no hidden inputs. Time, network, and storage sit behind interfaces that have a real implementation and a fake one. The fake one runs under the check that you wrote in this module. From then on, every change to the store must keep that check green.
