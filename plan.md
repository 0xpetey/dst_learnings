# Plan: Building the DST Curriculum

This file describes the task of producing the curriculum in `CURRICULUM.md`. `CURRICULUM.md` is the syllabus that a learner reads. This file is the work plan for the person who writes the modules. When you complete a module, update the status table at the bottom.

## Goal

Produce a complete, self-paced course on Deterministic Simulation Testing in Rust. The course has fourteen modules (0 to 13), as outlined in `CURRICULUM.md`. Every module ends in a coding task that extends one shared Cargo workspace. A learner who finishes the course has a replicated KV store and a VOPR-style simulator that they built themselves.

## Deliverables

1. `CURRICULUM.md`, the syllabus. It is written. When a module reveals a mistake in the plan, revise it.
2. `modules/NN-slug/README.md`, one full lesson per module. The template is below.
3. `project/`, the shared Cargo workspace that the learner builds. It ships as a skeleton for Module 1. Later modules add crates and files.
4. `solutions/NN-slug/`, a reference implementation of each module's task, kept out of the learner's path. It proves that each task is feasible. It also gives later modules a known starting point.
5. `README.md`, the entry point. It says what the course is, how to start, and how to find your way.

## Repository layout

```
dst_learnings/
  README.md
  CURRICULUM.md
  plan.md
  modules/
    00-why-dst/README.md
    01-determinism/README.md
    02-architecture/README.md
    03-rust-hazards/README.md
    04-virtual-time/README.md
    05-sim-network/README.md
    06-sim-disk/README.md
    07-fault-injection/README.md
    08-workloads-checking/README.md
    09-vopr/README.md
    10-consensus/README.md
    11-debugging-shrinking/README.md
    12-ci/README.md
    13-landscape/README.md
  project/            # learner's workspace, skeleton only
    Cargo.toml
    rust-toolchain.toml
    clippy.toml
    crates/
  solutions/
    01-determinism/
    02-architecture/
    ...
```

## Per-module lesson template

Each `modules/NN-slug/README.md` has these sections, in this order:

1. Why this module exists. Two or three paragraphs that connect the module to the previous module and to the end goal.
2. Objectives. The list from `CURRICULUM.md`, refined.
3. Lesson. The teaching content. It gives the learner enough to do the task without reading every linked source. Use ASCII or Mermaid diagrams where a picture helps (event queue, network delivery, disk sectors).
4. Readings. Each reading has a working URL, a one-line reason to read it, and a marker for required or optional. When you write a module, make sure that every link works.
5. Coding task, with these sub-sections:
   - Starting point. The solution directory or prior task that the learner begins from.
   - Requirements. Numbered, specific, testable.
   - Acceptance criteria. What `cargo test`, `cargo clippy`, and any scripted checks must show. Prefer checks that the learner can run locally and that CI runs too.
   - Hints. Collapsed or at the end, so that the learner opts in.
   - Stretch goals. Optional extensions.
6. Check your understanding. A few questions, with the answers at the bottom.
7. What's next. One paragraph that hands off to the following module.

## Conventions for the coding tasks

- Rust stable, pinned in `rust-toolchain.toml`. `Cargo.lock` is committed.
- The workspace profile sets `overflow-checks = true` and `debug-assertions = true` for every profile that the simulator uses.
- From Module 3 onward, the core crate is `#![no_std]` + `alloc` and `#![forbid(unsafe_code)]`. `clippy.toml` bans the nondeterministic methods and types that Module 3 lists.
- The Module 1 determinism check (run a seed twice, compare the trace hashes) is a test in the acceptance criteria of every later module.
- Every task is cumulative. A learner who completes Module N has exactly the starting point for Module N+1. If a later module needs something that earlier tasks did not produce, fix the earlier task. Do not hand-wave.
- When a seed finds a real bug in the reference solution, that seed becomes a regression test in the solution. The lesson mentions it as an example.
- Dependencies are minimal and justified. The simulator crate can use `rand_pcg`/`rand_xoshiro`, `clap`, and a serialization crate. The core crate uses nothing that touches the OS.

## Workflow for writing each module

1. Re-read the module's entry in `CURRICULUM.md`, and the lesson and solution of the previous module.
2. Gather the readings and make sure that each link works. Replace anything dead or weak. If a source contradicts a claim in the syllabus, fix the syllabus.
3. Build the reference solution first, from the previous solution. This shows what the task requires and how long it takes.
4. Write the task specification from the solution: requirements, acceptance criteria, and tests. The tests must pass against the solution and fail against the starting point.
5. Write the lesson so that it teaches exactly what the task needs, and no more.
6. Write the questions and the hand-off paragraph.
7. Review the module against the template and against `CURRICULUM.md`. If the module drifted, update the syllabus. Update the status table below.

## Order of work

Write the modules in numeric order, because each task depends on the last. There are two exceptions:

- Module 0 has no code dependency. Write it at any time.
- Module 13 depends on everything. Write it last, after a real port to `turmoil` or `madsim`, so that the comparison is honest.

Within Part I, write Modules 1, 2, and 3 together before you polish any of them. The Module 3 workspace restructure changes what Modules 1 and 2 must look like.

## Open decisions

Resolve each decision before or during the module that needs it. Record the decision in that module's lesson.

- Consensus protocol for Module 10. Raft is known to more learners and has many reference implementations. Viewstamped Replication is what TigerBeetle uses and fits the storage fault model better. The plan leans toward VSR. Decide after Module 9 shows how much protocol complexity the simulator can bear.
- Event loop ownership (Module 2). DECIDED: a hand-rolled single-threaded loop with explicit state machines (`kv_core::env::Driver`). The Module 2 lesson records it. Two related decisions were made there. The core defines its own `Instant(u64)`, because a fake clock cannot construct `std::time::Instant`. The driver is generic over the environment traits and does not use `dyn`.
- Tick model versus pure event queue (Module 4). DECIDED: a pure event queue, `sim::Scheduler`, with `(Instant, seq)` as the heap key. `World::run_for` in a loop gives the tick model, and a test proves that both drivers produce the same trace. The Module 4 lesson records the trade-offs. Two related decisions: the `sim` crate is `no_std` and depends on `kv-core` for `Instant`, and the fakes hold no handle to the scheduler. The world mediates.
- Linearizability checker depth (Module 8). A small brute-force checker for tiny histories, or a port of Porcupine's algorithm. Start with brute force. If the Module 10 histories are too long, revisit.
- Where solutions live. A separate directory (the current plan) or a git branch per module. The directory is simpler for a learner without git. If the repository becomes a git repository, revisit.
- Panic strategy (Module 3). DECIDED: `panic = "abort"` in the dev and release profiles. If the Module 11 shrinker wants to catch panics in-process instead of spawning child processes, revisit.
- HashMap policy (Module 3). DECIDED: `clippy.toml` bans the `std` types outright, because the lint cannot see the hasher of a defaulted `HashMap<K, V>`. `BTreeMap` is the default. If ordered maps are too slow, a `hashbrown` + `FxBuildHasher` wrapper is the escape hatch.

## Notes from writing Part I

- The acceptance tests for each task are copied into `modules/NN-slug/tests/`, so that a learner never has to open `solutions/`. Keep the two copies in sync.
- The Module 2 `known_seed_fingerprint` test pins a trace hash. Any change to a trace line in the node, the client, or the fake environment changes it. When a later module touches those, update the constant on purpose and say why in the commit.
- A plain-English hook lints the lessons (short sentences, no modal verbs, no semicolons). Write new lessons to pass it from the start.
- The readings were checked against their sources on 2026-10-10. Two syllabus claims did not survive. The matklad "Deterministic Walk" is a P99 CONF 2023 talk, not a blog post. TigerBeetle no longer has a standalone VOPR docs page. Both are fixed in `CURRICULUM.md`.
- Module 4 changed the fingerprint on purpose. Every trace line now carries the scheduler's timestamp, and the workload draws jitter from the seed. The acceptance tests for Module 4 live in two sub-directories, `tests/sim/` and `tests/kv-env/`, because they belong to two crates.
- The one-year test does not assert on wall-clock time. A CI machine under load makes that flaky. It prints the speed and the lesson states the expected range.
- The line "thousands of cluster-years per day on commodity hardware" in Module 0 was too strong. TigerBeetle's own figure is about 700-1000x real time per core, which is about two simulated years per core-day. The thousands come from a fleet. `CURRICULUM.md` now says so.

## Definition of done

- All fourteen lessons follow the template, and every link works.
- A new learner who follows the modules in order can complete every task. The reference solutions pass every acceptance criterion.
- The final reference solution runs tens of thousands of seeds clean on the consensus implementation. The bug journal from Module 10 is published as part of the lesson.
- `CURRICULUM.md` matches what the modules teach.

## Status

| Module | Lesson | Task spec | Reference solution | Notes |
|---|---|---|---|---|
| 00 Why DST | draft | draft | n/a | Reference program lives in the lesson and `modules/00-why-dst/starter/`. Measured 68-80% failure rate on an M1. |
| 01 Determinism | draft | draft | done | `solutions/01-determinism`. Skeleton with stubs and tests in `project/`. |
| 02 Architecture | draft | draft | done | `solutions/02-architecture`. Decided: hand-rolled event loop, own `Instant` type, generics over `dyn`. |
| 03 Rust hazards | draft | draft | done | `solutions/03-rust-hazards`. Decided: ban `HashMap` type, `panic = "abort"`, `#[expect]` markers. `scripts/check.sh` is the full suite. |
| 04 Virtual time | draft | draft | done | `solutions/04-virtual-time`. New crate `sim` (scheduler, clock, timer). Decided: event queue, not ticks. Fingerprint changed to `0xd982_fd72_6e9e_453a`. Two simulated years in 0.4 s release, 2 s debug. |
| 05 Sim network | not started | not started | not started | |
| 06 Sim disk | not started | not started | not started | |
| 07 Fault injection | not started | not started | not started | |
| 08 Workloads & checking | not started | not started | not started | |
| 09 VOPR | not started | not started | not started | |
| 10 Consensus | not started | not started | not started | |
| 11 Debugging & shrinking | not started | not started | not started | |
| 12 CI | not started | not started | not started | |
| 13 Landscape | not started | not started | not started | |

Status values: `not started`, `draft`, `reviewed`, `done`.
