# Deterministic Simulation Testing in Rust

A self-paced course. You build a small replicated key-value store and the simulator that tortures it, in the spirit of TigerBeetle's VOPR and FoundationDB's simulation. The course has fourteen modules, one Cargo workspace, and Rust throughout.

## Start here

1. Read [`CURRICULUM.md`](CURRICULUM.md) for the syllabus and the shape of the whole course.
2. Install Rust with `rustup`. The project pins its toolchain in `project/rust-toolchain.toml`, so the right version installs itself on the first build. Module 3 also needs one extra target. Install it with `rustup target add thumbv7em-none-eabihf`.
3. Work through `modules/` in order. Each module directory has a `README.md` with the lesson, the readings, and a coding task with acceptance tests.
4. Your code lives in `project/`. It starts as a skeleton for Module 1 and grows by about one crate per module.

## Layout

```text
CURRICULUM.md        the syllabus
plan.md              the work plan for writing the course itself
modules/NN-slug/     one lesson per module, plus the acceptance tests for its task
project/             your workspace, skeleton for Module 1
solutions/NN-slug/   a reference solution per module; each is the starting point for the next
```

## How a module works

Each lesson has the same seven sections: why the module exists, objectives, the lesson, readings with working links, the coding task, questions with answers, and a hand-off to the next module. The coding task names its starting point, numbers its requirements, and lists the commands that must pass. The hints are collapsed. You decide whether to read them.

Every task is cumulative. When you finish Module N, you have exactly the starting point for Module N+1. If you get stuck, start the next module from `solutions/NN-slug/` and come back later.

## Status

Part I (Modules 0 to 3), Part II (Modules 4 to 6) and Modules 7 and 8 are written. See the status table at the bottom of [`plan.md`](plan.md).
