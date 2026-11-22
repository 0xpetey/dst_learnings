# Review of the DST course as a teaching tool

- Date of the review: 10 November 2026.
- Commit reviewed: `dd25dcc`.
- Reviewed: the 14 lessons, the given tests, the check scripts, the reference solutions and the runner.
- All experiments ran in scratch copies. Nothing in the repository changed.

## Contents

1. [Summary](#1-summary)
2. [How the review was done](#2-how-the-review-was-done)
3. [What works](#3-what-works)
4. [What a learner gets from each route](#4-what-a-learner-gets-from-each-route)
5. [The eight problems](#5-the-eight-problems)
6. [Module by module](#6-module-by-module)
7. [The runner](#7-the-runner)
8. [The curriculum as a whole](#8-the-curriculum-as-a-whole)
9. [What to change, in order](#9-what-to-change-in-order)
10. [Measurements](#10-measurements)
11. [Appendix A: index of findings](#appendix-a-index-of-findings)

## 1. Summary

The course explains deterministic simulation testing (DST) well. It trains the learner less well than it explains.

The lessons are concrete and honest about cost. They are accurate with about ten exceptions, and three of those teach a false idea (Problem 8). The runner's state handling works on the straight path. Every check fails before the work and passes with the reference.

The checks are the weak part. Three things limit what a learner takes away:

1. In 12 of the 13 modules with a check, a wrong solution passed. Often the hole is in the place where the module's DST idea lives.
2. The checks fail correct code that does not copy the reference's random draws.
3. The learner seldom finds a bug. The lesson tells the bug, and the task asks for the fix.

The effect depends on how much the learner writes. The review infers three cases, because no person took the course. A reader starts each module from the reference and mostly reads and runs it. A reader is likely to finish with correct knowledge, and never diagnoses an unknown failing seed. A learner who writes each module from the previous reference still meets that module's own check. Modules 7, 10 and 13 can fail correct code on this route too. A learner on the own-code route learns the most. For that learner a false failure is likely at Modules 7 and 10, and almost certain at Module 11.

The first pass produced 104 findings. About 70 are distinct defects. The second run re-ran 45 of the 104 and tried to refute them. 41 held, 4 held in part, and none was refuted. The second run lowered the severity of 11 findings and raised one. 15 first-pass findings rated major were not re-run, and Appendix A marks each one.

Appendix A holds 138 entries: 5 critical, 33 major, 81 moderate and 19 minor. 33 of them repeat another entry, and the appendix marks those. The 5 critical entries are two defects.

Two defects are critical. Both are checks that need the reference's exact random draws:

- Modules 11, 12 and 13 need seed `9916` to fail ([5.0], [8.0], [WB.0]). A learner who keeps their own Module 10 is almost certain to fail them.
- Four assertions in Module 7 hold for the seed in the test and fail for many other seeds ([3.0]).

Section 5 groups the findings into eight problems. Section 6 goes through the course module by module. Section 9 lists the changes, and starts with nine small ones.

## 2. How the review was done

### Three rounds

AI agents did all the work of this review. Claude, an AI model, directed them and wrote this report. No person took part.

- First pass. Nine reviewers read the course: seven took two modules each, one took the runner, one took the curriculum as a whole. Each ran short experiments on scratch copies. They reported 104 findings.
- Walkthrough. Three agents used the runner as a learner does, and wrote no code. Two walked Modules 0 to 13. At each module they ran the check on the fresh workspace, copied the reference in, and ran the check again. One ran 18 planned edge cases against the runner, and 8 more of its own.
- Second run. Eight independent agents re-ran 45 claims from the first pass. Their task was to refute each claim. A ninth agent, the critic, looked for gaps in the review itself.

The review also took direct measurements of its own. They are a test of every reading link, clean check times for every module, and the size of each module. They also cover the failure rates of the Module 0 program and of the Module 1 hazard tests. One more is a trial of a stretch goal ([N.7]).

### Terms

- The check: a module's `scripts/check.sh`, as `dst check` runs it.
- Given tests: the test files that a module's manifest copies into the workspace.
- The reference: the author's code under `solutions/`.
- Pinned hash: a `known_*_fingerprint` test that compares a trace hash with a constant.
- Own-code route: the learner writes every module and carries that code forward.
- Reference route: the learner starts each module from the previous module's reference, with `dst start NN --from-solution`, and then writes that one module.
- Reader: a learner on the reference route who mostly reads and runs the reference and writes little. The course names no such learner. The review uses the reader as the cheapest way through the course.
- Both routes write the current module. So a defect in a module's own check affects both routes, and only a reader avoids it.
- A defect that depends on code from earlier modules affects the own-code route alone.
- Mutant: a copy of the reference with one deliberate fault. A mutant that passes the check shows a hole in the check.
- Variation: a copy of the reference with one legal change. It is still correct. A variation that fails the check shows a check that is too strict.
- Finding ids: [3.0] is finding 0 of first-pass reviewer 3. [WA.n] and [WB.n] come from the two module walkthroughs, and [TT.n] from the runner edge cases. [C.n] comes from the critic, the agent that looked for gaps in the review. [N.n] is a new finding of the second run, and [N.7] is one of the review's own.
- Appendix A lists every id that has its own heading. A walkthrough id with no heading appears inside another entry or in a See also line.

### Severity

- Critical: a typical learner is blocked, or learns something false.
- Major: the defect weakens learning a lot, or costs hours.
- Moderate: clear friction, or a gap.
- Minor: polish.

Where the second run re-ran a finding, this report uses the second run's severity.

### Limits

- No person took the course. Every hours figure is an estimate by an AI agent, and no person checked it.
- The walkthrough wrote no code. Nobody walked the reference route as the terms define it, with `dst start NN --from-solution` at each module.
- The review ran one of the 74 stretch goals ([N.7]) and examined one more ([6.4]).
- Nobody wrote the course from an empty workspace. The review tested the own-code route with more than 30 small variations of the reference.
- Everything ran on macOS on arm64 with Rust 1.96.1. Nothing ran on Linux or Windows.
- The GitHub workflows of Module 12 never ran on GitHub.
- The second run did not re-run 59 of the 104 findings. Each finding states its status.
- Many experiments ran on a busy machine. The check times in section 10.1 come from a separate clean run.

## 3. What works

Keep these. Sections 6 to 8 give the evidence for each.

- The lessons read as a short, accurate book on DST. Each states a problem, shows the mechanism, compares it with TigerBeetle and FoundationDB, and records the decision with its cost.
- The prose is plain. Lesson sentences average 11 to 15 words, and general terms are defined at first use.
- The simulator output that the lessons print reproduces on the reference. Debug and release builds print the same trace hash that the Module 11 lesson pins.
- With each module's reference copied in whole, every check passes, and the state file stays correct. A learner on the same path still meets three defects that the course made. In six modules the workspace does not load after `dst start` ([7.8]). In Module 3 the lint rejects two given files ([7.1]). About one Module 1 check in ten fails by chance ([0.3]).
- Module 8 is the strongest module by the first-pass ratings. Its checker tests draw each history, and each planted bug must be found on at least 3 of 12 seeds.
- The journal of 15 real bugs in Module 10 teaches the habit of doubt. It includes the author's own mistakes.
- The readings name sections and say why each one is read. 39 of 40 named search strings still exist upstream.
- The runner has no dependencies. Its messages name the next command, and an interrupt leaves the state file intact.
- The 74 stretch goals already hold the open-ended exercises that the main path lacks.

## 4. What a learner gets from each route

The review infers these outcomes. No person took the course, so none of them is measured.

### A reader

A reader starts each module from the reference and mostly reads and runs it. A reader is likely to finish with an accurate, ordered model of DST. The model covers the seed, the PRNG, the trace hash, a sans-I/O core and an event scheduler. It covers fault models for network, disk, clock and process, BUGGIFY and swarm testing. It covers four families of oracle, liveness after a quiet phase, shrinking, CI batches and the limits. The reader also holds a working reference of about 16,800 lines with 291 passing tests, and watched planted bugs get caught.

The reader did not diagnose an unknown failing seed, design an oracle or make an architecture decision. The syllabus lists seven outcomes (`CURRICULUM.md:27-41`). Four arrive as knowledge: determinism, the Rust hazards, fault injection with checking, and CI with the limits. The architecture and the simulator arrive in part. The sixth outcome does not arrive: this learner does not find, reproduce, shrink and fix a real bug. The critic infers 40 to 80 hours for a reader.

### A learner on the reference route

This learner writes each module on top of the previous module's reference. That takes far more hours than reading. The learner meets their own bugs in Modules 9 and 10, so the experience of find, replay and fix arrives there.

The learner also meets every weak check and every seed-specific test of the module in hand. Modules 7, 10 and 13 can fail correct code on this route ([3.0], [5.1], [6.7]). Seed `9916` does not, because Module 11 starts from the Module 10 reference. On macOS, a start with `--from-solution` can leave stale build output ([1.5]). Nobody walked this route in the review.

### A learner on the own-code route

This is the only learner whose own protocol runs under their own simulator from the first module to the last. The route builds real skill in design for simulation and in the writing of a checker. Before Module 11 the only debugging tools are `--seed N --trace` and a sweep that stops at the first failure ([8.7]).

The checks are likely to stop this learner at two places, and almost certain to stop them at a third. In Module 7, a solution that differs in every unstated draw fails about 70% of the time ([3.0]). In Module 10, the lease hunt fails for about two in three independent solutions, by the second run's rough estimate ([5.1]). In Module 11, seed `9916` is almost certain to pass where the tests need it to fail ([5.0], [8.0]). The learner must then edit the given tests, force `dst next` past a failing check, or move to the reference route and lose their code. The route that teaches the most is the route that the checks punish.

### What no route asks for

No route asks the learner to apply anything to a codebase of their own. The closing paragraph of the last lesson starts with `Nothing in this course` (`modules/13-landscape/README.md:359`). The retrospective is about the course's own simulator.

## 5. The eight problems

Each problem lists its strongest evidence. The module sections hold the full entries.

### Problem 1: Checks that need the reference's exact random draws

A seed names a run only for one binary. The course says so itself (`modules/11-debugging-shrinking/README.md:24`). Several given tests still name seeds and counts that only the reference produces. A correct solution with different draws fails them.

Evidence:

- [5.0], [8.0]: Modules 11 to 13 hard-wire seed `9916`. It sits in `modules/11-debugging-shrinking/tests/vopr/shrink.rs:179`, in `inspect.rs:89`, in 18 lines of `modules/12-ci/tests/vopr/batch.rs`, and in three check scripts.
- The second run made six correct variations of the reference, each with one change at one place. Five changed the draws. All five failed all 15 check runs of Modules 11, 12 and 13.
- The sixth variation changed the trace but not the draws. It passed all three checks. So the tests tolerate a different trace and no different draw.
- One of the five is a different unbiased `below`. The Module 1 lesson allows that by name (`modules/01-determinism/README.md:85`).
- Module 10 is always the learner's own code, because `--from-solution` supplies the Module 9 reference. A learner who keeps that code passes Module 11 only with the reference's exact draws.
- A hunt over 20,000 seeds found no replacement seed in three of the five variations. In a fourth it found one seed that fails on another assertion.
- Five more tests of Module 11 pin seed `2` under the stale-read bug. One variation failed all six tests of `shrink.rs`.
- [3.0]: four assertions in `modules/07-fault-injection/tests/kv-env/fault_injection.rs` (lines 290, 354, 418 and 177) depend on the seed. On the unmodified reference they fail for 46%, 31%, 18% and 1% of 2,000 seeds.
- Four of six legal variations of Module 7 failed at least one of the four.
- [5.1]: the Module 10 lease test (`modules/10-consensus/tests/vopr/vopr.rs:193`) hunts 24 seeds. The reference catches the planted bug on one of them. Two of five legal variations caught it on none.
- [6.7]: one Module 13 test needs three fault families among four pinned seeds. At the reference's own odds, about one port in six misses a family by chance.
- [3.6]: `modules/06-sim-disk/tests/kv-env/sim_disk.rs:345` pins the position of the world's generator. Three designs with a forked generator fail only that line and the pinned hashes.
- [2.1], [3.4], [4.5]: the requirements state the reference's hashes, and the hints tell the learner to overwrite them. A later `dst start` that re-issues the test file replaces the learner's own pin.
- [WB.0]: one unused extra `next_u64()` call leaves a 2,000-seed sweep green and fails 19 of 291 tests.
- [5.8], [8.9] (see Problem 8): the network, the disks, the crashes and the partitions draw from one shared random stream. One extra draw shifts every later draw, so each seed then names another run.

Who it affects: every learner on the own-code route, at Modules 7, 10, 11, 12 and 13. A learner on the reference route writes Module 10 and then starts Module 11 from the Module 10 reference. So that learner's own code never meets seed `9916`. The same learner still meets [3.0], [5.1] and [6.7], because both routes write Modules 7, 10 and 13.

What to change:

- Where a planted bug shows on many seeds, replace the literal seed with a hunt over a range. Keep the first seed that shows the behaviour. The Module 8 and Module 9 hunts already work this way.
- Do not rest the Module 11 tests on a hunt. In three of five variations, 20,000 seeds showed no failure.
- Test the shrinker and the batch runner with a scripted oracle. The reference holds such tests for the shrinker (`solutions/11-debugging-shrinking/crates/vopr/src/shrink.rs:444`), and they need no seed.
- Replace the four Module 7 assertions with properties that hold for every seed. The second run found such properties for lines 354 and 418. Each held on 2,000 of 2,000 seeds.
- Keep pinned hashes in a file that no manifest replaces.
- Do not pin Module 1's draws as the cure. A swap of two calls in Module 7 is enough to fail [3.0].

### Problem 2: Checks that pass wrong code

Nothing in the repository runs the given tests against a wrong solution. `dst dev verify` only compares each manifest with its solution. Where the course plants a bug, in Modules 8 to 10, the given tests catch it. Elsewhere a wrong solution often passes.

Each mutant in this table passed the full check of its module in the second run:

| Module | Mutant that passes | Finding |
|---|---|---|
| 01 | Parts of seven requirements broken: modulo `below`, no shuffle, an unspecified hash | [0.0] |
| 02 | Thirteen separate requirement violations, alone and together | [1.2] |
| 03 | A `static` atomic counter in the core | [1.4] |
| 04 | An event heap ordered by time alone | [2.0] |
| 05 | Partitions that are traced but never applied, after the learner re-pins the hash | [2.2] |
| 06 | A byte sum in place of CRC-32C | [3.5] |
| 08 | Two wrong linearizability checkers | [4.6] |
| 09 | Log truncation and erasure deleted | [4.0] |
| 09 | The liveness, log and state checks of the simulator deleted | [4.3] |
| 10 | A journal bug (seed 19) put back | [5.2] |
| 11 | A shrinker that does not test for the same failure | [5.5] |
| 12 | A batch runner that prints a constant hash | [6.3] |
| 13 | A port that applies no fault and still reports fault counts | [6.1] |

More evidence:

- [4.0]: in Module 9, truncation ran zero times in the given tests and in 5,000 seeds. The mutant's sweep hash equals the reference's.
- [5.2]: in Module 10, a mutant without the atlas, without the `whole` rule, or without the core fails one test each. That test is a pinned hash, and the hint tells the learner to overwrite it.
- [2.2]: over 20 seeds, the Module 5 mutant logged 93 partitions and cut no link. The reference dropped 1,553 messages on cut links.
- [1.4]: no probe in `hazard-check.sh` is rejected by a ban in `clippy.toml`. Twelve of the 16 probes fail because `std` is absent. Three probes can never report a failure.
- The final reference holds 63 unit tests inside its source files. The learner never gets them. Several of the mutants above fail those tests.
- For [1.2], [3.5], [5.5] and [6.3], the second run first removed those unit tests from the mutant.
- A pinned hash is both too strict and too weak. It compares the learner's trace with itself, so it cannot replace a test of behaviour.

Who it affects: learners on both routes who make one of these mistakes in a module that they write. A learner who skips the module is not affected. For Modules 4, 5, 8 and 13 the second run calls such learners a minority. The Module 1 mutant needs only common slips. A careful learner gets no proof that the code is right.

What to change:

- Hand the reference's unit tests to the learner as given tests.
- Add one assertion of effect per DST idea. Module 4: three ties after one earlier event. Module 5: a count of messages dropped on cut links. Module 6: the CRC-32C check value. Module 8: a history that only a correct checker rejects.
- Extend `dst dev verify`. Give it a directory of wrong patches that must fail, and one build with different draws that must pass. That one tool guards Problems 1 and 2.

### Problem 3: Requirements that do not state what the tests need

In Modules 2, 9, 10 and 13 the requirements do not state all that the tests need. The learner recovers part of the contract from the test files. Modules 4 to 8 name almost every public item.

Evidence:

- [1.3], [2.8], [4.2], [6.6]: the tests use names that the lesson never gives. The second run counts about 13 in Module 2. The first pass counts about 25 in Module 9 and 22 in Module 13.
- The gap varies. Modules 4 to 8 name almost every public item. Modules 2, 9 and 13 do not.
- [1.3]: the second run narrowed the Module 2 case. Most missing names appear as compile errors that name them. One rule is real coupling: request ids start at 1, and no text says so.
- [0.6]: the known answer in Module 1's requirement 3 cannot be reached through the public API. The natural test fails on the reference and passes on code that omits the newline.
- One Module 10 test counts 14 start-up messages. The lesson's own steps give 12. The difference is a duplicate message that the lesson does not describe ([5.1], under Problem 1).
- `dst start 10` replaces the Module 9 tests, so the ordering hint of Module 10 cannot be followed ([5.6], under Problem 6).
- The Module 13 lesson gives no odds, no latency range and no fault count for the port. It names no version of turmoil ([6.7], under Problem 1).
- [1.10], [2.10], [6.5]: some requirements list work that the runner already did.

Who it affects: every learner who writes the module, on both routes. The gaps are widest in Modules 2, 9 and 13.

What to change: add the missing names and rules to the requirements. Give each bare assertion a message that states the rule. Remove requirements that the runner already meets.

### Problem 4: Slow, late and silent feedback

One module is one verdict. The verdict arrives late, often names the wrong thing, and sometimes says nothing.

Evidence:

- [7.0], [2.4], [3.7]: the check runs format, lint and tests in that order and stops at the first failure. A module of 300 to 2,600 lines shows no partial progress.
- [TT.2]: on the Module 1 skeleton, the first `dst check` stops at a dead-code lint in a given file. The output is the same after the learner finishes `prng.rs`.
- [1.11], [4.7], [6.8], [7.8]: six modules add a crate (02, 03, 04, 08, 09 and 13). After `dst start`, a given test sits in a crate with no `Cargo.toml`, and every cargo command fails. The runner shows `fmt FAIL` and the usage text of cargo-fmt.
- After `dst start 10`, six of the nine given test targets do not compile. The check stays at the lint step until all of them compile ([5.6], under Problem 6).
- [N.2]: the check scripts run `cargo test --workspace` with no `--no-fail-fast` (`modules/04-virtual-time/scripts/check.sh:21`). A failing pinned hash stops every later test binary.
- [1.8], [2.3], [3.2], [5.7], [7.6]: a failing seed is not named, and no trace is shown. In Module 5 the whole output of a failing seeded test is one anonymous assertion in `node.rs`.
- The given tests of Modules 9 and 10 swap the global panic hook (`modules/09-vopr/tests/vopr/vopr.rs:109-112`). Tests run in parallel. In 15 check runs of the second run, 67 of 79 failing tests in such files printed no message ([N.4]).
- [2.9]: a test that hangs gives no output. The runner showed the step name and nothing more for 480 seconds.
- An unchanged re-check takes 2 seconds in Module 1 and 85 seconds in Module 13. Section 10.1 has the table.

Who it affects: every learner, on both routes. The seed and trace faults bite hardest on the own-code route, from Module 5 on.

What to change:

- Add `--no-fail-fast`, and run the tests before the lint.
- Add `dst test [file]` for one test file.
- Let `dst start` write a stub crate where a module adds one. The stub `lib.rs` needs one comment line, because an empty file fails the format step.
- Give the Module 1 skeleton a guard type that prints the seed and the last trace lines on a panic. Use it in every given loop over seeds.
- Do not swap the global panic hook in tests.
- Give each step of the runner a time limit.

### Problem 5: The learner replays the author's discoveries

Each lesson tells the bug. A requirement states the fix. The task asks for the code. The learner builds the machine and seldom sees it find something.

Evidence:

- [3.3], [4.4], [5.9], [6.2], [8.1]: five reviewers reported this pattern. Four saw it in their module pairs, and one saw it across the course.
- [C.2]: the `## Lesson` sections of Modules 0 to 10 and 13 hold no command to run. Commands appear only in the acceptance block of each coding task, after the lesson. The 14 lessons hold five Rust code blocks.
- [0.1]: the Module 0 exercise asks the learner to reproduce one failure. In a run under load, the most common failure line was the whole output of 355 of 1,000 runs. So the exercise suggests the opposite of its point.
- [0.2]: after Modules 0 and 1 the learner saw no replay and no divergence report.
- [1.7]: the Module 3 hunt prints its divergence reports only with `--nocapture`. Without it the learner sees six passing tests.
- [8.7]: before Module 11 the only debugging tools are `--seed N --trace` and a sweep that stops at the first failure.
- A learner who mostly reads and runs the reference watches planted bugs get caught. That learner never diagnoses an unknown seed.
- Open versions exist as stretch goals. One asks the learner to weaken the commit rule and sweep 1,000 seeds (`modules/09-vopr/README.md:312`). In a trial by the review, that change alone gave no failing seed in 10,000 ([N.7]).

Who it affects: every learner. A reader loses the most. A learner who writes Modules 9 and 10, on either route, at least meets their own bugs there.

What to change:

- Give the learner a hunt before the lesson tells the bug. The stretch goal at `modules/09-vopr/README.md:312` is meant as one. Repair it first, because the review's trial found no failing seed ([N.7]). Then make it the required first step of Module 10.
- Put the journal table and its fixes in collapsed blocks.
- Plant one bug in Module 11 whose diagnosis the lesson does not print. Ask for its shrunk command line.
- Add a short block to each lesson with one to three commands to run in the reference. The lesson already prints their output.
- Use the read barrier of Module 10 (Problem 8) as a hunt. The swarm as drawn missed it in 60,000 runs. A configuration picked by hand found it on 11 of 4,600 seeds.

### Problem 6: Size, pace and focus

The stated budget is probably too low for a learner who writes every module. Modules 2, 9 and 10 are the largest, each under one check.

Evidence:

- [8.3]: `CURRICULUM.md:68-80` budgets 18 weeks at 5 to 8 hours, so 90 to 144 hours. The module reviewers' estimates in section 6 sum to about 170 to 275 hours. The curriculum reviewer estimates 250 to 450 hours. Both are estimates for a learner who writes every module, and nobody measured them.
- [1.1]: the Module 2 reference adds 2,478 Rust lines, about 500 of them unit tests. The learner gets no skeleton and ten given tests.
- [4.1], [8.4]: the Module 9 reference adds 2,608 Rust lines. The replica is 1,692 of them (65%), and the `vopr` crate is 414 (16%).
- [5.6]: the Module 10 reference adds 1,575 Rust lines under one check, 1,129 of them in `replica.rs`. `CURRICULUM.md:21-23` says that the learner needs no consensus experience.
- [8.5]: one check covers a median of about 1,440 new Rust lines. No small exercise exists for the small ideas.
- [C.3]: the course requires 76 readings. Only three of them state a time, all in Module 0. The critic estimates 35 to 45 hours for all of them.
- 24 reading links point at a moving branch, and 17 of them are in required lists. One optional link is dead (`modules/02-architecture/README.md:183`).
- [0.8]: Module 0 puts about three hours of reading and video before a 25-line task.

Who it affects: every learner who writes these modules, on either route. The reading load affects every learner. Only a reader spends far less time.

What to change:

- State hours per module, and mark them as estimates until a learner times them.
- Split Modules 2, 9 and 10, or ship skeletons for them.
- Give each reading a time. Mark one or two per module as the core.
- Pin every source link to a commit, as Module 12 already does for one link.

### Problem 7: The runner off the straight path

The runner is mostly sound on the straight path. It misleads a learner who fails a check after a pass, skips a module, redoes one, or mistypes a flag.

Evidence:

- [7.7], [TT.1]: a failed check does not clear an earlier pass (`tools/dst/src/main.rs:443-454`). `dst next` then records a broken module as done.
- [1.5], [TT.0]: `--from-solution` copies files with `fs::copy` (`tools/dst/src/main.rs:643`). The copies keep their old file times. On macOS, after an earlier build, cargo then runs old build output. The reference then fails its own tests.
- [7.3]: `--from-solution` overwrites the learner's files with no backup, and marks the skipped module as done.
- [C.1]: the reference route has no map of the code that it hands over. A learner who starts Module 10 from the reference inherits 11,511 lines.
- [7.1]: the Module 3 lint rejects two files from Module 1 that no manifest re-issues. Every learner who does Module 3 gets 14 lint errors in files that the course wrote.
- [7.2]: a manifest holds only the files that its module adds or changes. A learner who redoes an earlier module and returns keeps stale files.
- [7.10]: the runner ignores unknown flags. `dst start 04 --from-soluton` answered `module 03 is not done`, and `dst next --dry-run` advanced the course.
- [0.3]: the Module 1 hazard tests fail by chance. A clean measurement by the review gave 15 failures in 3,000 runs of the test binary. The check runs that binary 21 times, so about one Module 1 check in ten fails by chance.

Who it affects: learners who leave the straight path, and every learner in Modules 1 and 3. [C.1] affects every learner on the reference route.

What to change: clear the pass on a failed check. Write file contents in place of `fs::copy`. Back up before `--from-solution`. Apply manifests from Module 1 up to the target. Reject unknown flags. Re-issue `hazards.rs` in Module 3. Give each module a reading order for the reference. Section 7 has the full list.

### Problem 8: False statements in the lessons, and bugs in the reference

Most of these are small. Three teach a false idea: the read barrier, the regression seeds and the CI pipe.

False statements:

| Where | What the course says | What the review found | Finding |
|---|---|---|---|
| `modules/10-consensus/README.md:94` | The read barrier costs nothing in correctness | The unmodified reference returns a stale read | [5.4] |
| `modules/10-consensus/README.md:7` | Each journal seed is a regression test | None of the ten pinned seeds caught the second run's rebuild of its own bug | [5.3], [8.2] |
| `modules/09-vopr/README.md:166` | The four seeds that found bugs are regression tests | Seed `4668` still passes with its fix removed | [4.0] |
| `modules/12-ci/README.md:191` | GitHub runs the step with `pipefail` on | The workflow names no shell, so GitHub's documented default has no `pipefail`. Nobody ran it on GitHub | [6.0] |
| `modules/06-sim-disk/README.md:58` | A checksum that catches one flipped bit catches anything larger | A byte sum catches every single flip and accepts two swapped bytes | [3.10] |
| `modules/11-debugging-shrinking/README.md:222` | A run of zero requests passes by definition | Reference seed `7` fails with `--requests 0` under a planted bug | [N.5] |
| `modules/09-vopr/README.md:312` | A majority commit rule alone yields a failing seed in 1,000 | With that one change, 10,000 seeds passed in the review's trial | [N.7] |
| `modules/00-why-dst/README.md:588` and `:881-882` | Each failing run lost a different increment | An increment can be lost at only four points | [0.1] |
| `modules/01-determinism/README.md:97` | The cross-process check catches an environment variable read at startup | Both child processes inherit one environment, and the check passed 20 of 20 times | [0.5] |
| `modules/00-why-dst/README.md:596` | The parameter set `2 200 200 50` fails 16% of runs | It failed 78% and 85% of runs on the review machine | [WA.9] |
| `modules/07-fault-injection/README.md:325` | With a `cfg` in the macro body, a test of `kv-core` alone sees no site fire | With that design the `kv-core` tests passed 10 of 10 | [3.9] |
| `modules/13-landscape/README.md:324` | A `select!` build without the flag shows two hashes that differ | Three `select!` builds gave the hashes of the `timeout` build on 40 seeds | [6.4] |

Bugs in the reference and in the given tests:

| Where | Bug | Rate | Finding |
|---|---|---|---|
| `solutions/10-consensus/crates/kv-core/src/replica.rs:2224` | A beat sent before a read can answer that read | 11 of 4,600 seeds in a favourable configuration | [5.4] |
| `solutions/07-fault-injection/crates/kv-env/src/fake.rs:1001` | `World::resume` panics after the resumed node halts in its pump | 26 of 40,000 seeds | [3.1] |
| `modules/06-sim-disk/tests/kv-env/sim_disk.rs:540-543` | The chaos oracle races with a node that still reads its disk | 12 of 3,000 seeds | [3.1] |
| `modules/06-sim-disk/tests/kv-env/sim_disk.rs:467-471` | A second oracle race stays after the guard for the first one | 5 of 37,000 more seeds | [N.3] |
| `solutions/05-sim-network/crates/kv-core/src/heartbeat.rs:302` | The heartbeat panics for a constant latency that is a multiple of the interval | Every run with 100, 200 or 300 ms | [2.6] |
| `solutions/13-landscape/crates/kv-turmoil/src/lib.rs:717` and `:863` | Two generators start from one seed, so the fault count follows the latency | 6 of the 6 seeds that ran | [N.6] |
| `solutions/09-vopr/crates/vopr/src/lib.rs:152-153` | The report prints the count of heals under `crashes=` | Every run | [5.11] |
| `solutions/02-architecture/crates/kv-env/src/fake.rs:193` | The world's trace keeps no lines in Modules 2 and 3 | Every run | [1.0] |
| `solutions/11-debugging-shrinking/crates/vopr/src/shrink.rs:298` | The shrinker never tries zero requests | Seed `7` under a planted bug | [N.5] |

The read barrier deserves a note. The primary counts any beat that arrives after the read. It cannot know that the beat left before the read. A schedule built by hand delays one beat by 250 ms across a view change. The old primary then answers with a value that a newer view already replaced. The course's own checker rejects that history. The swarm as drawn missed it in 60,000 runs. The case is a ready lesson: many clean seeds do not show that a rule is safe.

One shared random stream ([5.8], [8.9]). Module 1 teaches derived streams: each part of a simulator forks its own generator, so its draws do not shift another part's draws. The simulator does not follow that lesson. The network, every disk, the crashes and the partitions draw from one stream. Only the `buggify!` fault points get a fork. The effect shows in Module 11. The shrink of seed `9916` removes requests and keeps every fault family. With one family off, every later family draws other values. The cure is one stream per family. Today it is a stretch goal (`modules/11-debugging-shrinking/README.md:249`).

Who it affects: every reader of the lessons, on both routes. The false statements block nobody. The oracle race of [3.1] can fail correct code. By the second run's estimate, about 8% of learners meet it at the end of Module 6.

What to change: correct each statement in the table. Fix each reference bug, or turn it into a journal entry with a scenario that reaches it.

## 6. Module by module

Each module has the same layout: size, ratings, what works, and one entry per finding.

The first-pass reviewers gave five ratings from 1 (poor) to 5 (strong):

- Scaffolding: 5 means that the learner always knows the next small step, and the API is given or fully stated. 1 means that the learner must recover a large API from the tests.
- Alignment: 5 means that a test enforces every requirement, and every test traces to a requirement. 1 means many loose ends.
- Check strength: 5 means that a pass implies that the module's DST idea is really built. 1 means that a pass is easy without it.
- Feedback: 5 means that a failure tells the learner what is wrong and where to look. 1 means that it does not.
- DST focus: 5 means that almost all the work is the DST idea. 1 means that most of it is other code.

The reviewers gave the ratings before the second run, and nobody adjusted them afterwards. So a rating can be higher than the findings below it suggest. Module 07 is the clearest case.

| Module | Hours, estimate | Scaffolding | Alignment | Check strength | Feedback | DST focus | Highest severity |
|---|---|---|---|---|---|---|---|
| 00 | 4–6 | 5 | 3 | 1 | 3 | 4 | moderate |
| 01 | 3–5 | 5 | 2 | 3 | 3 | 4 | major |
| 02 | 14–22 | 2 | 2 | 2 | 2 | 3 | major |
| 03 | 4–7 | 4 | 3 | 3 | 3 | 4 | moderate |
| 04 | 6–10 | 3 | 4 | 2 | 3 | 5 | moderate |
| 05 | 11–17 | 3 | 3 | 3 | 3 | 4 | major |
| 06 | 14–22 | 3 | 3 | 4 | 2 | 4 | major |
| 07 | 12–20 | 3 | 2 | 4 | 2 | 4 | critical |
| 08 | 10–16 | 4 | 4 | 4 | 3 | 5 | moderate |
| 09 | 30–50 | 2 | 2 | 2 | 3 | 2 | major |
| 10 | 35–60 | 2 | 2 | 3 | 3 | 3 | major |
| 11 | 10–16 | 3 | 3 | 2 | 2 | 3 | critical |
| 12 | 5–8 | 4 | 3 | 3 | 4 | 2 | critical |
| 13 | 9–14 | 3 | 2 | 2 | 3 | 3 | critical |

The hours are estimates for a learner who writes the module. They sum to about 170 to 275 hours. "Highest severity" is the highest rating among the module's entries in this section.

### Module 00: Why deterministic simulation?

- Teaches: why distributed systems resist testing, and what determinism, simulation and fault injection each add.
- Size: no check and no given tests. The task is three function bodies, about 25 lines, under 8 numbered requirements.
- Reviewer's estimate: 4–6 hours, with about 3 hours of required reading and video and under 1 hour of code. This is an estimate, not a measurement.
- Ratings from 1 (poor) to 5 (strong): scaffolding 5, alignment 3, check strength 1, feedback 3, DST focus 4.
- First `dst check` after `dst start`: one line that says Module 00 has no automated check, with exit status 0.

**What works**

- The starter builds with no warnings and prints three `FAIL` lines. The lesson promises a skeleton that compiles and reports `FAIL` (`modules/00-why-dst/README.md:434-435`).
- The race shows up at once. Both bug classes appear: lost updates on key 0 and stale replies on private keys. With the lesson's reference bodies, the first pass saw 914 of 1,000 runs fail under load.
- The experiment is cheap. In the walkthrough one run took 30 to 60 ms, and 1,000 runs took 30 to 37 s under load.
- The starter gives the argument parser, the thread setup and the final comparison (`modules/00-why-dst/starter/src/main.rs:104-190`). It also gives `run.sh`. The learner's time goes to observation.
- The lesson's arithmetic holds. The quiet-cluster and VOPR figures reproduce by hand (`modules/00-why-dst/README.md:215-231`).
- The lesson states its limits and ends with an exact definition of a clean run (`modules/00-why-dst/README.md:337-340`).

**Findings**

#### [0.1] The "reproduce one failure" exercise does not show that a failure is unreproducible

- Severity: moderate. First rating: major. Second run: held.
- Problem 5 and Problem 8.
- What is wrong: the task defines the same failure as the same key and the same count lost (`modules/00-why-dst/README.md:482-484`). Most such failures recur within a few runs, so the learner's own count contradicts the lesson.
- Evidence: the second run measured at a load average of 23 to 38. The most common failure line was the whole output of 355 of 1,000 runs. Of 481 sampled failures, 370 recurred, at a median of 3 tries. A hash of the arrival order differed in all 1,000 runs. Lines 588 and 881-882 say that each failure lost a different increment. With the default parameters, an update can be lost at only four points.
- Who it affects: both routes, unless the machine is heavily loaded. Nobody is blocked.
- Fix: make `store` print a hash of the arrival order, and ask the learner to reproduce it. Say that passing runs differ too. Correct lines 588 and 881-882.

#### [0.7] Module 0 sits outside the runner, and `dst next` always advances

- Severity: moderate. Second run: none.
- Problem 7.
- What is wrong: the learner's first contact with the runner is a module where the runner does nothing. `dst next` marks Module 0 done with no evidence.
- Evidence: `tools/dst/src/main.rs:463-464` exempts Module 0 from the pass rule. In the walkthrough, `dst next` opened Module 1 with no `NOTES.md` and an untouched starter ([WA.9]). The first pass saw the same. Most criteria at `modules/00-why-dst/README.md:491-506` are mechanical: a build, a lint, a failure count and one named `FAIL` line.
- Who it affects: both routes. A learner in a hurry skips the experiment that the course's motivation rests on.
- Fix: let `dst start 00` copy the starter to a fixed, git-ignored directory. Add a check for it: build, lint, 200 runs with at least one failure, and `NOTES.md`.

#### [0.8] About three hours of reading and video come before a 25-line task

- Severity: moderate. Second run: none.
- Problem 6.
- What is wrong: the module says that the learner must feel the problem first. The task starts at line 410 of 961, after the survey and five required readings.
- Evidence: the claim is at `modules/00-why-dst/README.md:11-13`. Two required readings are videos of 45 and 50 minutes (lines 352 and 379). The starter's comments give all three bodies as pseudo-code (`modules/00-why-dst/starter/src/main.rs:67-70`, lines 85-86 and 97-100). Hint 2 gives the loop of `call` in full (`modules/00-why-dst/README.md:528-537`).
- Who it affects: both routes. Terms such as VOPR and swizzle clogging arrive before any experiment anchors them.
- Fix: put the starter and the 1,000-run experiment first, then the lesson as the explanation. Require only the Wilson talk and the Eaton post here.

#### [0.11] Small factual slips in the Module 0 lesson and task

- Severity: minor. Second run: none.
- Problem 8.
- What is wrong: several statements disagree with the module's own numbers or files. One more slip sits in Module 1.
- Evidence: sixteen cluster-years a day give about four hits in a morning, not `a dozen` (`modules/00-why-dst/README.md:240-242`). Line 270 says `double-digit percentages`, and line 391 says 8% and 9%. Line 460 forbids a `[dependencies]` table, and `modules/00-why-dst/starter/Cargo.toml:7` holds an empty one. The starter already meets requirements 6 and 7 (lines 471-475). `CURRICULUM.md:115-116` says `how rarely it fails`, and the default fails 68% of runs. `modules/01-determinism/README.md:166` blames `your barrier`, which sits in a given test.
- Who it affects: a careful reader on either route. The cost is minutes.
- Fix: write "one day" for "one morning". Align the corruption figures. Say "an empty `[dependencies]` table". Mark requirements 6 and 7 as given.

#### [WA.9] The lesson's low failure rates did not reproduce

- Severity: minor. Found by the walkthrough. Second run: none.
- Problem 8.
- What is wrong: the task asks the learner to see the rate move (`modules/00-why-dst/README.md:510-511`). The lesson's table gives two low rates for small parameter sets. On the review machine those sets failed about as often as the defaults, on a busy machine and on a quiet one.
- Evidence: at a load average of 10 to 30, the walkthrough saw the defaults fail 823, 783 and 860 of 1,000 runs. The lesson gives 683. A later run of the review at a load average of about 3 gave 789 and 897 of 1,000. There, the set `2 200 200 50` failed 234 and 254 of 300 runs, against 16% at line 596. The set `2 500 500 50` failed 249 of 300, against 42% at line 597. The set `2 2000 500 10` failed 191 of 200, close to the lesson's 97%. The machine was an 8-core Apple M1, as in the lesson.
- Who it affects: a learner whose machine behaves like the review machine, on either route. That learner cannot show a low rate with the lesson's own numbers.
- Fix: measure the table again on the current system, and say that the rates differ between machines. Add one parameter set that stays low. The walkthrough suggests a first shared increment away from `i = 0`.

See also: [8.6] nothing checks the written deliverables. [7.8] and [TT.7] the first runner command fails. [8.3] the stated hours are too low. [C.3] 76 required readings across the course.

### Module 01: Determinism fundamentals

- Teaches: a seeded generator with a pinned algorithm, a hashed trace, and a check that runs one seed twice.
- Size: the reference adds 454 Rust lines under `src/`, 164 of them unit tests in the source files. 18 given tests, 10 numbered requirements. The skeleton holds every signature, and the first pass counts about 185 lines for the learner to write.
- Reviewer's estimate: 3–5 hours to write it yourself. This is an estimate, not a measurement.
- Ratings from 1 (poor) to 5 (strong): scaffolding 5, alignment 2, check strength 3, feedback 3, DST focus 4.
- First `dst check` after `dst start`: `clippy FAIL` on `field keep_lines is never read` in the skeleton, in 28 lines of output. The output is the same after `prng.rs` is done.

**What works**

- The skeleton declares every item that the 18 given tests use. It holds eleven `todo!()` bodies, and eight of them name what to write (for example `project/crates/determinism/src/prng.rs:33`).
- An independent witness checks the generator. One test compares 10,000 outputs for six seeds with `rand_xoshiro` (`modules/01-determinism/tests/reference_prng.rs:27-35`).
- The run-twice check resists the obvious shortcuts. In the first pass, a version that shares one generator across runs failed four control tests.
- The catalog is honest about limits. One test documents that the in-process check can pass on a heap address (`modules/01-determinism/tests/hazards.rs:178-194`). Three hazards come with a paired test of the fix.
- The check is quick. On the reference on a quiet machine, the first run took 7.3 s and an unchanged re-run took 2.0 s (see section 10.1).

**Findings**

#### [0.0] The check passes code that breaks parts of requirements 2 to 8

- Severity: major. Second run: held.
- Problem 2.
- What is wrong: the check enforces the generator, the run count and a difference on line 0, and little else. The lesson's rule of a pinned algorithm holds for `next_u64` only.
- Evidence: the second run wrote a mutant with a modulo `below`, an empty `shuffle` and a non-FNV hash with no newline. The mutant compared the first trace line only, kept the skeleton's `Display` and ignored `--seed=N`. The unmodified check passed it. The check also passed a second mutant made only of plausible slips. Of 11 assertions written from requirements 2 to 8, the first mutant failed 10 and the reference failed none. Every `*_is_caught` test asserts only index 0 on a one-line trace (for example `modules/01-determinism/tests/hazards.rs:45`). The reference holds 12 unit tests for these points, and the manifest ships none (for example `solutions/01-determinism/crates/determinism/src/trace.rs:115-176`). With the first mutant's crate, the Module 2 reference suite failed only `known_seed_fingerprint`. `modules/02-architecture/README.md:205` tells the learner to replace that constant.
- Who it affects: learners on both routes who make a slip and stop once `dst check` passes. The likely slips are the skeleton's `Display` left as shipped, a forgotten newline, a `first_divergence` that stops at the shorter trace, and no `--seed=N`. A learner who skips the module is not affected.
- Fix: ship the reference's unit tests as a fourth given test file on the public API. Add a divergence on line 1, a `Display` that contains both lines, and `--seed=8`.

#### [0.2] After Modules 0 and 1 the learner saw no replay and no divergence report

- Severity: major. Second run: none.
- Problem 5.
- What is wrong: Module 1 builds the generator, the trace and the check, but only given tests use them. The learner writes no hazard test and sees no divergence report. The Module 0 race never comes under the new tools.
- Evidence: the syllabus sketches an active task: introduce each hazard and record which check catches it (`CURRICULUM.md:153-155`). The module hands over `hazards.rs` complete, with 13 tests that pass silently. The lesson shows a report and says that the whole course is about it (`modules/01-determinism/README.md:99-108`). The given tests print nothing except one `eprintln!` (`modules/01-determinism/tests/hazards.rs:190`). In the first pass, ten added lines on the reference printed that exact report. Module 0 offers the deterministic rewrite only as a stretch goal (`modules/00-why-dst/README.md:831-836`).
- Who it affects: both routes. After an estimated 7 to 11 hours, the reason to start the much larger Module 2 rests on trust.
- Fix: end Module 1 with a given seeded, single-threaded model of the Module 0 clients and store, about 60 lines. The learner finds a seed that loses an update, replays it and prints the trace. Leave two or three hazard tests for the learner to write.

#### [0.3] Hazard tests fail by chance on the reference, and the check hides it

- Severity: moderate. Second run: held.
- Problem 7 and Problem 4.
- What is wrong: given tests fail by chance on the unmodified reference. The check then discards the failing output and prints a new run, which usually passes (`modules/01-determinism/scripts/check.sh:44-46`).
- Evidence: in a clean measurement at low load, the reference's hazard test binary failed 15 of 3,000 runs (0.5%). Of those, 14 were in `wall_clock_is_caught` (`modules/01-determinism/tests/hazards.rs:39-46`) and 1 was in another test. Cause: the wall clock on the test Mac ticks in whole microseconds, so two runs can read the same time. The check runs the binary 21 times, so about one check in ten fails by chance at this rate. The numbers under load are separate. Under load that was mostly heavy, the second run saw 153 failures in 19,000 runs (0.81%). Of those, 148 were in `completion_order_is_caught`. The walkthrough met the defect twice, as [WA.3] and as [TT.9] in section 7. At a load average of 17 to 27, [WA.3] saw 4 of 4 checks fail in one copy. `mise.toml:14-16` shows that the author knows the load case. No run used Linux.
- Who it affects: both routes. A second `dst check` usually passes. Each later check runs these tests once, through Module 13.
- Fix: print the failing run's output. Compare more than two runs in the clock tests. In the completion-order test, use more runs, or heavier and less ordered work per thread. Correct lines 166 and 200 of the lesson, which tell the learner to tune a given test.

#### [0.5] The lesson contradicts itself on environment variables and misnames modulo bias

- Severity: moderate. Second run: held.
- Problem 8.
- What is wrong: four places say that the cross-process check catches an environment variable read at startup. It does not, because both children inherit one environment. The lesson also calls modulo bias a nondeterminism.
- Evidence: the wrong statement is at `modules/01-determinism/README.md:97`, `project/crates/determinism/src/bin/trace_hash.rs:6-8`, `modules/01-determinism/tests/cross_process.rs:3-5` and `solutions/01-determinism/crates/determinism/src/harness.rs:13-17`. Lines 59 and 112 of the lesson state the truth. In the second run, `run_process_twice` passed 20 of 20 times on the environment hazard. Line 85 calls the bias `a subtle nondeterminism`. Question 5 at line 216 overstates its size by about eleven orders of magnitude.
- Who it affects: every reader of the lesson, on both routes.
- Fix: rewrite the four places: the cross-process check catches inputs that differ between two spawns. At line 85, say that the range algorithm is part of what a seed means.

#### [0.6] Correct code cannot reach requirement 3's known answer through the public API

- Severity: moderate. Second run: held.
- Problem 2 and Problem 3.
- What is wrong: requirement 3 gives the FNV-1a hash of the byte `a` for a known-answer test. `Trace::record` hashes the line plus a newline, so no public call on correct code returns that value. No given test pins the hash.
- Evidence: the requirement is at `modules/01-determinism/README.md:145`. On the reference, the second run's test of `trace!(t, "a")` against that value failed with `0x089bdc07b544e7b2`. The same test passed on a mutant that omits the newline, and the check passed that mutant. Only a unit test on a private byte-hash helper can use the value. The reference tests a test-local copy of the hash (`solutions/01-determinism/crates/determinism/src/trace.rs:133-147`). The skeleton says that tests pin the answer (`project/crates/determinism/src/trace.rs:3-4`).
- Who it affects: learners on both routes who write the requested test in its obvious form, `trace!(t, "a")`. Correct code fails it, and code without the newline passes it. A learner who skips the test sees nothing. A learner who skips the module is not affected.
- Fix: state the public-API answers. An empty trace hashes to `0xcbf29ce484222325`, and one line `a` hashes to `0x089bdc07b544e7b2`. Ship both as a given test.

#### [0.4] Module 1 leaves the range method open, and later modules depend on it

- Severity: moderate. Second run: none.
- Problem 1.
- What is wrong: the lesson allows `another unbiased scheme` for `below` (`modules/01-determinism/README.md:85`). Later pinned hashes and named seeds depend on the exact draws. See Problem 1 for the full account.
- Evidence: the first pass put bitmask rejection, another unbiased method, into the Module 2 reference. `known_seed_fingerprint` then failed (`modules/02-architecture/tests/fake_world.rs:263`).
- Who it affects: learners on the own-code route whose `below` draws differ from the reference's.
- Fix: see Problem 1. Pins in Module 1 alone do not close it.

#### [0.9] The lesson's `dst start 01` is refused, then the check stops at a lint

- Severity: minor. Second run: none.
- Problem 7 and Problem 4.
- What is wrong: the runner refuses the lesson's `dst start 01` until `dst next` closes Module 0 (`modules/01-determinism/README.md:137`). The first check then stops at a lint. This entry rates the lesson's line alone. [7.8] and [7.0] in section 7 cover the runner's side and rate it higher. See Problem 7 and Problem 4 for the full account.
- Evidence: [WA.7] reproduced the refusal and its circular advice (`tools/dst/src/main.rs:371-389`).
- Who it affects: learners at the start of Module 1. Only those who did not close Module 0 with `dst next` meet the refusal.
- Fix: change line 137 to say that `dst next` opens Module 1.

#### [0.10] Derived streams are an objective, but nothing exercises `fork`

- Severity: minor. Second run: none.
- Problem 2.
- What is wrong: objective 2 and question 3 are about forked generators. The skeleton already implements `fork`, requirement 2 still lists it as work, and no given test calls it.
- Evidence: the objective is at `modules/01-determinism/README.md:20`, the requirement at line 144, and the body at `project/crates/determinism/src/prng.rs:62-64`. The reference holds a test for `fork` that the manifest does not ship (`solutions/01-determinism/crates/determinism/src/prng.rs:167-178`).
- Who it affects: both routes. The learner never sees a shared stream shift.
- Fix: add one test for the learner to complete. Two components draw from a shared generator, then from two forks. One extra draw moves the other component in the shared case only. Remove `fork` from requirement 2.

See also: [7.0], [WA.0] and [TT.2] the check stops before the tests. [7.8] and [TT.7] the first runner command fails. [8.0] seed `9916` rooted in Module 1. [8.9] the reference shares one random stream. [8.5] hazard content suits small exercises. [7.1] Module 3's check rejects `hazards.rs`. [TT.9] the flaky test under load. [TT.1] a failed check keeps an earlier pass. [TT.5] edited given tests go unnoticed. [TT.10] passing steps hide the hashes. [7.10] and [WA.11] wrong step times in a terminal.

### Module 02: Architecting for simulation

- Teaches: a core of state machines that does no I/O, and an environment that is real or fake.
- Size: the reference adds 2,478 Rust lines under `src/`, 501 of them unit tests in the source files. 10 given tests, 11 numbered requirements.
- Reviewer's estimate: 14–22 hours to write it yourself, about 3 of them reading. This is an estimate.
- Ratings from 1 (poor) to 5 (strong): scaffolding 2, alignment 2, check strength 2, feedback 2, DST focus 3.
- First `dst check` after `dst start`: `fmt FAIL` with 42 lines of output. The output names the missing `crates/kv-env/Cargo.toml`, and 26 lines are cargo-fmt usage text. No line names `kv-core`.

**What works**

- The lesson gives exact shapes and records each decision with its cost. It prints the `Input` and `Output` enums (`modules/02-architecture/README.md:58-75`) and states the cost of the hand-written loop (`modules/02-architecture/README.md:148`).
- The given tests check durability in a way that only a fake allows. One test holds a disk completion back and asserts that readers see only applied state (`modules/02-architecture/tests/fake_world.rs:49-67`).
- The check catches a duplicate-apply fault. A mutant that executes a retry of a finished request again failed three tests, in the first pass and in the second run. One is the seeded workload (`modules/02-architecture/tests/fake_world.rs:231-235`).
- The torn-tail test compares the recovered log byte for byte (`modules/02-architecture/tests/fake_world.rs:156-162`).
- The lesson announces the pinned hash. Requirement 11 tells the learner to replace the constant (`modules/02-architecture/README.md:205`), and the assertion prints the new hash (`modules/02-architecture/tests/fake_world.rs:262-265`).

**Findings**

#### [1.2] Thirteen separate requirement violations pass the full check

- Severity: major. Second run: held.
- Problem 2.
- What is wrong: no given test enforces large parts of requirements 2 to 6 and 9.
- Evidence: the second run first removed the reference's 24 unit tests, which no learner receives. Each of its 13 mutants alone, and all 13 together, ended the full check with `all checks passed`. Eight follow the first pass: requests dropped during recovery, no `validate()`, no stale-request branch, no timeout doubling, no `sync_data`, and three removed asserts. With two of those asserts gone, clippy rejects the unread field `PendingWrite.op`, so the second run removed that field too. Five are new: a malformed log entry read as a torn tail, a decode panic on an unknown tag, and a wrapping `Instant`. The other two remove `pending_writes` and `len`, and submit a pending retry twice. By the second run's count, the given tests enforce 5 of 14 behaviours of requirements 5 and 6 (`modules/02-architecture/README.md:199-200`). In the reference, the reject test passes because the codec closes the connection, and no `Response::Rejected` crosses the wire (`modules/02-architecture/tests/real_server.rs:87`).
- Who it affects: every learner who writes Module 2, on both routes. A learner who skips the module is not affected. Two things soften it: the hint at `modules/02-architecture/README.md:233` asks for hand-written node tests, and Module 4 later tests timeout doubling (`modules/04-virtual-time/tests/kv-env/virtual_time.rs:38`).
- Fix: give the reference's unit tests for these paths as files under `crates/kv-core/tests/`. Make the reject test send an empty key. State in the lesson that a killed process cannot test `sync_data`.

#### [1.5] `--from-solution` leaves stale builds on macOS, so cargo runs deleted code

- Severity: major. First rating: moderate. Second run: held.
- Problem 7.
- What is wrong: `copy_solution` copies each file with `fs::copy` (`tools/dst/src/main.rs:643`). On macOS the copy keeps the old modification time of the reference file. Cargo sees sources older than its last build and does not rebuild.
- Evidence: the second run reproduced it in two scenarios. After `dst start 02 --from-solution` on a half-done Module 1, the sources equalled the reference. `cargo test -p determinism` still failed with three `not yet implemented` panics and printed no `Compiling determinism` line. After `dst start 03 --from-solution` on a half-done Module 2, `cargo test -p kv-env` failed with 10 errors such as `unresolved import kv_core::client`. After `touch` on the sources, both runs passed. The walkthrough reported the same defect as [TT.0] in section 7 and rated it critical. Linux was not tested.
- Who it affects: macOS learners who run `--from-solution` after a build of the crates that the copy replaces. That is the stuck learner whom the root `README.md:49` sends to this flag. In a clone, the reference files date from clone time, so the condition nearly always holds. Learners who never use the flag are not affected.
- Fix: copy with `fs::read` and `fs::write`, as `apply_manifest` does (`tools/dst/src/main.rs:606-619`). Or set the modification time of each file after the copy.

#### [1.1] About 2,000 lines sit behind one check, with no skeleton and no staged tests

- Severity: major. Second run: none.
- Problem 6.
- What is wrong: no given test compiles until nearly all of two new crates exists. See Problem 6 for the full account.
- Evidence: the manifest copies two test files and `check.sh` (`modules/02-architecture/manifest:3-5`). The reference keeps 24 unit tests inside `kv-core` source files. Of the 2,478 added Rust lines, 501 are unit tests in the source files, so about 2,000 remain for the learner. The walkthrough reports it as [WA.1].
- Who it affects: every learner who writes Module 2, on both routes.
- Fix: give those 24 tests as files under `crates/kv-core/tests/`.

#### [1.0] The reference world keeps no trace lines, so the pinned hash covers one line

- Severity: moderate. First rating: major. Second run: held.
- Problem 2 and Problem 8.
- What is wrong: the reference builds its observer with a derived `Default`, which keeps no trace lines (`solutions/02-architecture/crates/kv-env/src/fake.rs:193`). Its determinism tests therefore hash one stats line.
- Evidence: a probe after 10 puts printed `lines=0`. FNV-1a of the stats line plus a newline equals the pinned `0xd889_5968_d1f3_e188`. A mutant with wall-clock lines in the world trace passed all 8 tests. Module 3's reference has the same line, so its rule at `modules/03-rust-hazards/README.md:201` is untested.
- Who it affects: learners on both routes who derive `Default` the same way in Module 2. In Module 3 it affects every learner on the reference route, because the Module 2 reference holds the line. `Trace::new()` gives a real check, and Module 4 ends the problem.
- Fix: use `Trace::new()` in both references and pin the new hash. Make the skeleton's `Default` equal `Trace::new()` (`project/crates/determinism/src/trace.rs:9`).

#### [1.3] The tests need about 13 names and one convention that the README omits

- Severity: moderate. First rating: major. Second run: held in part.
- Problem 3.
- What is wrong: the README omits about 13 API items and the rule that request ids start at 1 (`modules/02-architecture/README.md:191`). See Problem 3 for the full account.
- Evidence: with ids from 0, two tests failed with a bare assertion (`modules/02-architecture/tests/fake_world.rs:76` and line 106).
- Who it affects: every learner who writes Module 2, on both routes, for under an hour by the second run's estimate. The id rule hits only learners who number requests from 0.
- Fix: add both to the README. A fixed client id is a real fault and fails at `modules/02-architecture/tests/real_server.rs:70`. Give that assert a message about sessions.

#### [1.9] The learner never sees the determinism check fail on their own store

- Severity: moderate. Second run: none.
- Problem 5 and Problem 2.
- What is wrong: Module 2 plants no hazard and no fault. Its workload runs one operation at a time (`modules/02-architecture/tests/fake_world.rs:206-209`), so iteration order in the fake environment changes no outcome.
- Evidence: a first-pass mutant keeps `World.clients` in a `std::collections::HashMap`, so `advance` ticks the clients in random order. Its `fake_world` tests printed `8 passed` three times in a row, pinned hash included. The full check was not run on it.
- Who it affects: every learner on both routes. The claim `Determinism is structural` stays untested until a later module (`modules/02-architecture/README.md:83`).
- Fix: add a guided exercise. Switch `World.clients` to `HashMap`, add a case where two clients time out in one `advance`, and read the first divergent line. Apply the fix of [1.0] first.

#### [1.8] The workload test does not name the failing seed

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: the model assert names the client and the request, and not the seed (`modules/02-architecture/tests/fake_world.rs:234`). See Problem 4 for the full account.
- Evidence: the test loops over 20 seeds at line 252. A planted duplicate-apply fault printed `client 2 request 48` and no seed.
- Who it affects: learners on both routes whose store fails the model.
- Fix: print the seed and the last 20 trace lines on a panic.

#### [1.11] The workspace does not load after `dst start 02` or `dst start 03`

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: `dst start` copies tests into a crate directory that holds no `Cargo.toml`. No cargo command loads the workspace, and neither README says so. See Problem 4 for the full account.
- Evidence: the workspace globs `crates/*` (`project/Cargo.toml:3`). `cargo test -p determinism` failed with the same manifest error ([WA.2], [TT.3]).
- Who it affects: every learner on both routes, for minutes.
- Fix: let `dst start` write a stub `Cargo.toml` and `lib.rs`.

#### [N.1] A zero-byte `lib.rs` fails the fmt step

- Severity: minor. Found by the second run.
- Problem 4.
- What is wrong: `cargo fmt --check` on the pinned toolchain rejects an empty source file. A learner who makes the workspace load with an empty `src/lib.rs` gets a fmt failure whose diff shows two blank lines.
- Evidence: in a scratch crate on toolchain 1.96.1, a zero-byte `src/lib.rs` made `cargo fmt -- --check` exit 1. A file with one newline or one `//!` line passed. On a half-done Module 2 with an empty `crates/kv-env/src/lib.rs`, `dst check` printed `fmt FAIL`.
- Who it affects: learners on both routes who create an empty `lib.rs` in Module 2 or Module 3. A truly empty stub from the fix for [1.11] fails the same way.
- Fix: write a one-line `//!` comment into each stub `lib.rs`.

See also: [7.0] and [WA.0] no test verdict before lint. [7.8] first command of Module 2 fails. [TT.0] the walkthrough's case of [1.5]. [7.3] and [TT.6] `--from-solution` keeps no backup. [8.4] 36% is TCP and codec. [0.4] Module 1 draws change later hashes. [2.1], [WA.4] and [TT.5] `dst start` replaces the learner's pin. [7.4] no link from test to requirement.

### Module 03: How Rust and `std` fight determinism

- Teaches: where Rust and `std` let nondeterminism in, and how `no_std`, lint bans and pinned tools make those hazards fail to build.
- Size: the reference adds 322 Rust lines under `src/`, 4 of them unit tests in the source files. 6 given tests, 10 numbered requirements.
- Reviewer's estimate: 4–7 hours to write it yourself, about 2 of them reading. This is an estimate.
- Ratings from 1 (poor) to 5 (strong): scaffolding 4, alignment 3, check strength 3, feedback 3, DST focus 4.
- First `dst check` after `dst start`: `fmt FAIL` with 42 lines of output. The output names the missing `crates/hazards/Cargo.toml` and then prints cargo-fmt usage text. Part 3 of the task stays out of sight until the crate exists.

**What works**

- The hunt's API is fully stated. Requirements 1 and 2 name all 12 functions, their signature, the binary and its flag (`modules/03-rust-hazards/README.md:165-172`). The first pass counts 0 unstated Rust items.
- The hunt's failure messages name the hazard (`modules/03-rust-hazards/tests/hunt.rs:14`). With `--nocapture`, the reference printed `run 1: chose peer Some(22)` and `run 2: chose peer Some(1)`. Finding [1.7] covers the default output.
- The bare-metal build is a cheap and strict step (`modules/03-rust-hazards/scripts/check.sh:16-17`). With `extern crate std;` in front, five `std` calls passed host clippy in the second run. All five failed on `thumbv7em-none-eabihf` with `E0463`. Answer 7 states what the build does not prove (`modules/03-rust-hazards/README.md:260`).
- `#[expect]` makes the learner mark each OS touch point (`modules/03-rust-hazards/README.md:117`). The first pass ran the reference of Module 2 with the given `clippy.toml` and saw 7 errors in `crates/kv-env/src/real.rs`. The reference of Module 3 ends with 6 markers in `kv-env`.
- The lesson's ASLR claim reproduces (`modules/03-rust-hazards/README.md:55`). On macOS a raw-address sort printed one hash in 12 of 12 processes. The hashed reference printed 6 hashes in 6 processes.
- The lesson ranks four mechanisms from strongest to weakest (`modules/03-rust-hazards/README.md:101`) and states the limit of lints (`modules/03-rust-hazards/README.md:123`).

**Findings**

#### [1.4] The probe script never exercises the clippy bans, and a static counter passes

- Severity: moderate. Second run: held.
- Problem 2 and Problem 8.
- What is wrong: `hazard-check.sh` hides the compiler output and accepts any failure (`modules/03-rust-hazards/scripts/hazard-check.sh:19`). No probe needs a `clippy.toml` ban. A `static` atomic counter in the core passes every step, against `modules/03-rust-hazards/README.md:9`.
- Evidence: the second run printed each probe's first diagnostic. Twelve fail because `std` is absent, two on `forbid(unsafe_code)`, one on a pointer lint and one on a missing type. Three probes cannot report `FAIL` in any crate. With `static REQUESTS_SERVED: AtomicU32` in `kv-core`, the full check passed. The walkthrough removed `clippy.toml` from a Module 7 workspace and still saw 16 `ok` lines ([WA.10]).
- Who it affects: every learner who finishes Module 3, on both routes. None is blocked.
- Fix: print each probe's first diagnostic and assert the expected mechanism. Document the static counter as a gap, or ban `core::sync::atomic`.

#### [1.7] The hunt's reports are hidden by default, and the pointer contrast fails on macOS

- Severity: moderate. Second run: held.
- Problem 5 and Problem 8.
- What is wrong: passing tests print the hunt's reports with `eprintln!`, so `cargo test` and `dst check` hide them (`modules/03-rust-hazards/tests/hunt.rs:16`). The lesson says the in-process check can miss the pointer hazard (`modules/03-rust-hazards/README.md:246`). On the test Mac it never missed.
- Evidence: default runs printed 0 report lines, and a `--nocapture` run printed 11. 30 of 30 new processes printed `pointer_order in-process verdict: false`. The lessons of Modules 1 and 3 do not name `--nocapture`. Linux was not tested.
- Who it affects: every learner in Module 3, on both routes, sees six passing tests and no report. The pointer contradiction reaches macOS learners who read the captured output.
- Fix: add one README line with `cargo test -p hazards -- --nocapture`. For the pointer hazard, pick a value that is stable inside one process, or soften question 4.

#### [1.6] A hidden-clock helper that returns a decision or milliseconds fails the hunt

- Severity: moderate. Second run: held in part.
- Problem 3.
- What is wrong: the hunt catches a clock only once its value reaches the trace and differs within 8 runs. Requirement 1 does not say so (`modules/03-rust-hazards/README.md:171`).
- Evidence: the second run ran four variations of `hazard_hidden_clock` 20 times each. Three failed every time: a traced `is_fresh` verdict, a traced millisecond, and a verdict on a seeded entry time. A traced nanosecond passed every time with no busy-wait. The reference traces the raw timestamp and spins for 2 ms (`solutions/03-rust-hazards/crates/hazards/src/lib.rs:205`).
- Who it affects: learners on both routes whose helper returns a decision or a coarse time. The second run guesses about half, at 10 to 30 minutes each. A learner who skips the module is not affected.
- Fix: add a hint with that rule. Or let the hunt record each miss, as `CURRICULUM.md:338-339` sketches.

#### [1.10] Part 3 lists finished work as tasks and leaves three rules unchecked

- Severity: moderate. Second run: none.
- Problem 3 and Problem 2.
- What is wrong: most of requirements 6 and 8 is configuration that the learner already holds (`project/Cargo.toml:13-24`). See Problem 3 for the full account.
- Evidence: a first-pass mutant passed every step that ran. It has a crate-level `#![allow]` in `kv-env`, no `#[expect]`, no `panic = "abort"` and only the `ref_as_ptr` lint in `kv-core`. The test step was not run on it.
- Who it affects: every learner who writes Module 3, on both routes.
- Fix: reword both requirements. Add check steps for the `#[allow]` ban and `panic = "abort"`.

#### [WA.5] The probe script adds sixteen clippy runs to every check from Module 3 on

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: this entry covers the first part of [WA.5], the probe script. The second part, the one-year test, is in [WA.5] in the Module 04 section. `hazard-check.sh` runs `cargo clippy` once for each of 16 probes (`modules/03-rust-hazards/scripts/hazard-check.sh:19`). It rewrites `kv-core/src/lib.rs` before each probe, at line 18, and that forces a rebuild. The check scripts of all later modules call it. The walkthrough rated [WA.5] moderate as a whole, and this part keeps that rating.
- Evidence: on a quiet machine, an unchanged re-run took 1.5 s in Module 2 and 6.2 s in Module 3 (section 10.1). Under load, the walkthrough measured 2.8 s and 11.5 s. The probe step took 4.3 s of those 11.5 s.
- Who it affects: every learner on both routes, on each check from Module 3 to Module 13.
- Fix: run the probes on a copy of the crate. Run them only after a change to `kv-core/src/lib.rs`, `clippy.toml` or `Cargo.toml`.

See also: [7.1] lint rejects two Module 1 files. [TT.8] probe step reverts edits to `lib.rs` and forces a rebuild. [WB.3] the rebuild in the times of later modules. [8.11] `RandomState` and pointer-sort wording. [8.5] hazards fit small exercises better. [8.8] no retrofit of async code. [0.3], [WA.3] and [TT.9] flaky Module 1 tests run here. [7.0] and [WA.0] no test verdict before lint. [1.0], [1.5], [1.11] and [N.1] under Module 02 also apply here.

### Module 04: Virtual time

- Teaches: a discrete-event scheduler that owns virtual time and gives events at one instant a fixed order.
- Size: the reference adds 656 Rust lines under `src/`. 26 given tests, 11 numbered requirements.
- Reviewer's estimate: 6–10 hours to write it yourself. This is an estimate.
- Ratings from 1 (poor) to 5 (strong): scaffolding 3, alignment 4, check strength 2, feedback 3, DST focus 5.
- First `dst check` after `dst start`: `fmt FAIL` at step 1 of 5, with 42 lines of output. Two of the four given test files land in `crates/sim/tests/`, and `crates/sim` holds no `Cargo.toml` yet. No cargo command loads the workspace until the learner creates that crate. After `cargo init --lib crates/sim`, the check fails at clippy in 27 lines and names the four missing `sim` items.

**What works**

- The lesson's tie claim reproduces (`modules/04-virtual-time/README.md:49`). On a heap ordered by time alone, four ties pop as 0, 2, 1, 3. One earlier event changes the order to 1, 3, 0, 2.
- The scheduler is built once, as `modules/04-virtual-time/README.md:9` promises. `scheduler.rs`, `timer.rs` and `clock.rs` are identical in the references for Modules 04 and 05.
- Two given tests back the lesson's claims about the event model. One requires equal trace hashes from 200 ticks and from one run (`modules/04-virtual-time/tests/kv-env/virtual_time.rs:113-156`). The other asserts 525,608 timer events in one simulated year (`modules/04-virtual-time/tests/kv-env/virtual_time.rs:187`).
- Assertion messages name the mistake. Examples are `pop_until does not move the clock to the bound` (`modules/04-virtual-time/tests/sim/scheduler.rs:88`) and `the armed timeout was cancelled` (`modules/04-virtual-time/tests/kv-env/virtual_time.rs:77`).
- The requirements name almost every public item (`modules/04-virtual-time/README.md:148-176`). The first pass counted about 6 unstated items, all small, and each one shows as a compile error.
- The lesson lists its five design decisions in one place (`modules/04-virtual-time/README.md:111-117`). Each required reading names what to look for (`modules/04-virtual-time/README.md:123-129`).

**Findings**

#### [2.0] A heap ordered by time alone passes every Module 04 test

- Severity: moderate. First rating: major. Second run: held.
- Problem 2.
- What is wrong: the module's named trap is tie order. The tie test schedules three events, and a three-element `BinaryHeap` happens to pop those in schedule order (`modules/04-virtual-time/tests/sim/scheduler.rs:34-42`).
- Evidence: the second run made `Slot` compare `at` alone (`solutions/04-virtual-time/crates/sim/src/scheduler.rs:49`, `:60`). The whole check passed, 26 of 26 tests. In Module 05 the mutant failed only `known_cluster_fingerprint`. It passed `a_symmetric_partition_is_detected_and_heals` only through the shape of the heap. On the references for Modules 06, 07, 08, 09 and 13, the mutant failed that test and the pinned hashes. Modules 10 to 12 were not run.
- Who it affects: learners on both routes who write `Ord` on the instant alone. The first hint names `(at, seq)`, so this is a minority.
- Fix: add one earlier event to the tie test. The wrong heap then pops three ties as 1, 0, 2.

#### [2.7] The world's seed, the jitter and the disk rule are not checked

- Severity: moderate. Second run: held.
- Problem 2 and Problem 8.
- What is wrong: no test checks that the world draws from its seed or applies jitter. No workload leaves two appends pending, so requirement 8's `max` rule never acts. The lesson's reason for that rule is wrong (`modules/04-virtual-time/README.md:85`, `:258`).
- Evidence: a `World::latency` that returns the base failed only `known_seed_fingerprint` (`solutions/04-virtual-time/crates/kv-env/src/fake.rs:457`). A mutant that schedules each completion at `now + delay` passed the whole check (`solutions/04-virtual-time/crates/kv-env/src/fake.rs:450`). Order holds without the rule: `complete_next()` always completes the oldest append. The rule only keeps an append from completing before its own latency.
- Who it affects: learners on both routes. For seed and jitter, those whose world never draws or ignores its seed. For the disk rule, those who schedule at `now + latency`.
- Fix: add a test that two world seeds with jitter give different hashes. Add one with two appends pending that asserts completion times. Correct the lesson and answer 5.

#### [N.2] A failing pinned hash stops every later test binary

- Severity: moderate. Found by the second run.
- Problem 1 and Problem 4.
- What is wrong: the check runs `cargo test --workspace` with no `--no-fail-fast` (`modules/04-virtual-time/scripts/check.sh:21`). Cargo stops at the first failing test binary. `fake_world.rs` holds the pinned hash and runs before most given tests.
- Evidence: on a Module 04 variation, the tests ended at `known_seed_fingerprint`, and 18 given tests in three files never started. On a Module 05 variation, 37 given tests never started, `sim_network.rs` among them. With `--no-fail-fast`, the same trees passed 25 of 26 and 53 of 55 given tests. The Module 05 hint asks for a passing chaos test first (`modules/05-sim-network/README.md:283`). The check cannot show that test while the `fake_world.rs` hash fails.
- Who it affects: every learner whose pinned hash fails, in Modules 04 and 05. By [2.1] that is the own-code route, and the reference route after one changed draw. No later `check.sh` passes `--no-fail-fast` either.
- Fix: add `--no-fail-fast` in every `check.sh`, or move the pinned hashes into a last step.

#### [2.1] Requirement 10 states a hash that only the reference produces

- Severity: moderate. First rating: major. Second run: held.
- Problem 1.
- What is wrong: requirement 10 states the reference hash (`modules/04-virtual-time/README.md:184`), and a collapsed hint waives it (`modules/04-virtual-time/README.md:230`). See Problem 1 for the full account.
- Evidence: one variation per module failed only the pinned hashes. `dst start` and `dst next` replace `fake_world.rs`, constant included ([WA.4]). Module 2 already tells the learner to replace the constant (`modules/02-architecture/README.md:205`). [3.4] in the Module 07 section and [4.5] in the Module 09 section report the same defect in later modules.
- Who it affects: every learner on the own-code route. The reference route too, as soon as one draw in the learner's code differs.
- Fix: call the hash the learner's own in requirements 04.10 and 05.16.

#### [2.4] No order of work, although the given tests split by crate

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: neither lesson gives an order of work, and the check runs no test before the module compiles ([WA.0], [WA.2]). See Problem 4 for the full account.
- Evidence: the given tests split by crate: 12 and 14 in Module 04, and 19, 10 and 26 in Module 05.
- Who it affects: both routes.
- Fix: give an ordered list of `cargo test -p` commands.

#### [WA.5] The one-year test file is the slowest test file of the check

- Severity: minor. First rating: moderate. Second run: none.
- Problem 4.
- What is wrong: this entry covers the second part of [WA.5], the one-year test. The first part, the probe script, is in [WA.5] in the Module 03 section and keeps the rating of moderate. `one_simulated_year_of_retries` runs two simulated years in a debug build on every check (`modules/04-virtual-time/tests/kv-env/virtual_time.rs:179`). Its file is the slowest test file of an unchanged check in both modules. It does not take most of the check.
- Evidence: on a quiet machine, an unchanged check took 9.8 s in Module 04 and 12.1 s in Module 05 (section 10.1). `tests/virtual_time.rs` took 2.1 s and 2.6 s of those times. The lesson gives about two seconds for the test in a debug build (`modules/04-virtual-time/README.md:107`). Under load, the walkthrough measured 24.9 s and 70.5 s for the same checks. The first rating rests on those times under load.
- Who it affects: both routes, on every check from Module 04 on.
- Fix: run the one-year test in a release build, or mark it `#[ignore]` and give it its own step.

#### [2.9] A missing `advance_to` hangs one test, and the check prints nothing

- Severity: minor. First rating: moderate. Second run: held in part.
- Problem 4 and Problem 8.
- What is wrong: without `advance_to` at the end of `run_until`, the loop at `modules/04-virtual-time/tests/kv-env/virtual_time.rs:148-151` never ends. The runner sets no time limit and prints a step's output only after a failure (`tools/dst/src/main.rs:799-805`). Answer 3 says this test "fails" (`modules/04-virtual-time/README.md:256`).
- Evidence: the second run removed the call (`solutions/04-virtual-time/crates/kv-env/src/fake.rs:334`). With the given pinned hash, the check stopped at `known_seed_fingerprint` and never reached the loop. With the mutant's own hash in place, `dst check` printed the step name `tests` and nothing more for 480 s. The hang also hid a failure of `one_simulated_year_of_retries` in the same test file.
- Who it affects: few learners. Requirement 8 names the call, and the hang shows only after the learner's own hash is in place.
- Fix: bound the loop with `assert!(ticks < 1_000, ...)` and correct answer 3. A time limit per step in the runner helps every module.

#### [2.10] Requirements 10 and 11 list work the runner already did

- Severity: minor. Second run: none.
- Problem 3 and Problem 8.
- What is wrong: requirements 10 and 11 describe files that `dst start 04` delivers (`modules/04-virtual-time/README.md:184-185`, `:141`). See Problem 3 for the full account.
- Evidence: Module 05 requirement 16 does the same (`modules/05-sim-network/README.md:230`). Rule 2 omits `advance_to` (`modules/04-virtual-time/README.md:40`). The reference `net.rs` holds 550 lines, not under four hundred (`modules/05-sim-network/README.md:62`).
- Who it affects: both routes.
- Fix: move them into "Starting point" and correct both statements.

See also: [0.4] Module 1 draws change later hashes. [7.0] no partial check run. [7.4] no link from failure to requirement. [8.5] thirteen large checks, coarse grain. [TT.3] a new crate breaks the workspace. [TT.5] `dst start` overwrites edited tests. [0.3] and [WA.3] flaky Module 1 hazard tests. [2.3] and [2.5] under Module 05 apply here too.

### Module 05: The simulated network

- Teaches: a seeded network model that decides the fate of every message, seen through heartbeats on three hosts.
- Size: the reference adds 1,360 Rust lines under `src/`. 55 given tests, 16 numbered requirements.
- Reviewer's estimate: 11–17 hours to write it yourself. This is an estimate.
- Ratings from 1 (poor) to 5 (strong): scaffolding 3, alignment 3, check strength 3, feedback 3, DST focus 4.
- First `dst check` after `dst start`: `clippy FAIL` at step 2 of 5, with 31 lines of output. The errors name `kv_core::heartbeat` and `kv_core::NodeId`. Four more test targets with 94 error lines stay hidden. The module ships no `check.sh` (`modules/05-sim-network/manifest:3-7`). `dst check` runs `project/scripts/check.sh`, which is still the Module 04 script (`tools/dst/src/main.rs:437-442`). The lesson states this (`modules/05-sim-network/README.md:192`).

**What works**

- The ledger tests use real events (`modules/05-sim-network/tests/kv-env/sim_network.rs:194-227`). One moves a `Deliver` event into another world, and one schedules a copy twice. Both must panic with `never sent`.
- The tests observe faults from inside the nodes. Every node must count duplicates and stragglers and record zero `downs` (`modules/05-sim-network/tests/kv-env/sim_network.rs:245-253`). The lesson's 304 ms bound makes that hold for any correct solution (`modules/05-sim-network/README.md:110`).
- The seeded workload catches a fault in Module 2 code that the Module 04 tests miss. With the stale-request branch removed from the node, all 8 Module 04 `fake_world.rs` tests passed. The Module 05 workload failed (`solutions/05-sim-network/crates/kv-core/src/node.rs:183-188`).
- The reference's chaos run cuts real links (`modules/05-sim-network/tests/kv-env/sim_network.rs:279`). The second run counted 107 partitions over 20 seeds and 1,553 messages dropped on cut links. Every seed dropped at least 36.
- A cut during flight is tested at two levels: the world (`modules/05-sim-network/tests/kv-env/sim_network.rs:173-192`) and the model (`modules/05-sim-network/tests/sim/net.rs:141-150`).
- The network requirement states its panic substrings (`modules/05-sim-network/README.md:205-206`). One hint explains the six same-instant beats that trip the ledger test (`modules/05-sim-network/README.md:273-278`).

**Findings**

#### [2.3] A failing seed is not named and no trace is shown

- Severity: major. Second run: held.
- Problem 4.
- What is wrong: a failing seed loop prints no seed and no trace. The lesson says that the trace explains a failing seed (`modules/05-sim-network/README.md:147`). See Problem 4 for the full account.
- Evidence: with the stale-request branch removed, the loop at `modules/05-sim-network/tests/kv-env/fake_world.rs:271` printed one bare assertion from `node.rs`. Module 04 holds the same loop (`modules/04-virtual-time/tests/kv-env/fake_world.rs:253`).
- Who it affects: learners on both routes whose code fails a seed loop, first in this module. On the own-code route that is likely, because this workload is the first test of Module 2's stale-request rule.
- Fix: print the seed and the trace tail on a panic.

#### [2.2] Seeded partitions can cut no link, and the check still passes

- Severity: moderate. First rating: major. Second run: held.
- Problem 2.
- What is wrong: no test asserts that a seed-driven `Partition` event cuts a link. The chaos test checks that the event fired, that views recover and that runs repeat (`modules/05-sim-network/tests/kv-env/sim_network.rs:307-323`, `:338`).
- Evidence: the second run deleted `self.net.apply(&shape);` (`solutions/05-sim-network/crates/kv-env/src/fake.rs:618`). Of 55 given tests only `known_cluster_fingerprint` failed, and the check passed with the mutant's own hash. Over 20 seeds the mutant traced 93 partitions, and the network counter `partitioned` stayed at 0. A mutant that applies the shape and then heals gave the same hash.
- Who it affects: a minority on both routes: learners who trace the shape and never apply it, or who heal after it. The pinned hash catches this only for a trace equal to the reference's.
- Fix: in `chaos`, assert `w.net.stats().partitioned > 0` before the quiet period. On the reference this holds for all 20 seeds.

#### [2.6] The reference heartbeat panics on a beat that arrives at the tick instant

- Severity: moderate. Second run: held.
- Problem 8.
- What is wrong: `check_invariants` asserts `next_beat > last_now` after every input (`solutions/05-sim-network/crates/kv-core/src/heartbeat.rs:302`). A beat that arrives at the next-beat instant breaks it. No test or seed produces that tie, as latency draws use nanoseconds.
- Evidence: the unmodified reference panicked at constant latencies of 100, 200 and 300 ms. No requirement bounds `reliable(latency)` (`modules/05-sim-network/README.md:200`). A unit probe gave the same panic on the Module 13 heartbeat (`solutions/13-landscape/crates/kv-core/src/heartbeat.rs:340`). In 20 chaos seeds, 1,453 of 10,052 instants held several events, and none paired a node's timer with a delivery to that node.
- Who it affects: every learner who runs the reference heartbeat, which the reference route does from Module 6 on. The panic needs a constant latency that is a positive multiple of the interval. Every learner shares the gap in seed coverage.
- Fix: make the assertion hold after a message at the tick instant. Then add a test with latency equal to the interval.

#### [2.5] The learner never watches the simulator find or replay a fault

- Severity: moderate. Second run: none.
- Problem 5.
- What is wrong: both modules are machinery that the given tests exercise. No required exercise asks the learner to break code and watch a seed find the fault.
- Evidence: the lesson's trace walk uses seed `3`, three hosts and one client (`modules/05-sim-network/README.md:122-147`). No test, example or command builds that world or prints its trace. Module 04's only "run this and look" command prints the speed of the one-year test (`modules/04-virtual-time/README.md:192`). The material exists: the Module 05 workload fails on a node with no stale-request branch.
- Who it affects: both routes.
- Fix: add one required "break it" exercise per module. In Module 04, order the heap by time alone and run a given tie probe. In Module 05, remove the stale-request branch, find the failing seed and print its trace.

#### [2.8] Three heartbeat tests need panic text that requirement 9 omits

- Severity: moderate. Second run: none.
- Problem 3.
- What is wrong: requirement 9 says the heartbeat panics and gives no text (`modules/05-sim-network/README.md:213`). See Problem 3 for the full account.
- Evidence: the tests expect `unknown node`, `listed itself` and `time go backwards` (`modules/05-sim-network/tests/kv-core/heartbeat.rs:153`, `:161`, `:168`). `modules/05-sim-network/tests/sim/net.rs:214` expects sorted `cuts()`, and `:102` fails on whole-millisecond latency.
- Who it affects: any learner who writes this module from the lesson, on both routes.
- Fix: state the three substrings, the order of `cuts()` and the nanosecond resolution.

See also: [2.1], [2.4], [2.10], [WA.5] and [N.2] under Module 04 apply here too. [TT.4] no `check.sh` to restore. [8.9] one shared random stream. [1.1] and [WA.1] large module with no skeleton. [0.4] Module 1 draws change later hashes. [3.4] pinned hashes in Modules 06, 07. [7.0] no partial check run.

### Module 06: The simulated disk

- Teaches: a disk model with sector faults, crash and restart over one disk, and a log recovery that tells a torn tail from a hole.
- Size: the reference adds 1,501 Rust lines under `src/`, 66 of them unit tests in the source files. 61 given tests, 21 numbered requirements.
- Reviewer's estimate: 14–22 hours to write it yourself. This is an estimate.
- Ratings from 1 (poor) to 5 (strong): scaffolding 3, alignment 3, check strength 4, feedback 2, DST focus 4.
- First `dst check` after `dst start`: the clippy step fails with 219 lines of output. The first error lists six missing names in `kv_core::log`. In a terminal that error scrolls off screen. Module 6 ships no `check.sh`. The check runs the Module 4 script, `modules/04-virtual-time/scripts/check.sh`.

**What works**

- The oracle catches real faults. The first pass built three mutants of the reference: no session rebuild, a node that serves a damaged log, and no checksum compare. Each one failed a directed test and the 20-seed chaos run (`modules/06-sim-disk/tests/kv-env/sim_disk.rs:142`, `modules/06-sim-disk/tests/kv-core/log.rs:85`).
- The lesson states its own limits. It lists three faults that recovery accepts on a single disk (`modules/06-sim-disk/README.md:115-131`). Given tests show them, for example `a_lost_write_at_the_tail_is_invisible` (`modules/06-sim-disk/tests/kv-env/sim_disk.rs:273`).
- The first compile error gives a work order. It lists the six missing names of `kv_core::log` (`modules/06-sim-disk/tests/kv-core/log.rs:5`). The errors of `sim` and `kv-env` show only after that crate compiles.
- The readings say what to search for. One names `Recovery decision table` and `torn_prepares` in the TigerBeetle journal (`modules/06-sim-disk/README.md:212`).

**Findings**

#### [3.1] The given chaos oracle reports a loss while a node still reads its disk

- Severity: major. Second run: held.
- Problem 8.
- What is wrong: this entry covers the first of the two defects of [3.1], a race in the given oracle. The second, a panic in the reference, is in [3.1] in the Module 07 section. The second run rated both together as major, and that rating belongs to this half. `check_state` compares a node with the oracle as soon as the host is up (`modules/06-sim-disk/tests/kv-env/sim_disk.rs:540-543`). A restarted node that still reads its disk holds no data, so the oracle reports a false loss. The sibling `check_restart` returns early in that case (`:487-489`).
- Evidence: the second run swept the unmodified reference. Seeds `0..3000` gave 12 failures (0.40%). The first 2,000 gave the seven seeds of the first pass: `250`, `675`, `748`, `1125`, `1235`, `1597`, `1635`. In all 12 the failing node was up and its `recovery()` was `None`. In seed `250` the client finishes at 57807.92 ms, and the node's disk read is due at 57808.36 ms. Module 7's copy of the file failed on 14 of 3,000 seeds (0.47%).
- Who it affects: learners on both routes, because each one writes the module and its seeds then map to other scenarios. With 20 seeds, about 8% meet it in the final state of Module 6 and about 9% in Module 7. Every edit that changes the trace draws the 20 seeds again. No change to the learner's code removes it. A learner who skips the module is not affected, because the reference passes its 20 seeds.
- Fix: in `check_state`, skip a node whose `recovery()` is `None`. With that guard, seeds `0..3000` gave 0 failures. Module 7 re-issues the file, so change that copy too. No run swept the later copies in Modules 9 and 10. Close the [N.3] race below before a sweep goes into `dst dev verify`.

#### [3.2] A failing chaos run names no seed and shows no trace

- Severity: major. Second run: none.
- Problem 4.
- What is wrong: the chaos loop prints a seed only on a hash mismatch (`modules/06-sim-disk/tests/kv-env/sim_disk.rs:675-676`). Module 7 has the same loop (`modules/07-fault-injection/tests/kv-env/fault_injection.rs:584-585`). See Problem 4 for the full account.
- Evidence: the first pass saw `node 2 key [107, 48] left: None` and no seed.
- Who it affects: learners on both routes, at the first oracle failure in their own code.
- Fix: catch the panic in the loop, then print the seed and a replay command.

#### [3.3] The learner builds fault machinery and never hunts with it

- Severity: major. Second run: none.
- Problem 5.
- What is wrong: Module 6 tells the session bug as a story (`modules/06-sim-disk/README.md:149`), and requirement 6 prescribes the fix (`:239`). Module 7 argues for swarm testing with no exercise (`modules/07-fault-injection/README.md:122`). See Problem 5 for the full account.
- Evidence: the lesson analyses seed `14` of the reference's run (`modules/06-sim-disk/README.md:123`). Neither module asks for a seed sweep.
- Who it affects: both routes.
- Fix: add one hunt per module. The two defects of [3.1] are ready material.

#### [N.3] A second oracle race survives the guard that closes [3.1]

- Severity: moderate. Found by the second run.
- Problem 8.
- What is wrong: the disk loses a write that it reported complete, and the node sends `PutOk`. The node crashes, restarts and recovers before the reply arrives. `check_restart` runs before the write is in `issued`, so nothing is marked lost. The late reply makes `ack` record the write as certain (`modules/06-sim-disk/tests/kv-env/sim_disk.rs:467-471`), and `check_state` fails. The test's own rule allows this loss (`:6-7`).
- Evidence: with only the [3.1] guard added, seeds `0..3000` gave 0 failures on the Module 6 reference. Seeds `3000..40000` gave 5: `6812`, `19880`, `23151`, `29376`, `33861`.
- Who it affects: the same learners as [3.1], at a far lower rate. It also fails a sweep added with only that guard.
- Fix: record the host's incarnation at submit time. In `ack`, treat the write as uncertain after a change of incarnation, unless `recovered_records()` holds the record. Then sweep at least 40,000 seeds.

#### [3.5] A fault-free random disk and a byte-sum checksum both pass Module 6

- Severity: moderate. Second run: held.
- Problem 2.
- What is wrong: no test checks that the storage chaos run injects a disk fault. No given test calls `crc32c`.
- Evidence: the second run ran every step of the check on two mutants. Mutant A sets the five fault rates of `DiskConfig::random` to `Chance::NEVER` (`solutions/06-sim-disk/crates/sim/src/disk.rs:95-99`). Only `known_storage_fingerprint` failed, and the learner sets that hash. Mutant B uses a wrapping byte sum for CRC-32C. Every step passed, pinned hashes included. The check value `0xE306_9283` is tested only inside the reference (`solutions/06-sim-disk/crates/kv-core/src/checksum.rs:60`). The lesson says the tests print the lost-write count (`modules/06-sim-disk/README.md:131`). The count goes only into the hashed trace (`modules/06-sim-disk/tests/kv-env/sim_disk.rs:663-670`).
- Who it affects: any learner on either route whose own checksum is not CRC-32C, or whose own random disk is weaker than intended. The second run calls a fault-free random disk unlikely by accident, and a wrong polynomial or bit order the realistic case. A learner who skips the module is not affected.
- Fix: add one given test of the check value. After the 20 seeds, assert that `torn`, `lost`, `misdirected`, `corrupted`, `read_faults` and `halts` each sum above zero.

#### [3.6] One test pins the world's generator and its draw position

- Severity: moderate. Second run: held.
- Problem 1.
- What is wrong: one test predicts the first read's faults with its own `Prng::from_seed(seed)` (`modules/06-sim-disk/tests/kv-env/sim_disk.rs:326-328`). Line 345 demands the same sector list. It passes only for a world that hands that exact generator to the disk and draws nothing earlier. The lessons of Modules 5 and 6 do not state this rule.
- Evidence: the second run made three variations: a forked world generator (`solutions/06-sim-disk/crates/kv-env/src/fake.rs:419`), a forked stream for the disks, and one for the network. Each failed only line 345 and the pinned hashes. The output was two sector lists with no message, for example `left: [2, 3] right: [0, 1, 3]`. With line 345 removed, the test passed on all three. Module 1 draws the forked design (`modules/01-determinism/README.md:39-41`).
- Who it affects: learners on both routes who fork a stream for the disk, or who draw earlier from the world generator in Module 6. On the own-code route, also those who forked the network stream or the world generator in an earlier module. This is probably a minority.
- Fix: delete line 345. Lines 354 to 357 already test the property from the observed faults.

#### [3.7] About 1,500 lines per module sit behind one verdict

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: Module 6 runs the Module 4 script, which runs clippy on the whole workspace before any test (`modules/04-virtual-time/scripts/check.sh:9-10`). Module 7's own script keeps that order. See Problem 4 for the full account.
- Evidence: the runner shows no stage for the new test files `log.rs` (10 tests), `disk.rs` (13) and `sim_disk.rs` (12).
- Who it affects: every learner who writes either module.
- Fix: give both modules a `check.sh` with one step per test file.

#### [3.8] The requirement lists record the reference, not what the tests need

- Severity: moderate. Second run: none.
- Problem 3.
- What is wrong: many requirements of Module 6 have no test, and some tested rules have no requirement. The learner cannot tell which requirements matter.
- Evidence: `modules/06-sim-disk/tests` holds no use of `crc32c`, `MAX_PAYLOAD_LEN`, `is_halted`, `log_len`, `stale_disk_events` or `StorageOp`. The tests use `WriteFate` and `DiskConfig::is_reliable`, and no requirement names them. One hint explains how to write the chaos oracle (`modules/06-sim-disk/README.md:319`), and the oracle is a given test. In Module 7, one test needs jumps in more than 10 and fewer than 40 of 50 draws (`modules/07-fault-injection/tests/sim/clock.rs:145`). Requirement 10 gives no distribution (`modules/07-fault-injection/README.md:256`). The reference flips a coin (`solutions/07-fault-injection/crates/sim/src/clock.rs:154`).
- Who it affects: every learner who writes either module from the requirements. The coin rule first shows as a failure that prints a bare number.
- Fix: tag each requirement with its test. Move the rest to "Decisions recorded". State the jump coin and `WriteFate` as requirements.

#### [3.10] The lesson's rule that a one-bit test covers anything larger is false

- Severity: moderate. Second run: held.
- Problem 8.
- What is wrong: the lesson says `A checksum that catches one flipped bit catches anything larger` (`modules/06-sim-disk/README.md:58`). A one-bit flip tests coverage: no byte lies outside the checksum. It does not test strength. The reference repeats the rule (`solutions/06-sim-disk/crates/sim/src/disk.rs:34-35`).
- Evidence: the second run put a wrapping byte sum in place of CRC-32C. The sum rejected all 4,096 single-bit flips of one record and passed the given bit-flip test (`modules/06-sim-disk/tests/kv-core/log.rs:85`). It accepted two swapped payload bytes as a clean log, and two bit flips that cancel. CRC-32C rejected all of them.
- Who it affects: every learner reads the sentence. It blocks nobody. With [3.5], no test corrects it.
- Fix: say that a one-bit flip proves coverage and that the polynomial gives strength. Add one corruption that swaps two bytes in a sector, with one test.

See also: [3.4] under Module 07 applies here too. [8.1] bugs narrated before discovery. [8.9] one random stream for all components. [8.5] thirteen large checks. [7.0] and [WA.0] lint runs before tests. [7.4] no progress signal inside a module. [TT.4] no `check.sh` in the manifest. [2.1], [WA.4] and [TT.5] `dst start` replaces edited tests. [WA.5] unchanged check time grows. [1.4] and [WA.10] hazard script hides the rejecting layer.

### Module 07: Fault injection, buggify, and swarm testing

- Teaches: process pauses, skewed clocks, `buggify!` fault points, a swarm configuration drawn from the seed, and a quiet period that tests liveness.
- Size: the reference adds 1,439 Rust lines under `src/`, 53 of them unit tests in the source files. 71 given tests, 22 numbered requirements.
- Reviewer's estimate: 12–20 hours to write it yourself. This is an estimate.
- Ratings from 1 (poor) to 5 (strong): scaffolding 3, alignment 2, check strength 4, feedback 2, DST focus 4.
- First `dst check` after `dst start`: the clippy step fails with 92 lines of output. The first errors name `determinism::Chance` and `kv_core::buggify`. Only the first failing test target shows.

**What works**

- The quiet-period liveness test catches a broken `stop_faults`. The first pass left the network and the disks faulty after the call. The swarm run failed with `node 0 does not see node 1` (`modules/07-fault-injection/tests/kv-env/fault_injection.rs:559`).
- The pause test catches a real fault. A mutant that drops held beats on resume failed it (`modules/07-fault-injection/tests/kv-env/fault_injection.rs:123`).
- The lesson's heartbeat margin table (`modules/07-fault-injection/README.md:83-92`) has a test with the same numbers (`modules/07-fault-injection/tests/kv-env/fault_injection.rs:229-258`).
- The check tests the production claim for `buggify!`. It builds `kv-core` for a bare-metal target with the fault points compiled in (`modules/07-fault-injection/scripts/check.sh:17-18`).
- The other assertions of the four tests in [3.0] do not depend on the seed. In the 2,000-seed sweep, each failed on 0 seeds.
- The 22 requirements name all but about 5 of the items that the tests use. The readings name the functions to read (`modules/07-fault-injection/README.md:221`).

**Findings**

#### [3.0] Four assertions depend on what the seed written in the test draws

- Severity: critical. Second run: held.
- Problem 1.
- What is wrong: four assertions in `modules/07-fault-injection/tests/kv-env/fault_injection.rs` depend on what one seed draws, not on a property of correct code. They are `clock_jumps > 0` (line 290), `every fire is in the trace` (354), `downs + 6` (418) and `pauses were noticed` (177). Three of the messages blame the code, and line 290 prints only the statistics.
- Evidence: on the unmodified reference, the second run varied only the world seed over 2,000 seeds. The four lines failed on 46.4%, 31.4%, 18.2% and 1.0% of them, in that order. Four of six legal variations also failed at least one of the four. A bitmask-rejection `below` failed lines 290 and 354. Swapped `new_buggify()` calls and a forked network generator each failed line 418. A draw on a point-range latency failed line 290. A modulo-rejection `below` and jitter in `1..=max` passed.
- Who it affects: every learner who writes Module 7, on both routes. One changed draw detail fails at the rate of the line it touches. A change in every detail fails about 70% of the time. Five unstated choices decide the draws: Lemire `below`, node fork before heartbeat fork, no draw on a point-range latency, one world generator, and a uniform first jump gap at construction. The fork order and the jump gap belong to Module 7, so the reference route makes those two choices too. A learner who skips the module is not affected.
- Fix: in line 354, compare `buggify_fires` with the node's fires plus the heartbeat's. In line 418, replace `6` with the count of pairs that went `Up` first. Both held on all 2,000 seeds. For line 290, run past the 500 ms gap bound or loop over seeds. No run tested a replacement for line 177.

#### [3.1] The reference `World::resume` panics after a paused node halts in the pump

- Severity: minor. First rating: major. Second run: held.
- Problem 8.
- What is wrong: this entry covers the second of the two defects of [3.1], a bug in the reference. The first, the oracle race, is in [3.1] in the Module 06 section, and the major rating belongs to it. This half is rated minor, because the panic reaches nobody at the 24 seeds of the given test. `resume` pumps the node and then unwraps its process with `expect("still up")` (`solutions/07-fault-injection/crates/kv-env/src/fake.rs:1000-1001`). The pump takes a halted process down (`:1448-1450`), so the unwrap panics.
- Evidence: the second run swept the swarm chaos run on the unmodified reference. 26 of 40,000 seeds panicked (0.065%), all at line 1001. Seed `183` is the only failure in the first 5,000. Its trace shows a legitimate halt over an unreadable record. The Module 8 reference holds the same line (`solutions/08-workloads-checking/crates/kv-env/src/fake.rs:1132`), and later references do not.
- Who it affects: nobody at the 24 seeds of the given test. It affects a person who sweeps the reference further, and learners who copy the pump-then-unwrap pattern.
- Fix: return from `resume` after the pump once the process is gone. With that change 0 of 40,000 seeds failed, and the given `fault_injection` tests still passed 16 of 16.

#### [3.4] Requirements state pinned hashes that only the reference can produce

- Severity: moderate. Second run: none.
- Problem 1.
- What is wrong: requirement 22 states four hashes as facts (`modules/07-fault-injection/README.md:276`), and requirement 21 of Module 6 states three (`modules/06-sim-disk/README.md:265`). The second run held the same defect as [2.1] in the Module 04 section. See Problem 1 for the full account.
- Evidence: `dst start 07` replaces three test files that differ only in the constant and its comment. Every variation of [3.0] and [3.6] failed the pinned hashes.
- Who it affects: every learner on the own-code route, in seven tests. The reference route too, as soon as one draw in the learner's code differs.
- Fix: keep the learner's constants in a file that no manifest replaces.

#### [3.9] The cfg-in-macro warning does not apply here, and one test needs an unstated flag

- Severity: moderate. Second run: held.
- Problem 8 and Problem 3.
- What is wrong: the lesson says a `cfg` in the macro body asks whether `kv-env` has the feature (`modules/07-fault-injection/README.md:108`). The hint adds that `kv-core` tests then see no fire (`:325`). All four `buggify!` calls sit inside `kv-core`, so that does not happen. Separately, `cargo test -p kv-core` alone does not compile the given test.
- Evidence: the second run moved the `cfg` into the macro body. The `kv-core` tests with the feature passed 10 of 10. The workspace tests, clippy, fmt and both `no_std` builds of `kv-core` passed too. Only a one-off caller in `kv-env` showed the rule: it printed `fired=false count=0`. Without `--features buggify`, the given test fails with 10 `E0599` errors.
- Who it affects: every learner reads the false consequence, and objective 4 (`:18`) and question 6 (`:359`) rest on it. The missing flag affects learners who test `kv-core` alone, not `dst check`.
- Fix: present the rule as a hazard for a caller in another crate. Name the flag in the lesson, or add `required-features = ["buggify"]` to the test target.

See also: [3.1], [3.2], [3.3], [3.6], [3.7], [3.8] and [N.3] under Module 06 apply here too. [0.4] free choice of `below`. [8.9] one random stream for all components. [7.0] and [WA.0] lint runs before tests. [2.1], [WA.4] and [TT.5] `dst start` replaces edited tests. [WA.5] unchanged check time grows. [7.2] and [WA.6] going back leaves later tests. [7.7] and [WA.8] failed check keeps earlier pass.

### Module 08: Workloads and checking

- Teaches: the oracle problem: a workload drawn from the seed, a linearizability checker, and three planted bugs that the checker must find.
- Size: the reference adds 1,403 Rust lines under `src/`, 25 of them unit tests in the source files. 23 given tests, 16 numbered requirements.
- Reviewer's estimate: 10–16 hours to write it yourself. This is an estimate.
- Ratings from 1 (poor) to 5 (strong): scaffolding 4, alignment 4, check strength 4, feedback 3, DST focus 5.
- First `dst check` after `dst start`: `fmt FAIL` at step 1 of 5, with 42 lines of output. Two lines name the missing `crates/checker/Cargo.toml`. About 30 lines of cargo-fmt usage text follow.

**What works**

- The check calibrates the oracle in both directions. Each planted bug must be found on at least 3 of 12 seeds. It must also stay hidden from a workload that cannot reach it (`modules/08-workloads-checking/tests/kv-env/workload.rs:340-365`). The walkthrough calls this test tolerant, because it uses a threshold over a seed range.
- The hunt numbers of the lesson reproduce on the reference (`modules/08-workloads-checking/README.md:238`). The first pass saw the lost update on 11 of 12 seeds, the stale read on 9 and the duplicate apply on 5.
- The lost-update lesson has a test of its own. A write-only workload passes the history checker. The log replay must still disagree with the map on at least 4 of 8 seeds (`modules/08-workloads-checking/tests/kv-env/workload.rs:368-397`). So the learner sees a bug that only the state check finds.
- Hand-drawn histories pin each rule of the checker with its expected order (`modules/08-workloads-checking/tests/checker/linearize.rs:116-236`). The cache test needs an `Illegal` verdict for eight writers with at most 1,032 cached pairs (`modules/08-workloads-checking/tests/checker/linearize.rs:305-314`).
- The requirements name almost the whole API (`modules/08-workloads-checking/README.md:201-225`). The first pass counted about eight small unnamed items. One example: requirement 2 names `checker::history::Op`, and the tests import `checker::{History, Op}` from the crate root (`modules/08-workloads-checking/tests/checker/linearize.rs:11`).
- The lesson lists four families of checks and states what each one misses (`modules/08-workloads-checking/README.md:29-34`). It also tells one of the author's own mistakes, precedence by instants (`modules/08-workloads-checking/README.md:106-110`).

**Findings**

#### [4.6] Two wrong checkers pass every Module 8 test

- Severity: moderate. Second run: held.
- Problem 2.
- What is wrong: two wrong checkers pass. One lets a pending operation take effect before its call. The other calls a key linearizable after the budget ran out.
- Evidence: each mutant passed the full check in the second run: 230 of 230 tests. Every pending write in the given tests is called at instant 0 (`modules/08-workloads-checking/tests/checker/linearize.rs:193-236`). The budget test calls `linearize::check` alone (`modules/08-workloads-checking/tests/checker/linearize.rs:274-285`). Two new hand histories told the mutants apart.
- Who it affects: learners who make one of these two mistakes, probably a small minority. On the own-code route, their checker then runs inside Module 9.
- Fix: add both histories to the given `linearize.rs`. First: a read of 1 returns, then a put of 1 is called and never returns: `Illegal`. Second: `kv::check` with limit 2 on the budget history reports `Unknown`. State in requirement 6 that `Unknown` counts as a failed key (`modules/08-workloads-checking/README.md:206`).

#### [4.7] After `dst start`, no cargo command works until the new crate exists

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: a given test lands in `crates/checker` (Module 9: `crates/vopr`) before the crate exists, so no cargo command works. See Problem 4 for the full account.
- Evidence: the walkthrough reproduced the failure in both modules ([WB.2]). Without `--features bugs`, `cargo test -p kv-core --test bugs` runs 0 tests and prints `ok` (`modules/08-workloads-checking/tests/kv-core/bugs.rs:4`).
- Who it affects: both routes.
- Fix: ship a stub crate. Name the `--features bugs` command in the lesson.

#### [WB.3] An unchanged check runs every step again

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: this entry covers the shared cause of [WB.3] and the times of Modules 8 and 9. The Module 13 times are in [WB.3] in the Module 13 section. The runner has no partial run. The hazard script also rewrites `crates/kv-core/src/lib.rs` on each probe (`modules/03-rust-hazards/scripts/hazard-check.sh:18`), which forces a rebuild. See Problem 4 for the full account.
- Evidence: on a quiet machine, the reference takes 31.4 s for the first Module 8 check and 19.7 s for an unchanged re-run. Module 9 takes 47.6 s and 35.1 s. Under load, the walkthrough measured 37.3 s and 22.2 s for Module 8, and 82.3 s and 93.5 s for Module 9. Section 10.1 holds the table.
- Who it affects: both routes.
- Fix: run the probes on a copy of `kv-core`.

#### [4.10] The lesson credits the pending-operation clause to Lowe

- Severity: minor. Second run: none.
- Problem 8.
- What is wrong: the lesson says that Lowe's paper "adds one clause that this module needs", the appended returns for pending operations (`modules/08-workloads-checking/README.md:83`). The first pass states that Herlihy and Wing's 1990 definition already holds that clause.
- Evidence: the claim rests on the reviewer's knowledge of the paper. No file in the repository settles it, and nothing was run. The reference repeats the credit in a header comment (`solutions/08-workloads-checking/crates/checker/src/linearize.rs:23-24`). The lesson sends the learner to section 2 of Herlihy and Wing for the definition (`modules/08-workloads-checking/README.md:177`).
- Who it affects: any learner who reads that section. The cost is doubt about the lesson.
- Fix: reword the sentence. Herlihy and Wing's definition holds the clause, and Lowe restates it with events. Correct the header comment the same way.

See also: [4.5] under Module 09: requirement 16 pins one hash. [WB.0] draws must match the reference. [8.1] every bug follows a recipe. [8.5] module too large for one check. [8.8] no planted liveness bug. [7.0] and [7.4] one verdict, no progress signal. [7.8], [WA.2] and [TT.3] workspace does not load. [7.2], [WA.6] and [WB.4] redo leaves stale tests. [7.3] and [WB.8] status wrong after skip ahead.

### Module 09: Assembling the VOPR

- Teaches: the VOPR, the course's name for its simulator binary that draws a whole run from one seed, and a primary-backup store as its target.
- Size: the reference adds 2,608 Rust lines under `src/`. 73 given tests, 22 numbered requirements.
- Reviewer's estimate: 30–50 hours to write it yourself. This is an estimate.
- Ratings from 1 (poor) to 5 (strong): scaffolding 2, alignment 2, check strength 2, feedback 3, DST focus 2.
- First `dst check` after `dst start`: `fmt FAIL` at step 1 of 6, with 42 lines of output. The error names the missing `crates/vopr/Cargo.toml`. That crate is the last requirement group, so the message points away from the replica.

**What works**

- The check tests determinism across processes. The binary's hash from two processes must equal the library's hash for the same seed (`modules/09-vopr/tests/vopr/vopr.rs:50-57`).
- The safety check has a control. One test plants `StaleReadAfterStepDown` and requires each failure to say "not linearizable". The first three caught seeds must then pass without the bug (`modules/09-vopr/tests/vopr/vopr.rs:122-156`). The reference caught the bug on 18 of 24 seeds, against a floor of 3.
- A hand test pins the commit rule. With one replica down, a write stays unanswered and `uncommitted()` is 1 (`modules/09-vopr/tests/kv-env/replication.rs:256-267`).
- The lesson's numbers reproduce on the reference. The configuration line at `modules/09-vopr/README.md:62` is byte-identical to the output of `vopr --seed 1`.
- The lesson states the cost of the protocol plainly: with one replica away, the cluster commits nothing (`modules/09-vopr/README.md:104-108`).
- The reference passes `modules/09-vopr/scripts/check.sh`. The second run counted 244 tests and the sweep hash `1ff9f24540c92648`.

**Findings**

#### [4.0] Truncation, erasure and the straggler rule have no test

- Severity: major. Second run: held.
- Problem 2 and Problem 8.
- What is wrong: a replica with no log truncation, no erasure and no straggler rule passes the check (requirements 8, 9 and 11, `modules/09-vopr/README.md:217-220`). The lesson calls the four bug seeds regression tests (`modules/09-vopr/README.md:166`), and seed `4668` still passes with its fix removed.
- Evidence: the second run deleted truncation and erasure: all 244 tests passed, with every pinned hash unchanged. Truncation never ran in the reference's tests or in 5,000 seeds. Erasure never ran in the tests or the 200-seed sweep, and ran 11 times in 5,000 seeds. Without the straggler rule (`solutions/09-vopr/crates/kv-core/src/replica.rs:1019` and `:1120`), only one pinned hash failed (`modules/09-vopr/tests/kv-env/sim_network.rs:360`). One re-pin, as the hint instructs, made the whole check pass. Two stories keep a live test: reverting the seed `12784` fix aborts seeds `12784` and `36`, and `modules/09-vopr/tests/kv-env/replication.rs:233-237` asserts the salvage-and-fetch repair.
- Who it affects: both routes: no test tells a learner whether their replica has these three rules. With the reference replica, the seed `4668` test protects nothing.
- Fix: add one scenario per rule: a delayed old beat during a gather, two records with one number, a hole that no peer fills. The swarm never reached truncation, so a reach count cannot replace the scenario. Keep a regression seed only after it fails with its fix reverted.

#### [4.1] The module is mostly a replication protocol

- Severity: major. Second run: none.
- Problem 6.
- What is wrong: the module named for the VOPR is mostly a replication protocol. See Problem 6 for the full account.
- Evidence: `solutions/09-vopr/crates/kv-core/src/replica.rs` holds 1,692 lines and the `vopr` crate 414. 14 of 22 requirements concern the replica. Modules 7 to 9 share about four weeks (`CURRICULUM.md:77`).
- Who it affects: both routes.
- Fix: split it: VOPR over the Module 8 store, then replication.

#### [4.2] The tests need about 25 public items the lesson never names

- Severity: major. Second run: none.
- Problem 3.
- What is wrong: the lesson names `stats` as one word (`modules/09-vopr/README.md:213`). The tests assert 13 counters, five `Report` fields and four trace strings that it never names. See Problem 3 for the full account.
- Evidence: `requests == 3` counts requests at the primary, forwarded ones included (`modules/09-vopr/tests/kv-env/replication.rs:84`).
- Who it affects: both routes.
- Fix: ship the replica's data types and `Report` as given code.

#### [4.3] Only the linearizability check of `vopr::run` has a test

- Severity: moderate. First rating: major. Second run: held.
- Problem 2.
- What is wrong: requirement 20 asks `vopr::run` to check liveness, the log prefixes and the primary's map (`modules/09-vopr/README.md:235`). A run without them passes, and no test expects a `liveness:` message.
- Evidence: the second run removed the liveness asserts (`solutions/09-vopr/crates/vopr/src/lib.rs:194-213`) and the log, map and disk asserts (`solutions/09-vopr/crates/vopr/src/lib.rs:237-260`): all 244 tests passed. That `check_logs` call cannot fail, because `converged()` requires equal logs (`solutions/09-vopr/crates/kv-env/src/fake.rs:960`). Given tests still catch a hung cluster on about 50 seeds (`modules/09-vopr/tests/vopr/vopr.rs:29` and `:92`). One given test holds the three checks for the learner to copy (`modules/09-vopr/tests/kv-env/workload.rs:232-274`).
- Who it affects: either route: learners who omit or miswrite these checks get sweeps that stay silent about a stuck cluster or a wrong map. No learner is blocked.
- Fix: plant a liveness bug and a state bug, and require the `liveness:` or `safety:` prefix. Test `check_logs` on a diverged pair.

#### [4.4] No task asks the learner to read a failing run

- Severity: moderate. Second run: none.
- Problem 5.
- What is wrong: reading a failing run is an objective (`modules/09-vopr/README.md:21`), and the lesson only tells the author's stories. See Problem 5 for the full account.
- Evidence: the one test with failing runs prints seeds only (`modules/09-vopr/tests/vopr/vopr.rs:144`). The binary takes no bug flag before Module 11 (`solutions/11-debugging-shrinking/crates/vopr/src/main.rs:12`).
- Who it affects: both routes.
- Fix: bring `--bugs` forward, and add one replay task with `--trace`.

#### [4.5] Requirements state seven hashes that only the reference produces

- Severity: moderate. Second run: none.
- Problem 1.
- What is wrong: requirements 16 (Module 8) and 22 state seven hashes (`modules/08-workloads-checking/README.md:225`, `modules/09-vopr/README.md:237`), and a collapsed hint waives them. The second run held the same defect as [2.1] in the Module 04 section. See Problem 1 for the full account.
- Evidence: `modules/09-vopr/manifest` replaces five test files, so earlier pins are lost ([WB.7]). [WB.0] in the Module 13 section covers the wider defect: the tests need the reference's exact draws.
- Who it affects: every learner on the own-code route. The reference route too, as soon as one draw in the learner's code differs.
- Fix: state that the learner's hash differs, and keep pins outside replaced files.

#### [WB.6] On a pass, the runner hides the result line of the sweep

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: this entry covers the first part of [WB.6], the sweep step of Module 9. The second part, the shrink report, is in [WB.6] in the Module 11 section. The sweep step pipes the simulator through `tail -1`, so that one `trace_hash=` line shows (`modules/09-vopr/scripts/check.sh:33`). The runner prints the output of a step only after a failure. A learner who passes sees nothing that the simulator said.
- Evidence: the walkthrough passed Module 9 under load. The sweep step showed `ok 13.0s` and no hash line. The same holds for the simulator steps of Modules 10 to 13. [TT.10] in section 7 gives the cause in the runner. The walkthrough of the runner rated the same defect minor there.
- Who it affects: both routes.
- Fix: on a pass, print the captured output of a step under its `ok` line, as long as it is short.

#### [N.7] The stretch goal that promises a failing seed gave none in 10,000 seeds

- Severity: moderate. Found by the review. Second run: none.
- Problem 8 and Problem 5.
- What is wrong: one stretch goal tells the learner to replace the all-replicas commit rule with a majority, to change nothing else, and to sweep 1,000 seeds. It then says to keep the first failing seed (`modules/09-vopr/README.md:312`). In a trial by the review, that change gave no failing seed.
- Evidence: the review changed `try_commit` in a copy of the Module 9 reference (`solutions/09-vopr/crates/kv-core/src/replica.rs:1495`). The new rule commits a write once a majority of the replicas hold it on disk. The 200-seed hash changed from `1ff9f24540c92648` to `cdadc7ca1f461cc6`, so the change was live. Seeds `0..10000` then all passed. The likely reason is that a new primary still waits for every replica before it serves, so no committed write is lost. The Module 10 hint expects failing seeds only after the view change also uses a majority (`modules/10-consensus/README.md:225`). This was one trial with one way to write the rule.
- Who it affects: learners on both routes who try this stretch goal. They look for a failing seed that probably does not come.
- Fix: run the stretch goal on the reference before the next release, and state what the learner must change to see a failure. Do this first, before the goal becomes a required step as Problem 5 proposes.

#### [4.8] The header of `replication.rs` describes the protocol before the seed `402` fix

- Severity: minor. First rating: moderate. Second run: held.
- Problem 8.
- What is wrong: the header says "the longest log" is pulled and that logs are prefixes "at all times" (`modules/09-vopr/tests/kv-env/replication.rs:6-8`). The lesson says the newest log wins and truncates a disagreeing tail (`modules/09-vopr/README.md:137`).
- Evidence: one test panics on any sampled disagreement between two logs (`modules/09-vopr/tests/kv-env/replication.rs:317-319`). By hand, the second run drove the reference into a legal state where `check_logs` fails. No seed of 5,000 reached that state. [5.11] in the Module 10 section is part of this finding, and the second run held it: the report prints the heal count as `crashes=` (`solutions/09-vopr/crates/vopr/src/lib.rs:152-153`). Seed `5` reported `crashes=0` with four crashes traced.
- Who it affects: learners who read the header as the protocol's summary. The false failure is a latent risk.
- Fix: rewrite the header and the comment at `solutions/09-vopr/crates/kv-env/src/fake.rs:964-966`. Let `check_logs` allow the tail that `modules/09-vopr/README.md:100` allows. Print `s.crashes`: no pinned hash moves.

#### [4.9] The cross-process sweep test runs one seed

- Severity: minor. Second run: held.
- Problem 2 and Problem 3.
- What is wrong: the test passes `--seeds 10..13` (`modules/09-vopr/tests/vopr/vopr.rs:58`), and the Module 1 helper appends `--seed 99` (`solutions/09-vopr/crates/determinism/src/harness.rs:187-188`). The reference parser lets the later flag win (`solutions/09-vopr/crates/vopr/src/main.rs:37-46`). No test runs a sweep in two processes, and no Module 9 test uses `--trace`.
- Evidence: with both flags the reference ran seed `99` alone and printed `trace_hash=0c46d446e05ec2d1`. With `--seeds 10..13` alone it printed three seeds and `trace_hash=5d23b157a365c107`. A variation that rejects two seed flags failed the test with a usage message. Requirement 21 gives no rule for both flags (`modules/09-vopr/README.md:236`).
- Who it affects: learners who write a strict command line, a minority. The second run estimates half an hour lost.
- Fix: call the binary with `--seeds` alone, through a helper that appends nothing. Or state the last-flag rule in requirement 21. Add one `--trace` test.

See also: [4.7] under Module 08: workspace does not load. [WB.3] under Module 08: check time grows. [5.3] and [8.2] regression seeds do not regress. [8.4] same defect as [4.1]. [8.1] every bug follows a recipe. [8.7] debugging tools arrive in Module 11. [8.8] no planted liveness bug. [8.9] one shared random stream. [8.10] syllabus promises bugs to find. [3.0] and [3.6] seed-specific asserts, re-issued here. [3.1] oracle race, re-issued, not swept. [7.0], [7.4] and [WB.1] one verdict, no progress signal. [7.2] and [WB.4] redo leaves stale tests. [N.4] under Module 11: swapped panic hook silences failing tests (`modules/09-vopr/tests/vopr/vopr.rs:109-112`).

### Module 10: Consensus under simulation

- Teaches: Viewstamped Replication with a majority commit rule, written against the simulator from the first line.
- Size: the reference adds 1,575 Rust lines under `src/`, 27 of them unit tests in the source files. 92 given tests, 16 numbered requirements.
- Reviewer's estimate: 35–60 hours to write it yourself. This is an estimate. The syllabus budgets 15 to 24 hours (`CURRICULUM.md:72`, `CURRICULUM.md:78`).
- Ratings from 1 (poor) to 5 (strong): scaffolding 2, alignment 2, check strength 3, feedback 3, DST focus 3.
- First `dst check` after `dst start`: `clippy FAIL` at step 2 of 6, with 121 lines of output. It shows 14 of 38 compile errors, and none is about the view change.

**What works**

- The bug journal is real, and its numbers reproduce on the reference. It lists fifteen failures with seed, symptom, cause and fix, sorted into three classes (`modules/10-consensus/README.md:118-136`). The first pass ran `the_swarm_reaches_the_consensus_code` and got the counts printed at `modules/10-consensus/README.md:144`.
- The hand-written scenarios in `consensus.rs` are strong tests. One reads the trace and proves that the superblock write completed before the vote (`modules/10-consensus/tests/kv-env/consensus.rs:223-277`). A first-pass mutant that never reads its promise back passed the 200-seed sweep and failed this test.
- A planted bug tests the simulator itself, with a control run. The stale-read test reruns the first three catching seeds without the bug (`modules/10-consensus/tests/vopr/vopr.rs:157-190`). The reference caught the bug on 20 of 24 seeds.
- The reference passed the full check in the second run, with 248 tests. The 200-seed sweep caught three of the ten journal bugs that the second run put back.

**Findings**

#### [5.1] Tests pin the reference's random draws and message counts

- Severity: major. Second run: held.
- Problem 1.
- What is wrong: the lease test needs one of 24 fixed seeds to catch the planted lease bug. Other tests pin six trace hashes, a start-up message count and event counts that the requirements do not state.
- Evidence: the reference catches the lease bug on 4 of 240 seeds, and only seed `10` is inside the test's 24 (`modules/10-consensus/tests/vopr/vopr.rs:200`, `modules/10-consensus/tests/vopr/vopr.rs:225`). At that rate, the second run estimates that a similar implementation catches nothing two times in three. Two of five legal one-line variations failed the lease test. One variation sends `DoViewChange` once, as step 2 of the lesson reads (`modules/10-consensus/README.md:40`). It failed the count at `modules/10-consensus/tests/kv-env/sim_network.rs:70`. Another skips the superblock write on an empty disk, and five count assertions failed. Requirement 16 states a hash as fact, and a hint says to replace it (`modules/10-consensus/README.md:207`, `modules/10-consensus/README.md:260`). The walkthrough edited a hash in `vopr.rs`, and `dst start 10 --force` replaced the file and kept no backup ([WB.7]).
- Who it affects: every learner, because `dst start 10 --from-solution` supplies only the Module 9 reference. The pinned hashes fail for all of them, and the hint covers that. The lease test and the counts fail only for some. Nobody is fully blocked, but `caught on []` on correct code can cost hours.
- Fix: give the lease a hand-written scenario like those in `consensus.rs`. State in the requirements that view 0 starts with a view change and one superblock write. Say in requirement 16 that the learner's hashes differ.

#### [5.2] Removing the atlas, the `whole` rule or the core fails only a pinned hash

- Severity: major. Second run: held.
- Problem 2.
- What is wrong: three of the seven objectives can be left out or made empty. In each case one pinned hash is the only test that fails. The hint tells the learner to overwrite that hash (`modules/10-consensus/README.md:260`).
- Evidence: the second run made four mutants and ran every step of the check on each. With the atlas never attached, 247 of 248 tests passed, and only `known_workload_fingerprint` failed. With the `whole` wait removed, only `known_swarm_fingerprint` failed. With the core ignored, only `known_vopr_fingerprint` failed. That mutant crashed core hosts n1 and n2 27 times on seed `0`. With the seed `19` fix reverted, all 248 tests passed and the sweep hash did not change. No file under `modules/10-consensus/tests` contains `atlas`, `vetoed` or `is_whole`. The 20,000-seed sweep that the lesson runs by hand stops the atlas mutant at seed `619` and the `whole` mutant at seed `5884`. It stops the seed `19` mutant at seed `251`. The check does not run that sweep, and the core mutant ran 1,500 seeds clean.
- Who it affects: every learner, because both routes write Module 10 and set their own hashes.
- Fix: add an atlas unit test, a hand-written scenario for seed `5884`, and a test that no fault touches a core host.

#### [5.3] None of the ten pinned regression seeds catches its own bug

- Severity: major. Second run: held.
- Problem 8.
- What is wrong: the lesson says `each seed is a regression test`, and the test comment agrees (`modules/10-consensus/README.md:7`, `modules/10-consensus/tests/vopr/vopr.rs:275-276`). A seed replays a failure only on the binary that failed, and later fixes changed the traces. This is the same defect as [8.2] in section 8. Module 12 repeats the claim (`modules/12-ci/README.md:33`).
- Evidence: the second run put the ten journal bugs back, one at a time, in a copy of the Module 11 reference. That replica differs from Module 10's only by the planted `forgotten_promise` branch. No bug was caught by its own seed, and 8 of 10 were caught by none of the ten, as the table shows. Seeds `5884` and `619` still catch their own bugs, and the lesson leaves both unpinned (`modules/10-consensus/README.md:218`). The reverts are the second run's reconstructions from the journal text. No seed in 3,000 caught the reverts of `525` and `7775`, so those two are not proven live.
- Who it affects: every learner on both routes. For a learner's own code, the ten numbers never named a failure.
- Fix: state what a pinned seed is worth after the next commit, as `modules/11-debugging-shrinking/README.md:24` does. For each entry worth keeping, replace the seed with a hand-written scenario or a planted `Bug` variant with a hunt. Pin seeds `5884` and `619` today.

| Bug put back | Pinned seeds that caught it | Catches in sweep `0..200` | Catches in `0..600` |
|---|---|---|---|
| `7` | none | `55` and `81` | 4 seeds |
| `13` | `525`, `563` and `7775` | 6 seeds | 29 seeds |
| `19` | none | none | `251`, `404` and `446` |
| `22` | none | none | `393` |
| `24` | `563` | 13 seeds | 25 seeds |
| `36` | none | none | none, then 2 seeds in `0..3000` |
| `525` | none | none | none in `0..3000` |
| `562` | none | none | none |
| `563` | none | none | `551` |
| `7775` | none | none | none in `0..3000` |

Both catch columns use the binary's default draws. The check runs only the sweep `0..200`.

#### [5.4] The read barrier returns a stale read on the unchanged reference

- Severity: major. Second run: held.
- Problem 8.
- What is wrong: the lesson says the barrier costs `nothing in correctness` and names it Raft's read index (`modules/10-consensus/README.md:94`). Answer 7 says the beats prove a majority `by their arrival` (`modules/10-consensus/README.md:309`). The code counts any beat newer than the last one received, whenever the peer sent it (`solutions/10-consensus/crates/kv-core/src/replica.rs:2224`). So a beat in flight across a view change lets the old primary answer. Raft's leader sends a new heartbeat round after the read arrives and waits for the answers.
- Evidence: the second run reproduced it three ways on the unchanged reference, and the course's own checker rejected each history. The first way is a hand-built schedule with one beat given 250 ms of latency, shown in the table. The model's maximum is 252 ms. The second way uses the model's own latency draws. 25 of 2,000 world seeds failed, and 1,270 did not reach the schedule's preconditions. The third way is the unchanged `vopr::run` under a hand-picked configuration inside the swarm's ranges: 11 of 4,600 seeds failed. The swarm as drawn did not reach it: 40,000 default seeds and 20,000 read-heavy seeds passed.
- Who it affects: every learner who reaches Module 10, on both routes, and nobody is blocked. A learner who builds requirement 10 as written gets the same hole. The reference repeats the claim in a comment (`solutions/10-consensus/crates/kv-core/src/replica.rs:37-40`).
- Fix: count a beat only after it echoes something that the primary sent at or after the read's arrival. The second run did not test that rule. Add the schedule as the last journal entry, in about 60 lines of public API. It teaches what a clean sweep is worth: 60,000 clean seeds did not show that the rule is safe.

| Time | Event in the hand-built schedule |
|---|---|
| 14 ms | n0, primary of view 0, acknowledges `put a=1` |
| 702 ms | A partition cuts n0 from n2 |
| 1000 ms | n1 sends a view 0 beat to n0, due at 1250 ms |
| 1030 ms | A second partition isolates n0 |
| 1066 ms | n1 is primary of view 1 |
| 1075 ms | n1 acknowledges `put a=2` |
| 1077 ms | `get a` arrives at n0, and n0 holds it |
| 1240 ms | The partition heals |
| 1250 ms | The beat arrives, and n0 answers with the old value |

The Raft description comes from the Ratis and openraft documentation, not from the dissertation.

#### [5.6] One check covers 1,575 new lines, and the ordering hint cannot be followed

- Severity: major. Second run: held.
- Problem 6.
- What is wrong: the only hint that orders the work says `Chain first, with every Module 9 test still passing` (`modules/10-consensus/README.md:225`). `dst start 10` already replaced those tests. See Problem 6 for the full account.
- Evidence: the change to `replica.rs` adds 1,129 lines. After `dst start 10`, 6 of the 9 given test targets do not compile ([WB.1]). An unchanged re-run of the check on the reference takes 44.8 s.
- Who it affects: every learner, because no route skips Module 10.
- Fix: reword the hint, and split the check into three stages.

#### [5.7] Failing tests do not name the seed

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: three seed loops in `vopr.rs` call `run`, and no panic message of `vopr::run` carries the seed. See Problem 4 for the full account.
- Evidence: the loops start at `modules/10-consensus/tests/vopr/vopr.rs:111`, `modules/10-consensus/tests/vopr/vopr.rs:251` and `modules/10-consensus/tests/vopr/vopr.rs:278`. In Module 11, a learner whose seed `9916` passes sees only `assertion failed: !output.status.success()` (`modules/11-debugging-shrinking/tests/vopr/inspect.rs:101`).
- Who it affects: any learner whose multi-seed test fails.
- Fix: put `seed=N` in every message that `vopr::run` raises.

#### [5.9] Nothing asks for the bug journal or checks it

- Severity: moderate. Second run: none.
- Problem 5.
- What is wrong: the lesson calls the journal `the real content of this module` (`modules/10-consensus/README.md:7`). No requirement asks for one, and `REGRESSION_SEEDS` holds the author's ten seeds (`modules/10-consensus/tests/vopr/vopr.rs:301`). See Problem 5 for the full account.
- Evidence: requirements 1 to 16 do not ask for a journal.
- Who it affects: every learner on both routes.
- Fix: make a journal file with three of the learner's own seeds a deliverable.

#### [5.10] Numbers and statements disagree inside the two modules

- Severity: minor. Second run: none.
- Problem 8.
- What is wrong: six figures or statements disagree with each other or with the reference. Four are in Module 10 and two are in Module 11.
- Evidence: the guard hint says `Most of the journal fired it` (`modules/10-consensus/README.md:232`), and the journal shows the guard in five of fifteen entries. The superblock comments draw two copies (`solutions/10-consensus/crates/kv-core/src/superblock.rs:5-8`, `solutions/10-consensus/crates/kv-core/src/superblock.rs:98`), and `COPIES` is 4. A test comment lists two `DoViewChange` messages (`modules/10-consensus/tests/kv-env/sim_network.rs:66-69`), and the reference sends four. The lesson calls the newest superblock copy `at most one write old` (`modules/10-consensus/README.md:86`), which omits a lost promise write. Module 11 gives `nineteen times in twenty` and `one chance in ten` for the same hunt (`modules/11-debugging-shrinking/README.md:38`, `modules/11-debugging-shrinking/README.md:243`). It promises `ten lines` for a query that printed 159 lines (`modules/11-debugging-shrinking/README.md:103`).
- Who it affects: a careful learner who redoes the arithmetic or runs the quoted command.
- Fix: correct each figure.

#### [5.11] The report prints the heal count under `crashes=`

- Severity: minor. Second run: held.
- Problem 8.
- What is wrong: the `faults:` line of the report labels `s.heals` as `crashes` (`solutions/10-consensus/crates/vopr/src/lib.rs:165-166`). The line comes from Module 9. [4.8] in the Module 09 section holds the full account.
- Evidence: on the Module 9 reference, the second run saw `crashes=0` for seed `5`, whose trace holds four crashes. For Modules 10 to 13 it read the same source line and ran no binary.
- Who it affects: learners on the reference route from Module 9 on, and any learner who compares their report with the reference's.
- Fix: print `s.crashes`. The report is not part of the trace, so no pinned hash moves.

See also: [8.4] most work is the protocol. [8.1] bugs are narrated before found. [8.6] journal and objectives go unchecked. [8.7] debugging tools arrive in Module 11. [8.10] syllabus promises a learner journal. [8.3] stated hours are too low. [7.0] no partial check run. [7.2] and [WB.4] redo leaves stale tests. [4.5] and [WB.7] `dst start` overwrites learner pins. [0.4] Module 1 leaves `below` open. [8.2] same defect as [5.3]. [N.4] under Module 11: the swapped panic hook also silences tests here.

### Module 11: Debugging, shrinking, and tooling

- Teaches: three tools for a failing seed: a hunt over a seed range, a shrinker for the configuration, and a trace inspector.
- Size: the reference adds 1,678 Rust lines under `src/`, 312 of them unit tests in the source files. 9 given tests, 14 numbered requirements.
- Reviewer's estimate: 10–16 hours to write it yourself from the Module 10 reference. This is an estimate. On the own-code route the time is open-ended, because no seed is certain to fail.
- Ratings from 1 (poor) to 5 (strong): scaffolding 3, alignment 3, check strength 2, feedback 2, DST focus 3.
- First `dst check` after `dst start`: `clippy FAIL` at step 2 of 7, with five compile errors. They name the modules `vopr::overrides`, `vopr::shrink` and `vopr::inspect`, and the variant `Bug::ForgottenPromise`.

**What works**

- The lesson is honest about the limits of its own tool. It reports that seed `9916` shrank to 57 requests and kept every fault family, and it explains why (`modules/11-debugging-shrinking/README.md:70`).
- The lesson tells the author's own mistake. The first signature rule called seven failures different, and the run stayed at 85 requests (`modules/11-debugging-shrinking/README.md:64`).
- The printed outputs match the reference. The first pass compared the shrink report and the snapshot line for line (`modules/11-debugging-shrinking/README.md:75-79`, `modules/11-debugging-shrinking/README.md:94-99`).
- The first failure after `dst start 11` is small and ordered. The five errors map one to one to the requirement groups. The walkthrough rated it the best first contact of Modules 08 to 13.
- The `Oracle` trait ties the tools back to the panic decision of Module 3. One given test proves that the child process and the library report the same signature (`modules/11-debugging-shrinking/tests/vopr/shrink.rs:112-122`).
- The tools work on code that is not the reference. In the second run, variation E shrank its own seed `9921` from 150 to 12 requests.

**Findings**

#### [5.0] The checks of Modules 11 to 13 need seed `9916` to fail

- Severity: critical. Second run: held.
- Problem 1.
- What is wrong: the given tests and the check need seed `9916` to fail under `forgotten_promise`, with record 17 and views 18, 19 and 21. Five more tests pin seed `2` under the stale read. Whether a seed fails depends on every random draw of the learner's Modules 1 to 10. This is the same defect as [8.0] in the Module 12 section and in section 8. [WB.0] in the Module 13 section is the walkthrough's case.
- Evidence: the seed is written at `modules/11-debugging-shrinking/tests/vopr/shrink.rs:179`, `modules/11-debugging-shrinking/tests/vopr/inspect.rs:89` and `modules/11-debugging-shrinking/scripts/check.sh:39`. The checks of Modules 12 and 13 repeat it (`modules/12-ci/scripts/check.sh:53`, `modules/13-landscape/scripts/check.sh:53`). The second run made six variations of the reference and ran the three checks on each, as the table shows. A to E failed all 15 check runs at the tests step. Seed `9916` exited 0, and the shrink step printed `the seed passes: nothing to shrink`. The walkthrough added one unused draw to the finished Module 13 workspace. A 2,000-seed sweep still passed, and 19 of 291 tests failed.
- Who it affects: every learner who keeps their own Module 10. Module 10 is always the learner's own code, because `dst start 10 --from-solution` supplies only Module 9. A learner who starts Module 11 with `--from-solution` is not affected.
- Fix: three ways out keep the learner's workspace today, and no lesson names one. The learner can run `dst next --force` three times, rewrite the given files around their own seed, or copy the reference's draws. The second way needs a failing seed with the same assertion, and only the hunt of variation E found one. Instead, check the tools on inputs that do not depend on the learner's draws. The reference already holds scripted-oracle tests that need no particular seed (`solutions/11-debugging-shrinking/crates/vopr/src/shrink.rs:444-524`), and the second run found that direction feasible. Ship a trace file for the inspector, and keep seed `9916` as a guided exercise on the reference route.

| Variation | One legal change | Checks of Modules 11 to 13 | Hunt over 20,000 seeds |
|---|---|---|---|
| A | `below` by modulo rejection | failed | one seed, with another assertion |
| B | prepares sent in descending peer order | failed | nothing |
| C | Fisher-Yates shuffle that walks forward | failed, with 6 of 6 `shrink.rs` tests | nothing |
| D | duplicate coin rolled before the first latency draw | failed | nothing |
| E | clients attached by `id % nodes` | failed | seed `9921`, with the same guard |
| F | prepares pushed before the primary's own append | passed | seeds `9916` and `16345` |

F changes four pinned hashes and no draw. The checks accept another trace, but no other draw. B and E also fail the lease test of Module 10, see [5.1].

#### [5.5] No given test checks the shrinker's judgement or 19 of the 20 knobs

- Severity: major. Second run: held.
- Problem 2.
- What is wrong: no given test checks that a kept candidate fails the same way, or that an unstable result is refused. The tests name only `ShrinkError::Passes` and `Knob::Core`. The inspector's view rule is not tested.
- Evidence: the second run made two mutants. Mutant (a) keeps a candidate on any failure and skips the two final runs. Mutant (b) tries only `Knob::Core`, and its parser takes a view only from a `view=` token. Both passed every step of the Module 11 check, and both still printed `--requests 57` for seed `9916`. On the reference, every attempt on both pinned seeds is `kept` or `passed`. So the assertions in the loop at `modules/11-debugging-shrinking/tests/vopr/shrink.rs:104-109` never run. The reference's own unit tests catch both mutants, and the learner does not receive them.
- Who it affects: every learner on both routes. A learner who writes only what the tests need can skip most of requirements 2 and 6. A careful learner gets no proof that the judgement is right.
- Fix: move the `Pretend` and `Flaky` oracle tests into the given `shrink.rs` (`solutions/11-debugging-shrinking/crates/vopr/src/shrink.rs:444-524`). Add the test that every knob is live on some seed (`solutions/11-debugging-shrinking/crates/vopr/src/overrides.rs:380-401`). The oracle tests do not depend on seed `9916`, so they also close part of [5.0].

#### [N.4] Given tests silence the panic hook, so most failing tests print no message

- Severity: major. Found by the second run.
- Problem 4.
- What is wrong: the `failure` helper in the given `vopr.rs` of Modules 9 and 10 swaps the process-wide panic hook for an empty one (`modules/10-consensus/tests/vopr/vopr.rs:143-146`). The Module 11 hint prescribes the same for `run_catching` (`modules/11-debugging-shrinking/README.md:208`). Tests run in parallel. While one test is inside a run, an assertion that fails in another test prints nothing.
- Evidence: on the unchanged Module 11 reference, the second run set the constant in `known_vopr_fingerprint` to zero. Six of six runs printed `FAILED` and no hash. So the hint at `modules/10-consensus/README.md:260` fails for that hash under a plain `cargo test`. With `--test-threads=1` the hash appeared. In 15 runs of the checks on variations A to E, 79 tests failed in `vopr.rs`, `shrink.rs` and `batch.rs`. Only 12 printed a message. The message `seed 9916 fails` printed in 0 of 15 runs.
- Who it affects: every learner from Module 9 on, on both routes. A failing test in those files usually prints no message.
- Fix: do not swap the global hook in tests. Catch the unwind and let the hook print, or keep one hook that stays silent only on threads that set a flag. Until then, tell learners to rerun a failing test with `--test-threads=1`.

#### [5.8] The main shrink removes no fault family

- Severity: moderate. Second run: none.
- Problem 8.
- What is wrong: the shrink of seed `9916` keeps every fault family, because all families share one random stream. The cure is a stretch goal (`modules/11-debugging-shrinking/README.md:249`). This is the same defect as [8.9] in section 8. See Problem 8 for the full account.
- Evidence: only the request count changes in the report. The trace parser `inspect.rs` is 538 of the 1,678 new Rust lines.
- Who it affects: every learner on both routes.
- Fix: make a stream per fault family a requirement.

#### [N.5] A run of zero requests can fail, and the shrinker never tries it

- Severity: moderate. Found by the second run.
- Problem 8.
- What is wrong: the hint says `Zero passes by definition` (`modules/11-debugging-shrinking/README.md:222`). Every run also issues 30 quiet requests, so a planted bug can fail at `--requests 0`. The reference search starts at `lo = 0` and never runs that candidate (`solutions/11-debugging-shrinking/crates/vopr/src/shrink.rs:298`).
- Evidence: on the unchanged reference, seed `7` under the stale-read bug failed at `--requests 0`. The shrinker reported `--requests 1` as the smallest run. In variation A, seed `2` did the same, and the given test failed with `1 was not the smallest` (`modules/11-debugging-shrinking/tests/vopr/shrink.rs:64-70`).
- Who it affects: every learner reads the false premise. A learner who keeps their own Module 10 and whose seed `2` fails at zero requests also fails the given test.
- Fix: try the zero-request candidate first, or guard the assertion for a result above 1. Correct the hint.

#### [WB.6] On a pass, the runner hides the shrink report

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: this entry covers the second part of [WB.6], the shrink report. The first part, the sweep step, is in [WB.6] in the Module 09 section. [TT.10] in section 7 gives the cause in the runner. `dst check` prints a step's output only after the step fails. The check selects the shrink result with `tail -2` (`modules/11-debugging-shrinking/scripts/check.sh:39`), and the learner never sees it on a pass.
- Evidence: the walkthrough's passing check printed step names and times only. Run by hand, the shrink command printed a report that starts `shrunk seed 9916 in 26 runs`.
- Who it affects: every learner on both routes, in Modules 9 to 13.
- Fix: print the last lines of a short step output under its `ok` line.

See also: [8.9] one shared random stream. [8.1] the planted bug arrives diagnosed. [8.7] tools arrive after the debugging. [8.5] large checks, no small exercises. [0.4] Module 1 leaves `below` open. [7.0] no partial check run. [7.4] no progress inside a module. [WB.3] check time grows per module. [8.0] and [WB.0] same defect as [5.0].

### Module 12: Continuous simulation in CI

- Teaches: how to run the simulator unattended, with seeds from the commit, two reruns of each failure, and a cost ceiling.
- Size: the reference adds 1,040 Rust lines under `src/`, 80 of them unit tests in the source files. 8 given tests, 15 numbered requirements.
- Reviewer's estimate: 5–8 hours to write it yourself, plus 1–2 hours to push to a fork and watch the workflows. This is an estimate.
- Ratings from 1 (poor) to 5 (strong): scaffolding 4, alignment 3, check strength 3, feedback 4, DST focus 2.
- First `dst check` after `dst start`: `clippy FAIL` at step 2 of 9, with 21 lines of output. The one error is the unresolved import `vopr::batch` (`modules/12-ci/tests/vopr/batch.rs:21`).

**What works**

- The printed batch output matches the reference. The first pass ran the command at `modules/12-ci/README.md:62` and got the lines that follow it, including `sweep_hash=f46e8fb7d5ea701a`.
- The rule that no failure is retried into a pass has a strong test. A fake runner makes seed `5` fail once and then pass (`modules/12-ci/tests/vopr/batch.rs:154-197`). The test demands an unstable record with the disagreement `pass` (`modules/12-ci/tests/vopr/batch.rs:208-214`).
- The check compares one process with child processes. The batch's sweep hash must equal the hash of the one-process sweep (`modules/12-ci/scripts/check.sh:58-60`). The walkthrough ran both commands by hand on the Module 13 reference, and both printed `0e304adbce07ad30`.
- The first failure after `dst start 12` is one compile error, and the hints cover the hard parts. One hint gives the pool without threads and warns about an unread pipe (`modules/12-ci/README.md:311`). Another states which hashes the two reruns compare (`modules/12-ci/README.md:332`).
- The lesson teaches the cost ceiling as a measured budget in the seed's units. It gives the median, the 99th percentile and the maximum over 20,000 seeds (`modules/12-ci/README.md:109-113`). The one wall clock sits in `batch::Stopwatch` under `#[expect]` (`solutions/12-ci/crates/vopr/src/batch.rs:50-73`).
- The reference binary keeps the exit rule of `modules/12-ci/README.md:55`. In the second run's control runs, it exited 1 over the ceiling, and `--for 0s` ran no seed.

**Findings**

#### [8.0] Four batch tests and the shrinker step need seed `9916` to fail

- Severity: critical. Second run: held.
- Problem 1.
- What is wrong: this entry covers the part of [8.0] that concerns the tests of Modules 12 and 13. The part about Module 1 is in [8.0] in section 8. Four of eight given tests and the shrinker step need seed `9916` to fail. [5.0] in the Module 11 section is the full entry for this defect. See Problem 1 for the full account.
- Evidence: the four tests fail at `modules/12-ci/tests/vopr/batch.rs:88`, `:141`, `:325` and `:358`. The step is at `modules/12-ci/scripts/check.sh:53` and `modules/13-landscape/scripts/check.sh:53`. In the second run, five variations failed both checks. The four tests printed no message in 9 of 10 runs.
- Who it affects: every learner who keeps their own Module 10, in Modules 12 and 13. A learner who starts Module 11 with `--from-solution` is not affected.
- Fix: apply the fix of [5.0] in the Module 11 section.

#### [6.0] The nightly workflow hides a failing batch, and the lesson says the opposite

- Severity: major. Second run: none.
- Problem 8.
- What is wrong: the lesson says that GitHub runs a `run` step under bash with `pipefail` on (`modules/12-ci/README.md:191`). It concludes that `tail` cannot hide the batch's exit status. The given workflow names no shell. According to GitHub's workflow syntax page, such a step runs `bash -e {0}`, without `pipefail`. The step then takes the exit status of `tail`, and a night with a failing seed ends green.
- Evidence: the batch pipes into `tail -40` at `modules/12-ci/workflows/nightly.yml:20-25`. The file holds no `shell:` key and no `defaults:` key. The first pass fetched GitHub's page: only an explicit `shell: bash` adds `-o pipefail`. A local run of `bash -e -c 'false | tail -40; echo status=$?'` printed `status=0`. The course's own check sets `pipefail` by hand (`modules/12-ci/scripts/check.sh:3`), and so does the hunt task (`mise.toml:49`). Nobody ran the workflow on GitHub.
- Who it affects: every learner on both routes, because `dst start 12` copies the workflow. The learner keeps a false rule and a nightly template that reports each night as green.
- Fix: add `defaults: { run: { shell: bash } }` to `nightly.yml`, or remove the pipe. Correct the sentence: `pipefail` is on only for a step that names `shell: bash`. Say in one line why the batch step in `batch.yml` has no pipe.

#### [6.3] Two wrong batch implementations pass the whole check

- Severity: moderate. Second run: held.
- Problem 2.
- What is wrong: on a real run, the given tests only need two rerun hashes to be equal (`modules/12-ci/tests/vopr/batch.rs:326`), so a constant hash passes. No given test trips the step ceiling. None passes `--for` to the binary, calls `parse_budget`, or checks the exit status over the ceiling.
- Evidence: the second run made two mutants, and both passed the full check. Mutant 1 sets `output_hash` to zero (`solutions/12-ci/crates/vopr/src/batch.rs:196`, `:327`). Mutant 2 skips the step ceiling, misreads `m` and `h`, discards `--for`, and exits 0 over the ceiling. Only the fake runner supplies hashes that differ (`modules/12-ci/tests/vopr/batch.rs:172-174`). The ceiling test sets both ceilings to 1, so events always trip first (`modules/12-ci/tests/vopr/batch.rs:254-257`).
- Who it affects: any learner, on either route, who gets one of these behaviours wrong. The nightly workflow runs its batch with `--for` and `--shrink` (`modules/12-ci/workflows/nightly.yml:24`). No given test runs the binary's batch with either flag.
- Fix: on a real runner, assert that `output_hash` equals `hash_lines` of the trace. Add binary cases for `--max-steps 1`, `--for 0s` and `--shrink --report`, and a table test for `parse_budget`.

#### [6.5] `dst start 12` completes requirements 12 and 13, and no step reads the workflows

- Severity: moderate. Second run: none.
- Problem 3.
- What is wrong: the manifest copies `check.sh` and the three workflows (`modules/12-ci/manifest:3`, `:5-7`). Requirements 12 and 13 ask for those files (`modules/12-ci/README.md:287-288`). See Problem 3 for the full account.
- Evidence: for [WB.5], the walkthrough deleted two workflows and wrote invalid YAML into the third, and the check passed. The lesson's advice for a workspace in a subdirectory sets `working-directory` only (`modules/12-ci/README.md:131`). The first pass infers that the upload path then stays wrong (`modules/12-ci/workflows/batch.yml:32`). No run tested that.
- Who it affects: every learner on both routes.
- Fix: mark the files as given, or make both requirements labs.

#### [6.10] No test compiles until the whole batch API exists

- Severity: moderate. Second run: none.
- Problem 6.
- What is wrong: `batch.rs` imports twelve items for all eight tests (`modules/12-ci/tests/vopr/batch.rs:21-24`), and the lesson gives no order of work. Module 13's `port.rs` does the same (`modules/13-landscape/tests/kv-turmoil/port.rs:16-19`). See Problem 6 for the full account.
- Evidence: the first pass estimates that a quarter of Module 12's code carries the DST idea, and the rest is plumbing.
- Who it affects: every learner on both routes.
- Fix: split each test file by stage, and give the record format as code.

See also: [6.2] under Module 13: `unstable` needs a fake runner. [6.6] under Module 13: four output strings unstated. [5.3] and [8.2] regression seeds do not regress. [8.10] workflows never ran on GitHub. [7.6] a failing sweep hides the seed. [WB.6] and [TT.10] passing steps hide output. [N.4] under Module 11: silenced panic hook. [7.0] and [WB.1] one verdict per module. [WB.3] under Module 13: check time grows.

### Module 13: Other simulators, and the limits of DST

- Teaches: a port of the unchanged replica onto turmoil, a comparison of the two simulators, and the limits of DST.
- Size: the reference adds 1,439 Rust lines under `src/`. 11 given tests, 16 numbered requirements.
- Reviewer's estimate: 9–14 hours to write it yourself: 7–11 for the port and 2–3 for the retrospective. This is an estimate.
- Ratings from 1 (poor) to 5 (strong): scaffolding 3, alignment 2, check strength 2, feedback 3, DST focus 3.
- First `dst check` after `dst start`: `fmt FAIL` at step 1 of 10, with 42 lines of output. The error names the missing `crates/kv-turmoil/Cargo.toml`, and cargo-fmt usage text follows.

**What works**

- The lesson is honest about what simulation cannot find. It reports a bug in `real.rs` that 20,000 seeds never reached (`modules/13-landscape/README.md:100`). It names six gaps (`modules/13-landscape/README.md:148-162`).
- The printed report reproduces on the reference. The first pass ran seed `2` with 50 requests and got the lines at `modules/13-landscape/README.md:115-122`. The planted stale read was caught on 9 of the first 12 seeds, as `modules/13-landscape/README.md:108` says.
- The port shows what the sans-I/O design of Module 2 pays. Between the last two reference trees, `kv-core` and `checker` do not change. The port reuses `Driver`, `Client`, `Workload`, `History` and `checker::kv::check` (`solutions/13-landscape/crates/kv-turmoil/src/lib.rs:34-44`).
- The port has no pinned hash, and the lesson gives the reason (`modules/13-landscape/README.md:199`). In the second run, the eight port tests passed under the five variations that change the draws and fail the `vopr` tests.
- The lesson's general statement about `select!` is accurate (`modules/13-landscape/README.md:86`). The second run read the tokio and turmoil sources. `select!` picks its first branch at random, and turmoil seeds that generator only under `tokio_unstable`.

**Findings**

#### [WB.0] One unused random draw fails 19 tests, and the 2,000-seed sweep still passes

- Severity: critical. Second run: held.
- Problem 1.
- What is wrong: the Module 13 check still holds every pinned hash and pinned seed of the earlier modules. One changed draw fails them. The second run re-ran this defect as [5.0] in the Module 11 section, and it held. It did not repeat the walkthrough's own experiment. See Problem 1 for the full account.
- Evidence: the walkthrough added one unused `next_u64` call to the finished workspace. Seeds `0..2000` still passed, and 19 of 291 tests failed: 6 pinned hashes and 13 seed-specific assertions.
- Who it affects: every learner who keeps their own Module 10. A learner who starts Module 11 with `--from-solution` is not affected.
- Fix: apply the fix of [5.0] in the Module 11 section.

#### [6.2] The task states the storage fix before a seed can show the bug

- Severity: major. Second run: none.
- Problem 5.
- What is wrong: the port's first finding is a bug in the learner's Module 2 storage, which truncates after a write (`modules/02-architecture/README.md:203`). Requirements 4 and 14 state the fix first (`modules/13-landscape/README.md:239`, `:258`). See Problem 5 for the full account.
- Evidence: the first pass restored the truncation in the reference port. Seeds `34` and `42` failed, and `0..20` still passed. In Module 12, only a fake runner produces `unstable` (`modules/12-ci/tests/vopr/batch.rs:154-197`).
- Who it affects: every learner on both routes.
- Fix: let the learner copy `FileStorage` and sweep seeds `0..50` first.

#### [6.1] The port passes with no fault applied and no log check

- Severity: moderate. First rating: major. Second run: held.
- Problem 2.
- What is wrong: the given tests check that faults were counted, not that they reached the cluster. No test plants a log difference. The hostile datagram stops before the guard that it names.
- Evidence: the second run reduced `apply` to its three counters and removed the `check_logs` call (`solutions/13-landscape/crates/kv-turmoil/src/lib.rs:656-674`, `:962`). The full check passed. For seed `1`, the mutant printed `partitions=3 holds=5 crashes=2` with `view_changes=0`. The assertion `view_changes > 0` holds without faults (`modules/13-landscape/tests/kv-turmoil/port.rs:211`), as `modules/13-landscape/README.md:104` explains. The datagram's 25 zero bytes decode to `Err(Incomplete)` (`modules/13-landscape/tests/kv-turmoil/port.rs:120-123`).
- Who it affects: a learner on either route whose fault application is broken or does nothing. The second run calls a correct `apply` the typical case.
- Fix: on seed `34`, assert that some `Snapshot.boot` is above zero. Plant one log difference for `check_logs`. Give the datagram 33 zero bytes, which reach the guard, and match the error kind.

#### [6.7] The dealt-families test needs draws that the requirements do not fix

- Severity: moderate. Second run: held in part.
- Problem 1.
- What is wrong: one test sums the faults of seeds `1`, `2`, `5` and `34` and needs each family above zero (`modules/13-landscape/tests/kv-turmoil/port.rs:192`, `:207-210`). Requirement 10 gives the draw order and no odds or ranges (`modules/13-landscape/README.md:251`).
- Evidence: the second run gave each family odds of one in three. The test failed with `0 10 0`, and the other seven passed. At the reference's own odds, 3,252 of 20,000 groups of four seeds missed a family (16.3%). The first pass named two more tests that read fixed seeds, and they never failed in nine variations. The reference pins turmoil `0.7.2` (`solutions/13-landscape/crates/kv-turmoil/Cargo.toml:14`). The lesson names no version, and no run tested another.
- Who it affects: every learner on both routes, because each one writes the port. Even at the reference's exact odds, about one port in six fails this test by chance.
- Fix: set the families in the test, as `modules/13-landscape/tests/kv-turmoil/port.rs:129` does. Otherwise state the odds and the turmoil version.

#### [N.6] The reference port draws the fault count and the latency from one value

- Severity: moderate. Found by the second run.
- Problem 8.
- What is wrong: `RunConfig::from_seed` and `run` each start a generator from the same seed (`solutions/13-landscape/crates/kv-turmoil/src/lib.rs:717`, `:863`). The first draw of one is the latency ceiling (`:718`). The first draw of the other is the fault count (`:618`). So the latency fixes the fault count.
- Evidence: the second run ran six seeds of the reference, as the table shows. Each count equals `floor(13 * (latency - 2) / 149)`. The lesson presents seed `3`, with 9 ms and no fault, as a chance draw (`modules/13-landscape/README.md:104`). Requirement 10 promises one `Prng` (`modules/13-landscape/README.md:251`).
- Who it affects: every learner who reads or copies the reference port. Its seeds never pair a fast network with many faults.
- Fix: draw the schedule from the generator of `from_seed`, or fork a child for it. Then pick the seeds of the port tests and the lesson again.

| Seed | Latency ceiling | Faults dealt |
|---|---|---|
| `3` | 9 ms | 0 |
| `5` | 45 ms | 3 |
| `0` | 50 ms | 4 |
| `2` | 116 ms | 9 |
| `1` | 122 ms | 10 |
| `34` | 142 ms | 12 |

#### [6.4] The loop cannot show the `select!` hazard, and the stretch goal is unreachable

- Severity: moderate. Second run: held.
- Problem 2 and Problem 8.
- What is wrong: requirement 7 forbids `select!` (`modules/13-landscape/README.md:245`), and no test enforces it. A stretch goal asks for two hashes that differ in a build without `--cfg tokio_unstable` (`modules/13-landscape/README.md:324`). This loop cannot produce them.
- Evidence: the second run replaced both waits with `tokio::select!`, without that flag (`solutions/13-landscape/crates/kv-turmoil/src/lib.rs:396`, `:525`). It built random, socket-first and timer-first forms. Each passed 8 of 8 port tests. Every process printed `trace_hash=f36d20db5e8c20be` for seeds `0..40`, equal to the `timeout` build. Both arms are empty, and the loop steps the driver after either one.
- Who it affects: every learner reads a hazard that this code cannot show. A learner who takes the stretch goal looks for a difference that does not exist.
- Fix: show a loop in which arm order matters. Otherwise say that this loop is safe by its shape, and reword the stretch goal.

#### [6.9] No check, file name or reminder exists for the retrospective

- Severity: moderate. Second run: none.
- Problem 2 and Problem 5.
- What is wrong: the lesson calls the retrospective the course's written deliverable (`modules/13-landscape/README.md:9`), and it is requirement 16 (`modules/13-landscape/README.md:260`). No check step looks for it, and requirement 16 gives the file no name.
- Evidence: the lesson says that it `is read, not run` (`modules/13-landscape/README.md:272`). `dst next` then prints `The course is complete.` (`tools/dst/src/main.rs:481-483`). For Module 0, the runner reminds the learner of `NOTES.md` (`tools/dst/src/main.rs:314`). The lesson points only to the reference file (`modules/13-landscape/README.md:180`).
- Who it affects: every learner on both routes. A learner who follows the runner can finish without the one task that asks for a judgement of their own simulator.
- Fix: name the file. Add a check step: the file exists and holds the six headings. In `dst next` for Module 13, print a short self-review list.

#### [6.6] The tests need about 22 items that the lesson never names

- Severity: moderate. Second run: none.
- Problem 3.
- What is wrong: requirement 12 describes `Report` and names no field (`modules/13-landscape/README.md:253`). The tests read `FaultConfig::NONE`, `report.schedule.events` and `r.stats` (`modules/13-landscape/tests/kv-turmoil/port.rs:129`, `:144-147`). See Problem 3 for the full account.
- Evidence: two clients per node and ten quiet requests appear only in sample output, and `modules/13-landscape/tests/kv-turmoil/port.rs:134` needs both. Module 12 omits four output strings. Its requirement 9 says `The budget is checked between chunks` (`modules/12-ci/README.md:281`), and the test needs that check before the first chunk (`modules/12-ci/tests/vopr/batch.rs:298`).
- Who it affects: every learner on both routes.
- Fix: list the fields in the requirement lines, as Module 12 does.

#### [6.8] After `dst start 13`, no cargo command works until the new crate exists

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: the manifest places `port.rs` in a crate with no `Cargo.toml` yet (`modules/13-landscape/manifest:5`). The starting point does not say so (`modules/13-landscape/README.md:227`). See Problem 4 for the full account.
- Evidence: the walkthrough's first check failed at `fmt`. After the first pass added a stub crate, `--locked` refused to update `Cargo.lock`.
- Who it affects: every learner on both routes.
- Fix: tell the learner to create the crate, and then to run one cargo command without `--locked`.

#### [WB.3] Module 13 adds 33 seconds to each unchanged check

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: this entry covers the Module 13 part of [WB.3]. An unchanged check of the reference takes 84.9 s in Module 13, against 52.0 s in Module 12. [WB.3] in the Module 08 section gives the shared cause. See Problem 4 for the full account.
- Evidence: both times are clean (section 10.1). In the clean Module 13 run, the debug tests of `modules/13-landscape/tests/kv-turmoil/port.rs` took 19.6 s of the 84.9 s.
- Who it affects: every learner on both routes.
- Fix: run the port tests in release, or cut their seed counts.

#### [6.11] The lesson names Elle as the course's oracle, and the match is Knossos

- Severity: minor. Second run: none.
- Problem 8.
- What is wrong: the lesson says that Jepsen's checker Elle is the same oracle that this course built (`modules/13-landscape/README.md:168`). The course built a linearizability search. This is part of [8.11] in section 8, which holds the full account.
- Evidence: Module 8 names Knossos for that search, and says that Elle checks weaker models (`modules/08-workloads-checking/README.md:157`).
- Who it affects: a learner who follows the pointer to Elle.
- Fix: name Knossos as the match.

See also: [8.0] under Module 12: seed `9916` in this check. [6.10] under Module 12: one file, one verdict. [7.6] a failing sweep hides the seed. [8.1] bugs narrated before discovery. [8.6] written deliverables go unchecked. [8.8] no existing async code ported. [8.9] one shared random stream. [WB.2] and [TT.3] workspace does not load. [WB.5], inside [6.5] under Module 12: the walkthrough also found the retrospective unchecked. [WB.6] and [TT.10] passing steps hide output. [7.2] and [WB.4] redo leaves stale tests. [7.7] and [WB.8] status wrong after last module.

## 7. The runner

### What the runner does

The runner is `tools/dst/src/main.rs`: 904 lines, no dependencies, with the commands `init`, `start`, `check`, `next`, `list` and `dev verify` (`tools/dst/src/main.rs:70-93`). `dst start NN` applies the manifest of module NN alone, and each manifest line copies one given file into `project/` or deletes one (`tools/dst/src/main.rs:394`). With `--from-solution` it first copies `solutions/<NN-1>` over `project/` and marks that module done (`tools/dst/src/main.rs:380-383`). `dst check` runs `project/scripts/check.sh` under bash and reads each output line that starts with `== ` as a step header (`tools/dst/src/main.rs:807-823`). It prints one line per step, shows the output of the failed step alone, and takes the verdict from the exit status (`tools/dst/src/main.rs:847-861`). The state file `project/.dst-state` holds `current`, `done` and `passed` (`tools/dst/src/main.rs:224-236`). Outside Module 0 and without `--force`, `dst next` needs `passed` to equal `current` (`tools/dst/src/main.rs:464`).

### What works

- The runner has no dependencies (`tools/dst/Cargo.toml:8`). The first pass built the release binary in 2.09 s.
- Every state names the next command (`tools/dst/src/main.rs:306-325`). A refused start names three ways out (`tools/dst/src/main.rs:385-389`).
- Interrupts are safe. In four runs of the walkthrough, SIGINT or SIGTERM ended all child processes within 2 seconds. `lib.rs` and the state file were intact.
- `dst dev verify` compares each manifest file byte for byte with the reference of the same module (`tools/dst/src/main.rs:523-524`). Finding [7.1] names a blind spot.
- On the straight path, the check of every module from 01 to 13 failed before any code and passed with the reference.
- On that path the walkthrough copied each reference over the workspace. No file stayed behind from an earlier module, and the state file matched every `dst next`. `modules/10-consensus/manifest:8` deletes the one retired test.
- The runner worked from any sub-directory and with `DST_ROOT` (`tools/dst/src/main.rs:883-904`).

### Findings

Time costs in this section are estimates by the reviewers, not measurements. The walkthrough took its times under load.

The entries [7.n] come from the first pass. The entries [TT.n] report the walkthrough's own run of a defect. Where a [7.n] entry covers the same defect, the [TT.n] entry is short and points to it. Appendix A marks such pairs.

#### [7.0] `dst check` gives one verdict per module and no partial run

- Severity: major. Second run: none.
- Problem 4.
- What is wrong: `dst check` takes no arguments and runs the whole script from the top (`tools/dst/src/main.rs:85`). Each script runs fmt, then clippy with `-D warnings`, then the tests (`modules/10-consensus/scripts/check.sh:6-25`). A half-written module shows a lint or compile error and no test result. The runner has no other feedback command.
- Evidence: on the untouched Module 1 skeleton, the walkthrough saw `clippy FAIL` on `field keep_lines is never read` (`project/crates/determinism/src/trace.rs:13`). The output was the same after `prng.rs` was done ([WA.0], [TT.2]). Plain `cargo test --workspace --no-fail-fast` listed 18 failures with `not yet implemented` text. First contact in Modules 05, 06, 07, 10, 11 and 12 was a compile error under the step named `clippy`. Module 06 printed 219 lines, and Module 10 showed 14 of 38 errors ([WB.1]). Modules 02, 03, 04, 08, 09 and 13 failed at `fmt` (see [7.8]). The test step stops at the first failing test binary (`modules/01-determinism/scripts/check.sh:13`). On a variation of the Module 04 reference, the second run saw the pinned hash fail and four later test files never start. Section 10.1 gives the check times.
- Who it affects: every learner who writes a module, on both routes.
- Fix: add `--no-fail-fast` to the test step of every check script. Add a `dst test [file]` arm at `tools/dst/src/main.rs:70-93` that runs one given test file. Make the Module 1 skeleton pass clippy.

#### [7.2] `dst start` applies one manifest, so a redo leaves stale given tests

- Severity: major. Second run: none.
- Problem 7.
- What is wrong: `dst start NN` applies only the manifest of module NN (`tools/dst/src/main.rs:394`), but later modules re-issue earlier test files. A redo, a return or a jump of more than one module leaves the wrong test files in place. The refusal text and `README.md:31` say that one command brings the newer files back (`tools/dst/src/main.rs:361-363`). The walkthrough rated its own cases of this defect moderate, as [TT.4], [WA.6] and [WB.4].
- Evidence: the first pass went to Module 12, back to Module 9 with `--force`, and forward again. Six test files stayed at their Module 9 versions, and the tests no longer compiled. A script over all manifests found a stale file for 33 of 78 redo-and-return pairs. The walkthrough reproduced it ([TT.4], [WA.6], [WB.4]). After `dst start 07 --force` from Module 8, the complete Module 7 workspace failed at `fmt`. After `dst start 03 --force` and `dst start 06 --force`, `check.sh` was still the Module 03 script, because Modules 05 and 06 ship none.
- Who it affects: learners who redo a module or skip more than one module ahead.
- Fix: in `start`, apply manifests 01 to NN in order, deletes included. Remove or list the files that later manifests added. Add `check.sh` to the manifests of Modules 05 and 06.

#### [7.3] `--from-solution` keeps no backup and marks the skipped module done

- Severity: major. Second run: none.
- Problem 7.
- What is wrong: the flag copies the reference over the learner's files, deletes nothing and keeps no backup (`tools/dst/src/main.rs:643`). It records the skipped module as done (`tools/dst/src/main.rs:381-383`). `README.md:49` promises `you can come back later`, but the answer is then already in place. The walkthrough rated its own case of this defect moderate, as [TT.6].
- Evidence: the first pass ran `dst start 10 --from-solution` from Module 9, then `dst start 09 --force`. Outside the tests, `project/crates` then equalled the Module 9 reference. The walkthrough reproduced the overlay ([TT.6]). `dst start 03 --from-solution` reported the copy in one line, and the learner's edits in two files were gone. The learner's extra bin, extra test and extra crate each failed clippy, one error per check run. After a skip ahead, `dst list` showed the skipped module as `done` ([WB.8]).
- Who it affects: every learner who uses the flag after they wrote code of their own.
- Fix: before `copy_solution`, move `project/` aside and print where it went (`tools/dst/src/main.rs:627`). Record the module as skipped, and show that in `dst list`.

#### [TT.0] On macOS, `--from-solution` leaves stale builds

- Severity: major. First rating: critical. Second run: held.
- Problem 7.
- What is wrong: `copy_solution` uses `fs::copy`, which on macOS kept the old modification time of each reference file (`tools/dst/src/main.rs:643`). Those times are older than the learner's last build, so cargo does not rebuild. The second run re-ran this defect as [1.5] and rated it major. The Module 02 section gives the full account as [1.5].
- Evidence: the walkthrough built a wrong PRNG, then ran `dst start 02 --from-solution`. `prng.rs` equalled the reference, and `dst check` still failed twice in `tests/reference_prng.rs`. After `touch`, the check passed. Linux was not run.
- Who it affects: macOS learners who use the flag after a build of the crates that it replaces.
- Fix: read and write the bytes, as `apply_manifest` does at `tools/dst/src/main.rs:606-619`.

#### [TT.1] The walkthrough broke the code after a pass, and `dst next` still advanced

- Severity: major. Second run: none.
- Problem 7.
- What is wrong: this is the defect of [7.7] in this section, as the walkthrough met it. The rating is the walkthrough's, and the first pass rated the same defect moderate. `check` saves the state in the pass branch alone (`tools/dst/src/main.rs:443-445`), so a later failure leaves `passed` set.
- Evidence: in Module 01 the reference passed the check. With `next_u64` broken, `dst check` exited 1 at the `tests` step. The state file still held `passed = 01`, and `dst` printed `check: passed`. `dst next` then recorded Module 01 as done.
- Who it affects: any learner who breaks code after a pass and then runs `dst next`.
- Fix: see [7.7].

#### [TT.2] In Module 1 the check shows no partial progress

- Severity: major. Second run: none.
- Problem 4.
- What is wrong: this is the Module 1 case of [7.0]. The script runs fmt, then clippy with `-D warnings`, then `cargo test --workspace` (`modules/01-determinism/scripts/check.sh:6-13`). Cargo stops at the first failing test binary, and `dst check` takes no argument to run less (`tools/dst/src/main.rs:85`). See Problem 4 for the full account.
- Evidence: the untouched skeleton failed at `clippy` in 28 lines of output. With the reference `prng.rs` copied in, the output was identical. With `trace.rs` done too, the `tests` step stopped at `tests/cross_process.rs` with 3 failures. The tests of `hazards` and `reference_prng` never ran. `cargo test --no-fail-fast` showed 2 passed for `reference_prng`.
- Who it affects: every learner who writes Module 1. The script order is the same in Modules 02 to 13.
- Fix: see [7.0].

#### [7.8] Module 1's first command is refused, and six modules start with a broken workspace

- Severity: moderate. Second run: none.
- Problem 4 and Problem 7.
- What is wrong: after `dst init`, the command that the Module 1 lesson gives is refused with advice that loops. That part belongs to Problem 7. [0.9] in the Module 01 section rates the lesson's line alone. After `dst start` in Modules 02, 03, 04, 08, 09 and 13, no cargo command works. The runner creates `crates/<new>/tests/` for a given test (`tools/dst/src/main.rs:614-615`). `project/Cargo.toml:3` then makes that directory a workspace member with no `Cargo.toml`. See Problem 4 for the full account of this part.
- Evidence: `modules/01-determinism/README.md:137` says to run `dst init`, then `dst start 01`. The runner refuses and offers `--from-solution` (`tools/dst/src/main.rs:385-389`). That command is refused too, with the words `so run dst start 01` (`tools/dst/src/main.rs:371-377`). The walkthrough reproduced the loop ([TT.7], [WA.7]). In the six modules, the first `dst check` printed `fmt FAIL` in 42 lines of output ([TT.3], [WA.2], [WB.2]). Of those, about 26 to 30 lines were cargo-fmt usage text. `cargo test -p determinism --no-run` failed the same way, so tests that passed one module earlier cannot run. In Module 09 the `vopr` crate is the last requirement group, yet it must exist first.
- Who it affects: every learner on both routes, in six modules. The refused start affects learners who follow the Module 1 lesson to the letter.
- Fix: if the previous module is Module 0, let `dst start 01` mark it done. After `apply_manifest`, name each `crates/<name>` that holds no `Cargo.toml`, and print `cargo init --lib crates/<name>`. A stub `lib.rs` must hold one `//!` line, because the second run saw a zero-byte `lib.rs` fail the fmt step.

#### [7.1] Module 3's check rejects a Module 1 given test that no manifest re-issues

- Severity: moderate. First rating: major. Second run: held.
- Problem 7 and Problem 3.
- What is wrong: the clippy step of Module 3 fails in `crates/determinism/tests/hazards.rs`, a given test from Module 1. The reference adds one `#![allow]` line to that file (`solutions/03-rust-hazards/crates/determinism/tests/hazards.rs:11`). The Module 3 manifest does not re-issue the file (`modules/03-rust-hazards/manifest:3-6`), and the lesson does not name it.
- Evidence: the second run walked the straight path. With the Module 3 reference in every other file, the check failed with 13 errors. The Module 1 copy of `trace_hash.rs` added a 14th. `dst dev verify` still printed `every manifest matches its reference solution`, because it never replays earlier manifests (`tools/dst/src/main.rs:521-524`). Requirement 7 says `Do not use #[allow]` (`modules/03-rust-hazards/README.md:180`).
- Who it affects: every learner who writes Module 3, on both routes, once. `dst start 03 --from-solution` gives the same failure. The cost is minutes.
- Fix: add the Module 3 version of `hazards.rs` to `modules/03-rust-hazards/manifest`, and name `trace_hash.rs` in the lesson. Make `dst dev verify` replay manifests 01 to NN before it compares.

#### [7.7] A failed check does not clear an earlier pass

- Severity: moderate. Second run: none.
- Problem 7.
- What is wrong: `check` saves the state in the pass branch alone (`tools/dst/src/main.rs:443-445`). A later failure leaves `passed` set. `dst start NN --force` on that same module keeps the pass too (`tools/dst/src/main.rs:398-400`). The walkthrough rated the same defect major as [TT.1] and minor as [WA.8].
- Evidence: the walkthrough reproduced it in Module 01 ([TT.1]) and in Module 07 ([WA.8]). After a pass, a broken `next_u64` made `dst check` exit 1. The state file still held `passed = 01`, `dst` printed `check: passed`, and `dst next` recorded the module as done. After the last module, `dst` still offers `dst next` (`tools/dst/src/main.rs:317-318`). The walkthrough saw the same ([WB.8]).
- Who it affects: any learner who breaks code after a pass and then runs `dst next`.
- Fix: in the failure branch, set `state.passed = None` and save (`tools/dst/src/main.rs:448`). Clear it in `start` after any file was written.

#### [7.4] Nothing links a failing test to a numbered requirement

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: inside a module the runner gives one progress signal, the name of the failed step. The footer after any failure is the same sentence (`tools/dst/src/main.rs:450-453`). No given test names a requirement.
- Evidence: the first pass counted. In Modules 4 to 13, none of the 169 numbered requirements names any of the 429 given test functions. The 13 manifests hold 52 test-file lines, so the runner already knows each test file. After a formatting failure the footer names the hints, not `cargo fmt --all`.
- Who it affects: every learner who writes a module of 10 to 22 requirements.
- Fix: run each test file of the manifests alone, and print its passed and failed counts. Add a requirement range to each manifest line (`tools/dst/src/main.rs:673`).

#### [7.6] In Modules 12 and 13 a failing sweep hides the seed

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: the sweep step captures the simulator's output in a variable and echoes it one line later (`modules/12-ci/scripts/check.sh:47-48`). A failing sweep exits before the echo. Module 13 has the same lines. See Problem 4 for the full account.
- Evidence: with the planted bug `forgotten_promise`, the Module 12 form printed the panic line alone. The Module 9 form also printed `seed=9916 nodes=3 clients=6` (`modules/09-vopr/scripts/check.sh:33`).
- Who it affects: learners in Modules 12 and 13 whose change breaks one of the 200 seeds.
- Fix: write the sweep output to a file, and print its last lines on failure too.

#### [7.9] An output line that starts with `== ` becomes a step header

- Severity: moderate. Second run: none.
- Problem 7.
- What is wrong: the runner treats every output line with the prefix `== ` as a step header, whoever printed it (`tools/dst/src/main.rs:823-828`). The real step closes as `ok`, and the output before the line is discarded.
- Evidence: the walkthrough added a failing test that prints `== store dump ==` ([TT.11]). `dst check` printed `tests ok 0.9s`, then `store dump == FAIL 0.0s`. The output before the banner was gone, and so was the `Step N of M failed` line.
- Who it affects: learners who print a banner with that prefix while they debug a failing test.
- Fix: accept as headers only the step names that `count_steps` reads from the script (`tools/dst/src/main.rs:742-751`).

#### [7.5] The quick loop, a hint command, a reset and a platform note are missing

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: compared with rustlings, the runner lacks a re-run on save, a hint command, a clean reset and a way to run one test file. The first pass read the code and ran nothing.
- Evidence: the hints are one fixed pointer to the lesson (`tools/dst/src/main.rs:450-453`). The runner starts `bash` (`tools/dst/src/main.rs:807`), and `README.md` has no line on Windows, WSL or bash. The behaviour on Windows was inferred.
- Who it affects: learners who expect a save-and-see loop, and learners on native Windows.
- Fix: state the platform beside `README.md:8`. Add `dst hint`, which lists the `<summary>` titles of the current lesson.

#### [TT.3] A given test for a crate that does not exist yet breaks the workspace

- Severity: moderate. Second run: none.
- Problem 4.
- What is wrong: this is the workspace part of [7.8]. `dst start` writes a given test into `crates/<new>/tests/` before the crate exists (`tools/dst/src/main.rs:614-615`). See Problem 4 for the full account.
- Evidence: the walkthrough ran it in Modules 02, 03 and 04. Each first `dst check` printed `fmt FAIL`, the missing `Cargo.toml` and the cargo-fmt usage text, in 42 lines. `cargo test --test reference_prng` failed with the same error. The walkthrough of the modules saw the same in all six modules ([WA.2], [WB.2]).
- Who it affects: every learner on both routes, in six modules.
- Fix: see [7.8].

#### [TT.4] Going back and coming forward leave the wrong files in place

- Severity: moderate. Second run: none.
- Problem 7.
- What is wrong: this is the defect of [7.2] in this section, as the walkthrough met it. The rating is the walkthrough's, and the first pass rated the whole defect major. `dst start` applies one manifest (`tools/dst/src/main.rs:394`). It deletes only the files that this manifest names (`tools/dst/src/main.rs:591-595`).
- Evidence: from a later module, `dst start 01 --force` left the test directories of the later modules behind. `dst check` then failed at `fmt`. After `dst start 03 --force` and `dst start 06 --force`, `check.sh` equalled the Module 03 script. It lacked the line `cargo check -p sim --target thumbv7em-none-eabihf`. With `check.sh` removed, the runner said `run dst start 06` (`tools/dst/src/main.rs:438-440`). That command did not restore the file.
- Who it affects: learners who redo a module. The missing `check.sh` affects a return to Module 05 or 06.
- Fix: see [7.2].

#### [TT.5] An edited given test is not noticed, and `dst start` overwrites it

- Severity: moderate. Second run: none.
- Problem 1 and Problem 7.
- What is wrong: `apply_manifest` replaces a file with different content, prints the one word `replaced` and keeps no backup (`tools/dst/src/main.rs:608-619`). The lessons tell the learner to edit the pinned hash constants in these files (`modules/02-architecture/README.md:205`). `dst check` never compares the given tests with their originals.
- Evidence: in Module 04 the walkthrough changed the pinned hash in `fake_world.rs`. Plain `dst start 04` printed `replaced`, and the edit was gone. In Module 01, `reference_prng.rs` without its assertions passed the check beside a wrong PRNG. The walkthrough of the modules saw the same overwrite ([WA.4], [WB.7]). See [2.1] in the Module 04 section and [4.5] in the Module 09 section.
- Who it affects: learners on the own-code route who pin their own hashes, at each `dst next` that re-issues the edited file.
- Fix: before the runner replaces a file with different content, save a copy and print its path. On `dst check`, list the given tests that differ from the manifest.

#### [TT.6] After `--from-solution`, the learner's extra files fail the next check

- Severity: moderate. Second run: none.
- Problem 7.
- What is wrong: this is the defect of [7.3] in this section, as the walkthrough met it. The rating is the walkthrough's, and the first pass rated the whole defect major. `copy_solution` replaces each file that shares a path with the reference and leaves all other files (`tools/dst/src/main.rs:636-644`). Each directory under `crates/` is a workspace member (`project/Cargo.toml:3`).
- Evidence: `dst start 03 --from-solution` printed one line, `copied 29 files`, with no list. A method in `fake.rs` and a `pub mod util;` line in `lib.rs` were gone. The next three checks failed at clippy with `E0433`, `E0599` and `E0308`, one error per run. After the three extras were removed, the check passed.
- Who it affects: learners who use the flag after they wrote code of their own.
- Fix: see [7.3]. After the copy, also list the learner's files that the reference lacks.

#### [TT.7] The first command of the Module 1 lesson ends in advice that loops

- Severity: moderate. Second run: none.
- Problem 7.
- What is wrong: this is the refused start of [7.8]. The refusal offers `dst start 01 --from-solution` (`tools/dst/src/main.rs:385-389`). That command answers with the words `so run dst start 01` (`tools/dst/src/main.rs:371-377`).
- Evidence: after `dst init`, `dst start 01` exited 1. `dst start 01 --from-solution` exited 1 too. `dst check` on Module 00 printed `Module 00 has no automated check` and exited 0. Only `dst next` or `dst start 01 --force` leaves the loop.
- Who it affects: learners who follow `modules/01-determinism/README.md:137` to the letter.
- Fix: see [7.8].

#### [TT.8] The hazards step rewrites `kv-core/src/lib.rs` while the learner can edit it

- Severity: moderate. Second run: none.
- Problem 7 and Problem 4.
- What is wrong: `hazard-check.sh` appends a probe module to `lib.rs` and copies the saved file back after each of 16 probes (`modules/03-rust-hazards/scripts/hazard-check.sh:11-25`). An edit saved during the step is lost without a message. The cause is in the script, not in `tools/dst/src/main.rs`. [WA.5] in the Module 03 section and [WB.3] in the Module 08 section give the cost of the rebuild in time.
- Evidence: the walkthrough appended a comment to `lib.rs` three seconds into the step. The step ended `ok`, the check passed, and the comment was gone. After the script, an unchanged Module 13 tree also recompiled six crates in each profile.
- Who it affects: a learner who saves `lib.rs` during the step. Every learner from Module 03 on pays the rebuild in each check.
- Fix: run the probes on a copy of the crate under `target/`.

#### [TT.9] The twenty-run step of Module 1 prints a passing re-run after a failure

- Severity: moderate. Second run: held.
- Problem 7.
- What is wrong: the loop sends a failing run to `/dev/null` and prints a new run, which usually passes (`modules/01-determinism/scripts/check.sh:44-46`). The second run re-ran this defect as [0.3] and kept the rating. The Module 01 section gives the full account as [0.3].
- Evidence: in the second run, under heavy load, a real `dst check` on the reference printed `FAIL`, `hazards failed on run 13 of 20`, then 13 `ok` lines. The name of the failed test and its panic text never appeared.
- Who it affects: every learner on Module 1, on both routes.
- Fix: write each run to a file, and print that file on failure.

#### [7.10] Seven smaller runner defects

- Severity: minor. Second run: none.
- Problem 7.
- What is wrong: seven small defects, each one reproduced by a run ([TT.11]). The forced rebuild that the first pass listed here is in [TT.8].
- Evidence: unknown flags are ignored (`tools/dst/src/main.rs:96-98`). `dst next --dry-run` marked Module 00 done and started Module 01. `start` saves the state last, and a print to a closed pipe panics before that (`tools/dst/src/main.rs:401`). One non-UTF-8 byte in step output aborts the report (`tools/dst/src/main.rs:822`). On a terminal, `stop` joins a ticker that sleeps one second, so short steps read 1.0s or 0.0s (`tools/dst/src/main.rs:790`). The walkthrough reported that as [WA.11]. A bad value in the state file gives an error that names neither the file nor `dst init --force` (`tools/dst/src/main.rs:211`). A `check.sh` with no executable bit prints `Step 0 of 5 failed` (`tools/dst/src/main.rs:809`). The runner has no step timeout. In the second run of [2.9], a hung test left the `tests` line silent until the run was stopped after 480 seconds.
- Who it affects: learners who mistype a flag, pipe the output or damage the state file. None of these defects blocks a learner.
- Fix: reject unknown arguments in `run`. Save the state before the first print. Decode step output as lossy UTF-8. Read `elapsed` before `ticker.stop`.

#### [TT.10] A passing step hides the result lines that the lessons are about

- Severity: minor. Second run: none.
- Problem 4.
- What is wrong: the runner clears its buffer at each step header and prints the buffer on failure alone (`tools/dst/src/main.rs:828`, `tools/dst/src/main.rs:859-861`). A passing step shows `ok` and a time, so the hashes that the steps print never reach the learner. The rating is that of the walkthrough of the runner. The walkthrough of Modules 08 to 13 rated the same defect moderate. Its entries are [WB.6] in the Module 09 section and in the Module 11 section.
- Evidence: the two-process step of Module 1 echoes the trace hash (`modules/01-determinism/scripts/check.sh:19`), and `dst check` showed `ok 0.4s`. A passing Module 13 check printed 14 lines. By hand, the same commands printed the sweep hash, the shrinker's account and the batch report.
- Who it affects: every learner on both routes, in Module 01 and in Modules 09 to 13.
- Fix: on a pass, print the last lines of a step with short output under its `ok` line.

#### [TT.11] Six rough edges in parsing and messages

- Severity: minor. Second run: none.
- Problem 7.
- What is wrong: the walkthrough listed six small defects. [7.9] holds the step header, and [7.10] holds four more. The sixth is in [7.3]: a skipped module is listed as `done` (`tools/dst/src/main.rs:332-333`).
- Evidence: `dst start 04 --from-soluton` answered `module 03 is not done`. `dst -h | head -1` panicked with `failed printing to stdout: Broken pipe`. A state file with `current = 99` gave `there is no module 99; the last one is 13` for every command. Under a pseudo-terminal, `fmt` read `1.0s` against `0.2s` through a pipe.
- Who it affects: learners who mistype a flag, pipe the output or damage the state file.
- Fix: see [7.9] and [7.10]. Mark a skipped module as skipped in `dst list`.

### Edge cases the walkthrough ran

| Case | What happened | Finding |
|---|---|---|
| 1: commands before `dst init` | Each printed `not initialized; run dst init first`. | none |
| 1b: unknown command, bad module number | Clear messages. `dst init extra` ignores the extra word. | [7.10] |
| 1c: output piped into `head` | Panic. `start` copied files and left the state unsaved. | [7.10] |
| 2: `dst init` twice | The second `dst init` is refused and names `dst init --force`. | none |
| 2c: `dst init --force` after progress | Code kept, done list lost. A leftover test directory then broke the check. | [7.2] |
| 3: Module 00 | No check runs. `dst next` marks it done with nothing written. | [0.7] |
| 4: Module 01, untouched skeleton | `clippy FAIL` on a dead field. No test ran. | [7.0] |
| 5: Module 01 with `prng.rs` done | Same output as case 4. | [7.0] |
| 6: one line not formatted | `fmt FAIL`. The footer does not name `cargo fmt --all`. | [7.4] |
| 7: one unused variable | `clippy FAIL`. No test ran. | [7.0] |
| 8: pass, break the code, `dst next` | `dst next` recorded the module as done. | [7.7] |
| 8b: failing check after a pass | The state kept `passed = 01`. | [7.7] |
| 9: weakened given test | The check passed with a wrong PRNG. | [TT.5] |
| 10a, 10d: start out of order or a done module | Both refused. The promised way back fails for Modules 05 and 06. | [7.2] |
| 10b, 10f: `--force` on `start` and `next` | A skipped module is listed as done. | [7.3] |
| 10c, 10e: go back to an earlier module | Later test directories stayed, and the check failed at `fmt`. | [7.2] |
| 11: `--from-solution` over extra files | Edits replaced with no backup. An extra bin, an extra test and an extra crate failed clippy. | [7.3] |
| 12: Module 05, no `check.sh` of its own | The Module 04 script stayed and ran the new tests. | none |
| 13: sub-directory, `DST_ROOT`, mise | All worked. | none |
| 14: damaged state file | The error names no file and no cure. | [7.10] |
| 15: interrupt during a slow step | Child processes ended within 2 seconds. Files and state were intact. | none |
| 16: pseudo-terminal | Clean at 100 columns. Short steps read 1.0s. | [7.10] |
| 17: a test prints a `== ` line | `tests` reported `ok`, and an invented step failed. | [7.9] |
| 18: compile failure in Module 04 | `fmt FAIL` first, then 553 lines under `clippy`. | [7.8] |
| X1: build after `--from-solution` | Reference source failed its own test until `touch`. | [TT.0] |
| X2: `dst init`, then `dst start 01` | Refused. The two messages point at each other. | [7.8] |
| X3: edit `lib.rs` during the hazards step | The step passed, and the edit was gone. | [TT.8] |
| X4: redo Module 03, return to Module 06 | `check.sh` stayed the Module 03 script. | [7.2] |
| X5: reference test under heavy load | 2 of 200 runs failed. The step printed a passing re-run. | [TT.9] |
| X6: misspelt or misplaced flags | Unknown `--` flags were ignored. The error answered another command. | [7.10] |
| X7: `check.sh` missing or not executable | `dst start 06` does not restore it. | [7.2], [7.10] |
| X8: end of the course | `dst` still offers `dst next`. | [7.7] |

## 8. The curriculum as a whole

### What works across the course

- The lessons read as a short, accurate book on DST. Each one states a problem, shows the mechanism, compares it with TigerBeetle and FoundationDB, and records the decision with its cost (`modules/04-virtual-time/README.md:111-117`).
- The course is honest about limits. Module 0 defines a clean run by what the model, the faults and the checks cover (`modules/00-why-dst/README.md:337-340`).
- The lessons teach doubt of one's own tools. The Module 10 journal sorts fifteen bugs into three classes: the protocol's, the guard's and the fault model's (`modules/10-consensus/README.md:118-136`).
- Decisions that one module leaves open return later. The superblock is a stretch goal in Module 6, and seed `562` forces it in Module 10 (`modules/06-sim-disk/README.md:129`, `modules/10-consensus/README.md:82-84`).
- The content comes from primary sources. Module 7 gives FoundationDB's two BUGGIFY probabilities and turns the swarm testing paper into eight fault families (`modules/07-fault-injection/README.md:98`, `:118-137`). The readings name sections and say why (`modules/08-workloads-checking/README.md:177`).
- The hunts that need several catches do not depend on exact seeds. The Module 9 hunt runs 24 seeds and needs at least three catches (`modules/09-vopr/tests/vopr/vopr.rs:123-145`). The first pass changed `Prng::below` in the Module 11 reference. Then 9 of 10 tests in `vopr.rs` still passed, all but the pinned hash.
- The reference reproduces the numbers that the lessons print. The critic is the agent that looked for gaps in the review. The critic ran the Module 13 check on the reference: 291 tests passed, and debug and release builds printed the hash that `modules/11-debugging-shrinking/README.md:186` pins.
- The prose is plain, and the given tests read well. Lesson sentences average 11.0 to 15.2 words, and general terms are defined at first use (`modules/03-rust-hazards/README.md:39`). One test comment draws four operations where the code holds three (`modules/08-workloads-checking/tests/checker/linearize.rs:78-84`).
- The critic corrected the first pass in two directions. Undersold: the 74 stretch goals already hold the open-ended work that the findings ask for (`modules/09-vopr/README.md:312`). No reviewer named the quality of the reference code or of the prose. Oversold: three strengths hold only in part. The numbers reproduce on the reference binary and on no learner's own code. The independence from exact seeds fails for the Module 10 lease hunt ([5.1]). Skipping with `--from-solution` overwrites the learner's code and gives no tour of the new code ([7.3], [C.1]).

### Findings

#### [8.0] The last three checks need seed `9916`, and Module 1 allows other draws

- Severity: critical. Second run: held.
- Problem 1.
- What is wrong: this entry covers the part of [8.0] that concerns Module 1. The part about the tests of Modules 12 and 13 is in [8.0] in the Module 12 section. Module 1 allows any unbiased `below` (`project/crates/determinism/src/prng.rs:33`), yet three later checks need seed `9916` to fail. The tests then contradict the rule that the Module 11 lesson states: a seed names a run only for one binary. See Problem 1 for the full account.
- Evidence: the second run changed only `below`, and all three checks failed.
- Who it affects: every learner who keeps their own Module 10, at the checks of Modules 11, 12 and 13. A learner who starts Module 11 with `--from-solution` is not affected.
- Fix: see [5.0] in the Module 11 section, the full entry for this defect.

#### [8.1] Every named bug is planted to a recipe or told before the task

- Severity: major. Second run: none.
- Problem 5.
- What is wrong: the learner never goes from an unknown failing seed to a trace, a shrunk run and a fix. Modules 9 and 10 tell their 4 and 15 real bugs as stories, and numbered requirements state the fixes. The six planted bugs follow a written recipe. Module 11 uses its tools once, on a bug whose diagnosis the lesson prints. See Problem 5 for the full account.
- Evidence: the Module 9 lesson says `It is wrong, and seed 36 found out how` (`modules/09-vopr/README.md:114`), and requirement 9 states the repaired rule (`:218`). Module 10 calls the journal `the real content of this module` (`modules/10-consensus/README.md:7`), and requirements 6 to 11 state the fixes (`:194-199`). Module 11 prints the three inspector queries and the answer (`modules/11-debugging-shrinking/README.md:89-115`).
- Who it affects: every learner on both routes. On the own-code route a sweep can still expose the learner's own bugs, but no task guides that work. Syllabus outcome 6 depends on this skill (`CURRICULUM.md:37-39`).
- Fix: make the stretch goal at `modules/09-vopr/README.md:312` the required first step of Module 10. Put the journal table and requirements 6 to 11 in collapsed blocks. In Module 11, plant one bug whose diagnosis the lesson does not print.

#### [8.2] The ten regression seeds no longer catch their bugs

- Severity: major. Second run: held.
- Problem 8.
- What is wrong: Module 12 says the seeds run `on every push, forever` (`modules/12-ci/README.md:33`). Module 11 states the correct rule (`modules/11-debugging-shrinking/README.md:24`), and the course does not apply it to its ten seeds. [5.3] in the Module 10 section is the full entry for this defect. See Problem 8 for the full account.
- Evidence: the second run put the ten bugs back, and no seed caught its own bug. The 200-seed sweep of the check still caught three of the ten.
- Who it affects: every learner on both routes.
- Fix: see [5.3] in the Module 10 section.

#### [8.3] The stated budget is two to three times below the first-pass estimate

- Severity: major. Second run: none.
- Problem 6.
- What is wrong: the syllabus budgets 18 weeks at 5 to 8 hours a week, so 90 to 144 hours (`CURRICULUM.md:71-79`). The references add 17,953 Rust lines under `src/`. The first pass estimates 250 to 450 hours for an experienced Rust programmer who is new to DST. Both figures are estimates.
- Evidence: the budget implies 125 to 199 finished lines an hour, with no time for lessons, papers or debugging. The commits that add Modules 0 to 13 date from 2026-10-10 to 2026-10-12, so no trial with a learner stands behind the budget. No module README states hours.
- Who it affects: every learner who writes the modules and plans by the syllabus, on both routes. Module 2 gets about one week for 2,478 Rust lines with no skeleton. A learner who mostly reads the reference needs far less time.
- Fix: label the budget as not measured, and state a range per module in each README and in `dst list`. Time one or two real learners. Present `--from-solution` as a planned route.

#### [8.4] In Modules 9 and 10, two thirds of the new lines are replication

- Severity: major. Second run: none.
- Problem 6.
- What is wrong: across the course the split is sound, with about 64% of the final code in simulator, oracle and tooling. In Modules 9 and 10 the split reverses, with 67% of the new lines in `replica.rs`. See Problem 6 for the full account.
- Evidence: about 36% of Module 2 is TCP server and wire codec. [4.1] in the Module 09 section gives the line counts of Module 9.
- Who it affects: every learner in those modules.
- Fix: offer a track with a given replica.

#### [8.5] Thirteen large checks, and no small exercises for the small ideas

- Severity: major. Second run: none.
- Problem 6.
- What is wrong: one check covers a median of about 1,440 Rust lines. Syllabus outcome 1 is to find every source of nondeterminism (`CURRICULUM.md:27-28`), yet no task hides a hazard in unlabelled code. See Problem 6 for the full account.
- Evidence: Module 1's 13 hazard tests arrive written (`modules/01-determinism/README.md:152`).
- Who it affects: every learner on both routes.
- Fix: add single-file exercises with instant checks.

#### [C.1] The reference route has no map of the code it hands over

- Severity: major. Found by the critic. Second run: none.
- Problem 7.
- What is wrong: no first-pass finding looks at the reference as material to learn from. Its quality is high, but nothing guides a learner through it. The critic expects that the critical findings push most finishers onto the reference route, where each module starts inside thousands of unfamiliar lines.
- Evidence: the critic counted 16,847 source lines in the last reference, 15% of them doc comments. `replica.rs` opens with a diagram (`solutions/13-landscape/crates/kv-core/src/replica.rs:1-58`), but its `on_peer` is 359 lines long (`:1251`). The only guidance is one line from `dst next` (`tools/dst/src/main.rs:477`). A learner who starts Module 10 from the reference inherits 11,511 lines, and the lesson lists only test files (`modules/10-consensus/README.md:180`).
- Who it affects: every learner on the reference route.
- Fix: add a note to each module with five files in reading order. Make `dst next` print the files that changed between the two references. Hand over the reference's 63 unit tests as optional staged tests.

#### [8.6] Nothing checks the written work or the understanding objectives

- Severity: moderate. Second run: none.
- Problem 2.
- What is wrong: every check tests that the code conforms to an API. Nothing checks the written deliverables: the Module 0 notes, the Module 10 journal and the Module 13 retrospective. From Module 9 to Module 11 the questions ask for recall of the author's seeds.
- Evidence: the Module 13 lesson says `The retrospective is read, not run` (`modules/13-landscape/README.md:272`). `Keep a bug journal` is an objective of Module 10 (`modules/10-consensus/README.md:17`), and requirements 1 to 16 omit it (`:186-207`). Module 13 then asks for every bug since Module 6 (`modules/13-landscape/README.md:184`). One question reads `Name the seed` (`modules/10-consensus/README.md:294`).
- Who it affects: every learner on both routes. A learner can pass every check and still be unable to diagnose a seed.
- Fix: make the journal a requirement from Module 6. Make the capstone checkable: `dst` plants a hidden bug, and the learner submits a reproduction line.

#### [8.7] Hunt, shrink and inspect arrive after the modules that need them

- Severity: moderate. Second run: none.
- Problem 5.
- What is wrong: the learner's own seeds fail in Modules 9 and 10. The only tools there are `--seed N --trace` and a sweep that stops at the first failure. The hunt, the fault-family flags, the shrinker and the inspector arrive in Module 11. By then the only target is a planted bug with a printed diagnosis.
- Evidence: requirement 21 of Module 9 gives the binary two modes and one `--trace` flag (`modules/09-vopr/README.md:236`). A failing seed `aborts the sweep at its assertion` (`:248`). Module 10 says that each bug was `read from a trace of some thousands of lines` (`modules/10-consensus/README.md:318`).
- Who it affects: every learner who writes Module 9 or Module 10.
- Fix: move the hunt and the `--no-<family>` flags into Module 9. In Module 11, have the learner first run the shrinker on one of their own Module 10 seeds.

#### [8.8] Four ideas from practice are absent or only read about

- Severity: moderate. Second run: none.
- Problem 2.
- What is wrong: four ideas from practice are absent or only read about. The learner never makes existing async code simulable. The lessons do not mention mixed-version runs or coverage-guided search. All six planted bugs are safety bugs, so no test shows the liveness oracle as it fires. Only that last part belongs to Problem 2. The rest are additions to the course, not repairs.
- Evidence: a search of the syllabus and the 14 lessons found no file with `mixed-version`, `multiversion`, `rolling`, `coverage-guided` or `code coverage`. The port lesson says that nothing in `kv-core` changes (`modules/13-landscape/README.md:43`). The `Bug` enum lists only wrong-answer bugs (`solutions/13-landscape/crates/kv-core/src/bugs.rs:18`). No given test contains `liveness:`.
- Who it affects: every learner on both routes.
- Fix: plant one liveness bug, for example the rule behind seed `353`, and require the quiet phase to catch it. In Module 13, add a small async service that the learner makes deterministic under turmoil.

#### [8.9] Module 1 teaches forked streams, and the reference world shares one stream

- Severity: moderate. Second run: none.
- Problem 8.
- What is wrong: Module 1 teaches one forked stream per component (`modules/01-determinism/README.md:87-89`). The reference world passes one `Prng` to the network and to every disk (`solutions/13-landscape/crates/kv-env/src/fake.rs:1789`, `:1525`). No lesson draws the link. See Problem 8 for the full account.
- Evidence: the file's only `.fork()` is for Buggify (`solutions/13-landscape/crates/kv-env/src/fake.rs:1424`).
- Who it affects: every learner on both routes.
- Fix: see [5.8] in the Module 11 section.

#### [8.10] The syllabus and the plan state things that the modules do not deliver

- Severity: moderate. Second run: none.
- Problem 8.
- What is wrong: three statements do not hold. The syllabus says that the learner finds bugs and keeps a journal in Modules 9 and 10, but the modules state the fixes first. The syllabus and the Module 12 lesson report that traces agree across two operating systems, but the workflows never ran on GitHub. The plan's layout omits `tools/dst`.
- Evidence: the syllabus says `You will find bugs` (`CURRICULUM.md:577-578`) and `Keep a bug journal` (`CURRICULUM.md:630`). It says that `CI checks that it does` (`CURRICULUM.md:687-688`). The plan says `They have not run on GitHub` (`plan.md:224`), yet `three green workflows` is an acceptance criterion (`modules/12-ci/README.md:299`). `plan.md:24` lists the mise tasks with no `dst`.
- Who it affects: every learner who pushes the workflows in Module 12. After a failure, that learner cannot tell whose fault it is.
- Fix: run the three workflows once on a repository made from `solutions/12-ci`, and record both sweep hashes. Update the layout in `plan.md`.

#### [C.2] The lesson sections tell results, and twelve of them give no command to run

- Severity: moderate. Found by the critic. Second run: none.
- Problem 5.
- What is wrong: the lesson text narrates results that the learner can watch, and it shows almost no Rust.
- Evidence: the 14 lessons hold five Rust code blocks. The Module 3 README holds one code block, the acceptance block (`modules/03-rust-hazards/README.md:192-197`). The lesson bodies of Modules 0 to 10 and 13 hold no line with `cargo run`, `cargo test`, `mise run` or `dst`. Yet the first pass ran `vopr --seed 1` on the reference and got the line at `modules/09-vopr/README.md:62`, byte for byte.
- Who it affects: every learner on both routes.
- Fix: add a block to each lesson with one to three commands to run in `solutions/NN-slug`. Add a Rust snippet of six to ten lines for each named trap.

#### [C.3] Required reading is large, almost none states a time, and links follow moving branches

- Severity: moderate. Found by the critic. Second run: none.
- Problem 6.
- What is wrong: the course requires 76 readings, and only three of them state a time. Seven of the fourteen modules require source code in Zig, C++, Go or Clojure. Six of them, Modules 4 to 9, link that code on a moving branch.
- Evidence: the critic estimates 35 to 45 hours for the readings. The three stated times are in Module 0, for two videos and one demo (`modules/00-why-dst/README.md:352-353`, `:368`, `:379`). 24 distinct links point at `main` or `master`, and 17 of them are in required lists. One link is pinned to a commit (`modules/12-ci/README.md:239`). Two optional readings decayed. On 2026-11-10 the link to TigerBeetle's `src/io` returned 404 (`modules/02-architecture/README.md:183`). Module 10 tells the reader to search `replica.zig` for `do_view_change` (`modules/10-consensus/README.md:173`), and that string is no longer upstream. The other 39 of 40 named strings still resolve.
- Who it affects: every learner on both routes.
- Fix: pin every source link to a commit, as Module 12 does. Give each reading a time in minutes. Mark one or two readings per module as the core.

#### [C.4] A check takes over a minute by Module 13, and the README names no housekeeping

- Severity: moderate. Found by the critic. Second run: none.
- Problem 4.
- What is wrong: by Module 13 an unchanged check takes 85 seconds, and the runner has no shorter loop. `README.md` does not tell the learner to commit before a command that replaces files. The first pass timed no check, so the critic raised this.
- Evidence: the critic timed the Module 13 check under load, at a load average of 71 to 97. A first run took 413 seconds, and an unchanged re-run took 347 seconds. Later clean runs took 119.2 seconds at first and 84.9 seconds unchanged (section 10.1). The port tests in `tests/port.rs` took 19.6 of those 84.9 seconds. The build output of Module 13 is 1.6 GB. `README.md` never mentions git, yet `--from-solution` and `--force` replace files (`README.md:29`, `README.md:31`). `mise.toml:1-2` installs `node = "latest"`, and nothing uses it.
- Who it affects: every learner on both routes.
- Fix: add `dst test [file]` for one test file. Run the turmoil port tests in release or on fewer seeds. Remove node from `mise.toml`. Tell the learner in `README.md` to commit before every `dst start`.

#### [C.5] The lessons get dense where the subject is consensus, and no glossary exists

- Severity: moderate. Found by the critic. Second run: none.
- Problem 6.
- What is wrong: the critic filed the plain prose as a strength, with this caveat. This report rates the caveat. The lessons get much denser where the subject is consensus. The syllabus says that the learner needs no consensus experience (`CURRICULUM.md:21-23`).
- Evidence: Module 10 teaches the view change, the hash chain, the superblock, the read barrier, the atlas, the core and fifteen bugs in 3,522 words. It requires `The whole paper` of Raft (`modules/10-consensus/README.md:166`). No file holds a glossary, and no lesson expands the name VOPR. Module 0 alone is hard-wrapped, with bare URLs (`modules/00-why-dst/README.md:342-408`).
- Who it affects: learners who are new to consensus, on both routes.
- Fix: add a one-page glossary. In Modules 9 and 10, add a short primer on consensus, or state the prerequisite in the syllabus.

#### [8.11] Three factual slips in the syllabus and the lessons

- Severity: minor. Second run: none.
- Problem 8.
- What is wrong: three statements are inaccurate. `RandomState` takes its keys from the OS once per thread, not once per process. Elle is not the oracle that the course built: Knossos or Porcupine is the linearizability search ([6.11] in the Module 13 section). The syllabus says that a sort by pointer differs per run, and Module 3 says that such a sort survives ASLR.
- Evidence: the syllabus says `per process` (`CURRICULUM.md:216-217`), and so does Module 3 (`modules/03-rust-hazards/README.md:33`). The standard library keeps the keys in a `thread_local!` and increments one for each map. Module 13 names Elle (`modules/13-landscape/README.md:168`), and Module 8 gives the correct account (`modules/08-workloads-checking/README.md:157`). `CURRICULUM.md:234-236` and `CURRICULUM.md:334` disagree with `modules/03-rust-hazards/README.md:55`.
- Who it affects: every reader on both routes.
- Fix: write `once per thread, then incremented for each new map`. In Module 13, name Knossos or Porcupine. In the syllabus, change the hazard to a map keyed by a hash of an address.

### Where the reviewers disagreed

- Pinned hashes are called too strict ([2.1], [3.4], [4.5]) and too weak ([2.2], [3.5], [5.2]). Both hold, because the hash compares the learner's trace only with itself. The critic concluded that the weak checks need property tests, not pinned draws and not a dropped hash.
- Reviewer 8 said that the hunts and reach assertions of Modules 8 to 10 do not depend on exact seeds. [5.1] shows that the Module 10 lease hunt needs one of 24 fixed seeds. The critic kept the strength only for the hunts that need at least three catches.
- The fixes for Problem 1 conflict. [0.4] and [8.0] offer pinned draws in Module 1, [8.9] a forked stream per component, and [2.1] and [3.0] properties for every seed. The critic concluded that only properties are reachable, because [3.0] fails after a swap of two calls in Module 7.
- [7.5] calls large units and the full check justified by a cumulative project. [8.5], [1.1], [5.6] and [7.0] call the same things major defects. The critic split them by module type: large units suit Modules 4 to 10 and not the hazard content of Modules 1 and 3.
- Three strengths call the prose specification nearly complete. In the first pass, [1.3], [2.8], [4.2] and [6.6] counted about 15, 10, 25 and 22 unstated items. The second run counted about 13 for [1.3]. The critic found that Modules 4 to 8 are good and that Modules 2, 9 and 13 are poor.
- Reviewer 5 called the first failure after `dst start` small and ordered, and reviewer 6 said that Module 12 starts cleanly. [1.11], [4.7], [6.8] and [7.8] say that no cargo command works. Both hold: the workspace breaks only in the six modules that add a crate.
- Reviewer 1 called the Module 3 hunt a real first-divergence experience. The same reviewer's [1.7] says that cargo hides the divergence reports, because the tests pass. The critic concluded that the experience exists only with `--nocapture`.
- Three reviewers listed reproducible numbers as a strength of Modules 9, 10 and 12. [5.3], [8.2], [5.10] and [6.0] attack the same numbers. The critic resolved it by route: the numbers reproduce on the reference binary only.
- Severity is uneven within one class of defect: seed-specific assertions are critical in [3.0], [5.0] and [8.0], major in [5.1], and moderate in [3.6] and [6.7]. A failing seed with no name is major in [2.3] and [3.2] and moderate in [1.8], [5.7] and [7.6]. The critic reported the spread and did not resolve it. Each entry of a duplicate family carries the rating for its own module.
- The effort figures disagree, and nobody measured one. [8.3] infers 250 to 450 hours for the course, and [4.1] infers 30 to 50 hours for Module 9 alone. Neither separates the two routes. The critic expects far fewer hours for a learner who mostly reads the reference.
- Four runner defects carry two ratings, because two agents reported each one. The pairs are [7.7] and [TT.1], [7.2] and [TT.4], [7.3] and [TT.6], and [WB.6] and [TT.10]. Each entry keeps the rating of its own agent and names the other rating. No second run settled them.

## 9. What to change, in order

One decision comes before the list. The course can keep the own-code route as its main path, or it can present the reference route as the planned path. On the first choice, items 1 and 2 come first. On the second choice, item 5 and the map of [C.1] come first, and item 1 shrinks to Modules 7, 10 and 13.

The numbered items assume the first choice. Each names the smallest version that the review can propose. Where a run tested a change, the item says so.

### Start here: nine small changes

Each change is a line or two per file. Each removes a defect that a learner meets through no fault of their own.

- Add `--no-fail-fast` to the test step of every `check.sh` ([N.2], [7.0]).
- In `check`, clear `state.passed` and save the state in the failure branch at `tools/dst/src/main.rs:448` ([7.7]).
- In `copy_solution`, replace `fs::copy` at `tools/dst/src/main.rs:643` with a read and a write ([1.5]).
- Add the Module 3 version of `hazards.rs` to `modules/03-rust-hazards/manifest` ([7.1]).
- Name `shell: bash` for the nightly step in `modules/12-ci/workflows/nightly.yml`, or correct the lesson's sentence ([6.0]).
- Delete the assertion at `modules/06-sim-disk/tests/kv-env/sim_disk.rs:345`, and the same line in the later copies of that file ([3.6]).
- In `check_state` of the same file, skip a node whose `recovery()` is `None` ([3.1]). A second, rarer race stays ([N.3]).
- Print `s.crashes` in the `faults:` line of the report ([5.11]).
- Change `modules/01-determinism/README.md:137` to say that `dst next` opens Module 1 ([0.9]).

After these, do item 1 for Module 7 and for Modules 11 to 13. Those two changes remove both critical defects.

### 1. Remove the need for the reference's draws

- In Module 7, replace the four assertions of [3.0] with properties that hold for every seed. The second run tested two such properties, for lines 354 and 418, on 2,000 seeds. For line 290 it proposes a longer run or a loop over seeds. No run tested a replacement for line 177.
- In Module 13, set the fault families in the test of [6.7]. Another port test already does so.
- In Module 10, give the lease bug of [5.1] a scenario written by hand, or a hunt over a wider range. No run tested either one.
- State the start-up behaviour that the Module 10 counts of [5.1] need, or relax those counts. The lesson's own steps give 12 messages, and one test needs 14.
- Test the shrinker, the inspector and the batch runner of [5.0] without a hunt. Use a scripted oracle and a trace file that the course ships. The reference already holds such oracle tests for the shrinker.
- Do not rest those tests on a hunt. Three of five correct variations found no seed for the Module 11 bug in 20,000 tries.
- Change the shrink step of the three check scripts as well. It names seed `9916` today.
- Delete the assertion at `modules/06-sim-disk/tests/kv-env/sim_disk.rs:345`. Without it, the test passed on all three variations of [3.6].
- Move the pinned hashes into one file per workspace that no manifest lists. State in the requirements that the learner's hashes differ.
- Then repeat the test of [WB.0]: add one unused draw, and look for tests that still fail.
- Closes Problem 1: [0.4], [2.1], [3.0], [3.4], [3.6], [4.5], [5.0], [5.1], [6.7], [8.0], [WB.0].

### 2. Name the seed and show the trace on every seeded failure

- Add one guard type to the `determinism` crate, and hand it over with the Module 1 skeleton. It holds the seed. Its `Drop` prints the seed and the last 30 trace lines while the thread panics.
- The critic proposes this guard and estimates about 20 lines. No run built it.
- Use the guard in every given loop over seeds.
- Read one environment variable that selects a single seed.
- Stop the swap of the global panic hook. Catch the unwind, and let the hook print.
- In the check scripts of Modules 12 and 13, print the captured sweep output after a failure too.
- Closes the seed part of Problem 4: [1.8], [2.3], [3.2], [5.7], [7.6], and the silent failures of Modules 9 to 13.

### 3. Make the check fast and partial

- Add `--no-fail-fast` to the test step of every `check.sh`. The second run tested this on two variations, and every given test file then ran.
- Run the tests before the lint.
- Add `dst test [file]`, which runs one test file and nothing else.
- Let `dst start` write a minimal `Cargo.toml` and a one-line `lib.rs` for each new crate. The Module 13 check passes `--locked` to every cargo command, so that module also needs a fresh `Cargo.lock` ([6.8]).
- Run the slowest test files in a release build, or on fewer seeds. Section 10.1 names them.
- Run the hazard probes on a copy of `kv-core`. Today `hazard-check.sh` rewrites `kv-core/src/lib.rs`, so each check rebuilds that crate and every crate that depends on it. That rebuild costs 0.7 to 2.8 seconds, so the gain is small (section 10.1).
- Give each runner step a time limit, and print the output of a step that exceeds it.
- On a pass, print the last lines of a step whose output is short.
- Closes most of the rest of Problem 4: [1.11], [2.4], [2.9], [3.7], [4.7], [6.8], [7.0], [TT.2], [TT.3], [TT.8], [TT.10], [WB.3], [WB.6], and the workspace part of [7.8].
- It does not close [7.4] or [7.5].

### 4. Test the tests

- Hand the reference's 63 in-source unit tests to the learner as given tests. Where a test uses a private item, write it against the public API.
- Add the effect assertions that section 5 lists under Problem 2.
- Add a directory of wrong patches to the repository. Make `dst dev verify` apply each one and require a failing check. The mutants of section 5 are the first entries.
- Give each wrong patch a test that fails it. Section 6 names one for each mutant.
- Add one build with different draws, and require a passing check.
- Closes the mutants of Problem 2: [0.0], [1.2], [2.0], [2.2], [3.5], [4.0], [4.3], [4.6], [5.2], [5.5], [6.1], [6.3].
- It keeps Problem 1 closed. It does not close [8.6] or [8.8].

### 5. Repair the runner's state handling

- Clear `passed` after a failed check.
- Write file contents in `copy_solution`, as `apply_manifest` already does.
- Back up the workspace before `--from-solution`, and do not mark the skipped module as done.
- Apply manifests in order from Module 1 to the target module.
- Reject unknown flags.
- Re-issue `hazards.rs` in the Module 3 manifest, and name `trace_hash.rs` in the Module 3 lesson.
- Let `dst start 01` close Module 0, or change the first command of the Module 1 lesson.
- Make the Module 1 hazard tests independent of the clock's resolution and of thread timing. No run tested a fix.
- In the 20-run step of Module 1, print the output of the run that failed.
- Closes most of Problem 7: [0.3], [1.5], [7.1], [7.2], [7.3], [7.7], [TT.0], [TT.1], [TT.4], [TT.7], [TT.9], and the flags part of [7.10].
- It also closes the refused first command of [0.9] and [7.8]. It does not close [0.7], [7.9] or [C.1].

### 6. Let the learner find a bug before the lesson tells it

- Promote the open stretch goals to required first steps, starting with `modules/09-vopr/README.md:312`. Repair that goal first: in the review's trial its one change gave no failing seed in 10,000 ([N.7]).
- Collapse the journal table and the requirements that state the fixes.
- Add a block of commands to each lesson, so the learner sees the printed output on their own machine.
- Add the read barrier as journal entry 16, with the hand-built schedule as a given test. The second run wrote that schedule in about 60 lines on the public API. No run tested a fix of the barrier.
- Closes [8.1] and [C.2]. The other findings of Problem 5 need the steps that sections 6 and 8 give for each module.

### 7. Correct the false statements and the reference bugs

- Work through the two tables under Problem 8.
- Pin seeds `5884` and `619`, which still catch their own bugs. None of the ten pinned seeds does.
- State in the lessons what a pinned seed is worth after the next commit.
- After the fix of the oracle race, sweep at least 40,000 seeds. The second race showed only past seed 3,000.
- Closes the findings in the two tables. Sections 6 and 8 hold the smaller slips: [0.11], [4.8], [4.10], [5.10], [8.10], [8.11].

### 8. State honest sizes

- Give each module its hours, marked as an estimate.
- Split Modules 2, 9 and 10, or ship skeletons for them.
- Add the missing names and rules to the requirements. Modules 2, 9 and 13 need the most.
- Mark the requirements that the runner already meets as given.
- Give each reading a time, and pin each source link to a commit.
- Add a glossary of one page.
- Closes most of Problems 3 and 6. It does not add the small exercises of [8.5] or the second track of [8.4].

### 9. Run one trial with a person

Ask one engineer to take Modules 0 to 3 with a timer and a log of every stuck moment. This is the cheapest first test of the estimates in this report.

### What can wait

- The entries rated minor in Appendix A.
- The holes in a check that need an unlikely slip by the learner: [2.0], [2.2], [4.6] and [6.1].
- The reference bugs that no given seed reaches: [2.6], the `resume` panic of [3.1], and the second oracle race ([N.3]).
- The wider gaps of [8.8]. They are additions, not repairs.

## 10. Measurements

### 10.1 Time of one check

The times come from the unmodified reference solutions in a scratch copy, on an 8-core Apple M1. "First run" builds everything. "Unchanged re-run" repeats the check with no edit. All 26 runs passed. The load average before each run was between 2 and 13.

| Module | First run | Unchanged re-run | Tests in the workspace | Build output |
|---|---|---|---|---|
| 01 | 7.3 s | 2.0 s | 31 | 54 MB |
| 02 | 6.9 s | 1.5 s | 65 | 112 MB |
| 03 | 15.3 s | 6.2 s | 71 | 348 MB |
| 04 | 18.7 s | 9.8 s | 89 | 383 MB |
| 05 | 22.0 s | 12.1 s | 130 | 469 MB |
| 06 | 25.9 s | 16.0 s | 167 | 535 MB |
| 07 | 28.3 s | 17.8 s | 205 | 611 MB |
| 08 | 31.4 s | 19.7 s | 230 | 717 MB |
| 09 | 47.6 s | 35.1 s | 244 | 862 MB |
| 10 | 59.7 s | 44.8 s | 248 | 909 MB |
| 11 | 61.0 s | 47.3 s | 270 | 992 MB |
| 12 | 65.7 s | 52.0 s | 282 | 1.0 GB |
| 13 | 119.2 s | 84.9 s | 291 | 1.6 GB |

- Two separate runs of Modules 1, 7, 10 and 13 agreed within 5%.
- On a busy machine the checks took far longer. One unchanged Module 13 re-run took 347 seconds at a load average above 70.

Time of each step in an unchanged re-run:

| Step | Module 04 | Module 07 | Module 10 | Module 13 |
|---|---|---|---|---|
| Format | 0.1 s | 0.1 s | 0.2 s | 0.2 s |
| Lint | 0.4 s | 0.7 s | 1.0 s | 1.3 s |
| Build of the core crates with no `std` | 0.2 s | 0.3 s | 0.5 s | 0.6 s |
| Tests | 6.4 s | 12.6 s | 30.7 s | 55.9 s |
| Hazard probes on `kv-core` | 2.9 s | 3.6 s | 5.6 s | 5.9 s |
| The same run in debug and in release | none | none | none | 5.8 s |
| Sweep of 200 seeds | none | none | 6.3 s | 2.7 s |
| Shrink of the planted bug | none | none | none | 0.5 s |
| Batch of 200 seeds | none | none | none | 1.8 s |
| Port under turmoil, 20 seeds | none | none | none | 10.7 s |
| Total | 9.9 s | 17.3 s | 44.3 s | 85.4 s |

- The step times come from one more run. So the totals differ a little from the table above.
- The tests step takes most of each of these four checks. In Module 13 the slowest test files are `tests/port.rs` with 19.6 seconds and `tests/vopr.rs` with 8.1 seconds.
- An unchanged re-run still recompiles `kv-core` and every crate that depends on it. That is three crates in Module 04 and six in Module 13.
- The cause is in `hazard-check.sh`. At its end the script puts a copy of `kv-core/src/lib.rs` back, and the copy has a new file time (`modules/03-rust-hazards/scripts/hazard-check.sh:11-12`).
- That rebuild is cheap in a debug build. Cargo reported 0.7 seconds for it in Module 04 and 2.8 seconds in Module 13.
- A re-run after one touched file in `kv-core` therefore takes the same time as an unchanged re-run.

### 10.2 The Module 1 hazard tests

The check runs the hazards test binary 21 times. A failure of any run fails the check.

| Condition | Runs | Failures | Rate | Test that failed most |
|---|---|---|---|---|
| Reference, low load | 3,000 | 15 | 0.5% | `wall_clock_is_caught`, 14 times |
| Reference, under load that was mostly heavy | 19,000 | 153 | 0.81% | `completion_order_is_caught`, 148 times |

- At low load the cause is the clock. The wall clock on the test machine ticks in whole microseconds. In a probe, 97% of back-to-back reads returned the same value.
- Two runs of `wall_clock_is_caught` can therefore read the same time. The traces match, and `unwrap_err()` panics on `Ok` (`modules/01-determinism/tests/hazards.rs:39-46`).
- Under load the cause is thread timing. In 74 of the first 75 completion-order failures, all eight runs produced one fixed order.
- At 0.5% per run, about one Module 1 check in ten fails by chance.
- After a failure the script runs the tests once more and prints that run. That run usually passes, so the learner sees 13 passing tests under a `FAIL` line.

### 10.3 Assertions that depend on the seed

Each row comes from the unmodified reference with only the seed changed.

| Module | Assertion | Seeds tried | Failures |
|---|---|---|---|
| 07 | `fault_injection.rs:290`, a clock jump happened | 2,000 | 927 (46.4%) |
| 07 | `fault_injection.rs:354`, every fire is in the trace | 2,000 | 627 (31.4%) |
| 07 | `fault_injection.rs:418`, the up and down counts balance | 2,000 | 364 (18.2%) |
| 07 | `fault_injection.rs:177`, the pauses were noticed | 2,000 | 20 (1.0%) |
| 06 | `sim_disk.rs`, the chaos oracle | 3,000 | 12 (0.40%) |
| 07 | `World::resume` in the reference | 40,000 | 26 (0.065%) |
| 10 | The lease bug is caught | 240 | caught on 4 (1.7%) |

- The Module 10 lease test tries 24 seeds. At 1.7% per seed, a solution with other draws catches nothing about two times in three. The sample is small, so this is a rough estimate.
- In Module 13, 16.3% of 20,000 groups of four seeds missed one fault family at the reference's own odds.

### 10.4 Six correct variations of the reference against Modules 11 to 13

| Variation | One-line change | Changes the draws | Checks of Modules 11, 12, 13 |
|---|---|---|---|
| A | `below` by rejection of the modulo bias | Yes | All three fail |
| B | Prepares sent in descending peer order | Yes | All three fail |
| C | Fisher-Yates shuffle that walks forward | Yes | All three fail |
| D | The duplicate coin rolled before the first latency draw | Yes | All three fail |
| E | Clients attached by `id % nodes` | Yes | All three fail |
| F | Prepares pushed before the primary's own append | No | All three pass |

- All six pass every test of Modules 1 to 10, with two exceptions. All six fail pinned hashes, and B and E fail the Module 10 lease test.
- All six ran 2,000 seeds with no failure.
- With the Module 11 bug planted, a hunt over 20,000 seeds found nothing in B, C and D. It found one seed with a different assertion in A, and one usable seed in E.

### 10.5 The stale read of Module 10

| Method | Seeds | Stale reads |
|---|---|---|
| Schedule built by hand: one beat delayed by 250 ms across a view change | 1 schedule | 1, and the control without the delay passes |
| The same partitions at fixed times, latency from the model's own draws | 2,000 | 25, and 1,270 seeds did not reach the schedule's preconditions |
| Unmodified `vopr::run`, a configuration picked by hand inside the swarm's ranges | 4,600 | 11 |
| The same with 19% loss and no clock jumps | 1,500 | 0 |
| The default swarm with a read-heavy mix | 20,000 | 0 |
| The default binary | 40,000 | 0 |

### 10.6 Reading links

- 137 reading links tested. 136 resolved.
- The dead link is an optional reading: the TigerBeetle `src/io/` directory at `modules/02-architecture/README.md:183`.
- 24 links point at `main` or `master`, and 17 of them are in required lists. One link is pinned to a commit.
- An optional reading of Module 10 tells the reader to search one upstream file for `do_view_change` (`modules/10-consensus/README.md:173`). That string is no longer in the file.

## Appendix A: index of findings

The index holds 138 entries. 33 of them repeat another entry, and the column "Same defect as" names that entry. "Second part" marks the second heading of one id.

Ids with two numbers come from the first pass. [N] ids are new findings of the second run, and [N.7] is the review's own. [WA], [WB] and [TT] come from the walkthrough, and [C] comes from the critic.

| Id | Module | Severity | Second run | Problem | Finding | Same defect as | Section |
|---|---|---|---|---|---|---|---|
| [0.0] | 01 | major | held | 2 | The check passes code that breaks parts of requirements 2 to 8 | none | 6, Module 01 |
| [0.1] | 00 | moderate | held | 5,8 | The "reproduce one failure" exercise does not show that a failure is unreproducible | none | 6, Module 00 |
| [0.2] | 01 | major | none | 5 | After Modules 0 and 1 the learner saw no replay and no divergence report | none | 6, Module 01 |
| [0.3] | 01 | moderate | held | 7,4 | Hazard tests fail by chance on the reference, and the check hides it | none | 6, Module 01 |
| [0.4] | 01 | moderate | none | 1 | Module 1 leaves the range method open, and later modules depend on it | none | 6, Module 01 |
| [0.5] | 01 | moderate | held | 8 | The lesson contradicts itself on environment variables and misnames modulo bias | none | 6, Module 01 |
| [0.6] | 01 | moderate | held | 2,3 | Correct code cannot reach requirement 3's known answer through the public API | none | 6, Module 01 |
| [0.7] | 00 | moderate | none | 7 | Module 0 sits outside the runner, and `dst next` always advances | none | 6, Module 00 |
| [0.8] | 00 | moderate | none | 6 | About three hours of reading and video come before a 25-line task | none | 6, Module 00 |
| [0.9] | 01 | minor | none | 7,4 | The lesson's `dst start 01` is refused, then the check stops at a lint | [7.8] | 6, Module 01 |
| [0.10] | 01 | minor | none | 2 | Derived streams are an objective, but nothing exercises `fork` | none | 6, Module 01 |
| [0.11] | 00 | minor | none | 8 | Small factual slips in the Module 0 lesson and task | none | 6, Module 00 |
| [1.0] | 02 | moderate | held | 2,8 | The reference world keeps no trace lines, so the pinned hash covers one line | none | 6, Module 02 |
| [1.1] | 02 | major | none | 6 | About 2,000 lines sit behind one check, with no skeleton and no staged tests | none | 6, Module 02 |
| [1.2] | 02 | major | held | 2 | Thirteen separate requirement violations pass the full check | none | 6, Module 02 |
| [1.3] | 02 | moderate | held in part | 3 | The tests need about 13 names and one convention that the README omits | none | 6, Module 02 |
| [1.4] | 03 | moderate | held | 2,8 | The probe script never exercises the clippy bans, and a static counter passes | none | 6, Module 03 |
| [1.5] | 02 | major | held | 7 | `--from-solution` leaves stale builds on macOS, so cargo runs deleted code | none | 6, Module 02 |
| [1.6] | 03 | moderate | held in part | 3 | A hidden-clock helper that returns a decision or milliseconds fails the hunt | none | 6, Module 03 |
| [1.7] | 03 | moderate | held | 5,8 | The hunt's reports are hidden by default, and the pointer contrast fails on macOS | none | 6, Module 03 |
| [1.8] | 02 | moderate | none | 4 | The workload test does not name the failing seed | [2.3] | 6, Module 02 |
| [1.9] | 02 | moderate | none | 5,2 | The learner never sees the determinism check fail on their own store | none | 6, Module 02 |
| [1.10] | 03 | moderate | none | 3,2 | Part 3 lists finished work as tasks and leaves three rules unchecked | none | 6, Module 03 |
| [1.11] | 02 | moderate | none | 4 | The workspace does not load after `dst start 02` or `dst start 03` | [7.8] | 6, Module 02 |
| [2.0] | 04 | moderate | held | 2 | A heap ordered by time alone passes every Module 04 test | none | 6, Module 04 |
| [2.1] | 04 | moderate | held | 1 | Requirement 10 states a hash that only the reference produces | none | 6, Module 04 |
| [2.2] | 05 | moderate | held | 2 | Seeded partitions can cut no link, and the check still passes | none | 6, Module 05 |
| [2.3] | 05 | major | held | 4 | A failing seed is not named and no trace is shown | none | 6, Module 05 |
| [2.4] | 04 | moderate | none | 4 | No order of work, although the given tests split by crate | [7.0] | 6, Module 04 |
| [2.5] | 05 | moderate | none | 5 | The learner never watches the simulator find or replay a fault | none | 6, Module 05 |
| [2.6] | 05 | moderate | held | 8 | The reference heartbeat panics on a beat that arrives at the tick instant | none | 6, Module 05 |
| [2.7] | 04 | moderate | held | 2,8 | The world's seed, the jitter and the disk rule are not checked | none | 6, Module 04 |
| [2.8] | 05 | moderate | none | 3 | Three heartbeat tests need panic text that requirement 9 omits | none | 6, Module 05 |
| [2.9] | 04 | minor | held in part | 4,8 | A missing `advance_to` hangs one test, and the check prints nothing | none | 6, Module 04 |
| [2.10] | 04 | minor | none | 3,8 | Requirements 10 and 11 list work the runner already did | none | 6, Module 04 |
| [3.0] | 07 | critical | held | 1 | Four assertions depend on what the seed written in the test draws | none | 6, Module 07 |
| [3.1] | 06 | major | held | 8 | The given chaos oracle reports a loss while a node still reads its disk | none | 6, Module 06 |
| [3.1] | 07 | minor | held | 8 | The reference `World::resume` panics after a paused node halts in the pump | second part | 6, Module 07 |
| [3.2] | 06 | major | none | 4 | A failing chaos run names no seed and shows no trace | [2.3] | 6, Module 06 |
| [3.3] | 06 | major | none | 5 | The learner builds fault machinery and never hunts with it | none | 6, Module 06 |
| [3.4] | 07 | moderate | none | 1 | Requirements state pinned hashes that only the reference can produce | [2.1] | 6, Module 07 |
| [3.5] | 06 | moderate | held | 2 | A fault-free random disk and a byte-sum checksum both pass Module 6 | none | 6, Module 06 |
| [3.6] | 06 | moderate | held | 1 | One test pins the world's generator and its draw position | none | 6, Module 06 |
| [3.7] | 06 | moderate | none | 4 | About 1,500 lines per module sit behind one verdict | [7.0] | 6, Module 06 |
| [3.8] | 06 | moderate | none | 3 | The requirement lists record the reference, not what the tests need | none | 6, Module 06 |
| [3.9] | 07 | moderate | held | 8,3 | The cfg-in-macro warning does not apply here, and one test needs an unstated flag | none | 6, Module 07 |
| [3.10] | 06 | moderate | held | 8 | The lesson's rule that a one-bit test covers anything larger is false | none | 6, Module 06 |
| [4.0] | 09 | major | held | 2,8 | Truncation, erasure and the straggler rule have no test | none | 6, Module 09 |
| [4.1] | 09 | major | none | 6 | The module is mostly a replication protocol | none | 6, Module 09 |
| [4.2] | 09 | major | none | 3 | The tests need about 25 public items the lesson never names | none | 6, Module 09 |
| [4.3] | 09 | moderate | held | 2 | Only the linearizability check of `vopr::run` has a test | none | 6, Module 09 |
| [4.4] | 09 | moderate | none | 5 | No task asks the learner to read a failing run | none | 6, Module 09 |
| [4.5] | 09 | moderate | none | 1 | Requirements state seven hashes that only the reference produces | [2.1] | 6, Module 09 |
| [4.6] | 08 | moderate | held | 2 | Two wrong checkers pass every Module 8 test | none | 6, Module 08 |
| [4.7] | 08 | moderate | none | 4 | After `dst start`, no cargo command works until the new crate exists | [7.8] | 6, Module 08 |
| [4.8] | 09 | minor | held | 8 | The header of `replication.rs` describes the protocol before the seed `402` fix | none | 6, Module 09 |
| [4.9] | 09 | minor | held | 2,3 | The cross-process sweep test runs one seed | none | 6, Module 09 |
| [4.10] | 08 | minor | none | 8 | The lesson credits the pending-operation clause to Lowe | none | 6, Module 08 |
| [5.0] | 11 | critical | held | 1 | The checks of Modules 11 to 13 need seed `9916` to fail | none | 6, Module 11 |
| [5.1] | 10 | major | held | 1 | Tests pin the reference's random draws and message counts | none | 6, Module 10 |
| [5.2] | 10 | major | held | 2 | Removing the atlas, the `whole` rule or the core fails only a pinned hash | none | 6, Module 10 |
| [5.3] | 10 | major | held | 8 | None of the ten pinned regression seeds catches its own bug | none | 6, Module 10 |
| [5.4] | 10 | major | held | 8 | The read barrier returns a stale read on the unchanged reference | none | 6, Module 10 |
| [5.5] | 11 | major | held | 2 | No given test checks the shrinker's judgement or 19 of the 20 knobs | none | 6, Module 11 |
| [5.6] | 10 | major | held | 6 | One check covers 1,575 new lines, and the ordering hint cannot be followed | none | 6, Module 10 |
| [5.7] | 10 | moderate | none | 4 | Failing tests do not name the seed | [2.3] | 6, Module 10 |
| [5.8] | 11 | moderate | none | 8 | The main shrink removes no fault family | none | 6, Module 11 |
| [5.9] | 10 | moderate | none | 5 | Nothing asks for the bug journal or checks it | none | 6, Module 10 |
| [5.10] | 10 | minor | none | 8 | Numbers and statements disagree inside the two modules | none | 6, Module 10 |
| [5.11] | 10 | minor | held | 8 | The report prints the heal count under `crashes=` | [4.8] | 6, Module 10 |
| [6.0] | 12 | major | none | 8 | The nightly workflow hides a failing batch, and the lesson says the opposite | none | 6, Module 12 |
| [6.1] | 13 | moderate | held | 2 | The port passes with no fault applied and no log check | none | 6, Module 13 |
| [6.2] | 13 | major | none | 5 | The task states the storage fix before a seed can show the bug | none | 6, Module 13 |
| [6.3] | 12 | moderate | held | 2 | Two wrong batch implementations pass the whole check | none | 6, Module 12 |
| [6.4] | 13 | moderate | held | 2,8 | The loop cannot show the `select!` hazard, and the stretch goal is unreachable | none | 6, Module 13 |
| [6.5] | 12 | moderate | none | 3 | `dst start 12` completes requirements 12 and 13, and no step reads the workflows | none | 6, Module 12 |
| [6.6] | 13 | moderate | none | 3 | The tests need about 22 items that the lesson never names | none | 6, Module 13 |
| [6.7] | 13 | moderate | held in part | 1 | The dealt-families test needs draws that the requirements do not fix | none | 6, Module 13 |
| [6.8] | 13 | moderate | none | 4 | After `dst start 13`, no cargo command works until the new crate exists | [7.8] | 6, Module 13 |
| [6.9] | 13 | moderate | none | 2,5 | No check, file name or reminder exists for the retrospective | none | 6, Module 13 |
| [6.10] | 12 | moderate | none | 6 | No test compiles until the whole batch API exists | none | 6, Module 12 |
| [6.11] | 13 | minor | none | 8 | The lesson names Elle as the course's oracle, and the match is Knossos | [8.11] | 6, Module 13 |
| [7.0] | runner | major | none | 4 | `dst check` gives one verdict per module and no partial run | none | 7 |
| [7.1] | runner | moderate | held | 7,3 | Module 3's check rejects a Module 1 given test that no manifest re-issues | none | 7 |
| [7.2] | runner | major | none | 7 | `dst start` applies one manifest, so a redo leaves stale given tests | none | 7 |
| [7.3] | runner | major | none | 7 | `--from-solution` keeps no backup and marks the skipped module done | none | 7 |
| [7.4] | runner | moderate | none | 4 | Nothing links a failing test to a numbered requirement | none | 7 |
| [7.5] | runner | moderate | none | 4 | The quick loop, a hint command, a reset and a platform note are missing | none | 7 |
| [7.6] | runner | moderate | none | 4 | In Modules 12 and 13 a failing sweep hides the seed | none | 7 |
| [7.7] | runner | moderate | none | 7 | A failed check does not clear an earlier pass | none | 7 |
| [7.8] | runner | moderate | none | 4,7 | Module 1's first command is refused, and six modules start with a broken workspace | none | 7 |
| [7.9] | runner | moderate | none | 7 | An output line that starts with `== ` becomes a step header | none | 7 |
| [7.10] | runner | minor | none | 7 | Seven smaller runner defects | none | 7 |
| [8.0] | 12 | critical | held | 1 | Four batch tests and the shrinker step need seed `9916` to fail | [5.0] | 6, Module 12 |
| [8.0] | all | critical | held | 1 | The last three checks need seed `9916`, and Module 1 allows other draws | [5.0] | 8 |
| [8.1] | all | major | none | 5 | Every named bug is planted to a recipe or told before the task | none | 8 |
| [8.2] | all | major | held | 8 | The ten regression seeds no longer catch their bugs | [5.3] | 8 |
| [8.3] | all | major | none | 6 | The stated budget is two to three times below the first-pass estimate | none | 8 |
| [8.4] | all | major | none | 6 | In Modules 9 and 10, two thirds of the new lines are replication | [4.1] | 8 |
| [8.5] | all | major | none | 6 | Thirteen large checks, and no small exercises for the small ideas | none | 8 |
| [8.6] | all | moderate | none | 2 | Nothing checks the written work or the understanding objectives | none | 8 |
| [8.7] | all | moderate | none | 5 | Hunt, shrink and inspect arrive after the modules that need them | none | 8 |
| [8.8] | all | moderate | none | 2 | Four ideas from practice are absent or only read about | none | 8 |
| [8.9] | all | moderate | none | 8 | Module 1 teaches forked streams, and the reference world shares one stream | [5.8] | 8 |
| [8.10] | all | moderate | none | 8 | The syllabus and the plan state things that the modules do not deliver | none | 8 |
| [8.11] | all | minor | none | 8 | Three factual slips in the syllabus and the lessons | none | 8 |
| [N.1] | 02 | minor | new, second run | 4 | A zero-byte `lib.rs` fails the fmt step | none | 6, Module 02 |
| [N.2] | 04 | moderate | new, second run | 1,4 | A failing pinned hash stops every later test binary | none | 6, Module 04 |
| [N.3] | 06 | moderate | new, second run | 8 | A second oracle race survives the guard that closes [3.1] | none | 6, Module 06 |
| [N.4] | 11 | major | new, second run | 4 | Given tests silence the panic hook, so most failing tests print no message | none | 6, Module 11 |
| [N.5] | 11 | moderate | new, second run | 8 | A run of zero requests can fail, and the shrinker never tries it | none | 6, Module 11 |
| [N.6] | 13 | moderate | new, second run | 8 | The reference port draws the fault count and the latency from one value | none | 6, Module 13 |
| [N.7] | 09 | moderate | none | 8,5 | The stretch goal that promises a failing seed gave none in 10,000 seeds | none | 6, Module 09 |
| [WA.5] | 03 | moderate | none | 4 | The probe script adds sixteen clippy runs to every check from Module 3 on | none | 6, Module 03 |
| [WA.5] | 04 | minor | none | 4 | The one-year test file is the slowest test file of the check | second part | 6, Module 04 |
| [WA.9] | 00 | minor | none | 8 | The lesson's low failure rates did not reproduce | none | 6, Module 00 |
| [WB.0] | 13 | critical | held | 1 | One unused random draw fails 19 tests, and the 2,000-seed sweep still passes | [5.0] | 6, Module 13 |
| [WB.3] | 08 | moderate | none | 4 | An unchanged check runs every step again | none | 6, Module 08 |
| [WB.3] | 13 | moderate | none | 4 | Module 13 adds 33 seconds to each unchanged check | second part | 6, Module 13 |
| [WB.6] | 09 | moderate | none | 4 | On a pass, the runner hides the result line of the sweep | none | 6, Module 09 |
| [WB.6] | 11 | moderate | none | 4 | On a pass, the runner hides the shrink report | second part | 6, Module 11 |
| [TT.0] | runner | major | held | 7 | On macOS, `--from-solution` leaves stale builds | [1.5] | 7 |
| [TT.1] | runner | major | none | 7 | The walkthrough broke the code after a pass, and `dst next` still advanced | [7.7] | 7 |
| [TT.2] | runner | major | none | 4 | In Module 1 the check shows no partial progress | [7.0] | 7 |
| [TT.3] | runner | moderate | none | 4 | A given test for a crate that does not exist yet breaks the workspace | [7.8] | 7 |
| [TT.4] | runner | moderate | none | 7 | Going back and coming forward leave the wrong files in place | [7.2] | 7 |
| [TT.5] | runner | moderate | none | 1,7 | An edited given test is not noticed, and `dst start` overwrites it | none | 7 |
| [TT.6] | runner | moderate | none | 7 | After `--from-solution`, the learner's extra files fail the next check | [7.3] | 7 |
| [TT.7] | runner | moderate | none | 7 | The first command of the Module 1 lesson ends in advice that loops | [7.8] | 7 |
| [TT.8] | runner | moderate | none | 7,4 | The hazards step rewrites `kv-core/src/lib.rs` while the learner can edit it | none | 7 |
| [TT.9] | runner | moderate | held | 7 | The twenty-run step of Module 1 prints a passing re-run after a failure | [0.3] | 7 |
| [TT.10] | runner | minor | none | 4 | A passing step hides the result lines that the lessons are about | [WB.6] | 7 |
| [TT.11] | runner | minor | none | 7 | Six rough edges in parsing and messages | [7.9], [7.10] | 7 |
| [C.1] | all | major | none | 7 | The reference route has no map of the code it hands over | none | 8 |
| [C.2] | all | moderate | none | 5 | The lesson sections tell results, and twelve of them give no command to run | none | 8 |
| [C.3] | all | moderate | none | 6 | Required reading is large, almost none states a time, and links follow moving branches | none | 8 |
| [C.4] | all | moderate | none | 4 | A check takes over a minute by Module 13, and the README names no housekeeping | none | 8 |
| [C.5] | all | moderate | none | 6 | The lessons get dense where the subject is consensus, and no glossary exists | none | 8 |
