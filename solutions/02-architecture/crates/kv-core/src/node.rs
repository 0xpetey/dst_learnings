//! The single-node KV store as a state machine.
//!
//! ```text
//!   inputs                       outputs
//!   Request ─────────┐    ┌───── Reply
//!   ReadComplete ────┼──► │ ──── Append { op, bytes }
//!   AppendComplete ──┘    └───── Read { op }
//! ```
//!
//! Life cycle: a new node first asks the environment to read its log
//! (`Output::Read`). Until `ReadComplete` arrives it is *recovering* and
//! queues requests. Then it serves: reads answer from the in-memory map;
//! writes are appended to the log and applied, and answered, only when the
//! environment reports the append durable.
//!
//! Sessions: each client's requests carry increasing ids. The node remembers
//! the last id per client and its response, so a retried request (same id)
//! gets the same answer without being applied twice. This is what makes
//! client timeouts safe.

use crate::log::{self, Entry};
use crate::protocol::{ClientId, Request, RequestEnvelope, RequestId, Response, ResponseEnvelope};
use std::collections::{BTreeMap, VecDeque};

/// Identifies one storage operation from submission to completion.
pub type OpId = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// A client request arrived.
    Request(RequestEnvelope),
    /// The read issued by `Output::Read { op }` finished with these bytes.
    ReadComplete { op: OpId, data: Vec<u8> },
    /// The bytes from `Output::Append { op, .. }` are durable.
    AppendComplete { op: OpId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Read the whole log. Exactly one of these is ever issued, at start.
    Read { op: OpId },
    /// Append bytes to the log at `offset` and make them durable.
    Append { op: OpId, offset: u64, bytes: Vec<u8> },
    /// Send a reply to a client.
    Reply(ResponseEnvelope),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Recovering { op: OpId },
    Running,
}

#[derive(Clone, Debug)]
struct Session {
    last_request_id: RequestId,
    /// `None` while the request is still being made durable.
    last_response: Option<Response>,
}

#[derive(Clone, Debug)]
struct PendingWrite {
    op: OpId,
    client: ClientId,
    request_id: RequestId,
    entry: Entry,
}

/// Counters, useful to assert on in tests and to print at the end of a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub requests: u64,
    pub duplicates: u64,
    pub stale: u64,
    pub applied: u64,
    pub recovered_entries: u64,
}

#[derive(Debug)]
pub struct Node {
    state: State,
    map: BTreeMap<Vec<u8>, Vec<u8>>,
    sessions: BTreeMap<ClientId, Session>,
    pending: VecDeque<PendingWrite>,
    queued: VecDeque<RequestEnvelope>,
    next_op: OpId,
    /// Bytes of log known durable; the offset of the next append.
    log_len: u64,
    stats: Stats,
}

impl Default for Node {
    fn default() -> Self {
        Node::new(&mut Vec::new())
    }
}

impl Node {
    /// Create a node. It immediately asks for its log via `out`.
    pub fn new(out: &mut Vec<Output>) -> Node {
        let node = Node {
            state: State::Recovering { op: 0 },
            map: BTreeMap::new(),
            sessions: BTreeMap::new(),
            pending: VecDeque::new(),
            queued: VecDeque::new(),
            next_op: 1,
            log_len: 0,
            stats: Stats::default(),
        };
        out.push(Output::Read { op: 0 });
        node
    }

    pub fn is_running(&self) -> bool {
        self.state == State::Running
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Number of writes submitted to storage and not yet durable.
    pub fn pending_writes(&self) -> usize {
        self.pending.len()
    }

    /// Current value of `key` in the applied state. For tests and checkers.
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.map.get(key).map(Vec::as_slice)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Advance the state machine by one input, appending any effects to `out`.
    pub fn on_input(&mut self, input: Input, out: &mut Vec<Output>) {
        match input {
            Input::ReadComplete { op, data } => self.on_read_complete(op, &data, out),
            Input::Request(envelope) => match self.state {
                State::Recovering { .. } => self.queued.push_back(envelope),
                State::Running => self.on_request(envelope, out),
            },
            Input::AppendComplete { op } => self.on_append_complete(op, out),
        }
        self.check_invariants();
    }

    fn on_read_complete(&mut self, op: OpId, data: &[u8], out: &mut Vec<Output>) {
        assert_eq!(
            self.state,
            State::Recovering { op },
            "ReadComplete while not recovering"
        );
        let recovered = log::recover(data).expect("log is corrupt beyond a torn tail; refusing to start");
        for entry in recovered.entries {
            self.apply(&entry);
            self.stats.recovered_entries += 1;
        }
        self.log_len = recovered.valid_len as u64;
        self.state = State::Running;
        while let Some(envelope) = self.queued.pop_front() {
            self.on_request(envelope, out);
        }
    }

    fn on_request(&mut self, envelope: RequestEnvelope, out: &mut Vec<Output>) {
        assert!(self.is_running());
        self.stats.requests += 1;
        let RequestEnvelope {
            client,
            request_id,
            request,
        } = envelope;

        if let Some(session) = self.sessions.get(&client) {
            if request_id < session.last_request_id {
                // Older than something we already answered: a reordered
                // duplicate from the network. The client has moved on.
                self.stats.stale += 1;
                return;
            }
            if request_id == session.last_request_id {
                self.stats.duplicates += 1;
                // Retry of a finished request: same answer, no re-apply.
                // Retry of a request still being made durable: nothing to do,
                // the reply goes out when the append completes.
                if let Some(response) = &session.last_response {
                    out.push(Output::Reply(ResponseEnvelope {
                        client,
                        request_id,
                        response: response.clone(),
                    }));
                }
                return;
            }
        }

        if let Err(why) = request.validate() {
            self.finish(client, request_id, Response::Rejected(why), out);
            return;
        }

        match request {
            Request::Get { key } => {
                let response = Response::Value(self.map.get(&key).cloned());
                self.finish(client, request_id, response, out);
            }
            Request::Put { key, value } => self.submit(client, request_id, Entry::Put { key, value }, out),
            Request::Delete { key } => self.submit(client, request_id, Entry::Delete { key }, out),
        }
    }

    /// Record a completed request in its session and send the reply.
    fn finish(&mut self, client: ClientId, request_id: RequestId, response: Response, out: &mut Vec<Output>) {
        self.sessions.insert(
            client,
            Session {
                last_request_id: request_id,
                last_response: Some(response.clone()),
            },
        );
        out.push(Output::Reply(ResponseEnvelope {
            client,
            request_id,
            response,
        }));
    }

    /// Hand a write to storage and remember it until it is durable.
    fn submit(&mut self, client: ClientId, request_id: RequestId, entry: Entry, out: &mut Vec<Output>) {
        self.sessions.insert(
            client,
            Session {
                last_request_id: request_id,
                last_response: None,
            },
        );
        let op = self.next_op;
        self.next_op += 1;
        let mut bytes = Vec::new();
        entry.encode(&mut bytes);
        // The offset is where this append lands if all earlier pending
        // appends land first. Storage completes in order, so it will.
        let offset = self.log_len + self.pending.iter().map(|p| encoded_len(&p.entry)).sum::<u64>();
        self.pending.push_back(PendingWrite {
            op,
            client,
            request_id,
            entry,
        });
        out.push(Output::Append { op, offset, bytes });
    }

    fn on_append_complete(&mut self, op: OpId, out: &mut Vec<Output>) {
        assert!(self.is_running(), "AppendComplete before recovery finished");
        let pending = self
            .pending
            .pop_front()
            .expect("AppendComplete with nothing pending");
        // Storage is required to complete appends in submission order. A
        // completion for anything but the oldest pending write means the
        // environment broke its contract, and the node's offsets are wrong.
        assert_eq!(pending.op, op, "appends completed out of order");
        self.log_len += encoded_len(&pending.entry);
        let response = self.apply(&pending.entry);
        self.stats.applied += 1;

        let session = self
            .sessions
            .get_mut(&pending.client)
            .expect("pending write has a session");
        assert_eq!(
            session.last_request_id, pending.request_id,
            "session moved on while a write was pending"
        );
        assert!(session.last_response.is_none());
        session.last_response = Some(response.clone());
        out.push(Output::Reply(ResponseEnvelope {
            client: pending.client,
            request_id: pending.request_id,
            response,
        }));
    }

    /// Apply a durable entry to the map and compute the response it earns.
    fn apply(&mut self, entry: &Entry) -> Response {
        match entry {
            Entry::Put { key, value } => {
                self.map.insert(key.clone(), value.clone());
                Response::PutOk
            }
            Entry::Delete { key } => Response::DeleteOk {
                existed: self.map.remove(key).is_some(),
            },
        }
    }

    /// Everything that must hold between inputs. Cheap enough to run always;
    /// a simulator that only checks invariants in debug builds has two
    /// behaviours.
    fn check_invariants(&self) {
        match self.state {
            State::Recovering { .. } => {
                assert!(
                    self.pending.is_empty(),
                    "no writes may be pending during recovery"
                );
                assert!(self.map.is_empty() || self.stats.recovered_entries > 0);
            }
            State::Running => assert!(self.queued.is_empty(), "queue must drain when recovery completes"),
        }
        let mut last_op = 0;
        for p in &self.pending {
            assert!(p.op > last_op, "pending ops must be strictly increasing");
            last_op = p.op;
            let session = &self.sessions[&p.client];
            assert_eq!(session.last_request_id, p.request_id);
            assert!(
                session.last_response.is_none(),
                "a pending write cannot already have a response"
            );
        }
        let in_flight = self
            .sessions
            .values()
            .filter(|s| s.last_response.is_none())
            .count();
        assert_eq!(
            in_flight,
            self.pending.len(),
            "every in-flight session has exactly one pending write"
        );
        assert!(self.next_op > last_op);
    }
}

fn encoded_len(entry: &Entry) -> u64 {
    let mut v = Vec::new();
    entry.encode(&mut v);
    v.len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(client: ClientId, id: RequestId, request: Request) -> Input {
        Input::Request(RequestEnvelope {
            client,
            request_id: id,
            request,
        })
    }

    fn put(k: &str, v: &str) -> Request {
        Request::Put {
            key: k.into(),
            value: v.into(),
        }
    }

    fn get(k: &str) -> Request {
        Request::Get { key: k.into() }
    }

    fn started() -> (Node, Vec<Output>) {
        let mut out = Vec::new();
        let mut node = Node::new(&mut out);
        assert_eq!(out, vec![Output::Read { op: 0 }]);
        out.clear();
        node.on_input(Input::ReadComplete { op: 0, data: vec![] }, &mut out);
        assert!(node.is_running());
        (node, out)
    }

    #[test]
    fn put_is_applied_only_when_durable() {
        let (mut node, mut out) = started();
        node.on_input(req(1, 1, put("k", "v")), &mut out);
        let Output::Append { op, offset, .. } = &out[0] else {
            panic!("expected append, got {out:?}")
        };
        assert_eq!(*offset, 0);
        assert_eq!(node.get(b"k"), None, "not applied before durability");
        let op = *op;
        out.clear();
        node.on_input(Input::AppendComplete { op }, &mut out);
        assert_eq!(node.get(b"k"), Some(&b"v"[..]));
        assert_eq!(
            out,
            vec![Output::Reply(ResponseEnvelope {
                client: 1,
                request_id: 1,
                response: Response::PutOk
            })]
        );
    }

    #[test]
    fn get_reads_applied_state_only() {
        let (mut node, mut out) = started();
        node.on_input(req(1, 1, put("k", "v")), &mut out);
        out.clear();
        node.on_input(req(2, 1, get("k")), &mut out);
        assert_eq!(
            out,
            vec![Output::Reply(ResponseEnvelope {
                client: 2,
                request_id: 1,
                response: Response::Value(None)
            })]
        );
    }

    #[test]
    fn retry_gets_same_answer_without_double_apply() {
        let (mut node, mut out) = started();
        node.on_input(req(1, 1, Request::Delete { key: b"k".into() }), &mut out);
        node.on_input(Input::AppendComplete { op: 1 }, &mut out);
        out.clear();
        // The reply was lost; the client resends the same request id.
        node.on_input(req(1, 1, Request::Delete { key: b"k".into() }), &mut out);
        assert_eq!(
            out,
            vec![Output::Reply(ResponseEnvelope {
                client: 1,
                request_id: 1,
                response: Response::DeleteOk { existed: false }
            })]
        );
        assert_eq!(node.stats().applied, 1);
        assert_eq!(node.stats().duplicates, 1);
    }

    #[test]
    fn retry_while_pending_is_absorbed() {
        let (mut node, mut out) = started();
        node.on_input(req(1, 1, put("k", "v")), &mut out);
        out.clear();
        node.on_input(req(1, 1, put("k", "v")), &mut out);
        assert!(out.is_empty(), "no second append, no reply yet");
        assert_eq!(node.pending_writes(), 1);
        node.on_input(Input::AppendComplete { op: 1 }, &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn stale_request_is_ignored() {
        let (mut node, mut out) = started();
        node.on_input(req(1, 5, get("k")), &mut out);
        out.clear();
        node.on_input(req(1, 4, get("k")), &mut out);
        assert!(out.is_empty());
        assert_eq!(node.stats().stale, 1);
    }

    #[test]
    fn requests_during_recovery_are_queued_then_served_in_order() {
        let mut out = Vec::new();
        let mut node = Node::new(&mut out);
        out.clear();
        node.on_input(req(1, 1, get("a")), &mut out);
        node.on_input(req(2, 1, get("b")), &mut out);
        assert!(out.is_empty());
        let mut log = Vec::new();
        Entry::Put {
            key: b"a".into(),
            value: b"1".into(),
        }
        .encode(&mut log);
        node.on_input(Input::ReadComplete { op: 0, data: log }, &mut out);
        assert_eq!(out.len(), 2);
        assert!(
            matches!(&out[0], Output::Reply(r) if r.client == 1 && r.response == Response::Value(Some(b"1".to_vec())))
        );
        assert!(matches!(&out[1], Output::Reply(r) if r.client == 2 && r.response == Response::Value(None)));
        assert_eq!(node.stats().recovered_entries, 1);
    }

    #[test]
    fn offsets_account_for_pending_appends() {
        let (mut node, mut out) = started();
        node.on_input(req(1, 1, put("k", "v")), &mut out);
        node.on_input(req(2, 1, put("k2", "v2")), &mut out);
        let offsets: Vec<u64> = out
            .iter()
            .map(|o| match o {
                Output::Append { offset, .. } => *offset,
                _ => panic!(),
            })
            .collect();
        let first_len = match &out[0] {
            Output::Append { bytes, .. } => bytes.len() as u64,
            _ => unreachable!(),
        };
        assert_eq!(offsets, vec![0, first_len]);
    }

    #[test]
    fn rejected_requests_never_touch_storage() {
        let (mut node, mut out) = started();
        node.on_input(
            req(
                1,
                1,
                Request::Put {
                    key: vec![],
                    value: vec![],
                },
            ),
            &mut out,
        );
        assert_eq!(
            out,
            vec![Output::Reply(ResponseEnvelope {
                client: 1,
                request_id: 1,
                response: Response::Rejected(crate::protocol::Rejection::EmptyKey)
            })]
        );
        assert_eq!(node.pending_writes(), 0);
    }

    #[test]
    #[should_panic(expected = "out of order")]
    fn out_of_order_completion_is_an_environment_bug() {
        let (mut node, mut out) = started();
        node.on_input(req(1, 1, put("a", "1")), &mut out);
        node.on_input(req(2, 1, put("b", "2")), &mut out);
        node.on_input(Input::AppendComplete { op: 2 }, &mut out);
    }
}
