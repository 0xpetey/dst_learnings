# Module 6: The simulated disk

## Why this module exists

Until now the simulated disk was honest: every append became durable, in order, with every byte intact. Real disks are not. Bairavasundaram and colleagues watched 1.5 million drives for 41 months. About one nearline drive in a hundred returned data that failed its checksum at least once. Pillai and colleagues found 60 crash vulnerabilities in eleven well-known programs, most from an assumption about the order in which writes reach the disk. Rebello and colleagues showed that after a failed `fsync`, common file systems mark the dirty pages clean. The data is gone and the next `fsync` reports success.

Alagappan and colleagues concluded in "Protocol-Aware Recovery": a replicated system that treats a corrupt log entry as a missing one loses data, and one that treats it as a crash is unavailable. Module 10 builds the repair. This module builds the disk that makes repair necessary, and the detection that makes it possible.

The module adds three things. The simulator gets a disk model with sector-level faults. The world learns to kill a process and start a new one over the same disk. The log gets a header with a checksum and a sequence number, and a recovery procedure that tells a torn tail from a hole. The acceptance tests assert that the node never serves corruption.

## Objectives

By the end of this module you can:

- Model a disk as a queue of sector writes with a fate per write, and say why a process crash and a power loss differ.
- Name the storage faults and implement each one in a few lines: torn, lost and misdirected writes, bit rot, and latent sector errors.
- Explain why storage faults are not network faults, and what that does to a replicated system.
- Lay out a log in whole sectors with a checksum on every record, and say what the checksum covers and why.
- Write a recovery that classifies the end of the log as clean, torn or damaged, and halt on damage.
- Name the cases a single disk cannot distinguish, and say what TigerBeetle adds to distinguish them.
- Restart a node from its disk alone, and find the state that was only in memory.
- Say what the fault atlas in TigerBeetle's simulator is for.

## Lesson

### A disk is a queue of sectors

The disk model has the shape of the network model. It decides, and the scheduler queues:

```text
   node ──► Append ──► disk.submit_write(now, op, offset, bytes, prng) ──► done_at
                                                                              │
                        DiskComplete @ done_at ◄──────────────────────────────┘
                              │
                              ▼
                        disk.complete_next(prng) ──► Write { fate: landed | lost | misdirected, faults }
                                                 ──► Read  { data, faults }
```

`submit_write` draws a latency and returns the completion instant. The world schedules one `DiskComplete` event for it. When the event fires, `complete_next` pops the oldest operation and decides what became of it. Operations complete in submission order. That is the contract the real `FileStorage` keeps by writing and syncing one append at a time, and the node's offsets depend on it.

The medium is a byte array in whole sectors. A sector is the unit a device writes atomically, so it is the unit the faults act on. A record that spans three sectors can land in one of them and not the others.

A crash is a method on the disk. A process crash leaves the kernel and the device running, so on a reliable disk every write in flight lands in full. In a power loss, the write in flight lands in part, or not at all. The `torn_write` knob is the probability of the second case. FoundationDB's `AsyncFileNonDurable` makes the same split. The comment on its `kill` says that it simulates "a power failure which prevents all written data from being persisted to disk".

### The fault model, one fault at a time

| Fault | What it looks like in production | When the model rolls it | What the model does |
|---|---|---|---|
| Torn write | Power loss during a multi-sector write | At a crash, per write in flight | A strict prefix of the write's sectors lands. Later writes in flight are lost |
| Lost write | An `fsync` or a drive cache that lied | At completion, per write | The completion is reported. The bytes never land |
| Misdirected write | Firmware writes the right data to the wrong address | At completion, per write | The whole write lands at another sector-aligned offset |
| Bit rot | A sector decays on the platter | At completion, per sector written | One bit of the sector is flipped on the medium |
| Latent sector error | A sector that reads back wrong, this time | At completion, per sector read | One bit of the sector is flipped in the returned copy |

Each row is a few lines. The whole model is about five hundred lines with its documentation.

A flipped bit is the smallest corruption there is. A checksum that catches one flipped bit catches anything larger, so one flipped bit per sector is the strongest test of the reader. TigerBeetle's simulated storage does the same thing. When a sector is faulty, its `read_sectors` flips one byte of the returned buffer.

An unreadable sector and a rotten sector both fail a reader's checksum, so the model has no `EIO`. A latent sector error differs from bit rot in that the next read is clean. The disk remembers which sectors it corrupted on the medium, so a test can ask whether the node accepted one of them.

The rates in `DiskConfig::random` are per mille where the network's are per cent. A disk fault that strikes one write in a hundred ends every run in the first second. One in a thousand lets a run crash and restart a few times. It still produces a halt in about half of the seeds.

### Storage faults are not network faults

Three things are true of a disk and false of a network.

Faults persist. A node that restarts reads the same corrupt sector again. `a_lost_write_in_the_middle_halts_the_node` restarts the node twice and it halts twice.

The fault is in the only copy. When the network loses a request, the client still has it. When the disk loses a record, nobody has it, unless another machine does. That is the argument for replication, and it is why Module 10 is the next time this log changes.

Corruption can hit a majority. A partition leaves a majority side that goes on. Bit rot is independent per disk, and nothing stops it from hitting the same record on two of three replicas. When a replica finds its own log damaged, Protocol-Aware Recovery says how it asks the others. TigerBeetle's simulator adds a `ClusterFaultAtlas` for this reason. Its header says that read and write faults "are distributed between replicas according to ClusterFaultAtlas". The aim, in its words, is that "at least one replica will have a valid copy to help others repair". With one replica, "its WAL can only be corrupted by a crash, never a read/write". The model in this module has no atlas yet, because there is one disk per node and no repair. Module 10 adds one.

### The log grows a header

Module 2's log format, a length prefix and a tag, survives a torn tail. It cannot survive a flipped bit in a value, because nothing says the value changed. It cannot survive a hole in the middle, because a hole with a plausible length prefix decodes as an entry. The Module 6 format is:

```text
   sector 0        sector 1        sector 2        sector 3
   ┌───────────────┬───────────────┬───────────────┬───────────────┐
   │ hdr │ payload │ hdr │ payload │ ... payload   │ hdr │ payload │
   │ seq=1         │ seq=2 (two sectors)           │ seq=3         │
   └───────────────┴───────────────┴───────────────┴───────────────┘

   header, 24 bytes:  magic "KVL1"  len u32  seq u64  crc32c u32  reserved u32
```

Every record starts on a sector boundary and is zero padded to a whole number of sectors. The checksum covers the whole record except itself, padding included, so a flipped bit anywhere in the record fails the check. CRC-32C is the polynomial that iSCSI, ext4 and FoundationDB use. The course writes it out in forty lines, so that no dependency can change what a stored checksum means.

The sequence number says which record belongs at this offset. A write that landed in the wrong place carries the wrong number for its position. A stale record left behind a torn tail carries a number that was already used. Both fail recovery at that offset. TigerBeetle's journal does the same with the `op` in each header, which "helps WAL recovery detect misdirected reads/writes".

The sector alignment lets recovery skip. When a record fails its check, recovery does not know how long it was. It does know that the next record starts on a sector boundary. It probes every boundary after the bad one for a valid header. That probe is the difference between a torn tail and a hole.

The payload carries the client id and the request id, for the reason in "The bug the module found".

### Recovery: torn or damaged?

Recovery reads records front to back. It accepts each record that has the right magic, a valid checksum, the expected sequence number and a payload that decodes. It stops at the first record that does not, and looks for a valid record anywhere after it.

| Bytes after the last valid record | Verdict | What happened | What the node does |
|---|---|---|---|
| Nothing, or only zeros | `Clean` | The log ends here | Serves |
| Not a record, and no record after | `Torn` | A crash in the middle of an append | Serves. The next append overwrites the torn bytes |
| Not a record, and a valid record after | `Damaged` | A record that was once there is missing or broken | Halts with `Output::Halt` |
| A valid record with the wrong sequence number | `Damaged` | A write landed in the wrong place | Halts |

The reasoning behind `Damaged` rests on completion order. When the crash came, a record that was never acknowledged was in flight. Writes complete in order, so nothing after it was written, and no valid record follows it. If a valid record follows, the broken one completed before it, and the node acknowledged it. Serving on top of the hole answers reads with a state that is missing an acknowledged write, so the node refuses. The driver stops stepping on `Output::Halt`, the real server exits with the reason, and the world takes the host down and records `last_halt`.

The disk model keeps completion order at a crash by landing writes in order and losing everything after the first torn one. TigerBeetle's simulator tears each in-flight write independently, and its journal recovery has a sixteen-column decision table and a function named `torn_prepares` to cope. The lesson's version is the one-column table above.

`every_bit_flip_in_the_valid_region_is_caught` in the log tests flips every bit of a three-record log, one at a time, and checks the verdict for each. A flip in records one or two is `Damaged`. A flip in record three is `Torn`.

### What a single disk cannot tell you

Three faults produce a log that recovery accepts. In each case an acknowledged write is gone and nothing on the disk says so.

- A lost write at the tail. `fsync` lied about the last record. The medium ends where the previous record ends. The log is `Clean`.
- A rotten last record. One bit of the last record flipped. The bytes are not a record and nothing follows. The log is `Torn`, which is also what a crash during that append produces.
- A read fault on the last record. Same bytes, same verdict, but the next read is clean.

Seed 14 of `storage_chaos_is_a_pure_function_of_the_seed` shows the third case. A node acknowledges a write, restarts, serves the old value for that key, restarts again, and serves the new one. The record was there the whole time. The first read flipped a bit in it, so recovery took it for a torn tail. The oracle in that test treats a lost write as a write of unknown fate. It does not treat it as gone, because it comes back.

The root cause is the same in all three. The node's only record of what it acknowledged is the log, and the log is what was damaged. Three things fix it, and the course adds them in order.

1. A replica. Module 10's node asks its peers what the last record was. If a peer has record 4, the local record 4 was lost and not torn. This is the recovery in the Protocol-Aware Recovery paper. TigerBeetle's documentation says that only corruption on every replica in the cluster loses a record.
2. Redundant headers. TigerBeetle writes every prepare's header a second time, into a ring of headers in a separate zone, after the prepare is durable. A valid header for a corrupt prepare says the prepare was once there.
3. A superblock. A small, checksummed record of how far the log reached, written separately. With one, a `Clean` log that ends early is damage. Without one, it is an empty log.

Until then the lesson's rule is honest. On one disk, a tail that reads back wrong is a torn tail. The acceptance tests count the acknowledged writes this loses and print the count. They assert that the count is zero on every disk that cannot lose a write.

### Crash and restart in the world

A `Host` is now a `Disk` and an `Option<Process>`. The process is the KV node behind its driver and the heartbeat. `World::crash` takes the process, calls `disk.crash`, and cancels the host's heartbeat timer. `World::restart` builds a new process, which issues a `Read` and recovers from whatever the disk returns. There is no other path from the old process to the new one.

Each start increments the host's `incarnation`, and every `DiskComplete` event carries the incarnation that submitted it. A completion that fires for a dead process is stale. The crash already settled that operation, and the world counts the event and drops it.

A message delivered to a down host goes nowhere. The network did its job, so it is not a network drop. The trace line is `n0 down, dropped request client=1 id=3`, and the client retries after its timeout. From the client's side a crashed node and a partitioned node look the same.

The new process's heartbeat counts beats from one. Its peers have seen higher numbers, so the first beats after a restart count as stragglers. `a_crash_looks_like_a_partition_from_the_outside` asserts on that counter. It is evidence of a restart that the protocol can see.

A `CrashSchedule` does what the `PartitionSchedule` does. After a gap the seed picks a host that is up and kills it. After a down time it restarts the host and schedules the next crash. `World::stop_crashes` cancels the schedule and brings every down host up, including one that halted over damage. That host reads its disk again. If the damage was a read fault, it comes back. If the damage is on the medium, it halts again, and the quiet period's liveness check skips it on purpose.

### The bug the module found

The first version of the crash tests failed in a way the Module 2 node never showed. The client sent a delete. The node made it durable and replied. The reply was lost, the node crashed, and the client retried the same request id. The restarted node had no session for that client. It applied the delete again, replied `existed: false`, and wrote a third record for two requests.

Sessions were in memory. The log entries carried the change and not who asked for it, so a restart had no way to rebuild them. The fix is in the format: every record carries the client id and the request id, and recovery rebuilds the session table as it replays. `retry_after_a_restart_is_not_applied_twice` in the node tests and `a_retry_across_a_restart_is_not_applied_twice` in the world tests pin it.

The fix found a second bug, in the tests this time. A world built over an old disk added a client with id 1, and the node ignored it. Client 1's session was in the log with a last request id of 4, and the new client started at 1. Its requests were stale. A client id names a session, and a session is durable now. A new client process takes a new id, which the `kv` binary already does. The ported tests use a new id after a restart and say why.

### Reading a trace

One host, one client, a disk that tears every write it is holding at a crash. The test loses the second write by hand. Timestamps are shortened, and some lines are left out.

```text
t=0        n0 out Read op=0
t=0        n0 disk read op=0 at=t=1ms
t=1ms      n0 disk read complete op=0 bytes=0
t=1ms      n0 in  Request client=1 id=1 Put { key: [97], value: [49] }
t=1ms      n0 disk write op=1 offset=0 sectors=1 at=t=6ms
t=6ms      n0 disk write complete op=1 landed
t=6ms      n0 out Reply client=1 id=1 PutOk
t=8ms      n0 disk write op=2 offset=512 sectors=1 at=t=13ms
t=13ms     n0 disk write complete op=2 lost
t=13ms     n0 out Reply client=1 id=2 PutOk
t=15ms     n0 disk write op=3 offset=1024 sectors=2 at=t=20ms
t=16ms     n0 crash landed=0 torn=1 lost=0
t=46ms     n0 restart
t=47ms     n0 disk read complete op=0 bytes=1536
t=47ms     n0 in  ReadComplete op=0 bytes=1536
t=114ms    timer client=1
t=115ms    n0 in  Request client=1 id=3 Put { key: [99], value: [120, ...] }
t=115ms    n0 disk write op=1 offset=512 sectors=2 at=t=120ms
t=121ms    client=1 done id=3 PutOk
```

The write at 13 ms is marked `lost` by the disk and `PutOk` by the node, in the same instant. The crash at 16 ms tears the two-sector write in flight, and one sector of it lands. The restart reads 1536 bytes: record 1, a sector of zeros where record 2 belongs, and half of record 3. Recovery accepts record 1, finds zeros, probes sector 2, finds half a record that fails its checksum, and reports a torn tail. The retry at 115 ms lands at offset 512, on top of the hole. Key `b`, the write acknowledged at 13 ms, is gone, and no line in the trace after the restart says so. The one at 13 ms does.

### How the other simulators do it

TigerBeetle's `Storage` in `src/testing/storage.zig` has the same knobs under other names. `read_fault_probability` and `write_fault_probability` mark a sector faulty. `crash_fault_probability` faults a sector of each pending write at a crash. `write_misdirect_probability` moves a whole write to another offset in the same zone. The comments limit it to one per storage, "because double-faults are not covered by our fault model". Its reads of a sector that was never written return random bytes, where the model here returns zeros. Its latencies are exponential with a minimum, where the model here is uniform.

Two things in TigerBeetle's model have no counterpart here yet. The fault atlas decides which sectors each replica's faults are allowed to hit. The zones give each kind of data its own tolerance. The superblock permits one fault per area. The write-ahead log permits faults by the atlas, and the grid of data blocks permits faults only with three or more replicas. Module 13 returns to the zones.

FoundationDB's `AsyncFileNonDurable` wraps a real file. Writes wait a random time before they reach the file, and a kill in that window applies one of three `KillMode`s drawn at open: no corruption, dropped writes only, or full corruption of the sectors that were being written. There is no persistent fault model. A sector that was written correctly stays correct.

### Decisions recorded

- The disk decides and the scheduler queues, as the network does. `Disk` holds the medium, the queue, the fault set and the counters. The world schedules one event per operation.
- Operations complete in submission order, and a crash lands in-flight writes in order until the first torn one. This matches the real environment, which syncs each append before the next. It makes "a valid record after a bad one" mean "an acknowledged write is gone".
- A process crash lands what was in flight. A power loss is the `torn_write` probability. The lesson keeps both under one `crash` method.
- Corruption is one flipped bit, on the medium for a write fault and in the returned copy for a read fault. There is no `EIO`.
- Records are whole sectors with a 24-byte header, and the CRC-32C covers everything in the record except itself, padding included.
- Recovery never fails. It reports `Clean`, `Torn` or `Damaged`, and the node decides. On `Damaged` the node emits `Output::Halt` and the driver stops stepping.
- Log records carry the client id and the request id, and recovery rebuilds sessions. A client id names a durable session, so a new client process takes a new id.
- A host is a disk and an optional process. Disk events carry an incarnation, and a stale one is dropped and counted.
- The world has no superblock. A tail that reads back wrong is a torn tail, and the oracle treats a lost acknowledged write as a write of unknown fate. Recorded as an open decision for Module 10.
- The three fingerprints changed on purpose. The seeded workload is `0x0965_0d64_15e6_a7b3`, the network chaos run is `0x0bee_545c_7d3c_dcd4`, and the new storage chaos run is `0x3dec_d7ad_dae6_8e98`.

## Readings

Required:

- [Alagappan et al., "Protocol-Aware Recovery for Consensus-Based Storage" (FAST '18)](https://www.usenix.org/conference/fast18/presentation/alagappan). Read sections 2 and 3 for the storage fault taxonomy. The paper behind TigerBeetle's storage model.
- [Pillai et al., "All File Systems Are Not Created Equal" (OSDI '14)](https://www.usenix.org/conference/osdi14/technical-sessions/presentation/pillai). Read the persistence properties in section 3 and the vulnerability table. Every row is a write that landed in a different order, or in part.
- [Dan Luu, "Files are hard"](https://danluu.com/file-consistency/). A short tour of the two papers above and the Bairavasundaram study.
- TigerBeetle: [`src/testing/storage.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/testing/storage.zig). Read the header comment, `Options`, `reset` and the comment above `overlays`. `ClusterFaultAtlas` lives in this file. `src/testing/cluster.zig` imports it as `StorageFaultAtlas`.
- TigerBeetle: [`src/vsr/journal.zig`](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/journal.zig). Search for "Recovery decision table" and for `torn_prepares`. Read the comment on `headers_redundant` for why the header is written twice.

Optional:

- [Bairavasundaram et al., "An Analysis of Data Corruption in the Storage Stack" (FAST '08)](https://www.usenix.org/legacy/event/fast08/tech/full_papers/bairavasundaram/bairavasundaram_html/index.html). The numbers behind "disks lie". Read section 3 for the checksum mismatch rates.
- [Rebello et al., "Can Applications Recover from fsync Failures?" (ATC '20)](https://www.usenix.org/conference/atc20/presentation/rebello). The lost write, measured. Read section 3 for what each file system does with a dirty page after a failed `fsync`.
- FoundationDB: [`fdbrpc/include/fdbrpc/AsyncFileNonDurable.h`](https://github.com/apple/foundationdb/blob/main/fdbrpc/include/fdbrpc/AsyncFileNonDurable.h). Read the class comment, `KillMode` and the comment above `write`. A crash-only storage model, for contrast.
- [TigerBeetle, "Safety"](https://docs.tigerbeetle.com/concepts/safety/). The storage fault tolerance section, in the project's own words.

## Coding task

### Starting point

Start from your Module 5 workspace with `dst start 06`. If you did not finish Module 5, `dst start 06 --from-solution` starts from `solutions/05-sim-network/` instead. The command adds `disk.rs` to `crates/sim/tests/` and `log.rs` to `crates/kv-core/tests/`. In `crates/kv-env/tests/` it replaces `fake_world.rs`, `virtual_time.rs` and `sim_network.rs` and adds `sim_disk.rs`. `scripts/check.sh` does not change.

### Requirements

#### The log, in `kv_core::log` and `kv_core::checksum`

1. `checksum::crc32c(&[u8]) -> u32` and `crc32c_update(u32, &[u8]) -> u32` implement CRC-32C with no dependency. `crc32c(b"123456789")` is `0xE306_9283`.
2. `SECTOR_SIZE` is 512 and `HEADER_LEN` is 24. `record_len(payload_len)` rounds `HEADER_LEN + payload_len` up to whole sectors. `MAX_PAYLOAD_LEN` fits a tag, a client id, a request id and the largest key and value.
3. `Record { client, request_id, entry }` has `encode(seq, &mut Vec<u8>)` and `encoded_len()`. The header is `[magic "KVL1" u32][len u32][seq u64][crc32c u32][reserved u32]`, little endian. The checksum covers the whole padded record except the checksum field.
4. `recover(&[u8]) -> Recovered { records, valid_len, tail }` never fails. Record `i` has sequence number `i + 1`. `tail` is `Tail::Clean`, `Tail::Torn { at }` or `Tail::Damaged(Damage { at, seq, next_valid_at, next_valid_seq })`. A valid record at `at` with the wrong sequence number is damage with `next_valid_at == at`. `Damage` displays as `record 2 at offset 512 is unreadable, record 3 is at offset 1024`.

#### The node, in `kv_core::node`

5. `Output::Halt { damage }`. On a `Damaged` log the node drops its queued requests, emits `Halt`, and answers nothing afterwards. `is_halted()`, `damage()`, `recovery() -> Option<Recovery { records, valid_len, tail }>` and `recovered_records()` are queries. `Recovery` is the node's copy of `log::Recovered`, with the same fields. `Stats` gains `dropped`.
6. Recovery rebuilds the session of every record's client, so that a retry after a restart gets the same answer. `log_seq()` and `log_len()` are the records and bytes known durable. A pending append is numbered `log_seq + pending + 1`.
7. `Driver::is_halted()` is true after `Output::Halt`, and a halted driver's `step` returns zero. The real server loop exits with an error that names the damage.

#### The disk model, in `sim::disk`

8. `DiskConfig` has public fields `sector_size`, `read_latency_min`, `read_latency_max`, `write_latency_min`, `write_latency_max`, `lost_write`, `torn_write`, `misdirect_write`, `corrupt_write` and `corrupt_read`. `reliable(read, write)` has no faults. `random(&mut Prng)` draws in a fixed order with the fault rates per mille. `validate` panics with a message that contains `read_latency_min` on an inverted range. `Default` is `reliable(1 ms, 5 ms)`.
9. `Disk::new(DiskConfig)` and `with_medium(DiskConfig, Vec<u8>)`. The medium is always whole sectors. Queries: `config`, `set_config`, `stats`, `sector_size`, `medium`, `faults()` and `pending()`. Test knobs: `lose_next_write()` and `corrupt_sector(sector, &mut Prng)`.
10. `submit_read(now, op, &mut Prng) -> Instant` and `submit_write(now, op, offset, bytes, &mut Prng) -> Instant`. Completion is at `now + latency` or after the previous operation, whichever is later. An empty write panics.
11. `complete_next(&mut Prng) -> Completion`. For a write, the order of checks is `lose_next_write`, the `lost_write` dice, the `misdirect_write` dice, then landing with one `corrupt_write` roll per sector. A rewritten sector is no longer faulty. For a read, the whole medium is returned with one `corrupt_read` roll per sector, and the faulty sectors are listed in the completion. An empty queue panics with a message that contains `nothing pending`.
12. `crash(&mut Prng) -> CrashReport { landed, torn, lost, reads_dropped }`. Reads in flight are dropped. Writes in flight land in order. The first write that rolls `torn_write` keeps a strict prefix of its sectors, and every write after it is lost. `DiskStats` counts `reads`, `writes`, `landed`, `lost`, `torn`, `misdirected`, `corrupted`, `read_faults` and `crashes`.

#### The world, in `kv_env::fake`

13. `FakeStorage` is a doorstep. `read_all` and `append` push a `StorageOp` onto `ops`, and the world moves them to the host's disk. `complete(Input)` queues a completion.
14. `Host` has public `id`, `disk`, `process: Option<Process>`, `last_halt` and `last_read_faults`, and queries `is_up()`, `incarnation()`, `process()`, `node()` and `heartbeat()`. `Process` has public `driver` and `heartbeat`. `World::node(id)`, `heartbeat(id)`, `disk(id)` and `is_up(id)` are shortcuts.
15. `WorldConfig` replaces `disk_latency` and `disk_jitter` with `disk: DiskConfig` and gains `crashes: Option<CrashSchedule { gap_max, down_max }>`. Constructors are `new(Disk)`, `cluster(n, WorldConfig)` and `with_disks(Vec<Disk>, WorldConfig)`. When a constructor returns, the nodes are still reading their disks.
16. `Event` gains `DiskComplete { node, incarnation }`, `Crash` and `Restart { node }`. A `DiskComplete` whose incarnation is not the host's current one is dropped and counted under `stale_disk_events`.
17. `World::crash(node)` and `restart(node)`. A crash takes the process, calls `disk.crash`, and cancels the heartbeat timer. A restart builds a new process over the same disk. A message delivered to a down host is counted under `dropped_at_down_hosts`. A process that emits `Halt` is taken down, its damage goes into `last_halt`, and `halts` is counted.
18. With a `CrashSchedule`, `Crash` picks a random host that is up, and `Restart` brings it back and schedules the next crash. `stop_crashes()` cancels the schedule and restarts every down host.
19. `is_idle()` also requires that no disk has an operation queued. `WorldStats` gains `dropped_at_down_hosts`, `reads_completed`, `stale_disk_events`, `crashes`, `restarts` and `halts`. `appends_completed` keeps its name.
20. Trace lines, where `{now}` is the `Display` form of the current `Instant`:
    - `{now} n{id} disk read op={op} at={t}` and `{now} n{id} disk write op={op} offset={offset} sectors={n} at={t}` at submission.
    - `{now} n{id} disk read complete op={op} bytes={n}` and `{now} n{id} disk read fault sector={s}` per faulty sector.
    - `{now} n{id} disk write complete op={op} {fate}` where the fate is `landed`, `lost` or `misdirected to={offset}`, and `{now} n{id} disk corrupt sector={s}` per rotten sector.
    - `{now} n{id} crash landed={a} torn={b} lost={c}`, `{now} n{id} restart`, `{now} crashes stopped`.
    - `{now} n{id} halted: {damage}`, `{now} n{id} out Halt {damage}`, `{now} n{id} down, dropped {message}`.
21. The fingerprints. The seeded workload in `fake_world.rs` is `0x0965_0d64_15e6_a7b3` for seed 1. The network chaos run in `sim_network.rs` is `0x0bee_545c_7d3c_dcd4`. The storage chaos run in `sim_disk.rs` is `0x3dec_d7ad_dae6_8e98`.

### Acceptance criteria

```sh
scripts/check.sh                                # ends with "all checks passed"
cargo tree -p sim                               # sim, determinism and kv-core, nothing else
cargo tree -p kv-core                           # kv-core alone, plus determinism as a dev-dependency
cargo test --release -p kv-env --test virtual_time one_simulated_year -- --nocapture
cargo test --release -p kv-env --test sim_disk
```

The year test still prints `two simulated years in` under one second in a release build. The reference solution takes about 0.7 s, a little more than Module 5, because the log read at start is an event of its own. The storage chaos run tests twenty seeds three times each, with crashes, partitions and disk faults. It takes about 0.2 s in release and under a second in debug.

### Hints

<details>
<summary>The checksum field</summary>

Checksum the record in two pieces. Run `crc32c_update` over bytes 0 to 16, then over bytes 20 to the end. Patch the length and the checksum into the header after the payload and the padding are in place. `every_bit_flip_in_the_valid_region_is_caught` fails on any byte the checksum skips.

</details>

<details>
<summary>Probing for the next record</summary>

Write one probe function. It examines the bytes at an offset and returns short, invalid, or valid with the sequence number. Recovery calls it at the expected offset. When the answer is invalid, call it at every sector boundary after that until the end. Record the first valid answer. If there is one, the verdict is damage. If the remaining bytes are not all zero, it is torn. Otherwise it is clean.

</details>

<details>
<summary>Who owns the disk</summary>

The driver owns its storage, and the driver dies with the process. Keep the medium out of the driver. The world moves operations from `FakeStorage` to the host's `Disk` in `pump`, the way it moves replies to the network.

</details>

<details>
<summary>The torn prefix</summary>

Count the sectors a write touches from its first sector to the sector that holds its last byte. Draw `keep` below that count, so that at least one sector is missing. The bytes that land are the ones before the end of sector `first + keep`, clipped to the write. When `keep` is zero nothing lands, and the medium does not grow.

</details>

<details>
<summary>A new client after a restart</summary>

In a second world over an old disk, use a new client id. The reference tests use 2 or 9.

</details>

<details>
<summary>The chaos oracle</summary>

Keep one list per node. It holds the writes the client issued, in order, with a flag for whether the fate is certain. An acknowledged write is certain. A write the client gave up on is not. At every restart, walk the node's `recovered_records` against the list. An uncertain write is allowed to be present or absent. A certain write that is absent is a lost write, which is allowed only on a disk that can lose one, and it becomes uncertain. Check the state only for keys with no uncertain write.

</details>

<details>
<summary>Finding your fingerprints</summary>

Set each constant to zero and read the hash from the assertion message, after the three determinism tests pass.

</details>

### Stretch goals

- Add a superblock: a one-sector record, written and checksummed separately, that holds the sequence number of the last acknowledged record. Recovery then treats a `Clean` or `Torn` log that ends before that number as `Damaged`. Which tests change, and which of the three ambiguous cases does it close?
- Write every record's header a second time, into a separate ring of headers, after the record is durable. Use the ring to tell a torn record from a rotten one. Compare with the `headers_redundant` comment in TigerBeetle's journal.
- Give the heartbeat an incarnation number next to the sequence number, so that a restarted peer's first beats are not stragglers.
- Tear each in-flight write independently at a crash, as TigerBeetle's `reset` does. Which assertion in the module breaks, and what does recovery need to put it back?
- Replace the uniform latency with an exponential distribution and a minimum, as TigerBeetle's storage does. Decide whether read latency depends on the size of the read.
- Let the disk run out of space. Which output does the node need, and what does the client see?

## Check your understanding

1. The disk model lands every in-flight write at a crash on a reliable disk. Why is that the right default, and what is the `torn_write` knob a model of?
2. A write completes with fate `lost` and the node replies `PutOk`. Which of them is wrong?
3. Why does recovery need sector alignment to tell a torn tail from a hole?
4. The checksum covers the padding. What does that buy, and what does it cost?
5. A node halts on `Damaged` and serves on `Torn`. Give the argument that makes `Torn` safe, and name the assumption it rests on.
6. Name the three faults that a single disk cannot distinguish from a torn tail. Say what each of the three fixes in the lesson does about them.
7. Why did the sessions have to go into the log, and what did that change for a client that restarts?
8. The seeded workload fingerprint changed again. The syllabus says that an intended change makes a fingerprint change fine. What in this module changed the trace of a run with no disk faults at all?

<details>
<summary>Answers</summary>

1. A process crash does not stop the kernel or the device. Writes already handed over reach the medium. The `torn_write` probability models a power loss, in which the write in flight stops partway. Keeping both under one `crash` method lets the seed choose per run.
2. Neither, and that is the problem. The disk reported completion, and the node believed it, as every program believes `fsync`. The lie is in the disk, and only a restart reveals it. The trace line at the moment of the lie is the only place the loss is visible.
3. When a record fails its check, its length field cannot be trusted, so recovery does not know where the next record starts. Sector alignment gives it a finite set of candidates: every sector boundary after the bad one. A valid header at any of them proves that a record once followed the bad one.
4. It buys the strongest test. A flipped bit anywhere in the record fails the check, so the oracle can assert that no corrupted sector lies inside the accepted region. It costs nothing at runtime, because the padding is zeros that are in memory already. A tear that loses only padding still checks out, because the disk pads with zeros.
5. Writes complete in order. Take a record that fails its check with no valid record after it. It was the last write issued. The node acknowledges a write only at completion. So the torn record was never acknowledged, and nobody is owed it. The assumption is in-order completion. The real storage keeps it with one `fsync` per append. The disk model keeps it at a crash by losing everything after the first torn write.
6. A lost write at the tail, a rotten last record, and a read fault on the last record. A replica knows the true last record and repairs the local copy. A redundant header, written after the record, proves that a record was there. A superblock records how far the log reached, so that a log that ends early is damage and not an empty log.
7. A retry carries the same request id as the original, and the node answers it from the session without applying it again. After a restart there was no session, so the retry was applied twice. With the client id and the request id in every record, recovery rebuilds the sessions. The consequence is that a client id names a durable session, so a client that restarts with ids from one again is ignored. A new client process takes a new id.
8. The log read at start is now a disk operation with a latency and two trace lines. Every record is a whole sector, so the `Append` lines carry different byte counts. Every completion carries a fate. None of that is a fault, and all of it is in the trace.

</details>

## What's next

Module 7 turns the fault probabilities into a search. It adds the faults inside a process (pauses and clock jumps), the `buggify!` macro, and swarm testing, where each seed picks which faults to enable. The disk model does not change again.
