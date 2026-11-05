# Module 12: Continuous simulation in CI

## Why this module exists

Every tool of Module 11 starts from a seed that someone ran. The hunt that found seed 9916 took five minutes of a laptop's attention, and nobody gives that attention twice a day for long. The simulator finds bugs in proportion to the seeds it runs, so the running has to be automatic. A batch on every push. A sweep every night, for as long as the night allows. A failure that arrives in the morning as a file: the seed, the commit, the command line, the trace, and a shrunk run, checked twice.

Three things have to be right for that file to mean anything. The seeds have to follow the commit, so that a verdict is reproducible and the pushes add up to coverage. A failure must never be retried into a pass, because a failure that goes away on a rerun is a second bug, not a flake. And the binary CI ran has to be the binary an engineer builds at home. That takes a pinned toolchain, a committed lockfile and one profile.

The fourth thing is cost. A simulator that runs ninety seeds a second is worth running every night. One that slows down by three times finds a third of the bugs in the same night. Nobody notices, and the nightly is quietly useless. This module puts a ceiling on the simulator's own cost, in units that the machine does not choose.

## Objectives

- Run the simulator without anyone watching: a batch on every push, a sweep every night under a time budget, and what a fleet adds to that.
- Report a failing seed as an artifact: a record and a trace per failure, verified twice, shrunk, and never retried into a pass.
- Say what "the same binary" takes: a pinned toolchain, a committed lockfile, `--locked`, and a profile that is the same in debug and in release.
- Show that the trace is a function of the source and the toolchain, not of the machine: two operating systems, two CPU architectures, one sweep hash.
- Keep the simulator fast: a cost ceiling in scheduler events and checker steps, with wall-clock time reported and never asserted.

## Lesson

### Two kinds of test

The suite has two kinds of test, and CI needs both. The first kind is the regression suite: the acceptance tests of eleven modules, the ten regression seeds of Module 10, and the planted bugs of Modules 8 to 11. It asks whether what was true is still true, and it has to run on every push. The second kind is exploration: seeds that nobody has run before. A seed that passed once is worth nothing the second time, because the run is the same run. New seeds find new bugs, and only new seeds do.

TigerBeetle's advice on fuzzers in CI is the same idea in two sentences. Each CI run is kept as quick as possible, and the total fuzzing time racks up over many runs. The smoke run uses one fixed seed and asks only whether the fuzzer runs at all. The random seed is drawn outside the test process and passed in on the command line, so that a crash cannot lose it. This module does both. The regression suite is `scripts/check.sh`, as in every module since Module 3. The exploration is the batch.

### The seeds follow the commit

`vopr --batch 500 --commit SHA` runs five hundred seeds, and the first of them is the number spelled by the first twelve hex digits of the commit hash. The same commit therefore always runs the same seeds, and the next commit runs others. The first property makes a verdict reproducible. An engineer who sees a red build runs the same batch at home and sees the same seed fail. The second property is coverage. Every push explores five hundred seeds that no push explored before, and a year of pushes is a hunt that nobody had to start.

TigerBeetle's orchestrator draws a random seed for every run and records it with the commit. FoundationDB's TestHarness2 does the same. Random seeds are fine on one condition: every seed is written down. The hash does the writing down here. The batch's summary and every record carry the commit too, so nothing depends on a log line surviving.

The regression seeds are the other end of the scale. Seed 36, seed 562 and the rest are run on every push, forever, under the overrides of the day they were found. They live in the tests, not in the batch, because the batch moves on.

### Flake-free by construction

The usual CI flake has a familiar shape. A test that passes on a rerun. A port that was in use, a timer that fired late on a loaded runner, two tests that shared a file. Teams answer with a retry button, and the retry button teaches everyone that red means "try again".

None of those causes exists in a simulated run. There is no port, no wall clock and no shared file. A seed that fails has exactly two explanations. The system has a bug, or the binary is not a function of its seed. Both are findings, and the second is the Module 1 check failing in production. So the batch never retries a failure to see whether it goes away. It runs the failing seed twice more, with the trace echoed, and demands agreement three times over. The signature has to match the first run on both reruns, and the two reruns have to print the same thing, line for line. When they agree, the record carries the trace hash and the batch says `failed`. When they disagree, the record says `unstable` and what the rerun said instead, and the batch fails for that reason. A failure that passes on a rerun is reported as a failure that passes on a rerun. That is the finding.

FoundationDB's TestHarness2 runs its determinism check on five per cent of runs, chosen at random, and keeps both traces for a script to compare. The batch here checks only the failures. A passing run's hash is folded into the sweep hash, and the next section compares that across machines.

### The same binary

Module 11 said that a bug report is a seed, a commit hash and a command line. The three name one run on one condition: the binary is the same. This module says what "the same" takes.

The source is the commit, and the record carries its hash. The toolchain is pinned in `rust-toolchain.toml` to one version, and `rustup toolchain install` in the workflow reads that file and installs that version. The dependencies are pinned in `Cargo.lock`, which is committed, and every cargo command in `scripts/check.sh` and the workflows now carries `--locked`. With that flag, cargo builds from the lockfile or fails. It never resolves a dependency anew, so the build that CI ran is the build in the commit. The features are fixed by the crate graph: the `vopr` crate asks for `buggify` and `bugs`, and cargo unifies them. And the profile is one profile. Since Module 3 the workspace keeps overflow checks and debug assertions on in both the dev and the release profile. That is what lets a seed cross between them. The tests run in debug and the batch runs in release, and `scripts/check.sh` now proves on seed 1 that both print one trace hash.

What does not go into "the same binary" is the machine. The batch workflow runs on an Ubuntu runner, which is x86-64, and on a macOS runner, which is arm64. A third job downloads both reports and compares the sweep hashes. One line of difference is a Module 3 hazard that the lints did not catch: a pointer width in a trace line, a float, an unordered map. None is there, and the comparison is what says so.

Note what is not claimed. The trace is the same on both machines. The bytes of the binary are not, and the course does not need them to be. Reproducible builds, bit for bit, are a stronger property with their own discipline. Zig promises it for release builds, and TigerBeetle's release validation compares the SHA-256 of a downloaded release with one built locally. Rust makes no such promise, and the trace is the contract here.

### The batch

`vopr::batch` is Module 11's hunt grown up. The hunt ran one child at a time and stopped at nothing. The batch runs `jobs` children at a time and verifies every failure. It shrinks the failure on request, checks the cost of every pass, and writes a report directory at the end. Its exit status is the verdict: non-zero for a failure, for a run that disagreed with itself, and for a run over the cost ceiling.

The runs happen through a `Runner`, as the shrinker's happen through an `Oracle`. `Children` spawns `vopr --seed N <overrides>` with its stdout and stderr piped, `jobs` at a time, and reads the results back in seed order. There are no threads in the pool, because the workspace bans them. The pool spawns up to `jobs` children, waits on the oldest, pushes its result and spawns the next. A slow seed at the head of the queue holds the others for a moment, and the numbers below say what that costs. `InProcess` calls `vopr::run` under `catch_unwind` for the tests. A failure's output in-process is its message, because the world is gone by the time the panic is caught.

A pass is read out of the child's stdout. The `trace_hash=` line gives the hash, the `cost:` line gives the scheduler events and the request count, and the `check:` line gives the checker's steps. A failure is a non-zero exit, and its message is cut out of stderr as Module 11 cut it. The pass's hash goes into the sweep hash with the same fold that `vopr --seeds` uses, so `vopr --seeds 0..200` and `vopr --batch 200 --from 0` print one number.

```text
$ vopr --batch 4 --from 9915 --jobs 2 --nodes 3 --no-core --bugs forgotten_promise --report report
seed=9915 ok events/request=18 steps=351 in 0.01s
seed=9916 FAILED node # view #: told to truncate record #, committed through #
seed=9916 verified twice, trace_hash=ec57647632088049
seed=9917 ok events/request=13 steps=220 in 0.01s
seed=9918 ok events/request=14 steps=196 in 0.01s
batch: seeds 9915..9919 ran=4 passed=3 failed=1 unstable=0 over_ceiling=0
commit=none
overrides=--nodes 3 --no-core --bugs forgotten_promise
wall=0.0s seeds_per_second=93.1 runs=6 jobs=2
cost: most events per request 18 at seed 9915, ceiling 1000
cost: most check steps 351 at seed 9915, ceiling 800000
sweep_hash=f46e8fb7d5ea701a
failed: seed=9916 trace_hash=ec57647632088049 node # view #: told to truncate record #, committed through #
  cargo run --release -p vopr -- --seed 9916 --nodes 3 --no-core --bugs forgotten_promise
report: 3 files in report
```

The numbers, on eight cores. Two hundred seeds take 2.2 s in one process and 2.8 s as children one at a time. At four jobs they take 1.3 s, and at eight 1.0 s. Twenty thousand seeds take 96 s at eight jobs, about two hundred a second, against ninety in one process. The child costs a quarter of a seed's time, and four cores pay for it three times over. The pool's head-of-line wait is why eight jobs are not twice as fast as four.

### The record

The report directory has `summary.txt`, and for every failure `seed-N.txt` and `seed-N.trace`. The record is the bug report of Module 11, written down.

```text
seed=9916
commit=5b65c2d0a3f1e8b7c6d5e4f3a2b1c0d9e8f7a6b5
overrides=--nodes 3 --no-core --bugs forgotten_promise
signature=node # view #: told to truncate record #, committed through #
trace_hash=ec57647632088049
disagreement=none
reproduction=cargo run --release -p vopr -- --seed 9916 --nodes 3 --no-core --bugs forgotten_promise
shrunk=cargo run --release -p vopr -- --seed 9916 --requests 57 --nodes 3 --no-core --bugs forgotten_promise
failure:
  node 0 view 21: told to truncate record 17, committed through 17
```

`Record::parse` reads the file back, and the tests prove the round trip. The trace file is the failing run as the child printed it, six thousand lines up to the assertion. `vopr --inspect seed-9916.trace` reads it with no binary to run. The whole of Module 11's reading of seed 9916 can be done from the artifact, on a laptop, against a build that no longer exists. That is what the record is for.

### The ceiling

The simulator's speed is a budget. At ninety seeds a second per core, twenty thousand seeds are four minutes. A nightly of fifty minutes on four cores is about half a million seeds. A change that makes every run three times as expensive cuts that to a sixth, and nothing turns red. The bug the nightly misses is not attributed to the slowdown, because nobody knows about the slowdown.

Wall-clock time is the wrong assertion. Module 4 decided that the one-year test prints its speed and asserts on nothing. A loaded CI runner makes any wall-clock assertion flaky, and this module just said that flakes are forbidden. The right assertion is in the seed's units. A run's cost is the scheduler events it fired and the checker's steps. The first is how much the cluster did, and the second is how hard the oracle worked. Both are functions of the seed. A run that costs more than a ceiling allows is a regression on every machine, loaded or not. The same seed shows it to everyone.

The ceilings come from measurement. Twenty thousand seeds, in release, at eight jobs:

| Cost | Median | 99th percentile | Maximum |
|---|---|---|---|
| Events per request | 18 | 85 | 427, at seed 16471 |
| Checker steps | 216 | 5 994 | 358 481, at seed 4888 |
| Wall time per seed | 20 ms | 90 ms | 290 ms |

The default ceilings are 1 000 events per request and 800 000 checker steps, about twice the maximum each. A heartbeat that fires ten times too often, a retry storm, or a checker with a broken cache goes over on hundreds of seeds. The long tail is real, though. The maximum is twenty times the median in events, and a thousand times the median in steps. A ceiling at twice the median catches nothing but noise. Wall time is printed in the batch's summary for a human to read. The summary names the dearest seed by each measure, so a slow seed has a name.

TigerBeetle's devhub takes the other road. On every merge to `main`, a script builds in debug and in release and records the build times and the binary's size. It runs a benchmark and appends one JSON line to a file in a git repository. A page graphs the lines, and a change-point service looks for steps. The ceiling here is the cheap version of that: one number, deterministic, and no graph. The stretch goals add the file and the graph.

### The fleet

The nightly workflow is a fleet of one. Every night it builds the commit at the head of `main` and runs `vopr --batch` with a budget of fifty minutes. The seeds start at the run number times a million, so no two nights overlap and the seeds of any night follow from its number. Every failure is verified and shrunk, and the report is uploaded as an artifact that GitHub keeps for ninety days. A failing seed is a file to download in the morning.

TigerBeetle's orchestrator shows what a real fleet adds. It keeps as many fuzzers running as the machine has cores, in a loop with a budget of sixty minutes. At the end of the budget a supervisor refreshes the code and starts the loop again. A fuzzer that runs past a timeout of thirty minutes is killed and counted as a failure. The targets are the tip of `main`, the release branch, and every open pull request with a label that starts with `fuzz`. The targets are weighted, so a label gets its fuzzer's attention for a while. Every run is a record: the commit, the fuzzer, the seed, the verdict, the start and end times, and the exact command. The records are merged into a JSON file in a second git repository every five minutes. The merge keeps failing seeds ahead of passing ones, at most thirty-two commits, and four seeds per commit and fuzzer. The devhub page shows each failure as a command line to paste. The orchestrator left the main repository in August 2026, and the devhub went behind a login. The readings link the last version in the tree.

FoundationDB's fleet is Joshua, which uses FoundationDB to test FoundationDB. A test ensemble is a tarball with a `joshua_test` script at its top. Agents on many machines pick ensembles out of the coordinating cluster and run the script. Exit code zero is a pass, and anything else is a failure. An ensemble stops after a hundred failures, and `joshua tail --errors` lists them. Under it, TestHarness2 picks a test at random per invocation and draws a seed. It turns buggify on at random and prints one XML summary with the seed in it.

Both fleets have three parts that the nightly here does not: more than one machine, a store for the records that outlives the run, and a page. Self-hosted runners give the first. A JSON-lines file in a git repository, merged the way the orchestrator merges it, gives the second. The third is a stretch goal.

### The workflows

Three files under `.github/workflows/`. GitHub reads them only at the root of a repository, so a workspace that lives in a subdirectory needs `defaults: { run: { working-directory: project } }` under each job. The course repository cannot run them for you. Push the workspace to a fork of your own and watch the three of them go.

`check.yml` is the regression suite. One job, one script, and it is the script you run at home.

```yaml
name: check
on:
  push:
  pull_request:
jobs:
  check:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - run: rustup toolchain install && rustup target add thumbv7em-none-eabihf
      - uses: Swatinem/rust-cache@v2
      - run: scripts/check.sh
```

`batch.yml` is the exploration, on two machines, with the comparison. The upload runs on `if: always()`, because the report is the artifact whatever the verdict was. The `agree` job counts the distinct sweep hashes and demands one.

```yaml
name: batch
on:
  push:
jobs:
  batch:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - run: rustup toolchain install
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --release --locked -p vopr
      - run: >
          target/release/vopr --batch 500 --commit ${{ github.sha }}
          --jobs 4 --report report
      - if: always()
        uses: actions/upload-artifact@v4
        with:
          name: batch-${{ matrix.os }}
          path: report
  agree:
    needs: batch
    if: always()
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
        with:
          pattern: batch-*
      - run: |
          grep -H '^sweep_hash=' batch-*/summary.txt
          test "$(grep -h '^sweep_hash=' batch-*/summary.txt | sort -u | wc -l)" -eq 1
```

`nightly.yml` is the fleet of one. The `workflow_dispatch` trigger lets you start it by hand, which is how you test it. GitHub runs a `run` step under bash with `pipefail` on. The `tail` at the end of the pipe therefore does not hide the batch's exit status.

```yaml
name: nightly
on:
  schedule:
    - cron: '17 3 * * *'
  workflow_dispatch:
jobs:
  nightly:
    runs-on: ubuntu-latest
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v4
      - run: rustup toolchain install
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --release --locked -p vopr
      - run: >
          target/release/vopr --batch 1000000
          --from $(( ${{ github.run_number }} * 1000000 ))
          --commit ${{ github.sha }}
          --for 50m --jobs 4 --shrink --report report
          | tail -40
      - if: always()
        uses: actions/upload-artifact@v4
        with:
          name: nightly-${{ github.run_number }}
          path: report
          retention-days: 90
```

### The one wall clock

The batch's budget is wall-clock time on purpose. It bounds the machine's time, and no run reads it. The workspace bans `std::time::Instant` by lint, so the batch wraps it in `batch::Stopwatch`. The wrapper is marked with `#[expect]` and a reason, as the real environment marks its clock. A `grep` for the lint's name now lists every place the program touches a clock, and the batch is one of them. That is the Module 3 discipline, kept.

### Decisions recorded

- The seeds of a push come from the first twelve hex digits of the commit hash. The seeds of a night come from the run number times a million. Random seeds are fine too, on the condition that every one is recorded with its commit.
- A failure is run twice more with its trace, and the three runs have to agree. A disagreement is reported as `unstable` and fails the batch. Nothing is retried into a pass.
- The ceilings are 1 000 events per request and 800 000 checker steps, about twice the most any seed of twenty thousand needed. Wall-clock time is reported and never asserted.
- The one wall clock in the simulator binary is `batch::Stopwatch`, marked with `#[expect]`.
- Every cargo command in `scripts/check.sh` and the workflows carries `--locked`.
- No fingerprint changed. The report gained a `cost:` line, and the report is not in the trace.

## Readings

Required:

- TigerBeetle, [`src/scripts/cfo.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/e544c610/src/scripts/cfo.zig), the continuous fuzzing orchestrator, at the last commit before it left the tree. Read the doc comment at the top, then `Fuzzer`, the loop that keeps `concurrency` fuzzers running, and the merge rules for `data.json`. That is a fleet in one file.
- TigerBeetle, ["A Tale of Four Fuzzers"](https://tigerbeetle.com/blog/2025-11-28-tale-of-four-fuzzers/) (2025). The closing advice on seeds. A fixed seed for the smoke run, and a random seed drawn outside the process for coverage. A short CI run that adds up over many runs.
- FoundationDB, [Joshua](https://github.com/FoundationDB/fdb-joshua), the README. Ensembles, agents, exit codes, and `tail --errors`. Then [TestHarness2](https://github.com/apple/foundationdb/tree/main/contrib/TestHarness2), the README's sections on seeds and on the determinism check.
- The Cargo Book, ["Cargo.toml vs Cargo.lock"](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html), the guide page. Then `--locked`, under "Manifest Options" on [any command's page](https://doc.rust-lang.org/cargo/commands/cargo-test.html), for the flag. Why the lockfile is committed, and why CI refuses to resolve.
- The rustup book, ["Overrides"](https://rust-lang.github.io/rustup/overrides.html). What `rust-toolchain.toml` does and in what order the overrides apply.

Optional:

- GitHub Docs, ["Events that trigger workflows"](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows), the `schedule` event, and ["Storing and sharing data from a workflow"](https://docs.github.com/en/actions/tutorials/store-and-share-data) for artifacts. [Using a matrix](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/run-job-variations) for the two machines.
- [Swatinem/rust-cache](https://github.com/Swatinem/rust-cache). What it caches and what it keys the cache on, which is the lockfile.
- FoundationDB, ["Simulation and Testing"](https://apple.github.io/foundationdb/testing.html). Tens of thousands of simulations a night, and a trillion CPU-hours so far.
- TigerBeetle, ["Random Fuzzy Thoughts"](https://tigerbeetle.com/blog/2023-03-28-random-fuzzy-thoughts/) (2023). A seed is enough at a commit. Across commits, persist the generated data.
- [ClusterFuzz](https://google.github.io/clusterfuzz/), the documentation's overview. A fleet at the scale of thousands of machines, and what its records look like.
- [Reproducible Builds](https://reproducible-builds.org/docs/). The stronger property, bit for bit, and what it takes.

## Coding task

### Starting point

Start from your Module 11 workspace with `dst start 12`. If you did not finish Module 11, `dst start 12 --from-solution` starts from `solutions/11-debugging-shrinking/` instead. The command adds `batch.rs` to `crates/vopr/tests/`, replaces `scripts/check.sh`, and puts the three workflows under `.github/workflows/` at the root of the workspace. Every earlier test stays as it is, because no trace line changes.

### Requirements

#### The cost of a run

1. `vopr::Report` gains `events`, the scheduler's `fired` count at the end of the run. Its `Display` gains a last line: `cost: events=N requests=M per_request=K`. The trace does not change.

#### Seeds and costs

2. `vopr::batch::seeds_for_commit(commit, count) -> Result<Range<u64>, String>`. The start is the first twelve hex digits of the hash, read as a number. Fewer than twelve hex digits, or anything that is not one, is an error.
3. `Cost { events, requests, check_steps }` with `events_per_request`. `Ceiling { events_per_request, check_steps }`, with a `Default` of 1 000 and 800 000, and `exceeded(&Cost) -> Option<String>` that names what was exceeded and by what ceiling.

#### Runners

4. `Verdict::{Passed { hash, cost }, Failed(Failure)}`. `Observed { seed, verdict, output_hash, output, wall }`. `hash_lines(text)` hashes a text line by line as a trace does. `observe(seed, success, stdout, stderr, wall)` reads a child's verdict: the `trace_hash=`, `cost:` and `check:` lines of a pass, or the panic message of a failure.
5. The `Runner` trait, `fn run_many(&mut self, seeds, &Overrides, trace: bool) -> Vec<Observed>`, in seed order. `Children { binary, jobs, runs }` spawns `jobs` children at a time with piped output and passes `--trace` through. `of_this_binary(jobs)` uses `current_exe`. `InProcess` calls `vopr::run` under `catch_unwind`.

#### The batch

6. `Batch { seeds, overrides, commit, budget, ceiling, shrink, jobs }` and `Batch::new(seeds, overrides)`.
7. `Record { seed, commit, overrides, signature, trace_hash, disagreement, shrunk, message }`, with `stable`, `reproduction`, `to_text` and `parse`. The file is key-value lines, then `failure:` and the message indented by two spaces. The form is in the lesson.
8. `BatchReport { batch, ran, passed, failures, over_ceiling, sweep_hash, max_events, max_steps, wall, runs, traces }`, with `ok` and `unstable`, and a `Display` of the form in the lesson. The sweep hash is every pass's hash folded as `vopr --seeds` folds it.
9. `run_batch(&Batch, &mut dyn Runner, log) -> BatchReport`. Seeds go to the runner `jobs` at a time. Every failure is run twice more with the trace, and the signature and the output hash have to agree. A stable failure is shrunk on request. The budget is checked between chunks. `log` sees every step as a `Progress`: `Ran`, `Verified`, `Shrunk`, `OverCeiling` and `Stopped`.
10. `write_report(dir, &BatchReport)` writes `summary.txt`, and `seed-N.txt` and `seed-N.trace` per failure. `parse_budget` reads `50m`, `90s`, `2h` and a bare number of seconds.

#### The binary and the scripts

11. `vopr --batch COUNT [--from N] [--commit SHA] [--jobs J] [--for D] [--shrink] [--report DIR] [--max-events N] [--max-steps N] [overrides]`. The seeds start at `--from`, or where the commit's hash says. The commit is recorded either way. A batch that names neither is a usage error. The progress lines and the exit status are as the lesson shows.
12. `scripts/check.sh` carries `--locked` on every cargo command and compares seed 1's hash in debug and release. After the shrink it runs the same two hundred seeds as a batch at four jobs, and the two hashes have to be equal.
13. The three workflows of the lesson, under `.github/workflows/`.
14. The one wall clock is `batch::Stopwatch`, and clippy stays at `-D warnings`.
15. The fingerprints do not change. Seed 1 of `RunConfig::from_seed(1, 150)` is still `0xcc45_30a4_8c93_496b`.

### Acceptance criteria

```sh
scripts/check.sh                                                      # ends with "all checks passed"
cargo test -p vopr --test batch                                       # eight tests
cargo run --release -p vopr -- --batch 20000 --from 0 --jobs 8 | tail -8
                                                                      # clean, about two minutes on eight cores
git push                                                              # to a fork: three green workflows
```

The last two are run by hand. The nightly runs on `workflow_dispatch` from the Actions tab.

From the repository root, `mise run sweep` runs the same twenty thousand seeds against the final reference solution, and `mise run sweep --jobs 4` runs them four at a time. Its last line is the sweep hash, `d8cc45fc8ef93f20` on the pinned toolchain. If no trace line changed since Module 10, your workspace prints the same hash.

### Hints

<details>
<summary>A pool without threads</summary>

Keep a queue of `(seed, Child, Stopwatch)`. Spawn until the queue holds `jobs` children or the seeds run out. Then pop the front and `wait_with_output` on it, push its result, and go round again. The results come out in seed order with no sorting. A child whose stdout is piped and never read blocks once the pipe is full. Read every child to the end, and never leave one in the queue.

</details>

<details>
<summary>The child's stdout</summary>

Run `vopr --seed 1` by hand and read the lines. The hash is the last line. The cost is the `cost:` line, and the checker's steps are on the `check:` line. A small `field(line, "steps=")` helper that finds a token by its prefix is enough. With `--trace`, the configuration line comes first and the trace after it, and the whole of stdout is what you hash.

</details>

<details>
<summary>The seeds from the hash</summary>

`u64::from_str_radix(&hex[..12], 16)`. Twelve hex digits are forty-eight bits, so the start is below `2^48` and `start + count` cannot overflow a `u64`.

</details>

<details>
<summary>The two reruns</summary>

Rerun with `trace = true`, twice. Compare each rerun's signature with the first run's. Compare the second rerun's `output_hash` with the first rerun's. The first run had no trace, so its output hash is of a few lines and is not compared. A rerun that passes has the signature `pass`.

</details>

<details>
<summary>The shrink inside the batch</summary>

The shrinker takes an `Oracle`. A `Runner` is one, through an adapter that runs one seed with no trace and returns its failure. The shrink's `runs` count goes into the batch's.

</details>

<details>
<summary>The workflows</summary>

`rustup toolchain install` with no argument installs what `rust-toolchain.toml` names. The matrix's macOS runner is arm64, so the `agree` job compares two architectures, not two installs of one. `actions/download-artifact@v4` with a `pattern` puts each artifact in a directory of its own name.

</details>

### Stretch goals

- A records file. Have the nightly download the last night's artifact and merge the new records into one JSON-lines file. Keep failing seeds first and four seeds per commit, and upload the file. That is the orchestrator's `data.json`, and a page over it is the devhub.
- A canary. TigerBeetle has a fuzzer that fails on every hundredth seed, to test the pipeline and not the system. Plant `Bug::Canary`, which fails on one seed in a hundred. Run a small batch under it in `check.sh`, so the batch's own reruns are exercised on every push.
- A determinism sample. Rerun one passing seed in twenty with the trace and compare, as TestHarness2 does. Say what it finds that the sweep hash across the matrix does not.
- A label that fuzzes a pull request longer, as the orchestrator's `fuzz` label does.
- A real pool. The workspace bans threads in the core and the simulator. Say which crate a thread pool for child processes belongs in, and whether the ban is right for it.
- Change-point detection on cost. Keep the median and the maximum of every batch in the records file and flag a batch whose median moved by more than a tenth.
- Self-hosted runners. Two machines, one records file, and the merge conflicts the orchestrator retries thirty-two times.

## Check your understanding

1. Why do the seeds of a push come from the commit hash, and what is lost with seeds `0..500` on every push?
2. A seed fails in CI and passes on the rerun. What does the batch report, and why is that not a flake?
3. List what "the same binary" takes, and say which item each of `rust-toolchain.toml`, `Cargo.lock`, `--locked` and the profile provides.
4. The `agree` job finds two different sweep hashes. What kind of bug is that, and which module's lints were meant to catch it?
5. Why is the cost ceiling in scheduler events and checker steps rather than in seconds?
6. The maximum cost is twenty times the median. Why is the ceiling set from the maximum, and what does that mean for what it catches?
7. What three things does TigerBeetle's orchestrator have that the nightly here does not?
8. Where is the one wall clock in the simulator binary, and how is it marked?

<details>
<summary>Answers</summary>

1. So that the same commit runs the same seeds, and the next commit runs others. The first makes a verdict reproducible, and the second makes the pushes add up. Seeds `0..500` are the same run every time after the first, and find nothing new.
2. An `unstable` record with the rerun's answer, and a failed batch. A simulated run has no port, clock or shared file to blame. A failure that goes away is the binary failing the Module 1 check, which is a finding.
3. The source, the toolchain, the dependencies, the features and the profile. The toolchain file pins the compiler. The lockfile pins the dependencies, and `--locked` makes cargo refuse to change it. The profile keeps overflow checks and debug assertions on in dev and release, so the two are one run.
4. Nondeterminism that depends on the machine: a pointer width, a float, an unordered map, something that reached the OS. Module 3's lints and the bare-metal check.
5. Seconds depend on the machine and its load, and an assertion on them flakes. Events and steps are functions of the seed, and a regression in them shows the same on every machine.
6. The long tail is real: some seeds do twenty times the median work by the seed's own choice. A ceiling at twice the median fails on them. A ceiling at twice the maximum catches a change that moves the whole distribution, which is what a regression does.
7. More than one machine, a store for the records that outlives a run, and a page over the records.
8. `batch::Stopwatch`, marked with `#[expect(clippy::disallowed_types)]` and a reason, as the real environment marks its clock.

</details>

## What's next

The simulator now runs itself, and the course has built everything it set out to build: a replicated store under consensus, the simulator that tortures it, the tools that read a failure, and the machinery that runs the seeds every night. Module 13 steps back. It ports the Module 10 replica onto an existing Rust framework and compares the two. Then it asks what this simulator does not model. The gap between the fake disk and a real one, the bugs in the compiler and the kernel, and the regressions no trace hash sees. The retrospective there is the course's written deliverable.
