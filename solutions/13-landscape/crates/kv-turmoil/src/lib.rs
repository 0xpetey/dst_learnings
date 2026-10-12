//! The Module 10 replica on turmoil.
//!
//! The same `kv_core::Replica`, the same `Driver`, the same `Client`, the
//! same `Workload` and the same checker as the simulator of Modules 4 to
//! 12, under a simulator somebody else wrote. The port is three trait
//! implementations, a wire format the fake network never needed, and a
//! loop that waits the way an async program waits:
//!
//! ```text
//!   turmoil::Sim ─┬─ host "n0" ─── Driver<TokioTime, SharedStorage, UdpNetwork, TraceObserver>
//!                 ├─ host "n1" ─── Driver<...>            ▲ step, then wait on
//!                 ├─ host "n2" ─── Driver<...>            │ socket.readable() or the
//!                 ├─ client "c1" ─ Client + Workload      │ replica's next deadline
//!                 └─ client "c2" ─ Client + Workload
//!        │
//!        └─ the test code, outside: sim.step() in a loop, and between steps
//!           partition / repair / crash / bounce from a seeded schedule
//! ```
//!
//! What turmoil owns: the clock of every host (a paused tokio runtime
//! advanced one tick at a time), the network (UDP datagrams with a seeded
//! latency, a partition and a hold per link), and a host's life (crash
//! and bounce). What it does not own, because it has no model of it: the
//! disk. A host's log lives in a `Vec<u8>` outside the host, in
//! [`Disks`], and survives a crash whole. Nothing in flight is ever torn.
//! That is the first entry in the retrospective.
//!
//! What stays ours: the replica, the client, the workload, the history
//! and the linearizability check. The oracle crosses over untouched,
//! which is the point of keeping it `no_std`.

pub mod wire;

use checker::kv;
use checker::{History, Step, Workload, WorkloadConfig};
use determinism::{Prng, Trace};
use kv_core::bugs::Bug;
use kv_core::client::{self, Client};
use kv_core::env::{Driver, Network, Observer, Storage, Time};
use kv_core::heartbeat::{self, NodeId};
use kv_core::node::OpId;
use kv_core::protocol::MAX_FRAME_LEN;
use kv_core::replica::{self, Input, Message, Output, Status};
use kv_core::{log, superblock, ClientId, Duration, Instant, Request, Response, ResponseEnvelope};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::net::Ipv4Addr;
use std::rc::Rc;
use turmoil::net::UdpSocket;
use wire::Datagram;

/// Where a replica listens.
pub const NODE_PORT: u16 = 7000;
/// Where a client listens. A client is a host of its own, named for its
/// id, so that a reply from any replica can find it.
pub const CLIENT_PORT: u16 = 7001;
/// Candidates per key for the linearizability search, as in `vopr`.
pub const CHECK_LIMIT: u64 = 2_000_000;

/// The turmoil host name of replica `node`.
pub fn node_name(node: NodeId) -> String {
    format!("n{node}")
}

/// The turmoil host name of client `id`.
pub fn client_name(id: ClientId) -> String {
    format!("c{id}")
}

/// Virtual time since the simulation started, as the core's `Instant`.
/// Valid only on a host, which is where every caller runs.
pub fn sim_now() -> Instant {
    let elapsed = turmoil::sim_elapsed().expect("called from inside the simulation");
    Instant::from_nanos(u64::try_from(elapsed.as_nanos()).expect("virtual time fits in u64"))
}

// ---- the three traits -----------------------------------------------------

/// The host's clock: tokio's paused `Instant`, relative to the moment
/// this process started. turmoil advances it one tick at a time, so two
/// events in one tick share an instant. A restarted host starts from
/// zero again, as Module 7's hosts did.
pub struct TokioTime {
    start: tokio::time::Instant,
}

impl TokioTime {
    pub fn new() -> TokioTime {
        TokioTime {
            start: tokio::time::Instant::now(),
        }
    }
}

impl Default for TokioTime {
    fn default() -> Self {
        TokioTime::new()
    }
}

impl Time for TokioTime {
    fn now(&self) -> Instant {
        let elapsed = self.start.elapsed();
        Instant::from_nanos(u64::try_from(elapsed.as_nanos()).expect("uptime fits in u64"))
    }
}

/// One host's disk: its bytes and how many times its process started.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Disk {
    pub bytes: Vec<u8>,
    pub starts: u64,
}

/// Every disk in the cluster, outside every host, so that a crash loses
/// the process and keeps the bytes. turmoil's own filesystem is behind an
/// unstable feature. This is the shape a disk has without it.
pub type Disks = Rc<RefCell<Vec<Disk>>>;

pub fn new_disks(nodes: u8) -> Disks {
    Rc::new(RefCell::new(vec![Disk::default(); usize::from(nodes)]))
}

/// A disk as a `Vec<u8>`: a write lands at its offset and the bytes
/// past it stay, as the fake disk of Module 6 and `pwrite` on a file
/// behave. An append is durable when it returns, as one `fsync` per
/// append is. Nothing is ever in flight at a crash, so nothing is ever
/// torn.
///
/// The first version copied `FileStorage` and cut the file after every
/// write. The superblock lives at offset zero, so every promise erased
/// the log behind it, and seed 34 lost its records to a bounce. The fake
/// disk never cut anything, so the simulator never saw the real
/// environment do it. That is the second entry in the retrospective.
pub struct SharedStorage {
    disks: Disks,
    node: NodeId,
    completions: VecDeque<Input>,
}

impl SharedStorage {
    pub fn new(disks: Disks, node: NodeId) -> SharedStorage {
        SharedStorage {
            disks,
            node,
            completions: VecDeque::new(),
        }
    }
}

impl Storage for SharedStorage {
    fn read_all(&mut self, op: OpId) {
        let data = self.disks.borrow()[usize::from(self.node)].bytes.clone();
        self.completions.push_back(Input::ReadComplete { op, data });
    }

    fn append(&mut self, op: OpId, offset: u64, bytes: &[u8]) {
        let offset = usize::try_from(offset).expect("offset fits");
        let mut disks = self.disks.borrow_mut();
        let disk = &mut disks[usize::from(self.node)].bytes;
        let end = offset + bytes.len();
        if disk.len() < end {
            disk.resize(end, 0);
        }
        disk[offset..end].copy_from_slice(bytes);
        self.completions.push_back(Input::AppendComplete { op });
    }

    fn poll_completions(&mut self, inputs: &mut Vec<Input>) {
        inputs.extend(self.completions.drain(..));
    }
}

/// What the network saw, for the report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetStats {
    pub sent: u64,
    pub received: u64,
    /// Datagrams that did not decode, or decoded to something a replica
    /// does not take.
    pub rejected: u64,
}

/// One UDP socket, bound to [`NODE_PORT`] on the host. A reply goes to the
/// client's own host, a peer message to the peer's. turmoil's DNS does
/// the routing that the fake world's one address space did.
pub struct UdpNetwork {
    pub socket: UdpSocket,
    pub stats: NetStats,
    me: NodeId,
    buf: Vec<u8>,
}

impl UdpNetwork {
    pub fn new(socket: UdpSocket, me: NodeId) -> UdpNetwork {
        UdpNetwork {
            socket,
            stats: NetStats::default(),
            me,
            buf: vec![0; MAX_FRAME_LEN + 16],
        }
    }

    fn send_datagram(&mut self, datagram: &Datagram, host: &str, port: u16) {
        // A send fails only for an address that does not resolve. Every
        // host is registered before the run, so a failure is a bug in
        // this file, not a fault to swallow.
        self.socket
            .try_send_to(&datagram.to_bytes(), (host, port))
            .unwrap_or_else(|e| panic!("n{} could not send to {host}:{port}: {e}", self.me));
        self.stats.sent += 1;
    }
}

impl Network for UdpNetwork {
    fn send(&mut self, reply: ResponseEnvelope) {
        let host = client_name(reply.client);
        self.send_datagram(&Datagram::Reply(reply), &host, CLIENT_PORT);
    }

    fn send_peer(&mut self, to: NodeId, message: Message) {
        let host = node_name(to);
        self.send_datagram(
            &Datagram::Peer {
                from: self.me,
                message,
            },
            &host,
            NODE_PORT,
        );
    }

    fn poll(&mut self, inputs: &mut Vec<Input>) {
        // `WouldBlock` is the one error a receive has: the queue is empty.
        while let Ok((n, _from)) = self.socket.try_recv_from(&mut self.buf) {
            self.stats.received += 1;
            match Datagram::decode(&self.buf[..n]) {
                Ok(Datagram::Request(envelope)) => inputs.push(Input::Request(envelope)),
                Ok(Datagram::Peer { from, message }) => inputs.push(Input::Peer { from, message }),
                Ok(Datagram::Reply(_)) | Err(_) => self.stats.rejected += 1,
            }
        }
    }
}

/// One trace for every host, as in the fake world. Lines carry the
/// simulation's time, so that two runs of a seed can be compared line by
/// line, and hashed.
pub type SharedTrace = Rc<RefCell<Trace>>;

pub struct TraceObserver {
    pub node: NodeId,
    pub trace: SharedTrace,
}

impl Observer for TraceObserver {
    fn input(&mut self, _local: Instant, input: &Input) {
        let n = self.node;
        let now = sim_now();
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
            Input::Peer { from, message } => {
                t.record(format_args!("{now} n{n} in  Peer from=n{from} {message}"))
            }
            Input::Tick => t.record(format_args!("{now} n{n} in  Tick")),
        }
    }

    fn output(&mut self, _local: Instant, output: &Output) {
        let n = self.node;
        let now = sim_now();
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
            Output::Send { to, message } => t.record(format_args!("{now} n{n} out Send to=n{to} {message}")),
            Output::PeerUp(peer) => t.record(format_args!("{now} n{n} peer up n{peer}")),
            Output::PeerDown(peer) => t.record(format_args!("{now} n{n} peer down n{peer}")),
            Output::Status { view, status } => {
                t.record(format_args!("{now} n{n} status view={view} {status}"))
            }
        }
    }
}

// ---- the host ---------------------------------------------------------------

/// What every replica is told at start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cluster {
    pub nodes: u8,
    pub heartbeat: heartbeat::Config,
    pub bugs: Vec<Bug>,
}

/// What a replica last looked like, written by its host after every step,
/// because the host's future is dropped when the run ends and the checks
/// run after that.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub boot: u64,
    pub view: replica::View,
    pub status: Status,
    pub commit: u64,
    pub log_len: u64,
    pub stats: replica::Stats,
    pub net: NetStats,
    pub steps: u64,
}

pub type Snapshots = Rc<RefCell<BTreeMap<NodeId, Snapshot>>>;

/// Everything the hosts share with the test code.
#[derive(Clone)]
pub struct Shared {
    pub disks: Disks,
    pub trace: SharedTrace,
    pub snapshots: Snapshots,
    pub history: Rc<RefCell<History<Request, Response>>>,
    pub workload: Rc<RefCell<Workload>>,
}

/// The software of host `n{node}`: one replica behind its driver, run
/// until the simulation ends or the replica halts. Called again by
/// turmoil on every bounce, with the same disk and a larger boot id.
pub async fn replica_host(node: NodeId, cluster: Cluster, shared: Shared) -> turmoil::Result {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, NODE_PORT)).await?;
    let boot = {
        let mut disks = shared.disks.borrow_mut();
        let disk = &mut disks[usize::from(node)];
        disk.starts += 1;
        disk.starts - 1
    };
    let peers: Vec<NodeId> = (0..cluster.nodes).filter(|&p| p != node).collect();
    let mut driver = Driver::new(
        node,
        boot,
        &peers,
        cluster.heartbeat,
        TokioTime::new(),
        SharedStorage::new(shared.disks.clone(), node),
        UdpNetwork::new(socket, node),
        TraceObserver {
            node,
            trace: shared.trace.clone(),
        },
    );
    for &bug in &cluster.bugs {
        driver.replica.bugs_mut().plant(bug);
    }
    loop {
        driver.step();
        shared.snapshots.borrow_mut().insert(
            node,
            Snapshot {
                boot,
                view: driver.replica.view(),
                status: driver.replica.status(),
                commit: driver.replica.commit(),
                log_len: driver.replica.log_len(),
                stats: driver.replica.stats(),
                net: driver.network.stats,
                steps: driver.steps(),
            },
        );
        if driver.is_halted() {
            let damage = driver.replica.damage().expect("a halt has a cause");
            return Err(format!("n{node} halted: {damage}").into());
        }
        // Wait for a datagram or for the replica's next deadline. Not
        // `tokio::select!`: it polls its branches in an order drawn from
        // tokio's own random number generator, which turmoil seeds only
        // under `--cfg tokio_unstable`. `timeout` has one order.
        let wait = match driver.replica.next_deadline() {
            Some(deadline) => deadline.saturating_duration_since(driver.time.now()),
            None => Duration::from_millis(10),
        };
        let _ = tokio::time::timeout(wait, driver.network.socket.readable()).await;
    }
}

// ---- the client -------------------------------------------------------------

/// What one client does: `requests` steps of the workload under the
/// faults, a wait until `quiet_at`, then `quiet_requests` steps that must
/// all be answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub requests: u64,
    pub quiet_requests: u64,
    pub quiet_at: Duration,
    pub config: client::Config,
}

/// How a request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    GaveUp,
}

/// What the clients did, summed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientStats {
    pub submitted: u64,
    pub completed: u64,
    pub gave_up: u64,
    pub quiet_submitted: u64,
    pub quiet_completed: u64,
}

pub type SharedClientStats = Rc<RefCell<ClientStats>>;

/// The software of host `c{id}`. Every request goes to replica `target`,
/// which forwards to the primary if it is not the primary itself.
pub async fn client_host(
    id: ClientId,
    target: NodeId,
    plan: Plan,
    shared: Shared,
    stats: SharedClientStats,
) -> turmoil::Result {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, CLIENT_PORT)).await?;
    let mut client = Client::new(id, plan.config);
    let mut buf = vec![0u8; MAX_FRAME_LEN + 16];
    let mut out = Vec::new();
    let target_name = node_name(target);
    for (quiet, count) in [(false, plan.requests), (true, plan.quiet_requests)] {
        let mut done = 0;
        while done < count {
            let step = shared.workload.borrow_mut().next(id);
            let request = match step {
                Step::Think(d) => {
                    tokio::time::sleep(d).await;
                    continue;
                }
                Step::Request(request) => request,
            };
            done += 1;
            {
                let mut s = stats.borrow_mut();
                s.submitted += 1;
                s.quiet_submitted += u64::from(quiet);
            }
            let outcome = one_request(
                &mut client,
                &socket,
                &target_name,
                request,
                &shared.history,
                &mut out,
                &mut buf,
            )
            .await;
            let mut s = stats.borrow_mut();
            match outcome {
                Outcome::Completed => {
                    s.completed += 1;
                    s.quiet_completed += u64::from(quiet);
                }
                Outcome::GaveUp if quiet => {
                    return Err(format!("liveness: client {id} gave up in the quiet phase").into());
                }
                Outcome::GaveUp => s.gave_up += 1,
            }
        }
        if !quiet {
            let now = turmoil::sim_elapsed().expect("on a host");
            tokio::time::sleep(plan.quiet_at.saturating_sub(now)).await;
        }
    }
    Ok(())
}

async fn one_request(
    client: &mut Client,
    socket: &UdpSocket,
    target: &str,
    request: Request,
    history: &Rc<RefCell<History<Request, Response>>>,
    out: &mut Vec<client::Output>,
    buf: &mut [u8],
) -> Outcome {
    let id = client.id();
    let rid = client.submit(sim_now(), request.clone(), out);
    history.borrow_mut().call(id, rid, request, sim_now());
    loop {
        for o in out.drain(..) {
            match o {
                client::Output::Send(envelope) => {
                    socket
                        .try_send_to(&Datagram::Request(envelope).to_bytes(), (target, NODE_PORT))
                        .expect("a replica's name resolves");
                }
                client::Output::Done { response, .. } => {
                    history.borrow_mut().ret(id, rid, response, sim_now());
                    return Outcome::Completed;
                }
                client::Output::GaveUp { .. } => {
                    history.borrow_mut().give_up(id, rid);
                    return Outcome::GaveUp;
                }
            }
        }
        let deadline = client.next_deadline().expect("a request is in flight");
        let wait = deadline.saturating_duration_since(sim_now());
        let _ = tokio::time::timeout(wait, socket.readable()).await;
        while let Ok((n, _)) = socket.try_recv_from(buf) {
            if let Ok(Datagram::Reply(reply)) = Datagram::decode(&buf[..n]) {
                client.on_input(client::Input::Response(reply), out);
            }
        }
        client.on_input(client::Input::Tick { now: sim_now() }, out);
    }
}

// ---- faults, from outside ---------------------------------------------------

/// One thing done to the simulation between two steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Partition(NodeId, NodeId),
    Repair(NodeId, NodeId),
    Hold(NodeId, NodeId),
    Release(NodeId, NodeId),
    Crash(NodeId),
    Bounce(NodeId),
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Partition(a, b) => write!(f, "partition n{a} n{b}"),
            Action::Repair(a, b) => write!(f, "repair n{a} n{b}"),
            Action::Hold(a, b) => write!(f, "hold n{a} n{b}"),
            Action::Release(a, b) => write!(f, "release n{a} n{b}"),
            Action::Crash(n) => write!(f, "crash n{n}"),
            Action::Bounce(n) => write!(f, "bounce n{n}"),
        }
    }
}

/// Which families the seed may draw, and how many faults at most.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaultConfig {
    pub partitions: bool,
    pub holds: bool,
    pub crashes: bool,
    /// Faults in the chaos phase, at most. Each is a pair of actions.
    pub max_faults: u64,
    /// How long a fault lasts, at most.
    pub max_duration: Duration,
}

impl Default for FaultConfig {
    fn default() -> FaultConfig {
        FaultConfig {
            partitions: true,
            holds: true,
            crashes: true,
            max_faults: 12,
            max_duration: Duration::from_millis(1500),
        }
    }
}

impl FaultConfig {
    pub const NONE: FaultConfig = FaultConfig {
        partitions: false,
        holds: false,
        crashes: false,
        max_faults: 0,
        max_duration: Duration::ZERO,
    };

    pub fn families(&self) -> usize {
        usize::from(self.partitions) + usize::from(self.holds) + usize::from(self.crashes)
    }
}

/// The faults of one run, each at its instant, in order. Drawn from the
/// seed before the run and applied between steps, from outside the
/// simulation. turmoil has no scheduler to hand this to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schedule {
    pub events: Vec<(Duration, Action)>,
}

impl Schedule {
    /// Up to `max_faults` faults, each a start and an end, all within
    /// `chaos`. A crash ends with a bounce, a partition with a repair, a
    /// hold with a release. Nothing is drawn for a family that is off.
    pub fn draw(prng: &mut Prng, nodes: u8, chaos: Duration, config: &FaultConfig) -> Schedule {
        let mut events = Vec::new();
        if config.families() == 0 || nodes < 2 {
            return Schedule { events };
        }
        let chaos_nanos = u64::try_from(chaos.as_nanos()).expect("chaos fits");
        let max_nanos = u64::try_from(config.max_duration.as_nanos()).expect("duration fits");
        let count = prng.range(0, config.max_faults + 1);
        let mut families = Vec::new();
        if config.partitions {
            families.push(0);
        }
        if config.holds {
            families.push(1);
        }
        if config.crashes {
            families.push(2);
        }
        for _ in 0..count {
            let at = prng.below(chaos_nanos);
            let lasts = prng.range(1, max_nanos.max(2));
            let until = (at + lasts).min(chaos_nanos);
            let a = prng.below(u64::from(nodes)) as NodeId;
            let mut b = prng.below(u64::from(nodes) - 1) as NodeId;
            if b >= a {
                b += 1;
            }
            let (start, end) = match *prng.pick(&families) {
                0 => (Action::Partition(a, b), Action::Repair(a, b)),
                1 => (Action::Hold(a, b), Action::Release(a, b)),
                _ => (Action::Crash(a), Action::Bounce(a)),
            };
            events.push((Duration::from_nanos(at), start));
            events.push((Duration::from_nanos(until), end));
        }
        // Stable, so that two events at one instant keep the order they
        // were drawn in.
        events.sort_by_key(|(at, _)| *at);
        Schedule { events }
    }
}

/// Apply one action to the simulation. A crash of a host that is down,
/// or a bounce of one that is up, does what turmoil does with it: the
/// crash is nothing, the bounce is a restart.
pub fn apply(sim: &mut turmoil::Sim<'_>, action: &Action, counts: &mut FaultCounts) {
    match action {
        Action::Partition(a, b) => {
            counts.partitions += 1;
            sim.partition(node_name(*a), node_name(*b));
        }
        Action::Repair(a, b) => sim.repair(node_name(*a), node_name(*b)),
        Action::Hold(a, b) => {
            counts.holds += 1;
            sim.hold(node_name(*a), node_name(*b));
        }
        Action::Release(a, b) => sim.release(node_name(*a), node_name(*b)),
        Action::Crash(n) => {
            counts.crashes += 1;
            sim.crash(node_name(*n));
        }
        Action::Bounce(n) => sim.bounce(node_name(*n)),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FaultCounts {
    pub partitions: u64,
    pub holds: u64,
    pub crashes: u64,
}

// ---- one run ----------------------------------------------------------------

/// Everything one run is given, all drawn from the seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunConfig {
    pub seed: u64,
    pub nodes: u8,
    pub clients_per_node: u64,
    /// Requests per client in the chaos phase.
    pub requests: u64,
    /// Requests per client in the quiet phase. Every one must complete.
    pub quiet_requests: u64,
    /// How long the faults run.
    pub chaos: Duration,
    /// How long the cluster has after the faults before the quiet phase.
    pub settle: Duration,
    pub faults: FaultConfig,
    /// turmoil's link latency, the maximum. The minimum is one tick.
    pub max_latency: Duration,
    pub workload: WorkloadConfig,
    pub workload_seed: u64,
    pub client: client::Config,
    pub heartbeat: heartbeat::Config,
    pub bugs: Vec<Bug>,
    /// Keep every trace line, not only the hash.
    pub keep_trace: bool,
    /// Print every trace line as it is recorded.
    pub echo_trace: bool,
}

impl RunConfig {
    /// What `seed` means: the latency, the workload, its dice, the faults
    /// and the cluster size, in that order, from one `Prng`.
    pub fn from_seed(seed: u64, requests: u64) -> RunConfig {
        let mut prng = Prng::from_seed(seed);
        let max_latency = Duration::from_millis(prng.range(2, 151));
        let workload = WorkloadConfig::random(&mut prng);
        let workload_seed = prng.next_u64();
        let faults = FaultConfig {
            partitions: prng.chance(3, 4),
            holds: prng.chance(1, 2),
            crashes: prng.chance(3, 4),
            ..FaultConfig::default()
        };
        let nodes: u8 = if prng.chance(1, 4) { 5 } else { 3 };
        RunConfig {
            seed,
            nodes,
            clients_per_node: 2,
            requests,
            quiet_requests: 10,
            chaos: Duration::from_secs(20),
            settle: Duration::from_secs(5),
            faults,
            max_latency,
            workload,
            workload_seed,
            client: client::Config {
                initial_timeout: Duration::from_millis(100),
                max_timeout: Duration::from_millis(1000),
                max_attempts: 50,
            },
            heartbeat: heartbeat::Config::default(),
            bugs: Vec::new(),
            keep_trace: false,
            echo_trace: false,
        }
    }

    pub fn clients(&self) -> u64 {
        u64::from(self.nodes) * self.clients_per_node
    }
}

impl fmt::Display for RunConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "seed={} nodes={} clients={} requests={} quiet={} chaos={:?} latency<={:?} faults=[",
            self.seed,
            self.nodes,
            self.clients(),
            self.requests,
            self.quiet_requests,
            self.chaos,
            self.max_latency
        )?;
        let mut first = true;
        for (on, name) in [
            (self.faults.partitions, "partitions"),
            (self.faults.holds, "holds"),
            (self.faults.crashes, "crashes"),
        ] {
            if on {
                write!(f, "{}{name}", if first { "" } else { " " })?;
                first = false;
            }
        }
        write!(f, "] workload=[{}]", self.workload)?;
        if !self.bugs.is_empty() {
            write!(f, " bugs=[")?;
            for (i, b) in self.bugs.iter().enumerate() {
                write!(f, "{}{}", if i > 0 { " " } else { "" }, b.name())?;
            }
            write!(f, "]")?;
        }
        Ok(())
    }
}

/// What a run that passed has to say for itself.
#[derive(Clone, Debug)]
pub struct Report {
    pub config: RunConfig,
    pub clients: ClientStats,
    pub faults: FaultCounts,
    pub schedule: Schedule,
    /// Per replica, in node order, as the run ended.
    pub replicas: Vec<Snapshot>,
    pub history: usize,
    pub keys: usize,
    pub check: checker::Stats,
    /// The trace's hash. Two runs of one seed must agree on it.
    pub hash: u64,
    pub trace_lines: usize,
    /// turmoil steps, one tick each. The run's cost in the seed's units.
    pub steps: u64,
    /// Virtual time the whole run took.
    pub virtual_time: Duration,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = &self.clients;
        let views: u64 = self.replicas.iter().map(|r| r.stats.view_changes).sum();
        let promotions: u64 = self.replicas.iter().map(|r| r.stats.promotions).sum();
        let pulls: u64 = self.replicas.iter().map(|r| r.stats.pulls_sent).sum();
        let forwarded: u64 = self.replicas.iter().map(|r| r.stats.forwarded).sum();
        let sent: u64 = self.replicas.iter().map(|r| r.net.sent).sum();
        let rejected: u64 = self.replicas.iter().map(|r| r.net.rejected).sum();
        writeln!(
            f,
            "chaos: submitted={} completed={} gave_up={} in {:?}",
            c.submitted - c.quiet_submitted,
            c.completed - c.quiet_completed,
            c.gave_up,
            self.config.chaos
        )?;
        writeln!(
            f,
            "faults: partitions={} holds={} crashes={}",
            self.faults.partitions, self.faults.holds, self.faults.crashes
        )?;
        writeln!(
            f,
            "cluster: view_changes={views} promotions={promotions} pulls={pulls} forwarded={forwarded} datagrams={sent} rejected={rejected}"
        )?;
        writeln!(
            f,
            "quiet: submitted={} completed={}",
            c.quiet_submitted, c.quiet_completed
        )?;
        writeln!(
            f,
            "check: history={} keys={} steps={} cached={} depth={}",
            self.history, self.keys, self.check.steps, self.check.cached, self.check.depth
        )?;
        write!(
            f,
            "cost: ticks={} virtual={:?} trace_lines={}",
            self.steps, self.virtual_time, self.trace_lines
        )
    }
}

/// One seed in, one verdict out. A run that fails panics, as `vopr::run`
/// does: `safety:` for a wrong answer or disagreeing logs, `liveness:`
/// for a quiet request that was not answered, and `halt:` for a replica
/// that stopped.
pub fn run(config: &RunConfig) -> Report {
    let mut prng = Prng::from_seed(config.seed);
    let shared = Shared {
        disks: new_disks(config.nodes),
        trace: Rc::new(RefCell::new(if config.keep_trace {
            Trace::new()
        } else {
            Trace::hash_only()
        })),
        snapshots: Rc::new(RefCell::new(BTreeMap::new())),
        history: Rc::new(RefCell::new(History::new())),
        workload: Rc::new(RefCell::new(Workload::new(
            config.workload.clone(),
            Prng::from_seed(config.workload_seed),
        ))),
    };
    let client_stats: SharedClientStats = Rc::new(RefCell::new(ClientStats::default()));
    {
        let mut trace = shared.trace.borrow_mut();
        trace.set_echo(config.echo_trace);
        trace.record(format_args!("t=0ns config {config}"));
    }

    let mut sim = turmoil::Builder::new()
        .simulation_duration(Duration::from_secs(24 * 3600))
        .tick_duration(Duration::from_millis(1))
        .min_message_latency(Duration::from_millis(1))
        .max_message_latency(config.max_latency)
        .rng_seed(config.seed)
        .build();

    let cluster = Cluster {
        nodes: config.nodes,
        heartbeat: config.heartbeat,
        bugs: config.bugs.clone(),
    };
    for node in 0..config.nodes {
        let cluster = cluster.clone();
        let shared = shared.clone();
        sim.host(node_name(node), move || {
            replica_host(node, cluster.clone(), shared.clone())
        });
    }
    let plan = Plan {
        requests: config.requests,
        quiet_requests: config.quiet_requests,
        quiet_at: config.chaos + config.settle,
        config: config.client,
    };
    for id in 1..=config.clients() {
        let target = ((id - 1) % u64::from(config.nodes)) as NodeId;
        sim.client(
            client_name(id),
            client_host(id, target, plan, shared.clone(), client_stats.clone()),
        );
    }

    let schedule = Schedule::draw(&mut prng, config.nodes, config.chaos, &config.faults);
    let mut counts = FaultCounts::default();
    let mut next = 0;
    let mut steps = 0u64;
    loop {
        let finished = sim
            .step()
            .unwrap_or_else(|e| panic!("{}", classify(&e.to_string())));
        steps += 1;
        let now = sim.elapsed();
        while let Some((at, action)) = schedule.events.get(next) {
            if *at > now {
                break;
            }
            shared
                .trace
                .borrow_mut()
                .record(format_args!("{} fault {action}", Instant::from_nanos(nanos(now))));
            apply(&mut sim, action, &mut counts);
            next += 1;
        }
        if finished {
            break;
        }
    }
    let virtual_time = sim.elapsed();

    // The checks. The history first, because a wrong answer is the
    // finding that matters most.
    let history = shared.history.borrow();
    let report = kv::check(&history, CHECK_LIMIT);
    if let Some(failure) = &report.failed {
        panic!(
            "safety: history of key {:?} is not linearizable: {:?}\n{:#?}",
            String::from_utf8_lossy(&failure.key),
            failure.verdict,
            failure.history.ops()
        );
    }
    let snapshots = shared.snapshots.borrow();
    let replicas: Vec<Snapshot> = (0..config.nodes)
        .map(|n| snapshots.get(&n).cloned().expect("every replica took a step"))
        .collect();
    check_logs(&shared.disks.borrow(), &replicas);

    let clients = *client_stats.borrow();
    assert_eq!(
        clients.quiet_completed, clients.quiet_submitted,
        "liveness: a quiet request was not answered"
    );
    let trace = shared.trace.borrow();
    Report {
        config: config.clone(),
        clients,
        faults: counts,
        schedule,
        replicas,
        history: history.len(),
        keys: report.keys,
        check: report.stats,
        hash: trace.hash(),
        trace_lines: trace.len(),
        steps,
        virtual_time,
    }
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).expect("virtual time fits in u64")
}

/// A host's error, as the run reports it. A client that gave up in the
/// quiet phase says `liveness:` itself. A replica that halted is a
/// damaged log, which this port cannot produce, so it is its own word.
fn classify(error: &str) -> String {
    if error.contains("liveness:") {
        error.to_string()
    } else if error.contains("halted") {
        format!("halt: {error}")
    } else {
        format!("turmoil: {error}")
    }
}

/// Module 10's invariant: the committed prefixes of every pair of logs
/// agree. An uncommitted tail may differ, a committed record never does.
pub fn check_logs(disks: &[Disk], replicas: &[Snapshot]) {
    let logs: Vec<(NodeId, u64, log::Recovered)> = disks
        .iter()
        .zip(replicas)
        .enumerate()
        .map(|(n, (disk, snap))| {
            (
                n as NodeId,
                snap.commit,
                log::recover(superblock::log_bytes(&disk.bytes)),
            )
        })
        .collect();
    for (i, (a, ca, la)) in logs.iter().enumerate() {
        for (b, cb, lb) in &logs[i + 1..] {
            let n = usize::try_from((*ca).min(*cb)).expect("fits");
            let n = n.min(la.records.len()).min(lb.records.len());
            assert!(
                la.records[..n] == lb.records[..n] && la.views[..n] == lb.views[..n],
                "safety: n{a} and n{b} disagree within their committed prefixes"
            );
        }
    }
}

/// Run every seed in `seeds` in order, stop at the first failure, and
/// fold the hashes as `vopr --seeds` does.
pub fn sweep(seeds: std::ops::Range<u64>, requests: u64, bugs: &[Bug]) -> u64 {
    let mut folded = 0u64;
    for seed in seeds {
        let mut config = RunConfig::from_seed(seed, requests);
        config.bugs = bugs.to_vec();
        let report = run(&config);
        folded = folded.rotate_left(7) ^ report.hash;
    }
    folded
}
