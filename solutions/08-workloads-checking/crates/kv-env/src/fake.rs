//! The simulated environment.
//!
//! Module 4 put every delay on a [`sim::Scheduler`]. Module 5 put every
//! message through one [`sim::Net`]. Module 6 put every disk operation
//! through one [`sim::Disk`] per host and let a host die. Module 7 lets a
//! host stall, gives each host its own clock, lets the code under test
//! misbehave at marked points, and draws the whole fault configuration
//! from the seed:
//!
//! ```text
//!   client.submit ──► Request ──┐
//!   node ───────► Reply ────────┼──► net.send ──► Deliver @ t ──► net.deliver ──► destination
//!   heartbeat ──► Beat ─────────┘        │
//!                                        └──► Err(Dropped): traced, no event
//!   node ──► Read / Append ──► disk.submit ──► DiskComplete @ t ──► disk.complete_next ──► node
//!   client.next_deadline()    ──► ClientTimer @ deadline ──► client
//!   heartbeat.next_deadline() ──► NodeTimer   @ clock.to_true(deadline) ──► heartbeat
//!   seed ──► Crash ──► host.process = None, disk.crash()
//!        ──► Restart ──► new process, which reads the disk
//!        ──► Pause ──► the process handles nothing; inputs queue at the host
//!        ──► Resume ──► the process handles everything that queued
//!        ──► ClockJump ──► one host's clock leaps forward
//!   node, heartbeat ──► buggify!(site) ──► an unusual path, traced
//! ```
//!
//! A [`Host`] is a [`Disk`], a [`HostClock`] and, while it is up, a
//! [`Process`]: the KV node behind its [`Driver`] and a [`Heartbeat`]. A
//! crash drops the process. Everything the node knew is gone; only the
//! disk remains, and the disk decides what became of the writes that were
//! in flight. A restart builds a new process, which reads the disk and
//! recovers from it alone. That is the whole point: there is no back door
//! from the old process to the new.
//!
//! A pause keeps the process and stops driving it. Messages for it wait on
//! its doorstep, disk completions wait in its storage queue, and its timer
//! is disarmed. On resume everything is handled at once, at the time the
//! host's clock then reads. From the inside a pause is invisible, except
//! that the clock moved. From the outside it is a crash that keeps its
//! memory.
//!
//! The process reads the host's clock, which drifts, starts from its own
//! zero, and jumps. The disk and the network keep true time. Timers are
//! armed in true time by converting the heartbeat's local deadline.
//!
//! Nodes and clients share one address space, [`Addr`]. A request from a
//! client, a reply to it and a heartbeat between nodes are all the same
//! thing to the network. The scheduler is still the only queue.
//!
//! One trace serves every host. Each driver's observer holds a handle to it
//! and tags its lines with the host id. Its first line is the configuration
//! of the run, so a failing seed is readable before its first event.
//!
//! Module 8 added the oracle's side. The world records a [`History`] of
//! every client call and return, drives a [`Workload`] across many clients
//! at once with [`World::run_workload`], and can plant a [`Bug`] in every
//! node so that the checker can be shown to find it.

use checker::{History, Step, Workload};
use determinism::{Chance, Prng, Trace};
use kv_core::buggify::{Buggify, Site};
use kv_core::bugs::Bug;
use kv_core::client::{self, Client};
use kv_core::env::{Driver, Network, Observer, Storage};
use kv_core::heartbeat::{self, Heartbeat, NodeId};
use kv_core::log::Damage;
use kv_core::node::{Input, Node, OpId, Output};
use kv_core::{ClientId, Duration, Instant, Request, RequestEnvelope, Response, ResponseEnvelope};
use sim::{
    Clock, ClockConfig, Completion, Disk, DiskConfig, EventId, Fired, HostClock, MessageId, Net, NetConfig,
    Scheduler, Shape, Skew, Timer,
};
use std::cell::{Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::{self, Write as _};
use std::rc::Rc;

/// A disk operation the node asked for, waiting for the world to hand it
/// to the host's [`Disk`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageOp {
    Read { op: OpId },
    Write { op: OpId, offset: u64, bytes: Vec<u8> },
}

/// The node's side of the disk. Operations go out one queue and
/// completions come back in another. The world moves things between these
/// queues and the disk model, the same way it does for the network.
#[derive(Clone, Debug, Default)]
pub struct FakeStorage {
    pub ops: VecDeque<StorageOp>,
    completions: VecDeque<Input>,
}

impl FakeStorage {
    /// A disk operation finished; the node sees it on its next step.
    pub fn complete(&mut self, input: Input) {
        self.completions.push_back(input);
    }

    /// Completions the node has not seen yet. Non-zero only while the
    /// host is paused.
    pub fn completions_waiting(&self) -> usize {
        self.completions.len()
    }
}

impl Storage for FakeStorage {
    fn read_all(&mut self, op: OpId) {
        self.ops.push_back(StorageOp::Read { op });
    }

    fn append(&mut self, op: OpId, offset: u64, bytes: &[u8]) {
        self.ops.push_back(StorageOp::Write {
            op,
            offset,
            bytes: bytes.to_vec(),
        });
    }

    fn poll_completions(&mut self, inputs: &mut Vec<Input>) {
        inputs.extend(self.completions.drain(..));
    }
}

/// A node's doorstep: requests that arrived go in one queue, replies the
/// node wants sent come out another. The world moves things between these
/// queues and the network model.
#[derive(Clone, Debug, Default)]
pub struct FakeNetwork {
    inbox: VecDeque<RequestEnvelope>,
    pub outbox: VecDeque<ResponseEnvelope>,
}

impl FakeNetwork {
    pub fn deliver(&mut self, request: RequestEnvelope) {
        self.inbox.push_back(request);
    }

    /// Requests the node has not read yet. Non-zero only while the host
    /// is paused.
    pub fn inbox_len(&self) -> usize {
        self.inbox.len()
    }
}

impl Network for FakeNetwork {
    fn send(&mut self, reply: ResponseEnvelope) {
        self.outbox.push_back(reply);
    }

    fn poll_requests(&mut self, inputs: &mut Vec<Input>) {
        inputs.extend(self.inbox.drain(..).map(Input::Request));
    }
}

/// The one trace of the world, shared by every host's observer.
pub type SharedTrace = Rc<RefCell<Trace>>;

/// Records every input and output of one host into the shared trace.
///
/// The lines are stamped with true time from `clock`, not with the `now`
/// the driver passes in, which is the host's own clock. The world's lines
/// and the host's lines then share one timeline and the trace reads top to
/// bottom. The host's reading appears on its timer and resume lines.
#[derive(Debug)]
pub struct TraceObserver {
    pub node: NodeId,
    pub trace: SharedTrace,
    pub clock: Clock,
}

impl Observer for TraceObserver {
    fn input(&mut self, _local: Instant, input: &Input) {
        let n = self.node;
        let now = self.clock.now();
        let mut t = self.trace.borrow_mut();
        match input {
            Input::ReadComplete { op, data } => t.record(format_args!(
                "{now} n{n} in  ReadComplete op={op} bytes={}",
                data.len()
            )),
            Input::AppendComplete { op } => t.record(format_args!("{now} n{n} in  AppendComplete op={op}")),
            Input::Request(r) => t.record(format_args!(
                "{now} n{n} in  Request client={} id={} {:?}",
                r.client, r.request_id, r.request
            )),
        }
    }

    fn output(&mut self, _local: Instant, output: &Output) {
        let n = self.node;
        let now = self.clock.now();
        let mut t = self.trace.borrow_mut();
        match output {
            Output::Read { op } => t.record(format_args!("{now} n{n} out Read op={op}")),
            Output::Append { op, offset, bytes } => t.record(format_args!(
                "{now} n{n} out Append op={op} offset={offset} bytes={}",
                bytes.len()
            )),
            Output::Reply(r) => t.record(format_args!(
                "{now} n{n} out Reply client={} id={} {:?}",
                r.client, r.request_id, r.response
            )),
            Output::Halt { damage } => t.record(format_args!("{now} n{n} out Halt {damage}")),
        }
    }
}

pub type FakeDriver = Driver<HostClock, FakeStorage, FakeNetwork, TraceObserver>;

/// An address on the simulated network. Nodes and clients share it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Addr {
    Node(NodeId),
    Client(ClientId),
}

impl Addr {
    /// The node id, or a panic for a client.
    pub fn node(self) -> NodeId {
        match self {
            Addr::Node(n) => n,
            Addr::Client(c) => panic!("client {c} where a node was expected"),
        }
    }
}

impl fmt::Display for Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Addr::Node(n) => write!(f, "n{n}"),
            Addr::Client(c) => write!(f, "c{c}"),
        }
    }
}

/// Everything that travels on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Request(RequestEnvelope),
    Reply(ResponseEnvelope),
    Heartbeat(heartbeat::Message),
}

impl Message {
    /// Requests and replies, as opposed to heartbeats. The world is idle
    /// when none of these are on the wire.
    pub fn is_client_traffic(&self) -> bool {
        matches!(self, Message::Request(_) | Message::Reply(_))
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Message::Request(r) => write!(f, "request client={} id={}", r.client, r.request_id),
            Message::Reply(r) => write!(f, "reply client={} id={}", r.client, r.request_id),
            Message::Heartbeat(heartbeat::Message::Beat { seq }) => write!(f, "beat seq={seq}"),
        }
    }
}

/// Everything that can happen in the simulated world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A client's `next_deadline` arrived.
    ClientTimer { client: ClientId },
    /// A host's heartbeat `next_deadline` arrived, on the host's clock.
    NodeTimer { node: NodeId },
    /// One copy of a message reaches `to`, if the link is still up.
    Deliver {
        id: MessageId,
        from: Addr,
        to: Addr,
        message: Message,
    },
    /// The oldest pending operation on `node`'s disk is done. Stale if the
    /// process that submitted it has crashed since: `incarnation` says
    /// which process that was.
    DiskComplete { node: NodeId, incarnation: u64 },
    /// The seed cuts the network into a random shape.
    Partition,
    /// The seed heals it again.
    Heal,
    /// The seed kills a random host.
    Crash,
    /// The seed brings `node` back.
    Restart { node: NodeId },
    /// The seed stalls a random host.
    Pause,
    /// The seed lets `node` run again.
    Resume { node: NodeId },
    /// The seed makes a random host's clock leap forward.
    ClockJump,
    /// A client that was thinking is ready for its next operation.
    ClientWake { client: ClientId },
}

/// Seed-driven partitions. After each heal the next partition starts
/// within `gap_max`, and lasts up to `hold_max`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartitionSchedule {
    pub gap_max: Duration,
    pub hold_max: Duration,
}

impl fmt::Display for PartitionSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "every<={:?} for<={:?}", self.gap_max, self.hold_max)
    }
}

/// Seed-driven crashes. After each restart the next crash comes within
/// `gap_max`, and the host stays down for up to `down_max`. One host is
/// down at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrashSchedule {
    pub gap_max: Duration,
    pub down_max: Duration,
}

impl fmt::Display for CrashSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "every<={:?} down<={:?}", self.gap_max, self.down_max)
    }
}

/// Seed-driven pauses. After each resume the next pause comes within
/// `gap_max`, and lasts up to `hold_max`. One host is paused at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PauseSchedule {
    pub gap_max: Duration,
    pub hold_max: Duration,
}

impl fmt::Display for PauseSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "every<={:?} for<={:?}", self.gap_max, self.hold_max)
    }
}

/// The two BUGGIFY probabilities. `enable` is rolled once per site per
/// run; `fire` is rolled each time an enabled site is reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuggifyConfig {
    pub enable: Chance,
    pub fire: Chance,
}

impl Default for BuggifyConfig {
    /// FoundationDB's defaults.
    fn default() -> BuggifyConfig {
        BuggifyConfig {
            enable: Chance::percent(25),
            fire: Chance::percent(25),
        }
    }
}

impl fmt::Display for BuggifyConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "enable={} fire={}", self.enable, self.fire)
    }
}

/// One host, drawn from the seed, whose disk is `factor` times slower
/// than the others. A degraded drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlowHost {
    pub factor: u32,
}

impl fmt::Display for SlowHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "disk x{}", self.factor)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldConfig {
    pub seed: u64,
    /// The network fault model, for every message.
    pub net: NetConfig,
    /// The disk fault model, for every host that [`World::cluster`] builds.
    pub disk: DiskConfig,
    pub heartbeat: heartbeat::Config,
    /// Keep every trace line, or only the running hash. Turn this off for
    /// long runs; the hash is what the determinism check compares anyway.
    pub keep_trace_lines: bool,
    /// Random partitions, if any. Needs at least two hosts.
    pub partitions: Option<PartitionSchedule>,
    /// Random crashes, if any.
    pub crashes: Option<CrashSchedule>,
    /// Random pauses, if any.
    pub pauses: Option<PauseSchedule>,
    /// Per-host clock skew, drift and jumps, if any.
    pub clocks: Option<ClockConfig>,
    /// Fault points in the code under test, if any.
    pub buggify: Option<BuggifyConfig>,
    /// One slow disk, if any.
    pub slow_host: Option<SlowHost>,
    /// Bugs to plant in every node. Not a fault family: a test fixture,
    /// for proving that the checker finds them. Empty in every swarm.
    pub bugs: Vec<Bug>,
}

impl Default for WorldConfig {
    fn default() -> WorldConfig {
        WorldConfig {
            seed: 0,
            net: NetConfig::default(),
            disk: DiskConfig::default(),
            heartbeat: heartbeat::Config::default(),
            keep_trace_lines: true,
            partitions: None,
            crashes: None,
            pauses: None,
            clocks: None,
            buggify: None,
            slow_host: None,
            bugs: Vec::new(),
        }
    }
}

impl WorldConfig {
    /// Swarm testing: each fault family is on or off by a coin, and the
    /// ones that are on draw their own parameters. No two seeds run the
    /// same configuration, and no family is in every run, so a family that
    /// masks another's bugs is absent from half the swarm.
    ///
    /// The draws happen in a fixed order. Adding a family at the end keeps
    /// the meaning of every earlier draw.
    pub fn swarm(prng: &mut Prng) -> WorldConfig {
        let ms = Duration::from_millis;
        let seed = prng.next_u64();
        let net = if prng.chance(1, 2) {
            NetConfig::random(prng)
        } else {
            NetConfig::default()
        };
        let disk = if prng.chance(1, 2) {
            DiskConfig::random(prng)
        } else {
            DiskConfig::default()
        };
        let partitions = if prng.chance(1, 2) {
            Some(PartitionSchedule {
                gap_max: ms(prng.range(100, 3000)),
                hold_max: ms(prng.range(50, 1500)),
            })
        } else {
            None
        };
        let crashes = if prng.chance(1, 2) {
            Some(CrashSchedule {
                gap_max: ms(prng.range(200, 3000)),
                down_max: ms(prng.range(50, 800)),
            })
        } else {
            None
        };
        let pauses = if prng.chance(1, 2) {
            Some(PauseSchedule {
                gap_max: ms(prng.range(100, 2000)),
                hold_max: ms(prng.range(10, 600)),
            })
        } else {
            None
        };
        let clocks = if prng.chance(1, 2) {
            Some(ClockConfig::random(prng))
        } else {
            None
        };
        let buggify = if prng.chance(1, 2) {
            Some(BuggifyConfig {
                enable: Chance::percent(prng.range(10, 61) as u32),
                fire: Chance::percent(prng.range(5, 31) as u32),
            })
        } else {
            None
        };
        let slow_host = if prng.chance(1, 2) {
            Some(SlowHost {
                factor: prng.range(2, 11) as u32,
            })
        } else {
            None
        };
        WorldConfig {
            seed,
            net,
            disk,
            partitions,
            crashes,
            pauses,
            clocks,
            buggify,
            slow_host,
            ..WorldConfig::default()
        }
    }

    /// Which fault families are on, in the order of [`WorldConfig::swarm`].
    /// For tests that check the swarm covers every subset.
    pub fn families(&self) -> [bool; 8] {
        [
            !self.net.is_reliable(),
            !self.disk.is_reliable(),
            self.partitions.is_some(),
            self.crashes.is_some(),
            self.pauses.is_some(),
            self.clocks.is_some(),
            self.buggify.is_some(),
            self.slow_host.is_some(),
        ]
    }
}

impl fmt::Display for WorldConfig {
    /// One line, the head of every trace.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn opt<T: fmt::Display>(f: &mut fmt::Formatter<'_>, name: &str, v: &Option<T>) -> fmt::Result {
            match v {
                Some(v) => write!(f, " {name}=[{v}]"),
                None => write!(f, " {name}=none"),
            }
        }
        write!(
            f,
            "seed={} net=[{}] disk=[{}] heartbeat=[interval={:?} suspect_after={:?}]",
            self.seed, self.net, self.disk, self.heartbeat.interval, self.heartbeat.suspect_after
        )?;
        opt(f, "partitions", &self.partitions)?;
        opt(f, "crashes", &self.crashes)?;
        opt(f, "pauses", &self.pauses)?;
        opt(f, "clocks", &self.clocks)?;
        opt(f, "buggify", &self.buggify)?;
        opt(f, "slow_host", &self.slow_host)?;
        // Only when planted, so that a run without bugs has the trace it
        // had before Module 8.
        if !self.bugs.is_empty() {
            write!(f, " bugs=[")?;
            for (i, b) in self.bugs.iter().enumerate() {
                write!(f, "{}{b}", if i > 0 { " " } else { "" })?;
            }
            write!(f, "]")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldStats {
    pub client_timers_fired: u64,
    pub node_timers_fired: u64,
    pub requests_delivered: u64,
    pub replies_delivered: u64,
    pub beats_delivered: u64,
    /// Messages that arrived at a host that was down.
    pub dropped_at_down_hosts: u64,
    /// Beats that arrived at a paused host and waited for it.
    pub held_beats: u64,
    pub reads_completed: u64,
    pub appends_completed: u64,
    /// Disk completions for a process that had crashed by then.
    pub stale_disk_events: u64,
    pub partitions: u64,
    pub heals: u64,
    pub crashes: u64,
    pub restarts: u64,
    /// Processes that stopped themselves over a damaged log.
    pub halts: u64,
    pub pauses: u64,
    pub resumes: u64,
    pub clock_jumps: u64,
    /// Fault points that fired, across every host.
    pub buggify_fires: u64,
}

/// What came of one [`World::run_workload`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkloadRun {
    /// Requests submitted.
    pub submitted: u64,
    /// Of those, answered.
    pub completed: u64,
    /// Of those, given up on. Their fate is unknown.
    pub gave_up: u64,
    /// Clients whose host halted over a damaged log. Nothing will ever
    /// answer them, so they were given nothing more to do.
    pub stranded: u64,
}

/// A running process on a host: the KV node and its heartbeat.
pub struct Process {
    pub driver: FakeDriver,
    pub heartbeat: Heartbeat,
}

/// One simulated machine. The disk and the clock outlive the process.
pub struct Host {
    pub id: NodeId,
    pub disk: Disk,
    /// What this machine's clock reads. The process reads it; the disk
    /// and the network do not.
    pub clock: HostClock,
    /// `None` while the host is down.
    pub process: Option<Process>,
    /// Why the process last halted, if it ever did.
    pub last_halt: Option<Damage>,
    /// Sectors the disk corrupted in the most recent read. A checker can
    /// ask whether recovery accepted any of them.
    pub last_read_faults: Vec<usize>,
    /// Starts so far. A disk completion carries the incarnation of the
    /// process that asked for it.
    incarnation: u64,
    /// Up, but not running. Inputs queue until the resume.
    paused: bool,
    /// Beats that arrived during the pause, in arrival order.
    held_beats: Vec<(NodeId, heartbeat::Message)>,
    timer: Timer,
}

impl Host {
    pub fn is_up(&self) -> bool {
        self.process.is_some()
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn incarnation(&self) -> u64 {
        self.incarnation
    }

    /// What this host's clock reads now.
    pub fn local_now(&self) -> Instant {
        self.clock.now()
    }

    /// The running process. Panics if the host is down.
    pub fn process(&self) -> &Process {
        self.process
            .as_ref()
            .unwrap_or_else(|| panic!("node {} is down", self.id))
    }

    pub fn node(&self) -> &Node {
        &self.process().driver.node
    }

    pub fn heartbeat(&self) -> &Heartbeat {
        &self.process().heartbeat
    }
}

/// Some hosts and some clients, wired together by the scheduler, the
/// network model and one disk model per host.
pub struct World {
    /// Indexed by `NodeId`.
    pub hosts: Vec<Host>,
    pub net: Net<Addr>,
    pub scheduler: Scheduler<Event>,
    pub prng: Prng,
    pub config: WorldConfig,
    pub clients: BTreeMap<ClientId, Client>,
    /// Which node each client talks to.
    targets: BTreeMap<ClientId, NodeId>,
    client_timers: BTreeMap<ClientId, Timer>,
    /// Results delivered to clients: (client, request_id, response).
    pub completed: Vec<(ClientId, u64, Response)>,
    pub gave_up: Vec<(ClientId, u64)>,
    /// Every call and return, for the checker.
    pub history: History<Request, Response>,
    /// Clients between operations, waiting for a `ClientWake`.
    thinking: BTreeSet<ClientId>,
    pub stats: WorldStats,
    trace: SharedTrace,
    /// Scheduled `Deliver` events that carry a request or a reply.
    client_messages_in_flight: usize,
    /// The pending `Partition` or `Heal` event, if a schedule is running.
    partition_event: Option<EventId>,
    /// The pending `Crash` or `Restart` event, if a schedule is running.
    crash_event: Option<EventId>,
    /// The pending `Pause` or `Resume` event, if a schedule is running.
    pause_event: Option<EventId>,
    /// The pending `ClockJump`, if a schedule is running.
    clock_event: Option<EventId>,
    /// The fault points enabled for this run. Every process gets them.
    buggify_sites: Vec<Site>,
    /// The host with the slow disk, if any.
    slow_host: Option<NodeId>,
    client_out: Vec<client::Output>,
    host_out: Vec<heartbeat::Output>,
}

impl World {
    /// One host on `disk`, default configuration.
    pub fn new(disk: Disk) -> World {
        World::with_disks(vec![disk], WorldConfig::default())
    }

    /// `nodes` hosts with empty disks built from `config.disk`.
    pub fn cluster(nodes: u8, config: WorldConfig) -> World {
        let disks = (0..nodes).map(|_| Disk::new(config.disk.clone())).collect();
        World::with_disks(disks, config)
    }

    /// One host per disk. Each host starts its process, which asks the
    /// disk for its log. The read completes after the disk's latency, so
    /// the nodes are still recovering when this returns.
    pub fn with_disks(disks: Vec<Disk>, config: WorldConfig) -> World {
        assert!(!disks.is_empty(), "a world needs at least one host");
        assert!(
            disks.len() <= usize::from(NodeId::MAX),
            "too many hosts for a NodeId"
        );
        config.net.validate();
        config.disk.validate();
        if let Some(clocks) = &config.clocks {
            clocks.validate();
        }
        let trace = if config.keep_trace_lines {
            Trace::new()
        } else {
            Trace::hash_only()
        };
        let mut world = World {
            hosts: Vec::new(),
            net: Net::new(config.net.clone()),
            scheduler: Scheduler::new(),
            prng: Prng::from_seed(config.seed),
            config,
            clients: BTreeMap::new(),
            targets: BTreeMap::new(),
            client_timers: BTreeMap::new(),
            completed: Vec::new(),
            gave_up: Vec::new(),
            history: History::new(),
            thinking: BTreeSet::new(),
            stats: WorldStats::default(),
            trace: Rc::new(RefCell::new(trace)),
            client_messages_in_flight: 0,
            partition_event: None,
            crash_event: None,
            pause_event: None,
            clock_event: None,
            buggify_sites: Vec::new(),
            slow_host: None,
            client_out: Vec::new(),
            host_out: Vec::new(),
        };
        // The per-run draws, in a fixed order: clocks, slow disk, sites.
        // Each spends randomness only when its family is on.
        for (id, disk) in (0..).zip(disks) {
            let skew = match &world.config.clocks {
                Some(clocks) => clocks.draw_skew(&mut world.prng),
                None => Skew::IDEAL,
            };
            world.hosts.push(Host {
                id,
                disk,
                clock: HostClock::new(world.scheduler.clock(), skew),
                process: None,
                last_halt: None,
                last_read_faults: Vec::new(),
                incarnation: 0,
                paused: false,
                held_beats: Vec::new(),
                timer: Timer::new(),
            });
        }
        if let Some(slow) = &world.config.slow_host {
            let node = world.prng.below(world.hosts.len() as u64) as NodeId;
            let disk = &mut world.hosts[usize::from(node)].disk;
            let mut c = disk.config().clone();
            c.read_latency_min *= slow.factor;
            c.read_latency_max *= slow.factor;
            c.write_latency_min *= slow.factor;
            c.write_latency_max *= slow.factor;
            disk.set_config(c);
            world.slow_host = Some(node);
        }
        if let Some(buggify) = &world.config.buggify {
            world.buggify_sites = Buggify::draw_sites(buggify.enable, &mut world.prng);
        }
        world.log_config();
        for id in 0..world.hosts.len() as u8 {
            world.start(id);
        }
        if world.config.partitions.is_some() {
            assert!(world.hosts.len() >= 2, "partitions need at least two hosts");
            world.schedule_partition();
        }
        if world.config.crashes.is_some() {
            world.schedule_crash();
        }
        if world.config.pauses.is_some() {
            world.schedule_pause();
        }
        if world.config.clocks.as_ref().is_some_and(ClockConfig::has_jumps) {
            world.schedule_clock_jump();
        }
        world
    }

    /// The first two lines of every trace: the configuration the run was
    /// given, and what the seed drew from it.
    fn log_config(&self) {
        let now = self.now();
        self.log(format_args!("{now} config {}", self.config));
        let mut drawn = format!("{now} config drawn:");
        if self.config.clocks.is_some() {
            for h in &self.hosts {
                write!(drawn, " n{}=[{}]", h.id, h.clock.skew()).unwrap();
            }
        }
        if let Some(n) = self.slow_host {
            write!(drawn, " slow_host=n{n}").unwrap();
        }
        if self.config.buggify.is_some() {
            write!(drawn, " buggify_sites=[").unwrap();
            for (i, s) in self.buggify_sites.iter().enumerate() {
                write!(drawn, "{}{s}", if i > 0 { " " } else { "" }).unwrap();
            }
            write!(drawn, "]").unwrap();
        }
        self.log(format_args!("{drawn}"));
    }

    /// Add a client that sends its requests to `node`.
    pub fn add_client(&mut self, id: ClientId, config: client::Config, node: NodeId) {
        assert!(
            usize::from(node) < self.hosts.len(),
            "client {id} targets unknown node {node}"
        );
        let prev = self.clients.insert(id, Client::new(id, config));
        assert!(prev.is_none(), "duplicate client id {id}");
        self.targets.insert(id, node);
        self.client_timers.insert(id, Timer::new());
    }

    pub fn now(&self) -> Instant {
        self.scheduler.now()
    }

    /// What `node`'s clock reads now.
    pub fn local_now(&self, node: NodeId) -> Instant {
        self.hosts[usize::from(node)].local_now()
    }

    pub fn trace(&self) -> Ref<'_, Trace> {
        self.trace.borrow()
    }

    /// Addresses of every host, in id order.
    pub fn node_addrs(&self) -> Vec<Addr> {
        self.hosts.iter().map(|h| Addr::Node(h.id)).collect()
    }

    pub fn is_up(&self, node: NodeId) -> bool {
        self.hosts[usize::from(node)].is_up()
    }

    pub fn is_paused(&self, node: NodeId) -> bool {
        self.hosts[usize::from(node)].is_paused()
    }

    /// The KV node on `node`. Panics if the host is down.
    pub fn node(&self, node: NodeId) -> &Node {
        self.hosts[usize::from(node)].node()
    }

    /// The heartbeat on `node`. Panics if the host is down.
    pub fn heartbeat(&self, node: NodeId) -> &Heartbeat {
        self.hosts[usize::from(node)].heartbeat()
    }

    pub fn disk(&self, node: NodeId) -> &Disk {
        &self.hosts[usize::from(node)].disk
    }

    /// The fault points enabled for this run.
    pub fn buggify_sites(&self) -> &[Site] {
        &self.buggify_sites
    }

    /// The host whose disk the seed slowed down, if any.
    pub fn slow_host(&self) -> Option<NodeId> {
        self.slow_host
    }

    /// The node that client `id` talks to.
    pub fn target(&self, id: ClientId) -> NodeId {
        self.targets[&id]
    }

    /// The history of the clients that talk to `node`. Until Module 9
    /// replicates, each node is a store of its own and is checked alone.
    pub fn history_of(&self, node: NodeId) -> History<Request, Response> {
        self.history.filter(|op| self.targets[&op.client] == node)
    }

    /// Is client `id` between operations?
    pub fn is_thinking(&self, id: ClientId) -> bool {
        self.thinking.contains(&id)
    }

    /// Nothing is happening except heartbeats: every client is idle, no
    /// request or reply is on the wire, and no disk has work queued.
    pub fn is_idle(&self) -> bool {
        self.clients.values().all(Client::is_idle)
            && self.client_messages_in_flight == 0
            && self.hosts.iter().all(|h| h.disk.pending() == 0)
    }

    /// Client `id` starts `request`. Returns its request id.
    pub fn submit(&mut self, id: ClientId, request: Request) -> u64 {
        let now = self.now();
        assert!(!self.thinking.contains(&id), "client {id} is thinking");
        let client = self.clients.get_mut(&id).expect("unknown client");
        let rid = client.submit(now, request.clone(), &mut self.client_out);
        self.history.call(id, rid, request, now);
        self.route_client(id);
        self.arm_client_timer(id);
        rid
    }

    /// Client `id` does nothing for `d`, then is ready for its next
    /// operation.
    pub fn think(&mut self, id: ClientId, d: Duration) {
        let now = self.now();
        assert!(
            self.clients[&id].is_idle(),
            "client {id} thinks with a request in flight"
        );
        assert!(self.thinking.insert(id), "client {id} is already thinking");
        self.log(format_args!("{now} client={id} think for={d:?}"));
        self.scheduler.schedule_after(d, Event::ClientWake { client: id });
    }

    /// Drive `workload` across every client until `requests` have been
    /// submitted and answered or given up on. Each client that is idle
    /// gets its next step at once, so the number of operations in flight
    /// is the number of clients, less those thinking. A client whose host
    /// halted is left alone: nothing will ever answer it.
    pub fn run_workload(&mut self, workload: &mut Workload, requests: u64) -> WorkloadRun {
        let now = self.now();
        self.log(format_args!(
            "{now} workload clients={} requests={requests} {}",
            self.clients.len(),
            workload.config()
        ));
        let start = self.history.len();
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        let mut run = WorkloadRun::default();
        loop {
            run.stranded = 0;
            for &id in &ids {
                if run.submitted >= requests {
                    break;
                }
                if !self.clients[&id].is_idle() || self.thinking.contains(&id) {
                    continue;
                }
                let host = &self.hosts[usize::from(self.targets[&id])];
                if !host.is_up() && host.last_halt.is_some() {
                    run.stranded += 1;
                    continue;
                }
                match workload.next(id) {
                    Step::Request(request) => {
                        self.submit(id, request);
                        run.submitted += 1;
                    }
                    Step::Think(d) => self.think(id, d),
                }
            }
            let busy = ids
                .iter()
                .any(|id| !self.clients[id].is_idle() || self.thinking.contains(id));
            if !busy {
                // Every client is idle and was offered a step. Either the
                // budget is spent or every idle client is stranded.
                break;
            }
            assert!(self.step(), "the queue ran dry with a request in flight");
        }
        for op in &self.history.ops()[start..] {
            if op.is_complete() {
                run.completed += 1;
            } else {
                run.gave_up += 1;
            }
        }
        run
    }

    /// Pop and handle one event. Returns false if the queue is empty.
    pub fn step(&mut self) -> bool {
        let Some(fired) = self.scheduler.pop() else {
            return false;
        };
        self.handle(fired);
        true
    }

    /// Handle events until [`World::is_idle`]. Heartbeats keep the queue
    /// busy forever, so "until the queue is empty" is not a useful stop. A
    /// client that never gives up and never gets an answer never goes idle;
    /// bound such runs with [`World::run_until`] instead.
    pub fn run_until_idle(&mut self) {
        while !self.is_idle() {
            assert!(self.step(), "the queue ran dry while the world was busy");
        }
    }

    /// Handle every event due at or before `until`, then set the clock to
    /// `until`.
    pub fn run_until(&mut self, until: Instant) {
        while let Some(fired) = self.scheduler.pop_until(until) {
            self.handle(fired);
        }
        self.scheduler.advance_to(until);
    }

    /// Run for `d` of virtual time. Calling this in a loop with a fixed `d`
    /// is the tick model; one call with a large `d` is the event model.
    pub fn run_for(&mut self, d: Duration) {
        self.run_until(self.now() + d);
    }

    /// Handle events while `keep_going` holds. Returns false if the queue
    /// ran dry first.
    pub fn run_while(&mut self, mut keep_going: impl FnMut(&World) -> bool) -> bool {
        while keep_going(self) {
            if !self.step() {
                return false;
            }
        }
        true
    }

    /// Submit, run until the request completes, and return the response.
    /// Panics if the client gave up or the world went idle first.
    pub fn call(&mut self, id: ClientId, request: Request) -> Response {
        let rid = self.submit(id, request);
        let finished = |w: &World| {
            w.completed.iter().any(|(c, r, _)| (*c, *r) == (id, rid)) || w.gave_up.contains(&(id, rid))
        };
        let ran = self.run_while(|w| !finished(w));
        assert!(ran, "world went idle before client {id} request {rid} completed");
        self.take_completed(id, rid)
            .unwrap_or_else(|| panic!("client {id} gave up on request {rid}"))
    }

    pub fn take_completed(&mut self, id: ClientId, rid: u64) -> Option<Response> {
        let i = self
            .completed
            .iter()
            .position(|(c, r, _)| (*c, *r) == (id, rid))?;
        Some(self.completed.remove(i).2)
    }

    // ---- faults by hand ----------------------------------------------------

    /// Kill the process on `node`. Its memory is gone. The disk resolves
    /// the operations that were in flight, as the fault model decides.
    pub fn crash(&mut self, node: NodeId) {
        let now = self.now();
        let host = &mut self.hosts[usize::from(node)];
        assert!(host.process.take().is_some(), "node {node} is already down");
        host.paused = false;
        host.held_beats.clear();
        let report = host.disk.crash(&mut self.prng);
        host.timer
            .reconcile(&mut self.scheduler, None, || Event::NodeTimer { node });
        self.stats.crashes += 1;
        self.log(format_args!("{now} n{node} crash {report}"));
    }

    /// Start a new process on `node`. It knows nothing but what the disk
    /// tells it.
    pub fn restart(&mut self, node: NodeId) {
        let now = self.now();
        assert!(!self.is_up(node), "node {node} is already up");
        self.stats.restarts += 1;
        self.log(format_args!("{now} n{node} restart"));
        self.start(node);
    }

    /// Stall the process on `node`. It keeps its memory and handles
    /// nothing until [`World::resume`]. Requests, beats and disk
    /// completions wait for it. A garbage collector, a stopped VM, a
    /// `SIGSTOP`.
    pub fn pause(&mut self, node: NodeId) {
        let now = self.now();
        let host = &mut self.hosts[usize::from(node)];
        assert!(host.is_up(), "node {node} is down, nothing to pause");
        assert!(!host.paused, "node {node} is already paused");
        host.paused = true;
        host.timer
            .reconcile(&mut self.scheduler, None, || Event::NodeTimer { node });
        self.stats.pauses += 1;
        self.log(format_args!("{now} n{node} pause"));
    }

    /// Let the process on `node` run again. It handles everything that
    /// queued, at the time its clock reads now, then ticks its heartbeat.
    pub fn resume(&mut self, node: NodeId) {
        let now = self.now();
        let host = &mut self.hosts[usize::from(node)];
        assert!(host.paused, "node {node} is not paused");
        assert!(host.is_up(), "a paused host is up");
        host.paused = false;
        let local = host.clock.now();
        let held = std::mem::take(&mut host.held_beats);
        self.stats.resumes += 1;
        self.log(format_args!(
            "{now} n{node} resume held_beats={} local={local}",
            held.len()
        ));
        // The timer that expired during the stall first, then the buffered
        // beats. The process notices that time passed before it reads what
        // arrived meanwhile, so for a moment it believes every peer went
        // silent. A real event loop could do it the other way round; the
        // order is a modelling decision and the lesson says so.
        self.pump(node);
        let process = self.hosts[usize::from(node)].process.as_mut().expect("still up");
        process
            .heartbeat
            .on_input(local, heartbeat::Input::Tick, &mut self.host_out);
        for (from, message) in held {
            process.heartbeat.on_input(
                local,
                heartbeat::Input::Message { from, message },
                &mut self.host_out,
            );
        }
        self.route_host(node);
        self.arm_host_timer(node);
    }

    /// Move `node`'s clock forward by `by`, now. The process sees time
    /// leap: every deadline that fell inside the leap is due at once.
    pub fn jump_clock(&mut self, node: NodeId, by: Duration) {
        let now = self.now();
        let host = &mut self.hosts[usize::from(node)];
        host.clock.jump(by);
        let local = host.clock.now();
        self.stats.clock_jumps += 1;
        self.log(format_args!("{now} n{node} clock jump by={by:?} local={local}"));
        let host = &mut self.hosts[usize::from(node)];
        if let Some(process) = host.process.as_mut().filter(|_| !host.paused) {
            process
                .heartbeat
                .on_input(local, heartbeat::Input::Tick, &mut self.host_out);
            self.route_host(node);
            self.arm_host_timer(node);
        }
    }

    // ---- the quiet period --------------------------------------------------

    /// End the partition schedule and reconnect every link, including any
    /// cut by hand. This is the start of the quiet period in which the
    /// cluster must recover.
    pub fn stop_partitions(&mut self) {
        if let Some(id) = self.partition_event.take() {
            self.scheduler.cancel(id);
        }
        self.net.heal_all();
        let now = self.now();
        self.log(format_args!("{now} net heal (partitions stopped)"));
    }

    /// End the crash schedule and bring every host up, including any
    /// crashed by hand or halted over a damaged log. A halted host reads
    /// its disk again and may halt again.
    pub fn stop_crashes(&mut self) {
        if let Some(id) = self.crash_event.take() {
            self.scheduler.cancel(id);
        }
        let now = self.now();
        self.log(format_args!("{now} crashes stopped"));
        for node in 0..self.hosts.len() as u8 {
            if !self.is_up(node) {
                self.restart(node);
            }
        }
    }

    /// End the pause schedule and resume every paused host.
    pub fn stop_pauses(&mut self) {
        if let Some(id) = self.pause_event.take() {
            self.scheduler.cancel(id);
        }
        let now = self.now();
        self.log(format_args!("{now} pauses stopped"));
        for node in 0..self.hosts.len() as u8 {
            if self.is_paused(node) {
                self.resume(node);
            }
        }
    }

    /// End the clock jumps. Drift and offsets stay: they are not faults a
    /// cluster recovers from, they are the weather.
    pub fn stop_clock_jumps(&mut self) {
        if let Some(id) = self.clock_event.take() {
            self.scheduler.cancel(id);
        }
        let now = self.now();
        self.log(format_args!("{now} clock jumps stopped"));
    }

    /// Turn every fault off at once: schedules cancelled, links healed,
    /// hosts up and running, the network and every disk reliable at their
    /// minimum latencies, no fault point enabled. What follows is the
    /// quiet period, and the liveness check is whether the cluster comes
    /// back in it.
    pub fn stop_faults(&mut self) {
        let now = self.now();
        self.log(format_args!("{now} faults stopped"));
        self.stop_partitions();
        self.stop_clock_jumps();
        self.stop_pauses();
        self.stop_crashes();
        let latency = self.net.config().latency_min;
        self.net.set_config(NetConfig::reliable(latency));
        self.buggify_sites.clear();
        for host in &mut self.hosts {
            let c = host.disk.config();
            host.disk
                .set_config(DiskConfig::reliable(c.read_latency_min, c.write_latency_min));
            if let Some(p) = host.process.as_mut() {
                p.driver.node.buggify_mut().disable_all();
                p.heartbeat.buggify_mut().disable_all();
            }
        }
    }

    fn log(&self, args: fmt::Arguments<'_>) {
        self.trace.borrow_mut().record(args);
    }

    /// Fault points for one new state machine: this run's sites, the
    /// configured firing rate, and dice forked from the world's.
    fn new_buggify(&mut self) -> Buggify {
        match &self.config.buggify {
            Some(b) => Buggify::new(&self.buggify_sites, b.fire, self.prng.fork()),
            None => Buggify::off(),
        }
    }

    /// Build the process for `node` and let it ask for its log.
    fn start(&mut self, node: NodeId) {
        let host = &self.hosts[usize::from(node)];
        assert!(host.process.is_none());
        assert!(!host.paused);
        let local = host.clock.now();
        let node_buggify = self.new_buggify();
        let heartbeat_buggify = self.new_buggify();
        let host = &mut self.hosts[usize::from(node)];
        host.incarnation += 1;
        let mut driver = Driver::with_buggify(
            node_buggify,
            host.clock.clone(),
            FakeStorage::default(),
            FakeNetwork::default(),
            TraceObserver {
                node,
                trace: self.trace.clone(),
                clock: self.scheduler.clock(),
            },
        );
        for &bug in &self.config.bugs {
            driver.node.bugs_mut().plant(bug);
        }
        let peers: Vec<NodeId> = (0..self.hosts.len() as u8).filter(|&p| p != node).collect();
        let heartbeat = Heartbeat::with_buggify(
            heartbeat_buggify,
            node,
            &peers,
            self.config.heartbeat,
            local,
            &mut self.host_out,
        );
        self.hosts[usize::from(node)].process = Some(Process { driver, heartbeat });
        self.pump(node);
        self.route_host(node);
        self.arm_host_timer(node);
    }

    fn handle(&mut self, fired: Fired<Event>) {
        let now = fired.at;
        debug_assert_eq!(now, self.now());
        match fired.event {
            Event::ClientTimer { client: id } => {
                let timer = self.client_timers.get_mut(&id).expect("timer for unknown client");
                assert!(timer.fired(fired.id), "stale timer event for client {id}");
                self.stats.client_timers_fired += 1;
                self.log(format_args!("{now} timer client={id}"));
                self.clients
                    .get_mut(&id)
                    .unwrap()
                    .on_input(client::Input::Tick { now }, &mut self.client_out);
                self.route_client(id);
                self.arm_client_timer(id);
            }
            Event::NodeTimer { node } => {
                self.stats.node_timers_fired += 1;
                let host = &mut self.hosts[usize::from(node)];
                assert!(host.timer.fired(fired.id), "stale timer event for node {node}");
                assert!(!host.paused, "a paused host has no timer");
                let local = host.clock.now();
                self.log(format_args!("{now} timer node={node} local={local}"));
                let host = &mut self.hosts[usize::from(node)];
                host.process
                    .as_mut()
                    .expect("a down host has no timer")
                    .heartbeat
                    .on_input(local, heartbeat::Input::Tick, &mut self.host_out);
                self.route_host(node);
                self.arm_host_timer(node);
            }
            Event::Deliver {
                id,
                from,
                to,
                message,
            } => {
                if message.is_client_traffic() {
                    self.client_messages_in_flight -= 1;
                }
                match self.net.deliver(id) {
                    Err(reason) => {
                        self.log(format_args!(
                            "{now} net dropped {from}->{to} {id} {message} {reason}"
                        ));
                    }
                    Ok(endpoints) => {
                        assert_eq!(endpoints, (from, to), "the ledger disagrees with the event");
                        self.log(format_args!("{now} net deliver {from}->{to} {id} {message}"));
                        self.receive(now, from, to, message);
                    }
                }
            }
            Event::DiskComplete { node, incarnation } => {
                let host = &mut self.hosts[usize::from(node)];
                if host.incarnation != incarnation || host.process.is_none() {
                    // The process that asked for this is gone, and the
                    // crash already settled the operation.
                    self.stats.stale_disk_events += 1;
                    return;
                }
                // The device finishes whether or not the process is
                // paused. The completion waits in the storage queue.
                let completion = host.disk.complete_next(&mut self.prng);
                let lines = match completion {
                    Completion::Read { op, data, faults } => {
                        self.stats.reads_completed += 1;
                        let mut lines = vec![format!(
                            "{now} n{node} disk read complete op={op} bytes={}",
                            data.len()
                        )];
                        lines.extend(
                            faults
                                .iter()
                                .map(|s| format!("{now} n{node} disk read fault sector={s}")),
                        );
                        host.last_read_faults = faults;
                        host.process
                            .as_mut()
                            .expect("checked above")
                            .driver
                            .storage
                            .complete(Input::ReadComplete { op, data });
                        lines
                    }
                    Completion::Write { op, fate, faults } => {
                        self.stats.appends_completed += 1;
                        let mut lines = vec![format!("{now} n{node} disk write complete op={op} {fate}")];
                        lines.extend(
                            faults
                                .iter()
                                .map(|s| format!("{now} n{node} disk corrupt sector={s}")),
                        );
                        host.process
                            .as_mut()
                            .expect("checked above")
                            .driver
                            .storage
                            .complete(Input::AppendComplete { op });
                        lines
                    }
                };
                for line in lines {
                    self.log(format_args!("{line}"));
                }
                self.pump(node);
            }
            Event::Partition => {
                assert_eq!(self.partition_event, Some(fired.id), "stale partition event");
                let nodes = self.node_addrs();
                let shape = Shape::random(&mut self.prng, &nodes);
                self.net.heal_all();
                self.net.apply(&shape);
                self.stats.partitions += 1;
                self.log(format_args!("{now} net partition {shape}"));
                let hold_max = self.config.partitions.as_ref().expect("schedule").hold_max;
                let hold = self.jitter(hold_max);
                self.partition_event = Some(self.scheduler.schedule_after(hold, Event::Heal));
            }
            Event::Heal => {
                assert_eq!(self.partition_event, Some(fired.id), "stale heal event");
                self.net.heal_all();
                self.stats.heals += 1;
                self.log(format_args!("{now} net heal"));
                self.schedule_partition();
            }
            Event::Crash => {
                assert_eq!(self.crash_event, Some(fired.id), "stale crash event");
                let up: Vec<NodeId> = self.hosts.iter().filter(|h| h.is_up()).map(|h| h.id).collect();
                if up.is_empty() {
                    // Every host was crashed by hand. Try again later.
                    self.schedule_crash();
                    return;
                }
                let node = *self.prng.pick(&up);
                self.crash(node);
                let down_max = self.config.crashes.as_ref().expect("schedule").down_max;
                let down = self.jitter(down_max);
                self.crash_event = Some(self.scheduler.schedule_after(down, Event::Restart { node }));
            }
            Event::Restart { node } => {
                assert_eq!(self.crash_event, Some(fired.id), "stale restart event");
                if !self.is_up(node) {
                    self.restart(node);
                }
                self.schedule_crash();
            }
            Event::Pause => {
                assert_eq!(self.pause_event, Some(fired.id), "stale pause event");
                let running: Vec<NodeId> = self
                    .hosts
                    .iter()
                    .filter(|h| h.is_up() && !h.paused)
                    .map(|h| h.id)
                    .collect();
                if running.is_empty() {
                    self.schedule_pause();
                    return;
                }
                let node = *self.prng.pick(&running);
                self.pause(node);
                let hold_max = self.config.pauses.as_ref().expect("schedule").hold_max;
                let hold = self.jitter(hold_max);
                self.pause_event = Some(self.scheduler.schedule_after(hold, Event::Resume { node }));
            }
            Event::Resume { node } => {
                assert_eq!(self.pause_event, Some(fired.id), "stale resume event");
                // A crash in the meantime ended the pause already.
                if self.is_paused(node) {
                    self.resume(node);
                }
                self.schedule_pause();
            }
            Event::ClockJump => {
                assert_eq!(self.clock_event, Some(fired.id), "stale clock event");
                let node = self.prng.below(self.hosts.len() as u64) as NodeId;
                let jump_max = self.config.clocks.as_ref().expect("schedule").jump_max;
                let by = self.jitter(jump_max);
                self.jump_clock(node, by);
                self.schedule_clock_jump();
            }
            Event::ClientWake { client } => {
                assert!(self.thinking.remove(&client), "client {client} was not thinking");
                // `run_workload` offers the client its next step on the
                // next pass. Nothing to do here.
            }
        }
    }

    /// Hand a delivered message to its destination.
    fn receive(&mut self, now: Instant, from: Addr, to: Addr, message: Message) {
        match (to, message) {
            (Addr::Node(n), message) if !self.is_up(n) => {
                // The wire is fine; nobody is listening.
                self.stats.dropped_at_down_hosts += 1;
                self.log(format_args!("{now} n{n} down, dropped {message}"));
            }
            (Addr::Node(n), Message::Request(envelope)) => {
                // Into the doorstep queue. A paused host reads it later.
                self.stats.requests_delivered += 1;
                self.hosts[usize::from(n)]
                    .process
                    .as_mut()
                    .expect("checked above")
                    .driver
                    .network
                    .deliver(envelope);
                self.pump(n);
            }
            (Addr::Node(n), Message::Heartbeat(message)) if self.is_paused(n) => {
                self.stats.beats_delivered += 1;
                self.stats.held_beats += 1;
                self.hosts[usize::from(n)].held_beats.push((from.node(), message));
                self.log(format_args!(
                    "{now} n{n} paused, held {}",
                    Message::Heartbeat(message)
                ));
            }
            (Addr::Node(n), Message::Heartbeat(message)) => {
                self.stats.beats_delivered += 1;
                let input = heartbeat::Input::Message {
                    from: from.node(),
                    message,
                };
                let host = &mut self.hosts[usize::from(n)];
                let local = host.clock.now();
                host.process.as_mut().expect("checked above").heartbeat.on_input(
                    local,
                    input,
                    &mut self.host_out,
                );
                self.route_host(n);
                self.arm_host_timer(n);
            }
            (Addr::Client(c), Message::Reply(envelope)) => {
                self.stats.replies_delivered += 1;
                let client = self.clients.get_mut(&c).expect("reply for unknown client");
                client.on_input(client::Input::Response(envelope), &mut self.client_out);
                self.route_client(c);
                self.arm_client_timer(c);
            }
            (to, message) => panic!("{message} delivered to {to}, which cannot receive it"),
        }
    }

    /// Let a node process everything it has, then turn its effects into
    /// network sends and disk operations. A paused node processes nothing.
    /// A node that halted is taken down.
    fn pump(&mut self, node: NodeId) {
        let now = self.now();
        let host = &mut self.hosts[usize::from(node)];
        if host.paused {
            return;
        }
        let Some(process) = host.process.as_mut() else {
            return;
        };
        process.driver.run_until_idle();
        let replies: Vec<ResponseEnvelope> = process.driver.network.outbox.drain(..).collect();
        let ops: Vec<StorageOp> = process.driver.storage.ops.drain(..).collect();
        let fired = process.driver.node.buggify_mut().take_log();
        let halted = process.driver.is_halted();
        let incarnation = host.incarnation;
        self.log_buggify(node, &fired);
        for reply in replies {
            let to = Addr::Client(reply.client);
            self.transmit(Addr::Node(node), to, Message::Reply(reply));
        }
        for op in ops {
            let disk = &mut self.hosts[usize::from(node)].disk;
            let (at, line) = match op {
                StorageOp::Read { op } => {
                    let at = disk.submit_read(now, op, &mut self.prng);
                    (at, format!("{now} n{node} disk read op={op} at={at}"))
                }
                StorageOp::Write { op, offset, bytes } => {
                    let at = disk.submit_write(now, op, offset, &bytes, &mut self.prng);
                    let sectors = bytes.len().div_ceil(disk.sector_size());
                    (
                        at,
                        format!("{now} n{node} disk write op={op} offset={offset} sectors={sectors} at={at}"),
                    )
                }
            };
            self.log(format_args!("{line}"));
            self.scheduler
                .schedule_at(at, Event::DiskComplete { node, incarnation });
        }
        if halted {
            let host = &mut self.hosts[usize::from(node)];
            let process = host.process.take().expect("still up");
            let damage = process.driver.node.damage().expect("a halt has a cause");
            host.last_halt = Some(damage);
            host.timer
                .reconcile(&mut self.scheduler, None, || Event::NodeTimer { node });
            self.stats.halts += 1;
            self.log(format_args!("{now} n{node} halted: {damage}"));
        }
    }

    /// One trace line per fault point that fired.
    fn log_buggify(&mut self, node: NodeId, fired: &[Site]) {
        let now = self.now();
        for site in fired {
            self.stats.buggify_fires += 1;
            self.log(format_args!("{now} n{node} buggify {site}"));
        }
    }

    /// Ask the network what happens to a message, and schedule it.
    fn transmit(&mut self, from: Addr, to: Addr, message: Message) {
        let now = self.now();
        match self.net.send(now, from, to, &mut self.prng) {
            Err(reason) => {
                self.log(format_args!("{now} net dropped {from}->{to} {message} {reason}"));
            }
            Ok(deliveries) => {
                let mut line = format!("{now} net send {from}->{to} {} {message}", deliveries[0].id);
                for d in &deliveries {
                    let label = if d.copy == 0 { "at" } else { "dup" };
                    write!(line, " {label}={}", d.at).unwrap();
                }
                self.log(format_args!("{line}"));
                for d in deliveries {
                    if message.is_client_traffic() {
                        self.client_messages_in_flight += 1;
                    }
                    self.scheduler.schedule_at(
                        d.at,
                        Event::Deliver {
                            id: d.id,
                            from,
                            to,
                            message: message.clone(),
                        },
                    );
                }
            }
        }
    }

    /// Uniform in `[0, max]`. Spends no randomness when `max` is zero.
    fn jitter(&mut self, max: Duration) -> Duration {
        if max.is_zero() {
            return Duration::ZERO;
        }
        let nanos = u64::try_from(max.as_nanos()).expect("jitter fits u64");
        Duration::from_nanos(self.prng.below(nanos + 1))
    }

    fn schedule_partition(&mut self) {
        let gap_max = self.config.partitions.as_ref().expect("schedule").gap_max;
        let gap = self.jitter(gap_max);
        self.partition_event = Some(self.scheduler.schedule_after(gap, Event::Partition));
    }

    fn schedule_crash(&mut self) {
        let gap_max = self.config.crashes.as_ref().expect("schedule").gap_max;
        let gap = self.jitter(gap_max);
        self.crash_event = Some(self.scheduler.schedule_after(gap, Event::Crash));
    }

    fn schedule_pause(&mut self) {
        let gap_max = self.config.pauses.as_ref().expect("schedule").gap_max;
        let gap = self.jitter(gap_max);
        self.pause_event = Some(self.scheduler.schedule_after(gap, Event::Pause));
    }

    fn schedule_clock_jump(&mut self) {
        let gap_max = self.config.clocks.as_ref().expect("schedule").jump_gap_max;
        let gap = self.jitter(gap_max);
        self.clock_event = Some(self.scheduler.schedule_after(gap, Event::ClockJump));
    }

    /// Keep client `id`'s timer event in step with its deadline.
    fn arm_client_timer(&mut self, id: ClientId) {
        let want = self.clients[&id].next_deadline();
        self.client_timers
            .get_mut(&id)
            .unwrap()
            .reconcile(&mut self.scheduler, want, || Event::ClientTimer { client: id });
    }

    /// Keep host `node`'s timer event in step with its heartbeat deadline.
    /// The deadline is on the host's clock; the event is in true time. A
    /// down or paused host wants no timer.
    fn arm_host_timer(&mut self, node: NodeId) {
        let now = self.now();
        let host = &mut self.hosts[usize::from(node)];
        let want = if host.paused {
            None
        } else {
            host.process
                .as_ref()
                .and_then(|p| p.heartbeat.next_deadline())
                // A deadline the clock has already passed, after a jump,
                // fires now rather than in the past.
                .map(|local| host.clock.to_true(local).max(now))
        };
        host.timer
            .reconcile(&mut self.scheduler, want, || Event::NodeTimer { node });
    }

    /// Deliver what client `id` just produced.
    fn route_client(&mut self, id: ClientId) {
        let now = self.now();
        let target = Addr::Node(self.targets[&id]);
        for out in std::mem::take(&mut self.client_out) {
            match out {
                client::Output::Send(envelope) => {
                    assert_eq!(envelope.client, id);
                    self.transmit(Addr::Client(id), target, Message::Request(envelope));
                }
                client::Output::Done { request_id, response } => {
                    self.log(format_args!(
                        "{now} client={id} done id={request_id} {response:?}"
                    ));
                    self.history.ret(id, request_id, response.clone(), now);
                    self.completed.push((id, request_id, response));
                }
                client::Output::GaveUp { request_id } => {
                    self.log(format_args!("{now} client={id} gave up id={request_id}"));
                    self.history.give_up(id, request_id);
                    self.gave_up.push((id, request_id));
                }
            }
        }
    }

    /// Deliver what host `node`'s heartbeat just produced.
    fn route_host(&mut self, node: NodeId) {
        let now = self.now();
        let fired = match self.hosts[usize::from(node)].process.as_mut() {
            Some(p) => p.heartbeat.buggify_mut().take_log(),
            None => Vec::new(),
        };
        self.log_buggify(node, &fired);
        for out in std::mem::take(&mut self.host_out) {
            match out {
                heartbeat::Output::Send { to, message } => {
                    self.transmit(Addr::Node(node), Addr::Node(to), Message::Heartbeat(message));
                }
                heartbeat::Output::PeerUp(peer) => {
                    self.log(format_args!("{now} n{node} peer up n{peer}"));
                }
                heartbeat::Output::PeerDown(peer) => {
                    self.log(format_args!("{now} n{node} peer down n{peer}"));
                }
            }
        }
    }
}
