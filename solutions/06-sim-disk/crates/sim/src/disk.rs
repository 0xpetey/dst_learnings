//! The simulated disk.
//!
//! ```text
//!   submit_write(now, op, offset, bytes, prng)      complete_next(prng)
//!   ────────────────────────────────────────►  ┌──────────┐  ─────────────────►
//!                                  done_at ◄── │ queue    │   lost?  misdirected?
//!   submit_read(now, op, prng)                 │ medium   │   land, per sector:
//!   ────────────────────────────────────────►  │ faults   │     corrupt?
//!                                              └──────────┘   Completion
//!   crash(prng): land, tear or lose what is in flight
//! ```
//!
//! Like the network model, the disk decides and the scheduler queues. A
//! submit returns the instant at which the operation completes; the caller
//! schedules an event for it and calls [`Disk::complete_next`] when the
//! event fires. Operations complete in submission order, which is the
//! contract the real environment keeps by writing and syncing one append at
//! a time, and which the node's offsets rely on.
//!
//! The medium is a byte array in whole sectors. Faults act on sectors,
//! because that is the unit a real device writes atomically. Each fault is
//! a probability from the seed:
//!
//! | Fault | When it is rolled | What it does |
//! |---|---|---|
//! | Lost write | at completion | the completion is reported, the bytes never land |
//! | Torn write | at a crash, per write in flight | a strict prefix of the write's sectors lands |
//! | Misdirected write | at completion | the whole write lands at another sector-aligned offset |
//! | Corrupt write | at completion, per sector | one bit of the sector is flipped on the medium |
//! | Corrupt read | at completion, per sector | one bit of the sector is flipped in the returned copy |
//!
//! A corrupt read is a latent sector error: this read returns garbage, the
//! next may not. A corrupt write is bit rot: the medium holds garbage until
//! the sector is rewritten. A flipped bit is the smallest corruption there
//! is, so a checksum that catches it catches anything larger.
//!
//! The model keeps the set of sectors it has corrupted, so a test can ask
//! whether the system under test ever accepted one of them.

use crate::net::Chance;
use alloc::collections::{BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::fmt;
use determinism::Prng;
use kv_core::{Duration, Instant};

/// The fault model. The defaults are a perfect disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskConfig {
    /// Bytes per sector. Faults and torn writes act on whole sectors.
    pub sector_size: usize,
    /// A read completes after a uniform draw from this range.
    pub read_latency_min: Duration,
    pub read_latency_max: Duration,
    /// A write completes after a uniform draw from this range.
    pub write_latency_min: Duration,
    pub write_latency_max: Duration,
    /// Per write: reported complete, never lands. The `fsync` that lies.
    pub lost_write: Chance,
    /// Per write in flight at a crash: a strict prefix of its sectors lands.
    pub torn_write: Chance,
    /// Per write: lands at the wrong sector-aligned offset, whole.
    pub misdirect_write: Chance,
    /// Per sector written: one bit flipped on the medium.
    pub corrupt_write: Chance,
    /// Per sector read: one bit flipped in what is returned, this time.
    pub corrupt_read: Chance,
}

impl DiskConfig {
    /// No faults, constant latencies.
    pub const fn reliable(read: Duration, write: Duration) -> DiskConfig {
        DiskConfig {
            sector_size: 512,
            read_latency_min: read,
            read_latency_max: read,
            write_latency_min: write,
            write_latency_max: write,
            lost_write: Chance::NEVER,
            torn_write: Chance::NEVER,
            misdirect_write: Chance::NEVER,
            corrupt_write: Chance::NEVER,
            corrupt_read: Chance::NEVER,
        }
    }

    /// A fault configuration drawn from the seed, in a fixed order. The
    /// rates are per mille, because a disk fault that strikes one write in
    /// a hundred would end every run in the first second.
    pub fn random(prng: &mut Prng) -> DiskConfig {
        let read_latency_min = Duration::from_micros(prng.range(0, 1000));
        let read_latency_max = read_latency_min + Duration::from_micros(prng.range(0, 5000));
        let write_latency_min = Duration::from_micros(prng.range(100, 5000));
        let write_latency_max = write_latency_min + Duration::from_micros(prng.range(0, 20_000));
        let lost_write = Chance::new(prng.below(20) as u32, 1000);
        let torn_write = Chance::percent(prng.below(60) as u32);
        let misdirect_write = Chance::new(prng.below(10) as u32, 1000);
        let corrupt_write = Chance::new(prng.below(10) as u32, 1000);
        let corrupt_read = Chance::new(prng.below(10) as u32, 1000);
        DiskConfig {
            sector_size: 512,
            read_latency_min,
            read_latency_max,
            write_latency_min,
            write_latency_max,
            lost_write,
            torn_write,
            misdirect_write,
            corrupt_write,
            corrupt_read,
        }
    }

    pub fn is_reliable(&self) -> bool {
        self.lost_write.is_never()
            && self.torn_write.is_never()
            && self.misdirect_write.is_never()
            && self.corrupt_write.is_never()
            && self.corrupt_read.is_never()
    }

    /// Panics on a configuration that cannot mean anything.
    pub fn validate(&self) {
        assert!(self.sector_size > 0, "sector_size must be positive");
        assert!(
            self.read_latency_min <= self.read_latency_max,
            "read_latency_min {:?} above read_latency_max {:?}",
            self.read_latency_min,
            self.read_latency_max
        );
        assert!(
            self.write_latency_min <= self.write_latency_max,
            "write_latency_min {:?} above write_latency_max {:?}",
            self.write_latency_min,
            self.write_latency_max
        );
    }
}

impl Default for DiskConfig {
    fn default() -> DiskConfig {
        DiskConfig::reliable(Duration::from_millis(1), Duration::from_millis(5))
    }
}

/// Counters, for tests and the end-of-run summary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskStats {
    pub reads: u64,
    pub writes: u64,
    /// Writes that reached the medium in full, at the right place.
    pub landed: u64,
    pub lost: u64,
    pub torn: u64,
    pub misdirected: u64,
    /// Sectors flipped on the medium.
    pub corrupted: u64,
    /// Sectors flipped in a returned read.
    pub read_faults: u64,
    pub crashes: u64,
}

/// What became of a write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteFate {
    Landed,
    Lost,
    Misdirected { to: u64 },
}

impl fmt::Display for WriteFate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteFate::Landed => write!(f, "landed"),
            WriteFate::Lost => write!(f, "lost"),
            WriteFate::Misdirected { to } => write!(f, "misdirected to={to}"),
        }
    }
}

/// A finished operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    Read {
        op: u64,
        data: Vec<u8>,
        /// Sectors of `data` that were corrupted on the way out.
        faults: Vec<usize>,
    },
    Write {
        op: u64,
        fate: WriteFate,
        /// Sectors corrupted on the medium by this write.
        faults: Vec<usize>,
    },
}

/// What a crash did to the writes in flight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CrashReport {
    pub landed: u32,
    pub torn: u32,
    pub lost: u32,
    pub reads_dropped: u32,
}

impl fmt::Display for CrashReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "landed={} torn={} lost={}", self.landed, self.torn, self.lost)
    }
}

#[derive(Clone, Debug)]
enum Pending {
    Read { op: u64 },
    Write { op: u64, offset: u64, bytes: Vec<u8> },
}

/// The disk model: a medium, a queue and a fault policy.
#[derive(Clone, Debug)]
pub struct Disk {
    config: DiskConfig,
    /// The bytes on the platter, in whole sectors.
    medium: Vec<u8>,
    /// Sectors the model has corrupted and that have not been rewritten.
    faults: BTreeSet<usize>,
    pending: VecDeque<Pending>,
    /// Completion time of the last submitted operation.
    busy_until: Instant,
    /// Test knob: the next `n` writes are lost.
    lose_next: u32,
    stats: DiskStats,
}

impl Default for Disk {
    fn default() -> Disk {
        Disk::new(DiskConfig::default())
    }
}

impl Disk {
    pub fn new(config: DiskConfig) -> Disk {
        Disk::with_medium(config, Vec::new())
    }

    /// A disk that already holds `bytes`, as after a restart.
    pub fn with_medium(config: DiskConfig, bytes: Vec<u8>) -> Disk {
        config.validate();
        let mut disk = Disk {
            config,
            medium: bytes,
            faults: BTreeSet::new(),
            pending: VecDeque::new(),
            busy_until: Instant::ZERO,
            lose_next: 0,
            stats: DiskStats::default(),
        };
        disk.pad();
        disk
    }

    pub fn config(&self) -> &DiskConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: DiskConfig) {
        config.validate();
        assert_eq!(
            config.sector_size, self.config.sector_size,
            "sector size is fixed"
        );
        self.config = config;
    }

    pub fn stats(&self) -> DiskStats {
        self.stats
    }

    pub fn sector_size(&self) -> usize {
        self.config.sector_size
    }

    /// The bytes on the medium, faults included.
    pub fn medium(&self) -> &[u8] {
        &self.medium
    }

    /// Sectors corrupted by the model that nothing has rewritten since.
    pub fn faults(&self) -> impl Iterator<Item = usize> + '_ {
        self.faults.iter().copied()
    }

    /// Operations submitted and not yet complete.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    // ---- test knobs --------------------------------------------------------

    /// Lose the next write: report it complete, never land it.
    pub fn lose_next_write(&mut self) {
        self.lose_next += 1;
    }

    /// Flip one bit of `sector` on the medium, now.
    pub fn corrupt_sector(&mut self, sector: usize, prng: &mut Prng) {
        let start = sector * self.config.sector_size;
        assert!(start < self.medium.len(), "sector {sector} is past the end");
        self.flip(start, prng);
        self.faults.insert(sector);
        self.stats.corrupted += 1;
    }

    // ---- the decisions ----------------------------------------------------

    /// Queue a read of the whole medium. Returns when it completes.
    pub fn submit_read(&mut self, now: Instant, op: u64, prng: &mut Prng) -> Instant {
        self.stats.reads += 1;
        let d = uniform(prng, self.config.read_latency_min, self.config.read_latency_max);
        self.pending.push_back(Pending::Read { op });
        self.schedule(now + d)
    }

    /// Queue a write. Returns when it completes.
    pub fn submit_write(
        &mut self,
        now: Instant,
        op: u64,
        offset: u64,
        bytes: &[u8],
        prng: &mut Prng,
    ) -> Instant {
        assert!(!bytes.is_empty(), "empty write");
        self.stats.writes += 1;
        let d = uniform(prng, self.config.write_latency_min, self.config.write_latency_max);
        self.pending.push_back(Pending::Write {
            op,
            offset,
            bytes: bytes.to_vec(),
        });
        self.schedule(now + d)
    }

    fn schedule(&mut self, at: Instant) -> Instant {
        let at = at.max(self.busy_until);
        self.busy_until = at;
        at
    }

    /// Finish the oldest operation. Panics if nothing is pending.
    pub fn complete_next(&mut self, prng: &mut Prng) -> Completion {
        match self
            .pending
            .pop_front()
            .expect("disk completion with nothing pending")
        {
            Pending::Read { op } => {
                let (data, faults) = self.read(prng);
                Completion::Read { op, data, faults }
            }
            Pending::Write { op, offset, bytes } => {
                let (fate, faults) = self.write(offset, &bytes, prng);
                Completion::Write { op, fate, faults }
            }
        }
    }

    /// The process died. Reads in flight are dropped. Each write in flight,
    /// in order, lands in full or tears; after the first torn write nothing
    /// else lands. A reliable disk lands everything, because a process
    /// crash does not stop a device from finishing what it was given.
    pub fn crash(&mut self, prng: &mut Prng) -> CrashReport {
        self.stats.crashes += 1;
        let mut report = CrashReport::default();
        let mut torn = false;
        for pending in core::mem::take(&mut self.pending) {
            let Pending::Write { offset, bytes, .. } = pending else {
                report.reads_dropped += 1;
                continue;
            };
            if torn {
                report.lost += 1;
                self.stats.lost += 1;
            } else if self.config.torn_write.roll(prng) {
                torn = true;
                report.torn += 1;
                self.stats.torn += 1;
                let sectors = self.sectors_touched(offset, bytes.len());
                let keep = prng.below(sectors as u64) as usize;
                let end = (offset as usize / self.config.sector_size + keep) * self.config.sector_size;
                let keep_bytes = end.saturating_sub(offset as usize).min(bytes.len());
                if keep_bytes > 0 {
                    self.land(offset, &bytes[..keep_bytes], prng);
                }
            } else {
                report.landed += 1;
                self.stats.landed += 1;
                self.land(offset, &bytes, prng);
            }
        }
        self.busy_until = Instant::ZERO;
        report
    }

    fn read(&mut self, prng: &mut Prng) -> (Vec<u8>, Vec<usize>) {
        let mut data = self.medium.clone();
        let mut faults = Vec::new();
        if !self.config.corrupt_read.is_never() {
            let sectors = data.len() / self.config.sector_size;
            for sector in 0..sectors {
                if self.config.corrupt_read.roll(prng) {
                    flip_in(
                        &mut data,
                        sector * self.config.sector_size,
                        self.config.sector_size,
                        prng,
                    );
                    faults.push(sector);
                    self.stats.read_faults += 1;
                }
            }
        }
        (data, faults)
    }

    fn write(&mut self, offset: u64, bytes: &[u8], prng: &mut Prng) -> (WriteFate, Vec<usize>) {
        if self.lose_next > 0 {
            self.lose_next -= 1;
            self.stats.lost += 1;
            return (WriteFate::Lost, Vec::new());
        }
        if self.config.lost_write.roll(prng) {
            self.stats.lost += 1;
            return (WriteFate::Lost, Vec::new());
        }
        if self.config.misdirect_write.roll(prng) {
            let size = self.config.sector_size as u64;
            let last = (self.medium.len() as u64 + bytes.len() as u64) / size;
            let mut to = prng.below(last + 1) * size;
            if to == offset {
                to = if to == 0 { size } else { to - size };
            }
            self.stats.misdirected += 1;
            let faults = self.land(to, bytes, prng);
            return (WriteFate::Misdirected { to }, faults);
        }
        self.stats.landed += 1;
        let faults = self.land(offset, bytes, prng);
        (WriteFate::Landed, faults)
    }

    /// Put `bytes` on the medium at `offset`, rolling bit rot per sector.
    fn land(&mut self, offset: u64, bytes: &[u8], prng: &mut Prng) -> Vec<usize> {
        let offset = usize::try_from(offset).expect("offset fits usize");
        let end = offset + bytes.len();
        if end > self.medium.len() {
            self.medium.resize(end, 0);
            self.pad();
        }
        self.medium[offset..end].copy_from_slice(bytes);
        let mut faults = Vec::new();
        let first = offset / self.config.sector_size;
        for sector in first..first + self.sectors_touched(offset as u64, bytes.len()) {
            // Rewriting a rotten sector makes it clean again.
            self.faults.remove(&sector);
            if self.config.corrupt_write.roll(prng) {
                self.flip(sector * self.config.sector_size, prng);
                self.faults.insert(sector);
                self.stats.corrupted += 1;
                faults.push(sector);
            }
        }
        faults
    }

    fn flip(&mut self, start: usize, prng: &mut Prng) {
        let size = self.config.sector_size;
        flip_in(&mut self.medium, start, size, prng);
    }

    /// Sectors a write at `offset` of `len` bytes touches.
    fn sectors_touched(&self, offset: u64, len: usize) -> usize {
        let size = self.config.sector_size;
        let first = offset as usize / size;
        let last = (offset as usize + len).div_ceil(size);
        last - first
    }

    /// Keep the medium a whole number of sectors.
    fn pad(&mut self) {
        let size = self.config.sector_size;
        let len = self.medium.len().div_ceil(size) * size;
        self.medium.resize(len, 0);
    }
}

/// Flip one random bit in `buf[start..start + len]`.
fn flip_in(buf: &mut [u8], start: usize, len: usize, prng: &mut Prng) {
    let bit = prng.below(len as u64 * 8) as usize;
    buf[start + bit / 8] ^= 1 << (bit % 8);
}

/// Uniform in `lo..=hi`. Spends no randomness when the range is a point.
fn uniform(prng: &mut Prng, lo: Duration, hi: Duration) -> Duration {
    debug_assert!(lo <= hi);
    if lo == hi {
        return lo;
    }
    let span = u64::try_from((hi - lo).as_nanos()).expect("latency span fits u64");
    lo + Duration::from_nanos(prng.below(span + 1))
}
