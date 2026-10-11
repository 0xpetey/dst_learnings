//! The simulated environment.
//!
//! Module 2 had a clock that tests advanced by hand and a disk that
//! completed when a test said so. Module 4 replaces both with a
//! [`sim::Scheduler`]. Every effect that takes time is an [`Event`] with a
//! virtual timestamp:
//!
//! ```text
//!   client.submit ──► Send ──► DeliverRequest @ now+net ──► node
//!   node ─────────► Append ──► DiskComplete  @ now+disk ──► node ──► Reply
//!   node ─────────► Reply ───► DeliverReply  @ now+net ──► client
//!   client.next_deadline() ──► ClientTimer   @ deadline ──► client.on_tick
//! ```
//!
//! [`World::step`] pops one event, moves the clock to it, and handles it.
//! Nothing here sleeps and nothing reads a real clock, so a simulated year
//! costs as many steps as there are events in it, and no more.
//!
//! The network and disk models here are the simplest possible: a fixed
//! latency, with optional uniform jitter from the seed. Module 5 and 6
//! replace them with adversarial models. The scheduler does not change.

use determinism::{trace, Prng, Trace};
use kv_core::client::{self, Client};
use kv_core::env::{Driver, Network, Observer, Storage};
use kv_core::node::{Input, OpId, Output};
use kv_core::{Duration, Instant, Request, RequestEnvelope, Response, ResponseEnvelope};
use sim::{Clock, Fired, Scheduler, Timer};
use std::collections::{BTreeMap, VecDeque};

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

/// A loopback network: requests go in one queue, replies come out another.
/// The world moves things between the queues and the scheduler.
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

/// Records every input and output into a `Trace`.
#[derive(Debug, Default)]
pub struct TraceObserver {
    pub trace: Trace,
}

impl Observer for TraceObserver {
    fn input(&mut self, now: Instant, input: &Input) {
        match input {
            Input::ReadComplete { op, data } => {
                trace!(self.trace, "{now} in  ReadComplete op={op} bytes={}", data.len())
            }
            Input::AppendComplete { op } => trace!(self.trace, "{now} in  AppendComplete op={op}"),
            Input::Request(r) => trace!(
                self.trace,
                "{now} in  Request client={} id={} {:?}",
                r.client,
                r.request_id,
                r.request
            ),
        }
    }

    fn output(&mut self, now: Instant, output: &Output) {
        match output {
            Output::Read { op } => trace!(self.trace, "{now} out Read op={op}"),
            Output::Append { op, offset, bytes } => trace!(
                self.trace,
                "{now} out Append op={op} offset={offset} bytes={}",
                bytes.len()
            ),
            Output::Reply(r) => trace!(
                self.trace,
                "{now} out Reply client={} id={} {:?}",
                r.client,
                r.request_id,
                r.response
            ),
        }
    }
}

pub type FakeDriver = Driver<Clock, FakeStorage, FakeNetwork, TraceObserver>;

/// Everything that can happen in the simulated world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A client's `next_deadline` arrived.
    ClientTimer { client: u64 },
    /// A request reaches the node.
    DeliverRequest(RequestEnvelope),
    /// A reply reaches its client.
    DeliverReply(ResponseEnvelope),
    /// The oldest pending append is durable.
    DiskComplete,
}

/// Latencies and the seed. Jitter is added uniformly in `[0, jitter]` from
/// the world's own `Prng`, so two worlds with the same config and seed see
/// the same delays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldConfig {
    pub seed: u64,
    /// One-way network latency, client to node or node to client.
    pub net_latency: Duration,
    pub net_jitter: Duration,
    /// Time from `Append` to durable.
    pub disk_latency: Duration,
    pub disk_jitter: Duration,
    /// Keep every trace line, or only the running hash. Turn this off for
    /// long runs; the hash is what the determinism check compares anyway.
    pub keep_trace_lines: bool,
}

impl Default for WorldConfig {
    fn default() -> WorldConfig {
        WorldConfig {
            seed: 0,
            net_latency: Duration::from_millis(1),
            net_jitter: Duration::ZERO,
            disk_latency: Duration::from_millis(5),
            disk_jitter: Duration::ZERO,
            keep_trace_lines: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldStats {
    pub timers_fired: u64,
    pub requests_delivered: u64,
    pub requests_dropped: u64,
    pub replies_delivered: u64,
    pub replies_dropped: u64,
    pub appends_completed: u64,
}

/// A node plus some clients, wired together by the scheduler.
pub struct World {
    pub driver: FakeDriver,
    pub scheduler: Scheduler<Event>,
    pub prng: Prng,
    pub config: WorldConfig,
    pub clients: BTreeMap<u64, Client>,
    timers: BTreeMap<u64, Timer>,
    /// Results delivered to clients: (client, request_id, response).
    pub completed: Vec<(u64, u64, Response)>,
    pub gave_up: Vec<(u64, u64)>,
    /// Replies the network should lose, by (client, request_id). Each entry
    /// is consumed once, so the retry gets through.
    pub drop_replies: Vec<(u64, u64)>,
    /// While true, every client request is lost on the way to the node.
    pub partitioned: bool,
    pub stats: WorldStats,
    /// Appends for which a `DiskComplete` event exists.
    disk_events_issued: u64,
    /// Completion time of the last scheduled append. Appends complete in
    /// submission order, so the next one cannot complete before this.
    disk_busy_until: Instant,
    client_out: Vec<client::Output>,
}

impl World {
    pub fn new(storage: FakeStorage) -> World {
        World::with_config(storage, WorldConfig::default())
    }

    pub fn with_config(storage: FakeStorage, config: WorldConfig) -> World {
        let scheduler = Scheduler::new();
        let trace = if config.keep_trace_lines {
            Trace::new()
        } else {
            Trace::hash_only()
        };
        let driver = Driver::new(
            scheduler.clock(),
            storage,
            FakeNetwork::default(),
            TraceObserver { trace },
        );
        let mut world = World {
            driver,
            scheduler,
            prng: Prng::from_seed(config.seed),
            config,
            clients: BTreeMap::new(),
            timers: BTreeMap::new(),
            completed: Vec::new(),
            gave_up: Vec::new(),
            drop_replies: Vec::new(),
            partitioned: false,
            stats: WorldStats::default(),
            disk_events_issued: 0,
            disk_busy_until: Instant::ZERO,
            client_out: Vec::new(),
        };
        // Recovery: the node's initial log read completes at once.
        world.pump();
        world
    }

    pub fn add_client(&mut self, id: u64, config: client::Config) {
        let prev = self.clients.insert(id, Client::new(id, config));
        assert!(prev.is_none(), "duplicate client id {id}");
        self.timers.insert(id, Timer::new());
    }

    pub fn now(&self) -> Instant {
        self.scheduler.now()
    }

    pub fn trace(&self) -> &Trace {
        &self.driver.observer.trace
    }

    /// Client `id` starts `request`. Returns its request id.
    pub fn submit(&mut self, id: u64, request: Request) -> u64 {
        let now = self.now();
        let client = self.clients.get_mut(&id).expect("unknown client");
        let rid = client.submit(now, request, &mut self.client_out);
        self.route(id);
        self.arm_timer(id);
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

    /// Handle events until the queue is empty. A client that never gives up
    /// and never gets an answer keeps the queue busy forever; bound such
    /// runs with [`World::run_until`] instead.
    pub fn run_until_idle(&mut self) {
        while self.step() {}
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
    pub fn call(&mut self, id: u64, request: Request) -> Response {
        let rid = self.submit(id, request);
        let finished = |w: &World| {
            w.completed.iter().any(|(c, r, _)| (*c, *r) == (id, rid)) || w.gave_up.contains(&(id, rid))
        };
        let ran = self.run_while(|w| !finished(w));
        assert!(ran, "world went idle before client {id} request {rid} completed");
        self.take_completed(id, rid)
            .unwrap_or_else(|| panic!("client {id} gave up on request {rid}"))
    }

    pub fn take_completed(&mut self, id: u64, rid: u64) -> Option<Response> {
        let i = self
            .completed
            .iter()
            .position(|(c, r, _)| (*c, *r) == (id, rid))?;
        Some(self.completed.remove(i).2)
    }

    fn handle(&mut self, fired: Fired<Event>) {
        let now = fired.at;
        debug_assert_eq!(now, self.now());
        match fired.event {
            Event::ClientTimer { client: id } => {
                let timer = self.timers.get_mut(&id).expect("timer for unknown client");
                assert!(timer.fired(fired.id), "stale timer event for client {id}");
                self.stats.timers_fired += 1;
                trace!(self.driver.observer.trace, "{now} timer client={id}");
                self.clients
                    .get_mut(&id)
                    .unwrap()
                    .on_input(client::Input::Tick { now }, &mut self.client_out);
                self.route(id);
                self.arm_timer(id);
            }
            Event::DeliverRequest(envelope) => {
                self.stats.requests_delivered += 1;
                trace!(
                    self.driver.observer.trace,
                    "{now} net deliver request client={} id={}",
                    envelope.client,
                    envelope.request_id
                );
                self.driver.network.deliver(envelope);
                self.pump();
            }
            Event::DeliverReply(envelope) => {
                let id = envelope.client;
                self.stats.replies_delivered += 1;
                trace!(
                    self.driver.observer.trace,
                    "{now} net deliver reply client={id} id={}",
                    envelope.request_id
                );
                let client = self.clients.get_mut(&id).expect("reply for unknown client");
                client.on_input(client::Input::Response(envelope), &mut self.client_out);
                self.route(id);
                self.arm_timer(id);
            }
            Event::DiskComplete => {
                assert!(
                    self.driver.storage.complete_next(),
                    "DiskComplete with nothing pending"
                );
                self.stats.appends_completed += 1;
                trace!(self.driver.observer.trace, "{now} disk complete");
                self.pump();
            }
        }
    }

    /// Let the node process everything it has, then turn its effects into
    /// scheduled events.
    fn pump(&mut self) {
        self.driver.run_until_idle();
        let now = self.now();
        while let Some(reply) = self.driver.network.outbox.pop_front() {
            let key = (reply.client, reply.request_id);
            if let Some(i) = self.drop_replies.iter().position(|&k| k == key) {
                self.drop_replies.swap_remove(i);
                self.stats.replies_dropped += 1;
                trace!(
                    self.driver.observer.trace,
                    "{now} net dropped reply client={} id={}",
                    key.0,
                    key.1
                );
                continue;
            }
            let delay = self.latency(self.config.net_latency, self.config.net_jitter);
            self.scheduler.schedule_after(delay, Event::DeliverReply(reply));
        }
        while self.disk_events_issued < self.driver.storage.submitted() {
            let delay = self.latency(self.config.disk_latency, self.config.disk_jitter);
            let at = (now + delay).max(self.disk_busy_until);
            self.disk_busy_until = at;
            self.scheduler.schedule_at(at, Event::DiskComplete);
            self.disk_events_issued += 1;
        }
    }

    fn latency(&mut self, base: Duration, jitter: Duration) -> Duration {
        if jitter.is_zero() {
            return base;
        }
        let jitter_nanos = u64::try_from(jitter.as_nanos()).expect("jitter fits u64");
        base + Duration::from_nanos(self.prng.below(jitter_nanos + 1))
    }

    /// Keep client `id`'s timer event in step with its deadline.
    fn arm_timer(&mut self, id: u64) {
        let want = self.clients[&id].next_deadline();
        self.timers
            .get_mut(&id)
            .unwrap()
            .reconcile(&mut self.scheduler, want, || Event::ClientTimer { client: id });
    }

    /// Deliver what client `id` just produced.
    fn route(&mut self, id: u64) {
        let now = self.now();
        for out in std::mem::take(&mut self.client_out) {
            match out {
                client::Output::Send(envelope) => {
                    assert_eq!(envelope.client, id);
                    if self.partitioned {
                        self.stats.requests_dropped += 1;
                        trace!(
                            self.driver.observer.trace,
                            "{now} net dropped request client={id} id={}",
                            envelope.request_id
                        );
                        continue;
                    }
                    trace!(
                        self.driver.observer.trace,
                        "{now} net send client={id} id={}",
                        envelope.request_id
                    );
                    let delay = self.latency(self.config.net_latency, self.config.net_jitter);
                    self.scheduler
                        .schedule_after(delay, Event::DeliverRequest(envelope));
                }
                client::Output::Done { request_id, response } => {
                    trace!(
                        self.driver.observer.trace,
                        "{now} client={id} done id={request_id} {response:?}"
                    );
                    self.completed.push((id, request_id, response));
                }
                client::Output::GaveUp { request_id } => {
                    trace!(
                        self.driver.observer.trace,
                        "{now} client={id} gave up id={request_id}"
                    );
                    self.gave_up.push((id, request_id));
                }
            }
        }
    }
}
