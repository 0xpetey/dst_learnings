//! What the clients saw: a history of calls and returns.
//!
//! ```text
//!   client 1  ├── put k=1 ──┤                ├── get k ──┤
//!   client 2        ├──── get k ────┤
//!   client 3                   ├── delete k ──────────────── (gave up)
//!             ─────────────────────────────────────────────────► time
//! ```
//!
//! Every operation has a call instant. A completed operation also has a
//! return instant and a response. An operation the client gave up on has
//! neither: it may have taken effect at any time after its call, or never.
//! The checker must allow both, and this type keeps the information it
//! needs to do so.
//!
//! The history is per world, not per client, because the question it
//! answers is about the store. Two operations *overlap* if neither returned
//! before the other was called. Overlapping operations may take effect in
//! either order; non-overlapping ones may not.
//!
//! "Before" is decided by the order in which the events were recorded,
//! not by their instants. A simulator handles many events at one instant,
//! and a client that gets its reply and sends its next request in the
//! same event has two operations with equal timestamps that are not
//! concurrent. The instants are kept for people; the sequence decides.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use kv_core::{ClientId, Instant, RequestId};

/// One operation as a client saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Op<Req, Resp> {
    pub client: ClientId,
    pub request_id: RequestId,
    pub request: Req,
    /// When the client sent the first attempt.
    pub call: Instant,
    /// When the client received the reply, if it did.
    pub ret: Option<Instant>,
    /// What the reply said, if there was one.
    pub response: Option<Resp>,
}

impl<Req, Resp> Op<Req, Resp> {
    /// The client got an answer.
    pub fn is_complete(&self) -> bool {
        self.ret.is_some()
    }

    /// The client gave up. The operation may or may not have happened.
    pub fn is_pending(&self) -> bool {
        self.ret.is_none()
    }
}

/// Where an operation's call and return fall in the order of recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seq {
    call: u64,
    ret: Option<u64>,
}

/// The operations of a run, in call order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History<Req, Resp> {
    ops: Vec<Op<Req, Resp>>,
    /// One per op: the recording order of its call and its return.
    seqs: Vec<Seq>,
    next_seq: u64,
    /// Index of the open operation per client. A client has at most one.
    open: BTreeMap<ClientId, usize>,
}

impl<Req, Resp> Default for History<Req, Resp> {
    fn default() -> Self {
        History::new()
    }
}

impl<Req, Resp> History<Req, Resp> {
    pub fn new() -> History<Req, Resp> {
        History {
            ops: Vec::new(),
            seqs: Vec::new(),
            next_seq: 0,
            open: BTreeMap::new(),
        }
    }

    /// A history from operations already recorded. Sorted by call; an
    /// operation without a return is open. With no recording order to go
    /// by, events at one instant are ordered calls first, so that
    /// operations that touch at an instant count as overlapping.
    pub fn from_ops(mut ops: Vec<Op<Req, Resp>>) -> History<Req, Resp> {
        ops.sort_by_key(|op| op.call);
        // (instant, 0 for a call or 1 for a return, op index)
        let mut events: Vec<(Instant, u8, usize)> = Vec::new();
        for (i, op) in ops.iter().enumerate() {
            events.push((op.call, 0, i));
            if let Some(r) = op.ret {
                assert!(op.call <= r, "a return before its call");
                events.push((r, 1, i));
            }
        }
        events.sort();
        let mut seqs = alloc::vec![Seq { call: 0, ret: None }; ops.len()];
        for (seq, (_, kind, i)) in (0..).zip(&events) {
            match kind {
                0 => seqs[*i].call = seq,
                _ => seqs[*i].ret = Some(seq),
            }
        }
        let mut history = History::new();
        history.next_seq = events.len() as u64;
        for (op, seq) in ops.into_iter().zip(seqs) {
            if op.is_pending() {
                let prev = history.open.insert(op.client, history.ops.len());
                assert!(prev.is_none(), "client {} has two open operations", op.client);
            }
            history.ops.push(op);
            history.seqs.push(seq);
        }
        history
    }

    /// Client `client` sent `request` at `now`. Returns the operation's
    /// index. A client is sequential: it has at most one operation open.
    pub fn call(&mut self, client: ClientId, request_id: RequestId, request: Req, now: Instant) -> usize {
        if let Some(last) = self.ops.last() {
            assert!(last.call <= now, "calls must be recorded in time order");
        }
        let index = self.ops.len();
        let prev = self.open.insert(client, index);
        assert!(
            prev.is_none(),
            "client {client} called request {request_id} while another was open"
        );
        self.ops.push(Op {
            client,
            request_id,
            request,
            call: now,
            ret: None,
            response: None,
        });
        self.seqs.push(Seq {
            call: self.next_seq,
            ret: None,
        });
        self.next_seq += 1;
        index
    }

    /// Client `client` received `response` to `request_id` at `now`.
    pub fn ret(&mut self, client: ClientId, request_id: RequestId, response: Resp, now: Instant) {
        let index = self
            .open
            .remove(&client)
            .unwrap_or_else(|| panic!("client {client} returned with nothing open"));
        let op = &mut self.ops[index];
        assert_eq!(
            op.request_id, request_id,
            "client {client} returned the wrong request"
        );
        assert!(op.call <= now, "a return before its call");
        op.ret = Some(now);
        op.response = Some(response);
        self.seqs[index].ret = Some(self.next_seq);
        self.next_seq += 1;
    }

    /// Client `client` gave up on its open operation. The operation stays
    /// in the history as pending, and the client may call again.
    pub fn give_up(&mut self, client: ClientId, request_id: RequestId) {
        let index = self
            .open
            .remove(&client)
            .unwrap_or_else(|| panic!("client {client} gave up with nothing open"));
        assert_eq!(self.ops[index].request_id, request_id);
    }

    pub fn ops(&self) -> &[Op<Req, Resp>] {
        &self.ops
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn completed(&self) -> usize {
        self.ops.iter().filter(|op| op.is_complete()).count()
    }

    pub fn pending(&self) -> usize {
        self.ops.iter().filter(|op| op.is_pending()).count()
    }

    /// Operation `a` returned before operation `b` was called, so `a` took
    /// effect first in any legal order.
    pub fn precedes(&self, a: usize, b: usize) -> bool {
        self.seqs[a].ret.is_some_and(|r| r < self.seqs[b].call)
    }

    /// Neither precedes the other.
    pub fn overlaps(&self, a: usize, b: usize) -> bool {
        !self.precedes(a, b) && !self.precedes(b, a)
    }

    /// The earliest return among `ops`, in recording order, or `None` if
    /// none of them returned. An operation called after it is preceded
    /// by something in `ops`.
    pub(crate) fn min_ret_seq(&self, ops: impl Iterator<Item = usize>) -> Option<u64> {
        ops.filter_map(|i| self.seqs[i].ret).min()
    }

    /// Was `op` called before the return with sequence `ret_seq`?
    pub(crate) fn called_before(&self, op: usize, ret_seq: u64) -> bool {
        self.seqs[op].call < ret_seq
    }

    /// No two operations overlap. Each one returned before the next was
    /// called, except possibly the last, which may still be pending.
    pub fn is_sequential(&self) -> bool {
        self.first_overlap().is_none()
    }

    /// The first pair of operations that overlap, by index.
    pub fn first_overlap(&self) -> Option<(usize, usize)> {
        (1..self.ops.len())
            .find(|&i| !self.precedes(i - 1, i))
            .map(|i| (i - 1, i))
    }

    /// The largest number of operations open at once.
    pub fn max_concurrency(&self) -> usize {
        let mut best = 0;
        for (i, seq) in self.seqs.iter().enumerate() {
            let open = (0..=i)
                .filter(|&j| self.seqs[j].ret.is_none_or(|r| r > seq.call))
                .count();
            best = best.max(open);
        }
        best
    }
}

impl<Req: Clone, Resp: Clone> History<Req, Resp> {
    /// The operations for which `keep` is true, as a history of their own.
    /// The recording order is kept.
    pub fn filter(&self, mut keep: impl FnMut(&Op<Req, Resp>) -> bool) -> History<Req, Resp> {
        let mut history = History::new();
        history.next_seq = self.next_seq;
        for (op, seq) in self.ops.iter().zip(&self.seqs) {
            if keep(op) {
                if op.is_pending() {
                    history.open.insert(op.client, history.ops.len());
                }
                history.ops.push(op.clone());
                history.seqs.push(*seq);
            }
        }
        history
    }
}
