//! The single-node KV store as a state machine.
//!
//! ```text
//!   inputs                       outputs
//!   Request ─────────┐    ┌───── Reply
//!   ReadComplete ────┼──► │ ──── Append { op, bytes }
//!   AppendComplete ──┘    └───── Read { op }
//!                         └───── Halt { damage }
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
//! client timeouts safe. The log records carry the same identity, so the
//! sessions survive a restart. Module 6 added that after a crash between
//! "durable" and "replied" made a retried delete run twice.
//!
//! Damage: recovery can find a record that was once written and is now
//! unreadable, with later records to prove it. The node cannot know what
//! that record said, so it halts instead of serving a state with a hole in
//! it. On a single node that is the end. Module 10 fetches the record from
//! a replica instead.

use crate::log::{self, Damage, Entry, Record, Tail};
use crate::protocol::{ClientId, Request, RequestEnvelope, RequestId, Response, ResponseEnvelope};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;

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
    /// The log is damaged. Stop driving this node; it will not serve.
    Halt { damage: Damage },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Recovering { op: OpId },
    Running,
    Halted,
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
    seq: u64,
    record: Record,
}

/// Counters, useful to assert on in tests and to print at the end of a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub requests: u64,
    pub duplicates: u64,
    pub stale: u64,
    pub applied: u64,
    pub recovered_entries: u64,
    /// Requests that arrived after the node halted. They get no reply.
    pub dropped: u64,
}

/// What the node's log looked like when it started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub records: u64,
    pub valid_len: u64,
    pub tail: Tail,
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
    /// Records known durable; the sequence number of the last one.
    log_seq: u64,
    recovery: Option<Recovery>,
    /// The records recovery found, kept for checkers.
    recovered: Vec<Record>,
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
            log_seq: 0,
            recovery: None,
            recovered: Vec::new(),
            stats: Stats::default(),
        };
        out.push(Output::Read { op: 0 });
        node
    }

    pub fn is_running(&self) -> bool {
        self.state == State::Running
    }

    pub fn is_halted(&self) -> bool {
        self.state == State::Halted
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// What recovery found, once the log has been read.
    pub fn recovery(&self) -> Option<Recovery> {
        self.recovery
    }

    /// The records recovery applied, in order. A checker compares them
    /// with what clients were told.
    pub fn recovered_records(&self) -> &[Record] {
        &self.recovered
    }

    /// Why the node halted, if it did.
    pub fn damage(&self) -> Option<Damage> {
        match self.recovery?.tail {
            Tail::Damaged(d) => Some(d),
            _ => None,
        }
    }

    /// Number of writes submitted to storage and not yet durable.
    pub fn pending_writes(&self) -> usize {
        self.pending.len()
    }

    /// Records known durable.
    pub fn log_seq(&self) -> u64 {
        self.log_seq
    }

    /// Bytes known durable.
    pub fn log_len(&self) -> u64 {
        self.log_len
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
                State::Halted => self.stats.dropped += 1,
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
        let recovered = log::recover(data);
        self.recovery = Some(Recovery {
            records: recovered.records.len() as u64,
            valid_len: recovered.valid_len as u64,
            tail: recovered.tail,
        });
        if let Tail::Damaged(damage) = recovered.tail {
            // Serving on top of a hole would answer reads with state that
            // is missing an acknowledged write. Refuse.
            self.state = State::Halted;
            self.stats.dropped += self.queued.len() as u64;
            self.queued.clear();
            out.push(Output::Halt { damage });
            return;
        }
        for record in recovered.records {
            let response = self.apply(&record.entry);
            // The session comes back with the record, so a retry of this
            // request after the restart gets the same answer.
            self.sessions.insert(
                record.client,
                Session {
                    last_request_id: record.request_id,
                    last_response: Some(response),
                },
            );
            self.stats.recovered_entries += 1;
            self.log_seq += 1;
            self.recovered.push(record);
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
        let record = Record {
            client,
            request_id,
            entry,
        };
        // The offset and the sequence number are where this append lands
        // if all earlier pending appends land first. Storage completes in
        // order, so they will.
        let seq = self.log_seq + self.pending.len() as u64 + 1;
        let offset = self.log_len
            + self
                .pending
                .iter()
                .map(|p| p.record.encoded_len() as u64)
                .sum::<u64>();
        let mut bytes = Vec::new();
        record.encode(seq, &mut bytes);
        self.pending.push_back(PendingWrite { op, seq, record });
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
        assert_eq!(pending.seq, self.log_seq + 1);
        self.log_len += pending.record.encoded_len() as u64;
        self.log_seq = pending.seq;
        let response = self.apply(&pending.record.entry);
        self.stats.applied += 1;

        let session = self
            .sessions
            .get_mut(&pending.record.client)
            .expect("pending write has a session");
        assert_eq!(
            session.last_request_id, pending.record.request_id,
            "session moved on while a write was pending"
        );
        assert!(session.last_response.is_none());
        session.last_response = Some(response.clone());
        out.push(Output::Reply(ResponseEnvelope {
            client: pending.record.client,
            request_id: pending.record.request_id,
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
            State::Halted => {
                assert!(self.pending.is_empty() && self.queued.is_empty());
                assert!(self.damage().is_some(), "halted without damage");
            }
        }
        let mut last_op = 0;
        let mut seq = self.log_seq;
        for p in &self.pending {
            assert!(p.op > last_op, "pending ops must be strictly increasing");
            last_op = p.op;
            seq += 1;
            assert_eq!(p.seq, seq, "pending records are numbered in order");
            let session = &self.sessions[&p.record.client];
            assert_eq!(session.last_request_id, p.record.request_id);
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
        assert_eq!(
            self.log_len % log::SECTOR_SIZE as u64,
            0,
            "the log is whole sectors"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::SECTOR_SIZE;
    use alloc::vec;

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

    /// The log a node has written so far, replayed from its appends.
    fn log_of(outputs: &[Output]) -> Vec<u8> {
        let mut log = Vec::new();
        for o in outputs {
            if let Output::Append { offset, bytes, .. } = o {
                let offset = *offset as usize;
                log.resize(offset.max(log.len()), 0);
                log.truncate(offset);
                log.extend_from_slice(bytes);
            }
        }
        log
    }

    #[test]
    fn put_is_applied_only_when_durable() {
        let (mut node, mut out) = started();
        node.on_input(req(1, 1, put("k", "v")), &mut out);
        let Output::Append { op, offset, bytes } = &out[0] else {
            panic!("expected append, got {out:?}")
        };
        assert_eq!(*offset, 0);
        assert_eq!(bytes.len(), SECTOR_SIZE);
        assert_eq!(node.get(b"k"), None, "not applied before durability");
        let op = *op;
        out.clear();
        node.on_input(Input::AppendComplete { op }, &mut out);
        assert_eq!(node.get(b"k"), Some(&b"v"[..]));
        assert_eq!((node.log_seq(), node.log_len()), (1, SECTOR_SIZE as u64));
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
    fn retry_after_a_restart_is_not_applied_twice() {
        // Put k, delete k, both durable. The reply to the delete is lost,
        // the node restarts, and the client retries the delete.
        let (mut node, mut out) = started();
        node.on_input(req(1, 1, put("k", "v")), &mut out);
        node.on_input(Input::AppendComplete { op: 1 }, &mut out);
        node.on_input(req(1, 2, Request::Delete { key: b"k".into() }), &mut out);
        node.on_input(Input::AppendComplete { op: 2 }, &mut out);
        let log = log_of(&out);

        let mut out = Vec::new();
        let mut node = Node::new(&mut out);
        node.on_input(Input::ReadComplete { op: 0, data: log }, &mut out);
        out.clear();
        node.on_input(req(1, 2, Request::Delete { key: b"k".into() }), &mut out);
        assert_eq!(
            out,
            vec![Output::Reply(ResponseEnvelope {
                client: 1,
                request_id: 2,
                response: Response::DeleteOk { existed: true }
            })],
            "the session came back from the log"
        );
        assert_eq!(node.stats().duplicates, 1);
        assert_eq!(node.log_seq(), 2, "no third record");
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
        Record {
            client: 9,
            request_id: 1,
            entry: Entry::Put {
                key: b"a".into(),
                value: b"1".into(),
            },
        }
        .encode(1, &mut log);
        node.on_input(Input::ReadComplete { op: 0, data: log }, &mut out);
        assert_eq!(out.len(), 2);
        assert!(
            matches!(&out[0], Output::Reply(r) if r.client == 1 && r.response == Response::Value(Some(b"1".to_vec())))
        );
        assert!(matches!(&out[1], Output::Reply(r) if r.client == 2 && r.response == Response::Value(None)));
        assert_eq!(node.stats().recovered_entries, 1);
        assert_eq!(node.recovery().unwrap().tail, Tail::Clean);
    }

    #[test]
    fn offsets_and_sequence_numbers_account_for_pending_appends() {
        let (mut node, mut out) = started();
        node.on_input(req(1, 1, put("k", "v")), &mut out);
        node.on_input(req(2, 1, put("k2", &"v".repeat(600))), &mut out);
        node.on_input(req(3, 1, put("k3", "v")), &mut out);
        let offsets: Vec<u64> = out
            .iter()
            .map(|o| match o {
                Output::Append { offset, .. } => *offset,
                _ => panic!(),
            })
            .collect();
        let s = SECTOR_SIZE as u64;
        assert_eq!(offsets, vec![0, s, 3 * s]);
        let log = log_of(&out);
        let r = log::recover(&log);
        assert_eq!(r.records.len(), 3, "numbered 1, 2, 3 before any completed");
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
    fn a_damaged_log_halts_the_node() {
        let (mut node, mut out) = started();
        node.on_input(req(1, 1, put("a", "1")), &mut out);
        node.on_input(Input::AppendComplete { op: 1 }, &mut out);
        node.on_input(req(1, 2, put("b", "2")), &mut out);
        node.on_input(Input::AppendComplete { op: 2 }, &mut out);
        let mut log = log_of(&out);
        log[3] ^= 1; // a bit of record 1's header

        let mut out = Vec::new();
        let mut node = Node::new(&mut out);
        node.on_input(req(2, 1, get("b")), &mut out);
        out.clear();
        node.on_input(Input::ReadComplete { op: 0, data: log }, &mut out);
        assert!(node.is_halted());
        assert!(!node.is_running());
        let damage = node.damage().expect("damage recorded");
        assert_eq!((damage.at, damage.seq, damage.next_valid_seq), (0, 1, 2));
        assert_eq!(out, vec![Output::Halt { damage }]);
        assert!(node.is_empty(), "nothing was applied");
        assert_eq!(node.stats().dropped, 1, "the queued request got no answer");
        out.clear();
        node.on_input(req(2, 2, get("b")), &mut out);
        assert!(out.is_empty(), "a halted node answers nothing");
        assert_eq!(node.stats().dropped, 2);
    }

    #[test]
    fn a_torn_tail_is_overwritten_by_the_next_append() {
        let (mut node, mut out) = started();
        node.on_input(req(1, 1, put("a", "1")), &mut out);
        node.on_input(Input::AppendComplete { op: 1 }, &mut out);
        let mut log = log_of(&out);
        log.extend_from_slice(&[0xab; 100]);

        let mut out = Vec::new();
        let mut node = Node::new(&mut out);
        node.on_input(Input::ReadComplete { op: 0, data: log }, &mut out);
        assert!(node.is_running());
        assert_eq!(node.recovery().unwrap().tail, Tail::Torn { at: SECTOR_SIZE });
        out.clear();
        node.on_input(req(1, 2, put("b", "2")), &mut out);
        assert!(matches!(&out[0], Output::Append { offset, .. } if *offset == SECTOR_SIZE as u64));
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
