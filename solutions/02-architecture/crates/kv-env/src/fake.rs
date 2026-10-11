//! The in-memory environment.
//!
//! Deliberately trivial in Module 2: messages are delivered in FIFO order,
//! appends complete when [`FakeStorage::complete_next`] is called (or all at
//! once), and the clock moves only when told. Later modules replace each
//! piece with a seed-driven model; the traits stay the same.

use determinism::{trace, Trace};
use kv_core::client::{self, Client};
use kv_core::env::{Driver, Network, Observer, Storage, Time};
use kv_core::node::{Input, OpId, Output};
use kv_core::{Duration, Instant, Request, RequestEnvelope, Response, ResponseEnvelope};
use std::collections::{BTreeMap, VecDeque};

/// A clock that only moves when asked.
#[derive(Clone, Debug, Default)]
pub struct FakeTime {
    now: Instant,
}

impl FakeTime {
    pub fn advance(&mut self, d: Duration) {
        self.now = self.now + d;
    }
}

impl Time for FakeTime {
    fn now(&self) -> Instant {
        self.now
    }
}

/// A log held in a `Vec<u8>`, with explicit completion.
#[derive(Clone, Debug, Default)]
pub struct FakeStorage {
    pub log: Vec<u8>,
    pending_appends: VecDeque<(OpId, u64, Vec<u8>)>,
    completions: VecDeque<Input>,
    /// If true, appends complete on the next poll without being asked.
    pub auto_complete: bool,
}

impl FakeStorage {
    pub fn new(auto_complete: bool) -> FakeStorage {
        FakeStorage {
            auto_complete,
            ..Default::default()
        }
    }

    /// Start from an existing log, as after a restart.
    pub fn with_log(log: Vec<u8>, auto_complete: bool) -> FakeStorage {
        FakeStorage {
            log,
            auto_complete,
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

    pub fn pending(&self) -> usize {
        self.pending_appends.len()
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
        self.pending_appends.push_back((op, offset, bytes.to_vec()));
    }

    fn poll_completions(&mut self, inputs: &mut Vec<Input>) {
        if self.auto_complete {
            while self.complete_next() {}
        }
        inputs.extend(self.completions.drain(..));
    }
}

/// A loopback network: requests go in one queue, replies come out another.
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

pub type FakeDriver = Driver<FakeTime, FakeStorage, FakeNetwork, TraceObserver>;

/// A node plus some clients, wired together in memory. Enough of a
/// "simulator" to test the state machines end to end; Module 4 replaces the
/// hand-advanced clock with a real event scheduler.
pub struct World {
    pub driver: FakeDriver,
    pub clients: BTreeMap<u64, Client>,
    /// Results delivered to clients: (client, request_id, response).
    pub completed: Vec<(u64, u64, Response)>,
    pub gave_up: Vec<(u64, u64)>,
    /// Replies the network should lose, by (client, request_id). Each entry
    /// is consumed once, so the retry gets through.
    pub drop_replies: Vec<(u64, u64)>,
    client_out: Vec<client::Output>,
}

impl World {
    pub fn new(storage: FakeStorage) -> World {
        let driver = Driver::new(
            FakeTime::default(),
            storage,
            FakeNetwork::default(),
            TraceObserver::default(),
        );
        World {
            driver,
            clients: BTreeMap::new(),
            completed: Vec::new(),
            gave_up: Vec::new(),
            drop_replies: Vec::new(),
            client_out: Vec::new(),
        }
    }

    pub fn add_client(&mut self, id: u64, config: client::Config) {
        let prev = self.clients.insert(id, Client::new(id, config));
        assert!(prev.is_none(), "duplicate client id {id}");
    }

    pub fn now(&self) -> Instant {
        self.driver.time.now()
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
        rid
    }

    /// Move time forward and let clients notice.
    pub fn advance(&mut self, d: Duration) {
        self.driver.time.advance(d);
        let now = self.now();
        let ids: Vec<u64> = self.clients.keys().copied().collect();
        for id in ids {
            self.clients
                .get_mut(&id)
                .unwrap()
                .on_input(client::Input::Tick { now }, &mut self.client_out);
            self.route(id);
        }
    }

    /// Run node and network until nothing moves. Time does not advance.
    pub fn settle(&mut self) {
        loop {
            self.driver.run_until_idle();
            let mut moved = false;
            while let Some(reply) = self.driver.network.outbox.pop_front() {
                moved = true;
                let key = (reply.client, reply.request_id);
                if let Some(i) = self.drop_replies.iter().position(|&k| k == key) {
                    self.drop_replies.swap_remove(i);
                    trace!(
                        self.driver.observer.trace,
                        "{} net dropped reply client={} id={}",
                        self.now(),
                        key.0,
                        key.1
                    );
                    continue;
                }
                let client = self
                    .clients
                    .get_mut(&reply.client)
                    .expect("reply for unknown client");
                client.on_input(client::Input::Response(reply), &mut self.client_out);
                self.route(key.0);
            }
            if !moved && self.driver.step() == 0 {
                break;
            }
        }
    }

    /// Submit, settle, and return the response. Panics if the operation did
    /// not complete without time passing (i.e. something was dropped).
    pub fn call(&mut self, id: u64, request: Request) -> Response {
        let rid = self.submit(id, request);
        self.settle();
        self.take_completed(id, rid)
            .expect("request did not complete while settling")
    }

    pub fn take_completed(&mut self, id: u64, rid: u64) -> Option<Response> {
        let i = self
            .completed
            .iter()
            .position(|(c, r, _)| (*c, *r) == (id, rid))?;
        Some(self.completed.remove(i).2)
    }

    /// Deliver what client `id` just produced.
    fn route(&mut self, id: u64) {
        let now = self.driver.time.now();
        for out in self.client_out.drain(..) {
            match out {
                client::Output::Send(envelope) => {
                    assert_eq!(envelope.client, id);
                    trace!(
                        self.driver.observer.trace,
                        "{now} net send client={id} id={}",
                        envelope.request_id
                    );
                    self.driver.network.deliver(envelope);
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
