# Module 10: Consensus under simulation

## Why this module exists

Module 9 built a replicated store that was safe and not live. A write was answered once every disk had it, and a new primary served once every replica reported. One replica away stopped the cluster. This module pays for liveness with a majority, and finds out what a majority costs. The commit rule becomes "`f + 1` of `2f + 1`". The gather becomes a view change. The repair becomes a protocol of its own, with a hash chain and a fault atlas. Two things the earlier modules put off come due: a superblock, and a reason why every replica's disk is not the same thing as the cluster's log.

The simulator is the development tool here, not the afterthought. The protocol was written against it from the first line. Fifteen bugs went into the journal before the module was done, thirteen found by the sweep and two by hand. Each one is in the journal below, and each seed is a regression test. That journal is the real content of this module. The protocol itself is a few hundred lines. Knowing why each line is there is the work.

## Objectives

- Implement Viewstamped Replication over the Module 9 replica: majority commit, the three-message view change, and a new primary that serves once a majority holds its log.
- Make a log a hash chain. A replica then knows whether an offered record belongs at the end of its log, without asking.
- Put the view promise on disk before it is spoken, and know what else a restart must remember.
- Answer reads without a lease, and plant a lease to see what a slow clock does to one.
- Add a fault atlas to the disk model, and say why a model that deals more than `f` faults per record is testing nothing.
- Run TigerBeetle's liveness mode: a core that never breaks, whose clients must be answered throughout the chaos.
- Keep a bug journal, and read the three classes of bug it holds: the protocol's, the guard's, and the model's.

## Lesson

### Two of three

A cluster of `n = 2f + 1` replicas tolerates `f` of them being away. Three replicas tolerate one. The arithmetic is the whole protocol. A write is committed once `f + 1` disks have it, the primary's among them. A view change needs `f + 1` replicas to agree the old primary is gone, and `f + 1` logs to choose the new one from. Any two majorities share a replica, so a committed record is always in one of the logs a new primary sees.

That sentence hides every bug in this module. "Always in one of the logs" assumes the replica that has the record still has it, still knows it has it, and says so. A disk that lost a write, a replica that forgot a vote, and a beat that arrived late each break one of those assumptions. The journal is a list of the ways.

Module 9's replica already had the shapes. Views, the primary of view `v` as `v mod n`, records stamped with their view, logs summed up as runs, and the newest-log rule. Those are the shared core of Raft and Viewstamped Replication. What changes is the counting, and what the counting makes necessary.

### Choosing the protocol

Raft or Viewstamped Replication. Raft has more implementations to compare against. VSR is what TigerBeetle runs, and it fits this code better. The primary of a view is fixed by arithmetic, so there is no election to lose. The view change is a round of messages rather than a vote. Module 9's replica was already a VSR replica with a stricter quorum. The module keeps it.

Two Raft ideas are borrowed anyway. The consistency check on a record arriving at a backup is Raft's, done with a checksum instead of a term. And the view promise on disk is Raft's `currentTerm`. VSR as published keeps the view in memory and recovers it from the others. Seed 562 is why this module does not.

### The view change

Three messages, from Liskov and Cowling's paper, section 4.2.

1. A backup whose detector marks the primary `Down` moves to the next view whose primary it believes alive, and sends `StartViewChange { view }` to everyone. A replica that hears a higher view joins it. A primary that hears one steps down.
2. A replica with `f` such messages from others sends `DoViewChange` to the new primary: its log as runs, its commit number, the last view it was normal in, and whether its log is whole.
3. The new primary, with `f` offers plus its own, picks the log from the highest normal view, and among equals the newest by runs. If that log is not its own, it truncates what disagrees and pulls the rest from its holder. Then it sends `StartView` with the chosen runs, and serves once `f + 1` disks hold the whole log in the new view.

The last clause is the module's own. The published protocol lets the new primary serve at once and commit the tail of the chosen log on the way. This replica builds its node from the whole log as a committed thing, so the tail must be committed first. A client whose write is in that tail gets its answer from the session the record made.

Between views a replica is in `ViewChange`. Nothing it says counts. Its durable count is not of the new view's log. Its offer is for the primary only. Its beat is a vote, and to the new primary an offer, and nothing more. The new primary counts a backup's disk only once the backup reports itself a backup of the view.

```text
  n1 detector: n0 Down          n2 detector: n0 Down
      │ StartViewChange 1            │ StartViewChange 1
      ▼                              ▼
  n1 has f = 1 from n2 ──DoViewChange 1──► n1 is primary of 1
  n2 has f = 1 from n1 ──DoViewChange 1──► n1 (its own offer)
                                           picks the newest log
                                           pulls if it is not its own
                                           StartView 1 to all
  n2 takes the view's log, truncates, writes its superblock, reports Backup
  n1 serves once n2's durable count reaches its length
```

### The normal view, and who is whole

An offer carries `log_view`, the last view the sender was normal in. A replica is normal in a view once it holds the view's log, the one the primary vouched for. From then on it appends only what that primary sends. So a log from a higher normal view contains everything a lower one committed. That is the induction the view change rests on. The first word, below, is the first message a backup hears from the view's primary.

Seed 7 found the induction broken. A backup took view 17 on the primary's `StartView`, never received the records, and offered its short log as a view-17 log. The next primary preferred it to a longer log from view 15 and tried to drop six committed records. The guard caught it. The rule now has two parts. A backup claims a view once its log is as long as the primary's was at the first word. It also claims it once it holds any record prepared in the view. Seed 22 found the first version of that rule. It compared against a length that grew with every append, so a backup one record behind never claimed anything.

A restarted replica remembers nothing but its disk. Its claim is the view of its last record, which is never too high. Seed 36 made that claim before a repair put the records beyond a hole back. It offered ninety-two records under the view of the twenty-four before the hole. The claim is made again once the log is whole.

`whole` is the other field. It says the log lost nothing since a primary last vouched for it. A replica that restarted cannot tell a record it never had from one its disk acknowledged and then lost. Seed 5884 started a view from two such logs while the one replica that had the last two committed records was cut off. Nothing was heard about them, so no commit number helped. A view now starts only with `f + 1` whole logs, or with every log in the cluster. The second clause is the escape: after a total restart nobody is whole, and the newest of all the logs is the truth. This is the distinction the protocol-aware recovery paper draws between a replica that never had an entry and one that lost it.

### The chain

Every record carries the checksum of the record before it, in a header field. A log is a hash chain, and a record pins its whole prefix. A record is appended on one condition: its parent is the checksum of the last record in the log. The replica asserts it.

That one field does three jobs. A backup needs no `prevLogTerm`: a prepare that does not chain is ignored, and the primary's next beat says how far behind the backup is. A repair needs no judge. A record kept from beyond a hole, or fetched from a peer's own hole, is written back once its named parent is in place. A stray that names a record this log no longer has is dropped. And Module 9's `Truncate` message is gone. A backup compares its runs with the primary's at every beat and cuts its own tail at the first disagreement.

Seed 24 found what a beat is allowed to say. A beat from the primary arrived a hundred and thirty milliseconds late. Its log was three records shorter than the prepares the backup accepted since, and the backup cut to match. Within a view the primary only appends, so a shorter beat is older news. After the first word from a primary, a backup cuts only on a real disagreement: both logs have the record, in different views.

Truncation never goes below the commit number, and `truncate` asserts it. Five of the fifteen bugs fired that assertion. Three more fired its twin in the view change, which checks the chosen log against the commit number. It is the most useful line in the module. Seed 13 found it wrong once. A stepped-down primary adopted the new primary's commit number before dropping its own uncommitted tail. The guard compared record 7 against a commit that belonged to another log. Drop the tail first.

### The promise

Seed 562, with the fault model at full strength. Node 0 voted for view 71, crashed, restarted with no memory of it, found node 1 still serving view 70, and acknowledged two records there. Two views each had a majority. Later the legitimate view 71 log beat the view 70 log, and node 1 was told to drop a record it had committed.

Raft writes the term to disk before it votes. This module adds a superblock: four one-sector copies ahead of the log, written in turn, with a sequence number and a checksum. It holds the highest view the replica agreed to, the last view it was normal in, and the log's length at that claim. A replica writes it before it sends `StartViewChange`, and reports itself a backup only once the write is durable. The disk completes writes in order, so any acknowledgement for a record appended after the promise implies it.

Four copies cost four sectors. A write that tears, or a sector that rots, loses one copy. The newest copy that checks out is the truth, and it is at most one write old. TigerBeetle reads a quorum of its copies. This module does not. A read fault on every copy at once is left as an exercise in arithmetic.

The superblock also holds `log_view` and its length. A restarted replica claims the view it was normal in rather than the view of its last record. The claim holds on one condition: the log is at least as long as it was at the claim. A shorter log lost records, and the claim is void.

### Reads

Module 9's primary answered reads from its map. With every replica in every quorum that was safe. With a majority it is not: a primary cut off from the others keeps a map, and the others elect a replacement that commits writes the map never sees.

A read waits. At the read's arrival the primary notes each peer's last beat. It answers once `f` peers sent a beat in this view since. Those beats say a majority still follows this primary, so nothing newer than its map was committed anywhere. Raft calls this a read index. It costs up to one heartbeat interval of latency, and nothing in correctness.

A lease is the cheaper answer: "I heard from a majority a moment ago, so I am still the primary." The planted bug `Bug::LeaseRead` answers at once on a lease: two detector timeouts since a majority last beat, by the primary's own clock. Under the Module 7 clock model, drift reaches fifty per cent. The others elect a new primary after one timeout by their clocks. The old one holds its lease by its own. The hunt in `vopr.rs` finds it, and needs runs five times the hunt's usual hundred and twenty requests to see enough partitions.

### Repair and the atlas

A damaged log keeps what it finds beyond the hole and asks every peer for the record at the hole. Any answer is checked against the chain. Once every peer says `Missing`, the hole is erased with what follows it.

Seed 619 erased two committed records. Record 42 was committed on nodes 0 and 1. All three crashed within two hundred milliseconds. Nodes 0 and 1 each came back with a hole at 41. Each disk lost that write after acknowledging it. Node 2 was down while 41 and 42 were prepared. Every peer said `Missing`, both erased, and every read after that returned the value before the write.

Two disks lost the same record. That is two faults against a tolerance of one, and no protocol survives it. The fix is in the model, not the protocol. TigerBeetle's simulator has a `ClusterFaultAtlas`: faults are dealt so that at least one replica keeps a valid copy of every record. This module's `sim::Atlas` is asked before every fault the disk rolled, for every sector the fault touches. It vetoes a fault that puts the same sector at fault on more than `f` disks. A sector is at fault until its disk writes it again. The dice are rolled either way, so a seed means the same thing with or without the atlas.

Under the atlas, a `Missing` from any peer that has read its disk is the truth about that disk. A committed record is intact on all but `f` of the disks that have it, and a hole every peer calls missing was never committed. Seed 353 found the first version of this rule too strict. It counted a `Missing` only from a whole peer. A peer that restarted was never whole again without a primary. No primary was possible without the repairing replica's offer. One thousand two hundred and seventy-one views in nine hundred and fifty seconds, and nothing served. That is the livelock the TigerBeetle liveness post describes, in miniature.

### The core

A liveness check after the faults stop says the cluster recovers. It says nothing about whether the cluster serves during the faults, and a protocol that stalls under every partition passes it. TigerBeetle's liveness mode picks a core: a majority of replicas that are kept up and connected, while everything else is broken as hard as the seed likes. The core must make progress.

The seed draws a core one time in two, after every other draw, so that earlier seeds keep their meaning. Crashes and pauses skip the core's hosts, and partitions heal the links among them. Every request from a client of a core node must complete during the chaos. Twelve seeds forced crashes and partitions outside a core of two. They served over seven hundred requests between them, and no core client gave up.

The seed also draws five nodes one time in four. Nothing in the protocol is written for three.

### The journal

| Seed | What was seen | What was wrong | What changed |
|---|---|---|---|
| hand | Backups emptied their logs at a restart of the primary | The restarted primary's first beat, sent before its disk was read, said view 0 and an empty log, and the backups followed it | Beats carry the sender's status, and only a serving primary's log is followed |
| 7 | Told to truncate record 88, committed through 93 | A backup took the view before its log and offered a short log under the view's number | A backup claims a view once it holds the view's log |
| 13 | Told to truncate record 7, committed through 10 | The guard's own bug: the commit number adopted before the old tail was dropped | Drop first, then learn the commit |
| 19 | Durable count one behind the log forever | A truncation marked every pending append stale, including those below the cut | Only appends at or past the cut are stale |
| 22 | The chosen log has 50 records, 51 are committed | The catch-up target moved with every append, so a backup one behind never claimed the view | The target is the log's length at the first word, or any record of the view |
| 24 | Told to truncate record 33, committed through 33 | A beat reordered by 130 ms carried a shorter log than the prepares since | After the first word, cut only on a disagreement |
| 36 | The chosen log has 79 records, 89 are committed | A repaired log offered under the view of the records before the hole | Claim again once the log is whole |
| 525 | Told to truncate record 112, committed through 112 | A disk lost two acknowledged records, and a view started without them while the one replica that had them was slow | A view never starts with a log shorter than any commit heard, and a beat between views is an offer |
| 562 | Told to truncate record 27, committed through 27 | A vote forgotten at a restart, and two majorities | The superblock |
| 563 | Three records committed on one disk | An old durable count from before the view carried into it | An acknowledgement from a new backup sets the count, never raises it |
| 7775 | The node's pending writes out of order | A cut-off primary served a client that another primary had answered | A newer request while a write is pending is dropped as superseded |
| hand | View 1 ten milliseconds into a clean start | The restart rule fired on a primary's first beat | The rule applies only to a backup already following |
| 5884 | The chosen log has 35 records, 36 are committed | A view started from two logs that had lost or never had a record, with nothing heard | `whole`, and a majority of whole logs or every log |
| 619 | Every read after 28.6 s returned the value before an acknowledged write | Two disks lost the same record: beyond `f` | The fault atlas, and `Missing` counted from every peer |
| 353 | 1271 views in 950 s, nothing served | `Missing` counted only from whole peers, and no peer became whole again | `Missing` from any peer that read its disk |

Three classes. The protocol's: seeds 7, 22, 24, 36, 525, 562, 563, 7775, 5884, and the two found by hand. The guard's: seeds 13 and 19, where the check was wrong and the protocol was right. The model's: seeds 619 and 353, where the simulator dealt faults no protocol tolerates, and where a rule meant to be safe starved the cluster. A journal with only the first class means the checker and the model were never doubted.

### Reading a failing seed

The first word of the assertion says which. `safety:` is a wrong answer or a lost record, and the history or the two logs are printed. `liveness:` is a cluster that did not converge or a core client that gave up. Every replica's status, view, length and durable count are printed. The guard inside `truncate` says which record and which commit number. From there the trace, with `--trace`, and a search for the view in the message. Every seed in the journal was read that way, in a few minutes each. The trace has every message and every status change on one line.

### What the swarm reaches

Forty seeds of one hundred requests: 91 promotions, every seed with at least one, 295 pulls, 5 repairs, 51 step-downs, 9 truncations and 2433 reads through the barrier. Seventeen of the forty had a core and three had five nodes. Twenty thousand seeds of one hundred and fifty requests run clean in three minutes and forty-six seconds, about ninety seeds a second. Five-node seeds and the atlas cost about a tenth against Module 9.

### How TigerBeetle does it

`src/vsr/replica.zig` is twelve thousand lines, and the shapes are the same. `do_view_change` carries `log_view` and the log's headers. The new primary needs a quorum of them. It truncates an op only once enough replicas say they never had it, which the code calls a nack quorum. `src/vsr/superblock.zig` keeps `view` and `log_view`, written as four, six or eight copies with a quorum read. The headers carry a `parent` checksum, and the journal is repaired op by op along it. `src/testing/storage.zig` defines `ClusterFaultAtlas` and says what it is for: at least one replica will have a valid copy to help the others repair. The liveness post tells the view-change livelock that only a core found.

### Decisions recorded

- Viewstamped Replication, with Raft's persisted term and Raft's consistency check done by checksum. The primary of a view is `v mod n`.
- A new primary serves once `f + 1` disks hold its whole log in its view. The node is built from a committed log.
- No `Truncate` message. A backup cuts its own tail against the primary's runs, on the first word in full, afterwards only on a disagreement.
- Four superblock copies, no quorum read. The log begins at sector four.
- Reads wait for `f` peers' beats. No lease.
- The atlas is a cluster of three or more. One or two disks get every fault they roll, so Modules 6 to 8 keep their tests.
- A repairing replica does not offer. A view starts with `f + 1` whole logs or every log.
- The seed draws five nodes one time in four and a core one time in two, last.

## Readings

Required:

- Liskov and Cowling, ["Viewstamped Replication Revisited"](https://www.cs.princeton.edu/courses/archive/fall19/cos418/papers/vr-revisited.pdf) (2012). Sections 4.1 and 4.2 are the normal case and the view change this module implements. Section 4.3 is the recovery protocol this module replaces with a superblock, and section 5 says what disks change.
- Ongaro and Ousterhout, ["In Search of an Understandable Consensus Algorithm"](https://raft.github.io/raft.pdf) (2014). The whole paper now. Section 5.4 is the election restriction and why a leader never commits an old term's entry by counting, which is the problem `log_view` solves. Figure 8 is seed 5884 drawn by hand.
- Alagappan, Ganesan, Lee, Albarghouthi, Chidambaram, Arpaci-Dusseau and Arpaci-Dusseau, ["Protocol-Aware Recovery for Consensus-Based Storage"](https://www.usenix.org/system/files/conference/fast18/fast18-alagappan.pdf) (FAST 2018). The distributed log recovery section: how a replica that lost an entry says so, and why a quorum must know the difference. The `whole` flag is a small version.
- matklad, ["Simulation Testing For Liveness"](https://tigerbeetle.com/blog/2023-07-06-simulation-testing-for-liveness/) (TigerBeetle, 2023). The core, and the livelock that a safety check never sees. Read it again next to seed 353.

Optional:

- TigerBeetle: [`src/vsr/superblock.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/superblock.zig). The header, `view` and `log_view`, and the quorum of copies.
- TigerBeetle: [`src/vsr/replica.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/replica.zig). Search for `do_view_change` and `nack`. The code is long, and the comments above each are worth the search.
- TigerBeetle: [`src/testing/storage.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/testing/storage.zig). The header comment, and `ClusterFaultAtlas` near the end.

## Coding task

### Starting point

Start from your Module 9 workspace with `dst start 10`. If you did not finish Module 9, `dst start 10 --from-solution` starts from `solutions/09-vopr/` instead. The command adds `consensus.rs` to `crates/kv-env/tests/` and deletes `replication.rs`, which it takes the place of. The command replaces `vopr.rs` in `crates/vopr/tests/`, the six tests of Modules 4 to 9 in `crates/kv-env/tests/`, and `log.rs` in `crates/kv-core/tests/`. The superblock moved the log, the view change put messages on the wire, and every fingerprint changed. `scripts/check.sh` is unchanged.

### Requirements

#### The log and the superblock

1. `log::HEADER_LEN` is 28. The header gains `parent`, the checksum of the record before. `Record::encode(seq, parent, out)` and `encode_in_view(seq, view, parent, out)` return the record's checksum. `log::Header { seq, view, checksum, parent }`, `record_header(bytes) -> Option<Header>`, `salvage -> Vec<(Header, bytes)>`, and `Recovered` gains `checksums`. Recovery stops at a record whose parent is not the previous record's checksum, and reports damage.
2. `kv_core::superblock`: `Superblock { seq, view, log_view, log_view_len }`, `COPIES` of four, `LOG_OFFSET`, `encode`, `decode`, `read(disk)` for the newest copy that checks out, and `log_bytes(disk)`. Each write goes to copy `seq mod COPIES`.
3. The node chains its own records. `Node` recovers the last checksum and uses it as the parent of its next append.

#### The replica

4. `Message` is `Beat { boot, seq, view, status, log_view, runs, top, durable, commit, whole }`, `StartViewChange { view }`, `DoViewChange { view, log_view, runs, commit, whole }`, `StartView { view, runs, commit }`, `Prepare { view, seq, commit, bytes }`, `PrepareOk`, `Pull`, `Forward`, `Fetch` and `Missing { view, seq }`. `Status` gains `ViewChange`. The queries gain `log_view`, `commit`, `promised`, `superblock`, `is_whole`, `chain`, `pending_reads` and `f`.
5. The commit rule. The primary commits a record once the `(f + 1)`-th largest durable count reaches it, its own among them. Only peers that report themselves backups of this view count. A `PrepareOk` from a peer not yet a backup of the view sets its count. `commit` is carried in beats and prepares, and a backup takes the maximum.
6. The view change, as the lesson's three steps. `fail_over` picks the next view whose primary is not `Down`. A view change in progress times out after `suspect_after` of silence from what it waits on. A beat from a peer in `ViewChange` for the primary's view is that peer's offer. The new primary waits for a log at least as long as every commit number it ever heard. It also waits for `f + 1` whole offers, or an offer from every peer.
7. The promise. `start_view_change` writes the superblock before any message. `send_view_change` and `become_backup` wait for `promised >= view`. A restarted replica starts at the superblock's view. With a log at least `log_view_len` long it claims the superblock's `log_view`, otherwise the view of its last record. It claims again once the log is whole.
8. The chain in use. A prepare is accepted on two conditions: `seq == len + 1` and `parent == chain()`. A backup truncates itself against the primary's runs, in full on the first word, on a disagreement afterwards, and never below `commit`. A repair writes a retained record that chains and drops one that does not as a stray. It fetches at a hole, and erases once every peer answered `Missing`.
9. `note_caught_up`: a backup claims the view once `len >= start_len` or its last record is of the view, sets `whole`, and writes the superblock.
10. Reads. A `Get` at the primary waits until `f` peers have beaten in this view since it arrived, up to `READS_MAX` waiting. `Bug::LeaseRead` as a fifth planted bug answers at once within two `suspect_after` of a majority's last beat.
11. A new request from a client whose write is pending is superseded: the node drops it and counts it. A backup that hears its view's primary beat with `Reading`, `Joining`, `Repairing` or `Erasing` status fails over.
12. Invariants after every input, as before, plus four. A checksum per record. Views never decrease along the log. A backup and a primary hold a promise for their view. A primary's node is at `commit`.

#### The environment and the model

13. `sim::Atlas` with `allow(disk, first, count)` and `clear(disk, first, count)`. `Disk::set_atlas(id, atlas)`. Lost, misdirected, rotten and unreadable sectors ask the atlas after the roll. `DiskStats::vetoed`. `kv_env::fake::ClusterAtlas` with `f`, attached to every disk of a world with three or more hosts. `World::log_on_disk(node)`.
14. `WorldConfig::core`. Crashes and pauses skip the core, partitions heal the links among it, and the configuration line shows it.
15. `RunConfig::from_seed` draws `nodes` as five one time in four and a core one time in two, after every other draw. `run` asserts no core client gave up during the chaos. The disk check uses `log_on_disk`.
16. The fingerprints. Seed 1 of `RunConfig::from_seed(1, 150)` is `0xcc45_30a4_8c93_496b`. The five ported `kv-env` tests carry their own, and every one of them changed.

### Acceptance criteria

```sh
scripts/check.sh                                               # ends with "all checks passed"
cargo test -p vopr --release -- --nocapture the_swarm_reaches  # prints the counts
cargo test -p vopr --release -- --nocapture a_lease            # prints the seed it caught
cargo run --release -p vopr -- --seeds 0..20000 | tail -1      # one trace_hash line, in about four minutes
```

Ten seeds of the journal are in `regression_seeds_still_pass`, under three nodes and no core, because they were found before those draws existed. Module 9's four regression seeds are retired. Every fingerprint changed, so those seed numbers no longer name the runs that found the bugs. The other five entries, the two found by hand and seeds 5884, 619 and 353, are not pinned.

### Hints

<details>
<summary>Order of work</summary>

Chain first, with every Module 9 test still passing. Then the majority commit and the view change, with `whole` and the promise left out, and run the sweep. You will find seeds 7, 13, 19, 22 and 24 or their cousins within the first thousand. Then the superblock, which seed 562 or its cousin demands. Then the atlas, which seed 619 or its cousin demands. Each addition changes which seed fails next, so keep the sweep running after every change.

</details>

<details>
<summary>The guard</summary>

Put `assert!(at > self.commit)` in `truncate` before anything else. Most of the journal fired it. When it fires, the message names the record and the commit number, and the question is which of the two is a lie. Seed 13 is the case where the commit number was.

</details>

<details>
<summary>Who is a backup</summary>

Keep one predicate, `is_backup_in(view)`. Use it for the commit count, the read barrier and the new primary's last step. A peer is a backup of a view once a beat or a `PrepareOk` of that view said so. Its durable count from before that is of another log.

</details>

<details>
<summary>The promise and the queue</summary>

The disk completes in order. A superblock write submitted before a record's append is durable before the record's acknowledgement. So the promise needs no acknowledgement of its own, only a place in the queue ahead of anything that depends on it. The one place that needs the completion is the first `StartViewChange`, and `on_promised` sends it.

</details>

<details>
<summary>The atlas and determinism</summary>

Roll the dice first, then ask the atlas. Ask the atlas first and skip the roll, and a seed consumes different randomness with and without one. The single-host tests of Modules 6 to 8 change meaning.

</details>

<details>
<summary>Finding your fingerprints</summary>

Every fingerprint changes in this module. Set each constant to zero and read the hash from the assertion. Then run the binary twice and compare, as in Module 9.

</details>

<details>
<summary>The lease hunt finds nothing</summary>

Six clients finish a hundred and twenty requests in half a second of simulated time. That is one partition. The lease needs four things in order within two detector timeouts: the old primary cut off, the others elected, a write committed, and a read at the old primary. Six hundred requests give it room.

</details>

<details>
<summary>Reading a livelock</summary>

Count `status view=` lines per replica in the trace. A view that rises by one every timeout, with two replicas cycling through `view_change` and `gathering`, is a wait for something nobody can give. Find the one replica whose status never changes. In seed 353 it was the one repairing.

</details>

### Stretch goals

- Read a quorum of superblock copies, and fault a copy in a test to show why.
- Persist the commit number in the superblock and skip the `commit_heard` wait. Measure what it costs in writes.
- Let a repairing replica offer its prefix with a marker for the hole, so that a view can start without it. Say what the new primary must do with the records it holds beyond the hole.
- Carry records in `StartView` instead of runs, as the paper does, and remove the pull. Measure the message sizes.
- Elect by vote instead of arithmetic: Raft's `RequestVote`, with the view promise already on disk. Compare the number of view changes per seed.
- Five hundred thousand seeds overnight. Keep the first failure.

## Check your understanding

1. Why do any two majorities of `2f + 1` share a replica, and which three things does the view change assume about that replica?
2. What does `log_view` say that the view of the last record does not? At what point does a backup claim it?
3. What is `whole`? Why is a view allowed to start with every log, failing `f + 1` whole ones?
4. A record arrives at a backup with the right sequence number and the wrong parent. What happened, and what does the backup do?
5. Why does a backup cut its tail against the primary's beat in full the first time and only on a disagreement afterwards?
6. What is written before `StartViewChange`, and what happens without it? Name the seed.
7. Why does a read wait for beats instead of a lease, and what did the lease hunt need to catch the lease?
8. What does the atlas forbid, and why is forbidding it a correction to the model rather than a weakening of the test?
9. What made seed 353 a livelock, and what is the difference between that and the deadlock of two replicas waiting on each other?
10. Which journal entries are the guard's bugs and which are the model's, and why is a journal with neither class suspicious?

<details>
<summary>Answers</summary>

1. Two sets of `f + 1` out of `2f + 1` have `2f + 2` members between them, one more than there are replicas. The shared replica must still have the record, still know it has it, and say so. Lost writes, forgotten votes and late beats each break one.
2. `log_view` is the last view the replica was normal in: a view whose log the primary vouched for and the replica held. A record's view only says a primary once sent it. A backup claims the view once its log is as long as the primary's was at the first word. It also claims it once it holds any record of the view, because such a record chains through the whole start log.
3. `whole` says the log lost nothing since the last vouching. A restarted replica cannot tell a record it never had from one its disk lost, so its silence about a record proves nothing. When every log is offered, nothing is hidden, and the newest log is the truth even though nobody is whole.
4. The backup's prefix is not the primary's. Its last record is not the one the primary's record was chained to. It ignores the prepare. The primary's next beat shows the runs, the backup finds the first disagreement and cuts, and the primary pushes from there.
5. On the first word, a tail the primary lacks is from older views and uncommitted. Afterwards the primary's log only grows. A shorter beat is an older beat that the network delayed, as in seed 24, and the prepares since are the newer news.
6. The superblock, with the view the replica is about to vote for. Without it a crash erases the vote, the replica rejoins the old view, and two views each hold a majority. Seed 562.
7. The beats prove a majority follows this primary now, by their arrival and not by any clock. A lease is a clock's claim about the others' timers. The hunt needed runs five times longer, so that the chaos phase outlasted several partitions.
8. The same sector at fault on more than `f` disks at once. A protocol tolerates `f` faults, so a model that deals `f + 1` tests nothing a protocol can pass. The dice still roll, so a seed keeps its meaning.
9. The repairing replica waited for a word only a whole peer was allowed to give. The peers waited for a primary to make them whole. The primary waited for the repairing replica's offer. Views rose by one each timeout, so the cluster was busy and nothing was served. A deadlock stands still.
10. Seeds 13 and 19 are the guard's: the protocol was right and the check was wrong. Seeds 619 and 353 are the model's: the simulator dealt faults beyond the tolerance, and a rule meant to be safe starved the cluster. A journal without them means the checker and the model were taken on faith.

</details>

## What's next

Fifteen bugs, thirteen found by a seed, and each read from a trace of some thousands of lines. That took minutes for each, and only because the trace carries every message on one line and the guard names the record. Module 11 makes that faster. It runs a range of seeds past the first failure. It shrinks a failing seed's configuration to the smallest that still fails. It slices a trace by replica and by view. It turns a journal entry into a one-line reproduction. Module 12 puts the sweep in CI, where twenty thousand seeds a night is the floor and the first failing seed is the morning's work.
