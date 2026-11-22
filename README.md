# Deterministic Simulation Testing in Rust

A self-paced course. You build a small replicated key-value store and the simulator that tortures it, in the spirit of TigerBeetle's VOPR and FoundationDB's simulation. The course has fourteen modules, one Cargo workspace, and Rust throughout.

## Start here

1. Read [`CURRICULUM.md`](CURRICULUM.md) for the syllabus and the shape of the whole course.
2. You need `rustup`. The toolchain is pinned in `project/rust-toolchain.toml`. Module 3 and every later module also need one extra target: `rustup target add thumbv7em-none-eabihf`.
3. Work through `modules/` in order. Each module directory has a `README.md` with the lesson, the readings, and a coding task with acceptance tests.
4. Your code lives in `project/`. It starts as a skeleton for Module 1 and grows to eight crates by the end.
5. `dst` runs the module loop. Build it once with `cargo install --path tools/dst`. With [mise](https://mise.jdx.dev/) installed, `mise run dst` runs it without the install. Run `dst init` once, then `dst start NN`, `dst check` and `dst next` for each module. The section below explains each command.
6. The mise tasks for the reference solutions are `mise run check:NN` for one module and `mise run check` for all thirteen in order. Module 0 has no workspace, so no task. `mise run check:project` runs the suite in your workspace without `dst`. Two tasks belong to the end of the course. `mise run sweep` runs the final solution on twenty thousand seeds. `mise run hunt` plants a known bug and makes sure that the sweep catches it. Modules 9 to 12 introduce both.

## The dst command

`dst` tracks the current module and copies each module's files into your workspace. It works from any directory inside the repository.

```text
dst                     where you are, and the next command to run
dst init                write project/.dst-state and check the toolchain
dst start NN            copy module NN's tests and scripts into project/, make NN current
dst check               run project/scripts/check.sh and print one line per step
dst next                record the pass, then start module NN+1
dst list                every module as done, current or pending
```

`dst check` prints one line per step, with the time the step took. A step prints nothing until it ends, and the slow ones take minutes, so a quiet terminal is normal. When a step fails, `dst check` shows the output of that step and nothing else. When the whole script passes, `dst next` opens the next module and names the reference solution of the one you finished.

To skip a module, use `dst start NN --from-solution`. It first copies the reference solution of module NN-1 over your workspace. It replaces files with the same path and deletes nothing. Module 1 has no module before it with a solution, so it starts from `project/` as it is. The files that each `dst start NN` copies are listed in `modules/NN-slug/manifest`. The one module that retires an older test, Module 10, names that file in its manifest, and `dst start 10` deletes it.

A module that is done stays done until you start it again. `dst start NN` on a done module is refused, because its tests and scripts replace the newer ones in your workspace. `dst start NN --force` does it anyway, and `dst start MM --force` brings the newer files back.

## Layout

```text
CURRICULUM.md        the syllabus
plan.md              the work plan for writing the course itself
mise.toml            tasks: `dst`, `check`, `check:NN`, `check:project`, `sweep` and `hunt`
modules/NN-slug/     one lesson per module, plus the acceptance tests, scripts and manifest for its task
project/             your workspace, skeleton for Module 1
solutions/NN-slug/   a reference solution per module; each is the starting point for the next
tools/dst/           the `dst` command, a small Rust crate with no dependencies
```

## How a module works

Each lesson has the same seven sections: why the module exists, objectives, the lesson, readings with working links, the coding task, questions with answers, and a hand-off to the next module. The coding task names its starting point, numbers its requirements, and lists the commands that must pass. The hints are collapsed.

Every task is cumulative. When you finish Module N, you have exactly the starting point for Module N+1. If you get stuck, `dst start NN --from-solution` starts the next module from the reference solution, and you can come back later.

## Status

Every module is written, from Module 0 to Module 13. See the status table at the bottom of [`plan.md`](plan.md) for what each one holds.
