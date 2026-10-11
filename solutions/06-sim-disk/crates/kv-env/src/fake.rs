//! The simulated environment.
//!
//! Module 4 put every delay on a [`sim::Scheduler`]. Module 5 put every
//! message through one [`sim::Net`]. Module 6 puts every disk operation
//! through one [`sim::Disk`] per host, and lets a host die:
//!
//! ```text
//!   client.submit ──► Request ──┐
//!   node ───────► Reply ────────┼──► net.send ──► Deliver @ t ──► net.deliver ──► destination
//!   heartbeat ──► Beat ─────────┘        │
//!                                        └──► Err(Dropped): traced, no event
//!   node ──► Read / Append ──► disk.submit ──► DiskComplete @ t ──► disk.complete_next ──► node
//!   client.next_deadline()    ──► ClientTimer @ deadline ──► client
//!   heartbeat.next_deadline() ──► NodeTimer   @ deadline ──► heartbeat
//!   seed ──► Crash ──► host.process = None, disk.crash()
//!        ──► Restart ──► new process, which reads the disk
//! ```
//!
//! A [`Host`] is a [`Disk`] and, while it is up, a [`Process`]: the KV node
//! behind its [`Driver`] and a [`Heartbeat`]. A crash drops the process.
//! Everything the node knew is gone; only the disk remains, and the disk
//! decides what became of the writes that were in flight. A restart builds
//! a new process, which reads the disk and recovers from it alone. That is
//! the whole point: there is no back door from the old process to the new.
//!
//! Nodes and clients share one address space, [`Addr`]. A request from a
//! client, a reply to it and a heartbeat between nodes are all the same
//! thing to the network. The scheduler is still the only queue.
//!
//! One trace serves every host. Each driver's observer holds a handle to it
//! and tags its lines with the host id.

use determinism::{Prng, Trace};
use kv_core::client::{self, Client};
use kv_core::env::{Driver, Network, Observer, Storage};
use kv_core::heartbeat::{self, Heartbeat, NodeId};
use kv_core::log::Damage;
use kv_core::node::{Input, Node, OpId, Output};
use kv_core::{ClientId, Duration, Instant, Request, RequestEnvelope, Response, ResponseEnvelope};
use sim::{
    Clock, Completion, Disk, DiskConfig, EventId, Fired, MessageId, Net, NetConfig, Scheduler, Shape, Timer,
};
use std::cell::{Ref, RefCell};
use std::collections::{BTreeMap, VecDeque};
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
#[derive(Debug)]
pub struct TraceObserver {
    pub node: NodeId,
    pub trace: SharedTrace,
}

impl Observer for TraceObserver {
    fn input(&mut self, now: Instant, input: &Input) {
        let n = self.node;
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

    fn output(&mut self, now: Instant, output: &Output) {
        let n = self.node;
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

pub type FakeDriver = Driver<Clock, FakeStorage, FakeNetwork, TraceObserver>;

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
    /// A host's heartbeat `next_deadline` arrived.
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
}

/// Seed-driven partitions. After each heal the next partition starts
/// within `gap_max`, and lasts up to `hold_max`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartitionSchedule {
    pub gap_max: Duration,
    pub hold_max: Duration,
}

/// Seed-driven crashes. After each restart the next crash comes within
/// `gap_max`, and the host stays down for up to `down_max`. One host is
/// down at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrashSchedule {
    pub gap_max: Duration,
    pub down_max: Duration,
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
        }
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
}

/// A running process on a host: the KV node and its heartbeat.
pub struct Process {
    pub driver: FakeDriver,
    pub heartbeat: Heartbeat,
}

/// One simulated machine. The disk outlives the process.
pub struct Host {
    pub id: NodeId,
    pub disk: Disk,
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
    timer: Timer,
}

impl Host {
    pub fn is_up(&self) -> bool {
        self.process.is_some()
    }

    pub fn incarnation(&self) -> u64 {
        self.incarnation
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
    pub stats: WorldStats,
    trace: SharedTrace,
    /// Scheduled `Deliver` events that carry a request or a reply.
    client_messages_in_flight: usize,
    /// The pending `Partition` or `Heal` event, if a schedule is running.
    partition_event: Option<EventId>,
    /// The pending `Crash` or `Restart` event, if a schedule is running.
    crash_event: Option<EventId>,
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
            stats: WorldStats::default(),
            trace: Rc::new(RefCell::new(trace)),
            client_messages_in_flight: 0,
            partition_event: None,
            crash_event: None,
            client_out: Vec::new(),
            host_out: Vec::new(),
        };
        for (id, disk) in (0..).zip(disks) {
            world.hosts.push(Host {
                id,
                disk,
                process: None,
                last_halt: None,
                last_read_faults: Vec::new(),
                incarnation: 0,
                timer: Timer::new(),
            });
        }
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
        world
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
        let client = self.clients.get_mut(&id).expect("unknown client");
        let rid = client.submit(now, request, &mut self.client_out);
        self.route_client(id);
        self.arm_client_timer(id);
        rid
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

    /// Kill the process on `node`. Its memory is gone. The disk resolves
    /// the operations that were in flight, as the fault model decides.
    pub fn crash(&mut self, node: NodeId) {
        let now = self.now();
        let host = &mut self.hosts[usize::from(node)];
        assert!(host.process.take().is_some(), "node {node} is already down");
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

    fn log(&self, args: fmt::Arguments<'_>) {
        self.trace.borrow_mut().record(args);
    }

    /// Build the process for `node` and let it ask for its log.
    fn start(&mut self, node: NodeId) {
        let now = self.now();
        let host = &mut self.hosts[usize::from(node)];
        assert!(host.process.is_none());
        host.incarnation += 1;
        let driver = Driver::new(
            self.scheduler.clock(),
            FakeStorage::default(),
            FakeNetwork::default(),
            TraceObserver {
                node,
                trace: self.trace.clone(),
            },
        );
        let peers: Vec<NodeId> = (0..self.hosts.len() as u8).filter(|&p| p != node).collect();
        let heartbeat = Heartbeat::new(node, &peers, self.config.heartbeat, now, &mut self.host_out);
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
                self.log(format_args!("{now} timer node={node}"));
                let host = &mut self.hosts[usize::from(node)];
                assert!(host.timer.fired(fired.id), "stale timer event for node {node}");
                host.process
                    .as_mut()
                    .expect("a down host has no timer")
                    .heartbeat
                    .on_input(now, heartbeat::Input::Tick, &mut self.host_out);
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
            (Addr::Node(n), Message::Heartbeat(message)) => {
                self.stats.beats_delivered += 1;
                let input = heartbeat::Input::Message {
                    from: from.node(),
                    message,
                };
                self.hosts[usize::from(n)]
                    .process
                    .as_mut()
                    .expect("checked above")
                    .heartbeat
                    .on_input(now, input, &mut self.host_out);
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
    /// network sends and disk operations. A node that halted is taken down.
    fn pump(&mut self, node: NodeId) {
        let now = self.now();
        let host = &mut self.hosts[usize::from(node)];
        let Some(process) = host.process.as_mut() else {
            return;
        };
        process.driver.run_until_idle();
        let replies: Vec<ResponseEnvelope> = process.driver.network.outbox.drain(..).collect();
        let ops: Vec<StorageOp> = process.driver.storage.ops.drain(..).collect();
        let halted = process.driver.is_halted();
        let incarnation = host.incarnation;
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

    /// Keep client `id`'s timer event in step with its deadline.
    fn arm_client_timer(&mut self, id: ClientId) {
        let want = self.clients[&id].next_deadline();
        self.client_timers
            .get_mut(&id)
            .unwrap()
            .reconcile(&mut self.scheduler, want, || Event::ClientTimer { client: id });
    }

    /// Keep host `node`'s timer event in step with its heartbeat deadline.
    /// A down host wants no timer.
    fn arm_host_timer(&mut self, node: NodeId) {
        let host = &mut self.hosts[usize::from(node)];
        let want = host.process.as_ref().and_then(|p| p.heartbeat.next_deadline());
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
                    self.completed.push((id, request_id, response));
                }
                client::Output::GaveUp { request_id } => {
                    self.log(format_args!("{now} client={id} gave up id={request_id}"));
                    self.gave_up.push((id, request_id));
                }
            }
        }
    }

    /// Deliver what host `node`'s heartbeat just produced.
    fn route_host(&mut self, node: NodeId) {
        let now = self.now();
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
