# Module 11: Debugging, shrinking, and tooling

## Why this module exists

Module 10 found fifteen bugs, and each one was read from a trace by hand. The reading took minutes per seed, because the trace carries every message and the guard names the record. That was the good case. A failure at one hundred and fifty requests under eight fault families is six thousand lines of noise around ten lines of cause. The ten lines are there. Finding them is the work, and this module builds three tools for it.

The hunt runs a range of seeds, each in its own process, and lists every failure with the line that reproduces it. The shrinker takes a failing seed and turns its configuration down one knob at a time. It stops at the smallest run that still fails the same way. The inspector reads a trace back as the cluster's state at any instant, and slices it by replica and by view. None of the three touches the protocol. All three are tested the way Module 8 tested the checker: on a bug planted on purpose.

The bug is Module 10's seed 562, planted again. A replica restarts without reading its view promise back from the superblock. The hunt finds it on two seeds in twenty thousand. The shrinker cuts the run by two thirds. The inspector shows the split the guard refused, in three queries.

## Objectives

- Say what a complete bug report for a simulator is, and why a seed alone is not one.
- Hunt: run a range of seeds in child processes, never stop at the first failure, and list each failure with its reproduction.
- Shrink: reduce a failing run to fewer requests and fewer fault families, keep the seed, and decide whether two failures are the same bug.
- Inspect: reconstruct every replica's state at any instant from the trace, and slice the trace by replica and by view.
- Use all three on the forgotten promise, and read the result as the seed 562 story told in ten lines.
- Know what SimTigerBeetle shows, what Antithesis's multiverse debugger adds, and what a text trace does not give.

## Lesson

### A bug report

A seed names a run. That was Module 9's claim, and it holds under one condition: the binary is the same. The seed is a configuration under the draws the code makes, and a change to the code changes the run. So a bug report has three parts. The seed. The commit hash. The command line, with every flag the run was given. TigerBeetle's simulator documentation says the same: the seed and the git commit hash replay the exact simulation.

A fourth part proves the three. The trace hash. Two runs of the report that agree on it are the same run, bit for bit. The shrinker below ends with that check, on the smallest run it found.

### Why the process aborts

The workspace has said `panic = "abort"` since Module 3. A failing run ends the process at the assertion, and nothing runs after it. That was the point: a half-unwound world is evidence lost, and a `Drop` somewhere can hide it. The cost arrives in this module. A tool that needs more than one verdict cannot catch the panic in the binary.

So every tool that runs more than one seed spawns children. The child is this same binary, with `--seed N` and the flags, and its verdict is its exit status. The panic hook writes the message to stderr, after a line that says `panicked at` and the file and line. `shrink::panic_message` cuts the message out, from that line to the backtrace note. The test profile keeps unwinding, so the tests use `catch_unwind` through the `InProcess` oracle, and the binary uses `ChildProcess`. The Module 3 decision stays. Both oracles answer the same question, and the shrinker does not know which one it has.

### The hunt

`vopr --seeds A..B` stops at the first failure. For a sweep that is right. The first bug is the one to fix, and the rest wait. For a planted bug it is wrong. The question is how many seeds catch it, and which, and the answer needs every seed to run. `vopr --hunt A..B` runs each seed in its own process and goes on past the failures. At the end it lists each one with its signature and its reproduction, and exits non-zero.

A child process per seed costs something. The hunt runs about seventy seeds a second in release, against ninety in one process. Twenty thousand seeds under the forgotten promise took about five minutes and found two: seed 9916 and seed 16345. That rate is the reason Module 10's lesson said a hunt that finds nothing is the usual first result. Two in twenty thousand means a hunt of a thousand sees nothing nineteen times in twenty.

### Overrides

A seed has no neighbours. Seed 563 says nothing about seed 562, because the first draw differs and every draw after it. Hypothesis solved this for property-based tests by shrinking the stream of random choices rather than the value made from them. A shorter choice sequence is a simpler test, and the generator keeps every shrunk sequence valid. This simulator draws its configuration from the seed in one stream, and a change early in it reshuffles everything after. So the shrinker does not touch the seed. It keeps every die the run rolls and changes the configuration on top.

`vopr::overrides` is that configuration space. A `Knob` is one thing to turn off: a fault family, a rate inside one, or the core. Twenty of them, in a fixed order from coarse to fine. `Overrides` adds the request count, the cluster size, the clients per node and the planted bugs. Every override is a command-line flag, and the flags print in one order, so two equal overrides print the same line. That line is the reproduction.

```text
cargo run --release -p vopr -- --seed 9916 --requests 57 --nodes 3 --no-core --bugs forgotten_promise
```

Each `--no-` flag means what the swarm's coin means by off. `--no-disk` is the default disk, and `--no-net` is the default network. A shrunk configuration is therefore one the swarm itself reaches, and a bug under it is a bug the swarm finds.

### The shrinker

The loop is greedy, and it is the heart of every shrinker from QuickCheck to Hypothesis. Try a smaller candidate. Keep it on one condition: the run still fails the same way. Otherwise undo it and try the next. Each pass searches the request count by halving. Then it tries every live knob from coarse to fine, then five nodes to three, then two clients per node to one. A pass that kept a knob makes the request search before it stale, so the loop passes again. A pass that kept no knob is the last. Then the result runs twice more, and both runs have to fail the same way. A reproduction that fails only sometimes is no reproduction. It is a Module 1 problem, and the shrinker refuses it as `Unstable`.

Zeller's delta debugging does better than one knob at a time. It splits the input into halves, then quarters, and tests each piece and its complement, and the result is 1-minimal: no single piece can be removed. The knobs here are twenty, and a pass over all of them costs twenty runs, so the halving is not worth its bookkeeping. The request count is the one dimension with a thousand values, and it gets the binary search.

The order of the knobs is for cost, not correctness. Requests go first because every later candidate is then shorter. Families go before the rates inside them because one run that turns the disk off answers five questions about its rates.

### The same way

"The same way" is a judgment, and the shrinker makes it with `Failure::signature`. The first line of the assertion, cut at the first bracket, with every run of digits replaced by one mark. "Told to truncate record 27, committed through 27" and "told to truncate record 112, committed through 112" are one bug, found at two lengths. The checker's message carries the history that showed the bug, and that history changes with every candidate, so the cut at the bracket drops it.

The first version of the signature kept the whole line. The shrink of seed 2 under the stale read then met seven failures it called different. All seven were the stale read on another key or a longer history. The shrinker undid each one and the run stayed at eighty-five requests. With the cut it reaches thirty. Too fine a signature refuses every shrink. Too coarse a signature folds a second bug into the first. Each `Other` outcome is listed in the result under `also seen`, so a second bug met on the way is not lost.

### What the shrinker found

Seed 2 under the stale read after step-down, from the default draws. Twenty-one runs. One hundred and fifty requests became thirty, two clients per node became one, and the core went. Partitions and crashes stayed, and turning either off made the run pass. That is the bug's shape in one line: a view change with the old primary alive, and one client to read from it.

Seed 9916 under the forgotten promise, from three nodes and no core. Twenty-six runs. One hundred and fifty requests became fifty-seven and nothing else was kept. Every family off made the run pass, and so did every rate. That result says something too. The failure is a rare interleaving, and turning any family off reshuffles the dice of every family after it, so the interleaving is gone. The shrinker cannot tell "this family is needed" from "this family's dice are needed". Hypothesis can, because it shrinks the choices and leaves unrelated draws in place. The stretch goal that gives each family its own stream from the seed is the way to find out which families seed 9916 needs.

### The inspector

The trace is the run. Every line has a timestamp, and the run at any instant is a prefix of the lines. So time travel is a loop: read the lines up to T and keep the newest of each fact. `Timeline::parse` turns lines into events, each with its instant, the replica it is about and the view it belongs to. `Timeline::snapshot` replays them to T and returns one `ReplicaState` per replica, the partition in force and whether the faults are stopped.

The snapshot reconstructs nothing the trace does not carry. A replica's status and view come from its `status` lines. Its log's top, durable count, commit number and `whole` flag come from the last beat it sent. Its suspicions come from its `peer up` and `peer down` lines, and its life from `crash`, `restart`, `pause` and `resume`. A replica that restarted and has not beaten yet shows an empty log, because the trace has not said otherwise. That is a feature. The inspector shows what the trace shows, and a wrong picture means a missing trace line, which is a bug of its own.

Two filters slice the trace. `--replica N` keeps the lines about a replica: the ones it emitted, the deliveries to it, its timer, and any line that names it. `--view V` keeps the lines in a view: a message that carries `view=V`, or a line from a replica whose view was `V` at that instant. `--since` and `--until` bound the window, and `--at` picks the snapshot's instant. The binary reads the trace from a file. A failing seed with `--trace` prints its trace to stdout up to the assertion, and a file is where it lands.

### Reading seed 9916

The guard said: node 0, view 21, told to truncate record 17, committed through 17. The snapshot at the end, from `--inspect` with no filter:

```text
at 9.405184712s after 6225 lines:
  n0 gathering view=21 log_view=18 top=17 durable=17 commit=17 whole=true
  n1 view_change view=21 log_view=18 top=17 durable=17 commit=17 whole=false suspects=[n2]
  n2 view_change view=21 log_view=19 top=16 durable=16 commit=16 whole=true suspects=[n1]
  net connected
```

Two logs. Seventeen records committed under normal view 18, and sixteen under normal view 19. The new primary picks the log from the highest normal view, so it picks n2's sixteen, and record 17 has to go. Record 17 is committed. The guard is right, and the question is how a view-18 log came to commit a record after view 19 started.

`--at 8.33s` shows two primaries: n0 of view 18 and n1 of view 19, with n0 isolated by a partition. That is allowed across views. `--replica 1 --since 8.5s --until 9s` shows the rest in ten lines.

```text
8.537299361s n1 crash landed=0 torn=0 lost=0
8.603373858s n1 restart
8.605224319s n1 status view=18 joining
8.659376214s n1 status view=18 backup
8.785768578s n1 in  Peer from=n0 prepare view=18 seq=17 commit=16 bytes=512
8.796461906s n1 out Send to=n0 prepare_ok boot=2 view=18 durable=17
8.810089417s n0 in  Peer from=n1 prepare_ok boot=2 view=18 durable=17
```

The primary of view 19 crashed, came back, and said view 18. It had voted for 19 and forgotten. The partition had healed, n0 was still primary of 18, and n1 joined it as a backup. Record 17 was prepared in view 18, n1 acknowledged it, and two of three disks made it committed. Meanwhile n2 held the view-19 log, whole and sixteen long. Two views each held a majority, which is seed 562 word for word. Three queries, and no line of the six thousand was read that did not matter.

### Pictures

SimTigerBeetle at sim.tigerbeetle.com runs the real replica code in the browser and draws it. Clients at the top send to the primary, the primary replicates to the backups, and each replica shows its op number and its commit number. Three modes: a perfect network, then crashes and a flaky network with view changes, then storage corruption on top. The authors built it to explain how the system works, simple enough for a child, and it does. The picture gives what the snapshot does not: the shape of a partition and the flow of messages at a glance. It does not give the number that disagrees. The snapshot above is three lines, and the bug is in the second column.

Antithesis runs whole systems under a deterministic hypervisor and calls its debugger a multiverse. From any moment in a run, an engineer can branch: rewind a few seconds, attach a debugger, change one thing and run forward again. The counterfactual is the interesting part. "Turn this fault off and does the bug still happen" is exactly the shrinker's question, asked at a moment rather than at the start. The overrides here apply from the first line, so a branch at time T is a stretch goal, and the lesson says what it needs.

### How TigerBeetle does it

`src/vopr.zig` takes the seed and four overrides. `--lite` runs a small three-replica cluster and checks only for crashes. `--packet-loss-ratio` and `--requests-max` do what they say, and `--replica-missing` keeps a replica down. Those are this module's knobs, by hand. A liveness failure logs the final cluster state, which is `describe_cluster` here, and prints the seed to reproduce with. A build option, `-vopr-log=short`, prints only state transitions, which is the `status` lines of this trace. A shrinker is not in the tree, as far as it shows. `docs/internals/vopr.md` links a video in which a maintainer reads a real failure live, and that is the best reading on the craft.

### Decisions recorded

- The panic strategy of Module 3 stands. The binary aborts, the tools spawn children, and the tests unwind. The open decision is closed.
- A failure's signature is its first line, cut at the first bracket, with every number blanked. The result lists every other signature it met.
- The shrinker keeps the seed and shrinks the configuration. Each `--no-` flag means the swarm's off, so a shrunk run is a swarm run.
- The inspector reads only the trace. A fact the trace does not carry is not shown.
- The twenty-thousand-seed hunt is run by hand. The tests run the seed it found.
- No fingerprint changed. The bug is planted only on request, and the trace lines are the same.

## Readings

Required:

- Zeller and Hildebrandt, ["Simplifying and Isolating Failure-Inducing Input"](https://www.cs.purdue.edu/homes/xyzhang/spring07/Papers/delta-debugging.pdf) (IEEE TSE, 2002). Sections 1 to 4: the Mozilla crash, the `ddmin` algorithm, and 1-minimality. Read it to see what the greedy loop here gives up.
- MacIver and Donaldson, ["Test-Case Reduction via Test-Case Generation: Insights from the Hypothesis Reducer"](https://www.doc.ic.ac.uk/~afd/papers/2020/ECOOP_Hypothesis.pdf) (ECOOP 2020). Sections 2 and 3: choice sequences, the shortlex order, and why shrinking the choices keeps every reduced test valid. The stretch goal on per-family streams is this paper applied.
- The proptest book, ["Shrinking Basics"](https://proptest-rs.github.io/proptest/proptest/tutorial/shrinking-basics.html). `simplify` and `complicate` as a binary search over a value. That is the request search here, in Rust's own terms.
- Will Wilson, ["Debugging in the Multiverse"](https://www.antithesis.com/blog/multiverse_debugging/) (Antithesis, 2024). Branching from a moment, and the counterfactual as a debugging move.

Optional:

- Eaton and Greef, ["We Put a Distributed Database In the Browser"](https://tigerbeetle.com/blog/2023-07-11-we-put-a-distributed-database-in-the-browser) (TigerBeetle, 2023). The simulator is at [sim.tigerbeetle.com](https://sim.tigerbeetle.com/). Watch the third mode until a view change.
- Adrian Colyer, ["Simplifying and Isolating Failure-Inducing Input"](https://blog.acolyer.org/2015/11/16/simplifying-and-isolating-failure-inducing-input/) (the morning paper, 2015). The Zeller paper in two pages.
- TigerBeetle: [`src/vopr.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vopr.zig), the `CLIArgs` struct, and [`docs/internals/vopr.md`](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vopr.md) for the linked video.

## Coding task

### Starting point

Start from your Module 10 workspace, or from `solutions/10-consensus/`. The acceptance tests and the script are in `modules/11-debugging-shrinking/`. `tests/vopr/shrink.rs` and `tests/vopr/inspect.rs` are new. Your `tests/vopr/vopr.rs` and every earlier test stay as they are, because no trace line changes. `scripts/check.sh` gained the shrink.

### Requirements

#### The planted bug

1. `Bug::ForgottenPromise` is the sixth planted bug, named `forgotten_promise`. With it, a restarted replica keeps the superblock's sequence number and ignores its `view`, `log_view` and `log_view_len`. It remembers only its records, as it did before seed 562.

#### Overrides

2. `vopr::overrides::Knob`, twenty variants in this order. The eight families: `Net`, `Disk`, `Partitions`, `Crashes`, `Pauses`, `Clocks`, `Buggify` and `SlowHost`. Then `Core`. Then the rates: `Loss`, `Dup`, `Reorder`, `Lost`, `Torn`, `Misdirect`, `Rot` and `ReadFault`. Then `Jumps`, `Drift` and `Offset`. Each has a `flag` of the form `--no-slow-host`, a `from_flag`, an `is_live(&RunConfig)` that says whether it changes anything, and an `apply(&mut WorldConfig)`. A family's off value is the one the swarm draws for a coin that says off.
3. `Overrides { requests, nodes, clients_per_node, off, bugs }`, with `configure(seed) -> RunConfig`, `apply` and `to_args`. `parse(&[String]) -> Result<(Overrides, Vec<String>), String>` returns the flags it did not understand. A `Display` prints the flags in one fixed order. `--nodes 3` on a five-node seed keeps the core only as a majority of the three. `overrides::reproduction(seed, &overrides)` is the command line.

#### The shrinker

4. `vopr::shrink::Failure { message }`, with `first_line` and `signature`: the first line, cut at the first `[` or `(`, every run of digits as `#`, trimmed.
5. The `Oracle` trait, `fn run(&mut self, seed, &Overrides) -> Option<Failure>`. `InProcess` calls `run_catching`, which silences the panic hook and catches the unwind. `ChildProcess { binary, runs }` spawns `binary --seed N <flags>` and reads `panic_message(stderr)` on a non-zero exit. `of_this_binary` uses `current_exe`.
6. `shrink(seed, &start, &mut dyn Oracle, log) -> Result<Shrunk, ShrinkError>`. The passes of the lesson, each attempt reported to `log` as an `Attempt { change, overrides, outcome }` with `Outcome::{Kept, Passed, Other(signature)}`. Two more runs at the end, and both have to fail the same way. `ShrinkError::Passes` for a seed that passes, `ShrinkError::Unstable` for one that changes its mind.
7. `Shrunk { seed, start, result, signature, failure, attempts, others, runs }`, `kept()`, and a `Display` of the form in the lesson: the summary line, `failure:`, each `also seen:`, `from:` and `to:`.

#### The inspector

8. `vopr::inspect::Timeline::parse(lines)`. Each `Event { at, node, view, text }`. A line's node is the emitter of a host line or the node of a `timer node=` line. For `net deliver` it is the receiver, and for `net send` the sender. Its view is the `view=` token, or the emitting replica's view at that instant.
9. `Timeline::snapshot(at) -> Snapshot { at, replicas, partition, faults_stopped, lines }` from the lines up to `at`, reading only what the lesson lists. `ReplicaState` has `id`, `up`, `paused`, `status` and `view`. It also has `log_view`, `top`, `durable`, `commit`, `whole`, `suspects` and `last_seen`. `Snapshot::primaries` lists the replicas that call themselves primary.
10. `Query { file, at, since, until, replica, view }`, `Timeline::select(&Query) -> Vec<&Event>` with the two filters and the window, and `Timeline::render(&Query)`: the selected lines and their count under a filter, the last twelve lines without one, then the snapshot at `at`, `until` or the end. `parse_time` reads `12.5s`, `350ms`, `20us`, `1234ns`, `t=1234ns` and a bare number of nanoseconds. `fmt_time` prints seconds with the zeros trimmed.

#### The binary

11. `vopr --hunt A..B [overrides]` runs every seed in a child process and prints `seed=N ok` or `seed=N FAILED <signature>`. It ends with `hunt: F of T seeds failed` and a reproduction per failure, and exits non-zero on any failure.
12. `vopr --shrink --seed N [overrides]` prints one `try` line per attempt and the `Shrunk` report. `vopr --inspect FILE [--at T] [--since T] [--until T] [--replica N] [--view V]` prints the render. Every mode takes the override flags, and `--seed`, `--seeds` and `--requests` keep their meaning, so `run_process_twice` still works.
13. `scripts/check.sh` shrinks seed 9916 under the forgotten promise after the sweep.
14. The fingerprints do not change. Seed 1 of `RunConfig::from_seed(1, 150)` is still `0xcc45_30a4_8c93_496b`.

### Acceptance criteria

```sh
scripts/check.sh                                                         # ends with "all checks passed"
cargo test -p vopr --release -- --nocapture the_forgotten_promise        # prints the shrink of seed 9916
cargo run --release -p vopr -- --shrink --seed 9916 --nodes 3 --no-core --bugs forgotten_promise
                                                                         # to: ... --requests 57 ...
cargo run --release -p vopr -- --hunt 0..20000 --nodes 3 --no-core --bugs forgotten_promise | tail -5
                                                                         # seeds 9916 and 16345, in about five minutes
```

The last one is run by hand. The test suite runs the two seeds it found.

From the repository root, `mise run hunt` runs that hunt against the final reference solution. It prints only the failures and the summary. If it catches the bug, it exits non-zero. `mise run hunt --seeds 9900..10000` hunts a shorter range, and `--bugs` names another planted bug.

### Hints

<details>
<summary>The panic hook</summary>

A silenced hook is `std::panic::set_hook(Box::new(|_| {}))`. Take the old one first with `take_hook` and put it back after the run. Without this, a shrink of thirty candidates prints thirty messages, and the one that matters is the last.

</details>

<details>
<summary>The child's message</summary>

Run the binary once by hand on a failing seed and read its stderr. The message starts on the line after `panicked at`, and a multi-line message, like the checker's, runs until the `note: run with` line. Cut there, not at the first newline.

</details>

<details>
<summary>The request search</summary>

Binary search needs a `lo` that passes and a `hi` that fails. Zero passes by definition, because a run of zero requests has nothing to get wrong, and the current count fails. The loop ends with `hi - lo == 1`, and `hi` is the answer. A candidate that fails another way counts as passing for the search, because it is not kept.

</details>

<details>
<summary>The node of a line</summary>

Host lines start with `n` and digits and a space. `timer node=1` is about node 1. `net deliver n0->n1` is about the receiver, because that is where the message has its effect. Everything else is the world's. Get this wrong and `--replica 1` misses the prepare that was delivered to it.

</details>

<details>
<summary>The view of a line</summary>

Track each replica's view while parsing, from its `status` lines. A line with a `view=` token belongs to that view. A line without one belongs to the emitting replica's view at that instant. A world line belongs to no view.

</details>

<details>
<summary>The hunt finds nothing</summary>

Two seeds in twenty thousand. A hunt of a thousand has one chance in ten. Run the twenty thousand once, in the background, and keep the seeds it prints. The acceptance test runs seed 9916 only.

</details>

### Stretch goals

- Give every fault family its own `Prng`, forked from the seed in a fixed order. Then one family off leaves the others' dice in place. Shrink seed 9916 again and count the families kept. This is the Hypothesis paper's idea, one level up.
- Give the overrides a start time, so that a family is turned off from `T` on. A branch at a moment, as Antithesis does it.
- Add a second signature rule: the file and line of the assertion, read from the `panicked at` line. Say which rule is coarser, and find a shrink where they disagree.
- Shrink the workload: fewer keys, one operation kind, no think time.
- Measure a shrunk configuration's likelihood: run it on the next thousand seeds and count the failures.
- Run the hunt on N child processes at once. Keep the output in seed order.
- Draw the snapshots. One row per replica, time left to right, status as colour, and the partitions as bars. Compare it with SimTigerBeetle's picture.

## Check your understanding

1. What are the three parts of a bug report for this simulator, and which fourth thing proves that two reports name the same run?
2. Why do the hunt and the shrinker spawn child processes, and why do the tests not need to?
3. Why does the shrinker keep the seed and change the configuration, rather than searching nearby seeds?
4. A candidate turns a lost record into a stale read. What does the shrinker do with it, and where is that recorded?
5. What is the signature rule, and what went wrong with the first version?
6. Seed 9916 shrank to fewer requests and nothing else. What does that say about the failure, and what does the shrinker not know?
7. What does the inspector read a replica's `top` and `commit` from, and what does it show for a replica that restarted a moment ago?
8. In three queries, how was seed 9916 read? Name the replica that forgot and the record that was committed twice over.

<details>
<summary>Answers</summary>

1. The seed, the commit hash and the command line with every flag. The trace hash. Two runs that agree on it are the same run.
2. The workspace aborts on a panic, so the binary cannot catch one and run another seed. A child's exit status and stderr are the verdict. The test profile unwinds, so the tests catch the panic in-process.
3. A seed has no neighbours. Every draw depends on the ones before it, so a nearby seed is a different run. The configuration has neighbours: one family off, with every die the same.
4. It undoes the candidate, because the failure is not the same. The other signature is kept in `others` and printed under `also seen`.
5. The first line, cut at the first bracket, with every number blanked. The first version kept the whole line, and the checker's history made every candidate look like another bug.
6. The failure is a rare interleaving, and turning any family off reshuffles the dice of every later family. The shrinker cannot tell a needed family from needed dice. Per-family streams tell them apart.
7. From the last beat the replica sent. A replica that restarted and has not beaten yet shows an empty log, because the trace has not said otherwise.
8. The snapshot at the end showed seventeen records under view 18 and sixteen under view 19. `--at 8.33s` showed two primaries. `--replica 1` from 8.5 s showed n1 crash as primary of 19, come back in 18, and acknowledge record 17. Node 1 forgot, and record 17 was committed in view 18 after view 19 started.

</details>

## What's next

The three tools of this module all start from a seed that someone ran. Module 12 makes the running automatic. A batch of seeds on every push, and a sweep that never stops. A failing seed arrives in the morning as a reproduction line with its commit hash, already shrunk and checked twice. The build has to be the same binary every time for that to mean anything, and the module says what pinning it takes.
