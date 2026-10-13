# Retrospective: what the simulator models, and what it does not

This is the reference retrospective for Module 13. It is about the simulator in this workspace, built in Modules 1 to 12, as it stands after the turmoil port. It is honest about the gaps, because a reader who trusts the simulator more than it deserves is the worst outcome of the course.

## 1. What the simulator models

| Model | How | Decision and module |
|---|---|---|
| Time | One discrete-event scheduler with `(Instant, seq)` as the heap key. The core has its own `Instant(u64)`. Each host reads a `HostClock` with an offset, a drift of up to 500 000 ppm, and forward jumps. | Pure event queue, not ticks (Module 4). One monotonic clock per host, skewed (Module 7). |
| Network | One address space for nodes and clients. A one-way latency drawn uniformly from a range. Per message: loss, a duplicate, a reordering delay. Partitions as cut links in shapes, checked at send and at delivery. | No queue in the network, delivery instants are events (Module 5). |
| Disk | Whole sectors with CRC-32C and sequence numbers. Five faults: a lost write, a torn write, a misdirected write, a corrupted write, a corrupted read. A crash lands in-flight writes in order and loses everything after the first torn one. A fault atlas keeps every record intact on at least one disk of three or more. | Crash tearing in order (Module 6). The atlas (Module 10). |
| Hosts | A disk, a clock and an optional process. A crash drops the process. A pause stops driving it and queues its inputs. A slow host has a slow disk. A restart gets a larger boot id. | Resume order: tick first (Module 7). Boot ids (Module 9). |
| The code | Fault points behind `buggify!`, four sites, drawn per run. Six planted bugs behind a feature. | `Chance` in `determinism` (Module 7). Planted bugs (Module 8). |
| The run | Eight fault families drawn from the seed, a workload drawn from the seed, a chaos phase, `stop_faults`, convergence, a quiet phase. A core of `f + 1` replicas that is spared. | `RunConfig::from_seed` (Module 9). The core (Module 10). |
| The oracle | A history with recording-order precedence. Linearizability per key by Wing and Gong's search with Lowe's cache, under a budget. Committed prefixes of every pair of logs agree. The map is the log. A quiet request is always answered. | Pending operations land or not (Module 8). Committed prefixes, not whole logs (Module 10). |
| The tools | A trace with a hash. A hunt, a shrinker over the configuration, an inspector over the trace. A batch with seeds from the commit, two reruns per failure, and a cost ceiling. | Children, not unwinding (Module 11). Nothing is retried into a pass (Module 12). |

## 2. What each model leaves out

| Model | What it leaves out | What covers it |
|---|---|---|
| Time | A clock that steps backward. A wall clock. The time a step of the replica takes. | Assertions in production. A benchmark, because no trace hash sees a slow step. |
| Network | TCP: connections, resets, a half-open socket, a port reused by another process. A packet larger than the path allows. A message delivered to a host that just restarted and holds the same port. | A real cluster under Jepsen-style faults. A fuzzer on the wire format, which did not exist before Module 13. |
| Disk | An `fsync` that lies. A filesystem that reorders metadata. A sector that reads differently twice. A disk that is full. A write that tears inside a sector in the real pattern of the device. | Running the replica on the real file, under power loss, with a checker on the log. |
| Hosts | Memory: an allocation that fails, a bit flipped in RAM, an out-of-memory kill. A process that is slow in CPU rather than in I/O. | Assertions in production. Chaos on real machines. |
| The code | Any path without a fault point. The real environment, `kv_env::real`, which the simulator never runs. | A Jepsen-style run. A hypervisor such as Antithesis, which runs the real binary. |
| The run | States the distributions do not reach. A view change with no fault, which the swarm reaches thousands of times and no oracle names. | Reading traces. New oracles. A TLA+ model that explores a small cluster exhaustively. |
| The oracle | A wrong model of the store. A guard that is too strict, as in seeds 13 and 19 of Module 10, or too loose, which no seed ever shows. | The planted bugs, which check only for the bugs someone thought to plant. A second oracle, such as Elle on a real history. |
| The tools | The binary production runs. The compiler's correctness, as against its determinism. | Reproducible builds. Two compilers. |

The real environment deserves its own line. `real.rs` is a few hundred lines that production runs and the simulator never does. Module 13 found a bug in it that erased the whole log on every superblock write. The simulator ran twenty thousand seeds against the fake disk, which writes in place. The restart test of Module 2 ran a cluster of one, which writes its superblock once and first. Nothing in the course tested a second superblock write on a real file until the turmoil port's storage copied the real one.

## 3. The journal

Every bug the simulator found, from the first module whose seeds found one.

- Module 6, seed 14 of the storage chaos test: a node acknowledged a write, restarted, served the old value, restarted again, and served the new one. The record was there the whole time. A bit flipped on the first read, and recovery took the record for a torn tail. The node had no memory of how long its log was.
- Module 9, seed 36: a replica with a hole erased from the hole on at once, and the cluster lost fourteen committed writes. Repair by salvage and fetch, and erase only once every peer says it has no copy.
- Module 9, seed 402: two logs that disagreed were not told apart. Records carry the view they were prepared in, beats carry the log as runs, and the newest log wins at the first disagreement.
- Module 9, seed 4668: a restarted node applied a beat from before its restart. Boot ids on every beat and acknowledgement.
- Module 10, fifteen entries, in the lesson's table. Eleven were the protocol's: seeds 7, 22, 24, 36, 525, 562, 563, 7775, 5884, and two found by hand. Two were the guard's: seeds 13 and 19, where the check was wrong and the protocol right. Two were the model's: seed 619, where two disks lost the same record, beyond `f`, and seed 353, where a safe rule starved the cluster for 950 seconds.
- Module 11, seed 9916: the planted `ForgottenPromise`, found by the hunt, shrunk from 150 to 57 requests in 26 runs.
- Module 13, seeds 34 and 42 of the port: a bounce that lost records from a disk nothing tore. The bug was in `FileStorage`, in the real environment.
- Module 13, seed 3 of the port: five view changes with no fault. A first beat that arrives after the view it belongs to is read as a restart. The VOPR reaches the same state on a quarter of its seeds with only the network on, and has no oracle for it.

Three classes, as Module 10 named them. The protocol's bugs are what the simulator is for. The guard's bugs say the oracle was doubted, which is healthy. The model's bugs say the simulator dealt a fault no protocol tolerates, or that a rule meant to be safe was not live. A journal with only the first class means the checker and the model were never doubted. This one has all three, and Module 13 added a fourth: a bug outside the model, in the code the simulator never runs.

## 4. What the port found

The port is three trait implementations, a wire format and a loop, 1 400 lines with the tests, and `kv-core` did not change. That is the dividend of Module 2's decision to make the environment three traits and the core a state machine. The oracle crossed over without a change, which is the dividend of Module 8's decision to keep it `no_std`.

turmoil found nothing in the replica that the VOPR had not reached. It found the bug in `FileStorage` because the port copied the real storage. That finding says the simulator's coverage ends at the trait boundary. It showed the view change at a clean start in a trace that someone read. That finding says the swarm reaches more than the oracles name. `Bug::StaleReadAfterStepDown` is caught on nine of the first twelve seeds, with the same message, by the same checker.

What the port lacks is the list of this course's modules. No disk faults, no clock skew, no pauses, no fault points, no shrinker, no inspector, no batch, and no cost in the seed's units. What it has is a real async runtime, TCP and DNS, and nothing of its own to maintain. The comparison is in the lesson.

## 5. The numbers

| | VOPR | turmoil port |
|---|---|---|
| Seeds per second, one core, release | about 38 at three hundred requests, about 90 at one hundred and fifty | about 2.7 at three hundred requests |
| Virtual seconds per wall second | about 270 | about 73 |
| Cost of a seed, in the seed's units | about 10 scheduler events per request, median 18, maximum 427 | about 90 ticks per request |
| Cost ceiling | 1 000 events per request, 800 000 checker steps | none |
| Twenty thousand seeds | 96 s at eight jobs, 3 m 46 s in one process | not run |
| One seed in debug | under 50 ms | 2.4 s to 6 s |

The batch in `scripts/check.sh` runs two hundred seeds at four jobs at about 150 seeds a second. The port's twenty seeds in the same script take about eight seconds.

## 6. What to do differently next time

- Give the protocol a wire format in Module 9, the module that made the network two-sided. A protocol that lives only as Rust values is not finished, and the fuzzer that covers the parser cannot start until the parser exists.
- Test the real environment with the fake world's workload. `kv_env::real` has one integration test. A test that drives `FileStorage` and `TcpNetwork` through the same `Driver` with the Module 8 workload is a hundred lines. On a real file and a real socket, it finds the superblock bug at the first view change.
- Add the oracle for a run with no faults in Module 9: after the first `Primary` status, no view change. The spurious view change at a clean start is a liveness cost that the swarm dealt on a quarter of its seeds and nothing named.
- Put the primary's boot id in `StartView`, so that a backup can tell a first beat that arrived late from a restart. Beats carry the boot id already. When the rule in `replica.rs` has a boot id, it compares that and not the status.
- Decide the fingerprint policy for floating point. The port's latency draws go through `ln` in a crate the course does not control, and the sweep hash across two operating systems is untested. The VOPR has no floating point anywhere, by lint and by `no_std`, and that is the policy to keep.
- Keep the one address space of Module 5 as a known shortcut, and write down what it hides: a reply that finds its client because the world knows every address, where a real primary has to be told.
