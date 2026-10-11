//! The simulated environment.
//!
//! Module 4 put every delay on a [`sim::Scheduler`]. Module 5 puts every
//! message through one [`sim::Net`], and gives the world more than one host:
//!
//! ```text
//!   client.submit ──► Request ──┐
//!   node ───────► Reply ────────┼──► net.send ──► Deliver @ t ──► net.deliver ──► destination
//!   heartbeat ──► Beat ─────────┘        │
//!                                        └──► Err(Dropped): traced, no event
//!   node ─────────► Append ──► DiskComplete  @ now+disk ──► node ──► Reply
//!   client.next_deadline()    ──► ClientTimer @ deadline ──► client
//!   heartbeat.next_deadline() ──► NodeTimer   @ deadline ──► heartbeat
//! ```
//!
//! Nodes and clients share one address space, [`Addr`]. A request from a
//! client, a reply to it and a heartbeat between nodes are all the same
//! thing to the network: a message from one address to another, with a
//! fate the seed decides. The scheduler is still the only queue.
//!
//! Each host is a KV node (behind its [`Driver`]) and a [`Heartbeat`]. The
//! KV nodes are independent stores until Module 9 replicates them. The
//! heartbeats are what Module 5 is about: they are the traffic the network
//! faults act on, and their liveness view is what the tests read.
//!
//! One trace serves every host. Each driver's observer holds a handle to it
//! and tags its lines with the host id. This is the "tell the subscriber
//! which node is current" rule from Module 3, done structurally.

use determinism::{Prng, Trace};
use kv_core::client::{self, Client};
use kv_core::env::{Driver, Network, Observer, Storage};
use kv_core::heartbeat::{self, Heartbeat, NodeId};
use kv_core::node::{Input, OpId, Output};
use kv_core::{ClientId, Duration, Instant, Request, RequestEnvelope, Response, ResponseEnvelope};
use sim::{Clock, EventId, Fired, MessageId, Net, NetConfig, Scheduler, Shape, Timer};
use std::cell::{Ref, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::fmt::{self, Write as _};
use std::rc::Rc;

/// A log held in a `Vec<u8>`. Appends sit in a queue until the world's
/// `DiskComplete` event makes them durable, in order.
#[derive(Clone, Debug, Default)]
pub struct FakeStorage {
    pub log: Vec<u8>,
    pending_appends: VecDeque<(OpId, u64, Vec<u8>)>,
    completions: VecDeque<Input>,
    submitted: u64,
}

impl FakeStorage {
    pub fn new() -> FakeStorage {
        FakeStorage::default()
    }

    /// Start from an existing log, as after a restart.
    pub fn with_log(log: Vec<u8>) -> FakeStorage {
        FakeStorage {
            log,
            ..Default::default()
        }
    }

    /// Make the oldest pending append durable. Returns false if none.
    pub fn complete_next(&mut self) -> bool {
        let Some((op, offset, bytes)) = self.pending_appends.pop_front() else {
            return false;
        };
        // The node computes offsets assuming in-order completion. An offset
        // *past* the end is a node bug; one *before* the end is the node
        // overwriting a torn tail it found during recovery, which is correct.
        let offset = usize::try_from(offset).expect("offset fits usize");
        assert!(
            offset <= self.log.len(),
            "append offset {offset} beyond log length {}",
            self.log.len()
        );
        self.log.truncate(offset);
        self.log.extend_from_slice(&bytes);
        self.completions.push_back(Input::AppendComplete { op });
        true
    }

    /// Appends submitted and not yet durable.
    pub fn pending(&self) -> usize {
        self.pending_appends.len()
    }

    /// Appends ever submitted. The world uses this to notice new ones.
    pub fn submitted(&self) -> u64 {
        self.submitted
    }
}

impl Storage for FakeStorage {
    fn read_all(&mut self, op: OpId) {
        self.completions.push_back(Input::ReadComplete {
            op,
            data: self.log.clone(),
        });
    }

    fn append(&mut self, op: OpId, offset: u64, bytes: &[u8]) {
        self.submitted += 1;
        self.pending_appends.push_back((op, offset, bytes.to_vec()));
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
    /// The oldest pending append on `node` is durable.
    DiskComplete { node: NodeId },
    /// The seed cuts the network into a random shape.
    Partition,
    /// The seed heals it again.
    Heal,
}

/// Seed-driven partitions. After each heal the next partition starts
/// within `gap_max`, and lasts up to `hold_max`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartitionSchedule {
    pub gap_max: Duration,
    pub hold_max: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldConfig {
    pub seed: u64,
    /// The network fault model, for every message.
    pub net: NetConfig,
    pub heartbeat: heartbeat::Config,
    /// Time from `Append` to durable, plus uniform jitter in `[0, jitter]`.
    pub disk_latency: Duration,
    pub disk_jitter: Duration,
    /// Keep every trace line, or only the running hash. Turn this off for
    /// long runs; the hash is what the determinism check compares anyway.
    pub keep_trace_lines: bool,
    /// Random partitions, if any. Needs at least two hosts.
    pub partitions: Option<PartitionSchedule>,
}

impl Default for WorldConfig {
    fn default() -> WorldConfig {
        WorldConfig {
            seed: 0,
            net: NetConfig::default(),
            heartbeat: heartbeat::Config::default(),
            disk_latency: Duration::from_millis(5),
            disk_jitter: Duration::ZERO,
            keep_trace_lines: true,
            partitions: None,
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
    pub appends_completed: u64,
    pub partitions: u64,
    pub heals: u64,
}

/// One simulated machine: a KV node with its storage, and a heartbeat.
pub struct Host {
    pub id: NodeId,
    pub driver: FakeDriver,
    pub heartbeat: Heartbeat,
    timer: Timer,
    /// Appends for which a `DiskComplete` event exists.
    disk_events_issued: u64,
    /// Completion time of the last scheduled append. Appends complete in
    /// submission order, so the next one cannot complete before this.
    disk_busy_until: Instant,
}

/// Some hosts and some clients, wired together by the scheduler and the
/// network model.
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
    client_out: Vec<client::Output>,
    host_out: Vec<heartbeat::Output>,
}

impl World {
    /// One host with `storage`, default configuration.
    pub fn new(storage: FakeStorage) -> World {
        World::with_storages(vec![storage], WorldConfig::default())
    }

    /// `nodes` hosts with empty logs.
    pub fn cluster(nodes: u8, config: WorldConfig) -> World {
        World::with_storages((0..nodes).map(|_| FakeStorage::new()).collect(), config)
    }

    /// One host per storage. Each host runs recovery and sends its first
    /// heartbeats before this returns.
    pub fn with_storages(storages: Vec<FakeStorage>, config: WorldConfig) -> World {
        assert!(!storages.is_empty(), "a world needs at least one host");
        assert!(
            storages.len() <= usize::from(NodeId::MAX),
            "too many hosts for a NodeId"
        );
        config.net.validate();
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
            client_out: Vec::new(),
            host_out: Vec::new(),
        };
        let ids: Vec<NodeId> = (0..storages.len() as u8).collect();
        for (storage, &id) in storages.into_iter().zip(&ids) {
            let driver = Driver::new(
                world.scheduler.clock(),
                storage,
                FakeNetwork::default(),
                TraceObserver {
                    node: id,
                    trace: world.trace.clone(),
                },
            );
            let peers: Vec<NodeId> = ids.iter().copied().filter(|&p| p != id).collect();
            let heartbeat = Heartbeat::new(
                id,
                &peers,
                world.config.heartbeat,
                Instant::ZERO,
                &mut world.host_out,
            );
            world.hosts.push(Host {
                id,
                driver,
                heartbeat,
                timer: Timer::new(),
                disk_events_issued: 0,
                disk_busy_until: Instant::ZERO,
            });
            // Recovery: the node's initial log read completes at once.
            world.pump(id);
            world.route_host(id);
            world.arm_host_timer(id);
        }
        if world.config.partitions.is_some() {
            assert!(world.hosts.len() >= 2, "partitions need at least two hosts");
            world.schedule_partition();
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

    /// Nothing is happening except heartbeats: every client is idle, no
    /// request or reply is on the wire, and no append is waiting for a disk.
    pub fn is_idle(&self) -> bool {
        self.clients.values().all(Client::is_idle)
            && self.client_messages_in_flight == 0
            && self.hosts.iter().all(|h| h.driver.storage.pending() == 0)
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

    fn log(&self, args: fmt::Arguments<'_>) {
        self.trace.borrow_mut().record(args);
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
                host.heartbeat
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
            Event::DiskComplete { node } => {
                let host = &mut self.hosts[usize::from(node)];
                assert!(
                    host.driver.storage.complete_next(),
                    "DiskComplete on node {node} with nothing pending"
                );
                self.stats.appends_completed += 1;
                self.log(format_args!("{now} n{node} disk complete"));
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
        }
    }

    /// Hand a delivered message to its destination.
    fn receive(&mut self, now: Instant, from: Addr, to: Addr, message: Message) {
        match (to, message) {
            (Addr::Node(n), Message::Request(envelope)) => {
                self.stats.requests_delivered += 1;
                self.hosts[usize::from(n)].driver.network.deliver(envelope);
                self.pump(n);
            }
            (Addr::Node(n), Message::Heartbeat(message)) => {
                self.stats.beats_delivered += 1;
                let input = heartbeat::Input::Message {
                    from: from.node(),
                    message,
                };
                self.hosts[usize::from(n)]
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
    /// network sends and disk events.
    fn pump(&mut self, node: NodeId) {
        let now = self.now();
        let host = &mut self.hosts[usize::from(node)];
        host.driver.run_until_idle();
        let replies: Vec<ResponseEnvelope> = host.driver.network.outbox.drain(..).collect();
        let new_appends = host.driver.storage.submitted() - host.disk_events_issued;
        for reply in replies {
            let to = Addr::Client(reply.client);
            self.transmit(Addr::Node(node), to, Message::Reply(reply));
        }
        for _ in 0..new_appends {
            let delay = self.config.disk_latency + self.jitter(self.config.disk_jitter);
            let host = &mut self.hosts[usize::from(node)];
            let at = (now + delay).max(host.disk_busy_until);
            host.disk_busy_until = at;
            host.disk_events_issued += 1;
            self.scheduler.schedule_at(at, Event::DiskComplete { node });
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

    /// Keep client `id`'s timer event in step with its deadline.
    fn arm_client_timer(&mut self, id: ClientId) {
        let want = self.clients[&id].next_deadline();
        self.client_timers
            .get_mut(&id)
            .unwrap()
            .reconcile(&mut self.scheduler, want, || Event::ClientTimer { client: id });
    }

    /// Keep host `node`'s timer event in step with its heartbeat deadline.
    fn arm_host_timer(&mut self, node: NodeId) {
        let host = &mut self.hosts[usize::from(node)];
        let want = host.heartbeat.next_deadline();
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
