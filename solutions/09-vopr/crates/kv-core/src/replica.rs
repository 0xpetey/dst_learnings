//! The replicated store: primary-backup with an explicit failover rule.
//!
//! ```text
//!   inputs                                outputs
//!   Request ───────────┐          ┌─────── Reply
//!   Peer { from, msg } ┼──► ┌─────┴────┐ ─ Send { to, msg }
//!   ReadComplete ──────┼──► │ Replica  │ ─ Read / Append
//!   AppendComplete ────┼──► │  (Node)  │ ─ Halt
//!   Tick ──────────────┘    └──────────┘ ─ PeerUp / PeerDown / Status
//! ```
//!
//! A replica is a [`Node`] with company. The node of Modules 2 to 8 made a
//! write durable on one disk and then applied it. The replica makes it
//! durable on *every* disk first. That is the whole protocol, and it is
//! the simplest replication there is:
//!
//! * One node per view is the *primary*. The primary of view `v` is node
//!   `v mod n`. Only the primary runs a [`Node`], and only the primary
//!   answers clients. A backup forwards what it receives.
//! * The primary appends a write to its own log and sends it to every
//!   backup as a `Prepare`. A backup appends it at the same offset and
//!   answers `PrepareOk` once it is durable. The write is *committed*,
//!   applied and answered when every replica has it on disk.
//! * Every beat carries the sender's view and log length. A backup whose
//!   failure detector marks the primary `Down` moves to the next view
//!   whose primary it believes alive. Views only go up, and a replica that
//!   hears a higher view adopts it at once. A primary that hears one
//!   steps down.
//! * The new primary serves nothing until it has *gathered*: heard every
//!   replica report in the new view, pulled the longest log among them,
//!   and pushed that log to every replica's disk. Only then does it build
//!   its node and answer.
//!
//! Two facts follow, and the acceptance tests check both. Every committed
//! write is on every disk, so no replica ever has to throw a record away:
//! the logs are prefixes of one another at all times. And because the new
//! primary waits for every replica, there is never more than one primary
//! that can commit. The price is liveness. A cluster with one replica down
//! or unreachable commits nothing until it comes back. Module 10 pays for
//! liveness with a majority and a real view change.
//!
//! Damage: a replica whose log has a hole in it keeps the records it finds
//! beyond the hole, asks its peers for the missing one, and writes the
//! log out whole again. Only when no peer has the record, which means it
//! was never anywhere else, does it erase the hole and what follows. The
//! first version erased at once and reported the prefix. Seed 36 showed
//! why not: every disk had a hole from an old lost write, three restarts
//! in a row each threw away what came after their hole, and the cluster
//! forgot fourteen committed writes. A cluster of one has nobody to ask
//! and halts, as Module 6 decided.
//!
//! Stragglers: the network reorders, so a beat can arrive after a newer
//! one. Seed 4668 found a restarted primary that took an old beat's log
//! length for the truth, gathered too early and prepared a second record
//! 25. Every beat now carries the sender's boot id, handed in by the
//! environment, and its heartbeat number, and a replica applies a beat
//! only if it is newer than the last one it applied from that peer.
//!
//! Two records, one number: a primary can lose a write before anyone saw
//! it, and the next primary can number a new write the same. Seed 402
//! found a stray copy of the first one, salvaged from beyond a hole, put
//! back in place over the second. So every record carries the view it was
//! prepared in, a log is summed up as runs of views, beats carry the
//! runs, and two logs are compared by their first disagreement: the
//! record from the higher view is the one the cluster kept. A gathering
//! primary adopts the newest log, not the longest, and a backup whose log
//! disagrees with the primary's is told to truncate from the first
//! disagreement. That is Raft's log matching, without Raft's elections.
//!
//! Fault points: `Site::DropPrepare` makes the primary skip one `Prepare`.
//! The next beat from the backup says how far behind it is, and the
//! primary pushes again. Planted bug: `Bug::StaleReadAfterStepDown` makes
//! a primary that stepped down keep answering reads from its old map.

use crate::buggify::{Buggify, Site};
use crate::bugs::{Bug, Bugs};
use crate::heartbeat::{self, Heartbeat, Liveness, NodeId};
use crate::log::{self, Damage, Record, Tail};
use crate::node::{self, Node, OpId, Recovery};
use crate::protocol::{Request, RequestEnvelope, ResponseEnvelope};
use crate::time::Instant;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

/// A view number. The primary of view `v` is node `v mod n`.
pub type View = u64;

/// Records a replica sends in one batch when pushing or answering a pull.
pub const BATCH: u64 = 16;
/// Requests a replica holds for a primary that is not ready yet.
pub const QUEUE_MAX: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// "I am alive, in this view, with this log." Sent every interval to
    /// every peer. `boot` names the sender's incarnation and `seq` is the
    /// heartbeat's own counter, so a receiver can tell a straggler.
    Beat {
        boot: u64,
        seq: u64,
        view: View,
        /// The log, durable or not, as runs of `(view, last seq)`: which
        /// view prepared each record.
        runs: Vec<(u32, u64)>,
        /// The highest record held, in the log or kept from beyond a hole.
        top: u64,
        /// Records known durable.
        durable: u64,
    },
    /// "Your log disagrees with mine from record `at`. Erase from there."
    Truncate { view: View, at: u64 },
    /// Record `seq`, which belongs at `offset`. From the primary to a
    /// backup, from the holder of the longest log to a primary that
    /// pulled it, or in answer to a `Fetch`. `None` for a record salvaged
    /// from beyond a hole, whose place the sender does not know.
    Prepare {
        view: View,
        seq: u64,
        offset: Option<u64>,
        bytes: Vec<u8>,
    },
    /// "My log is durable through `durable`."
    PrepareOk { boot: u64, view: View, durable: u64 },
    /// "Send me your records from `from` on."
    Pull { view: View, from: u64 },
    /// A client request that arrived at a backup.
    Forward { view: View, envelope: RequestEnvelope },
    /// "My log has a hole at `seq`. Do you have the record?"
    Fetch { view: View, seq: u64 },
    /// "No."
    Missing { view: View, seq: u64 },
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Message::Beat {
                boot,
                seq,
                view,
                runs,
                top,
                durable,
            } => {
                write!(f, "beat boot={boot} seq={seq} view={view} runs=[")?;
                for (i, (v, until)) in runs.iter().enumerate() {
                    write!(f, "{}{v}..{until}", if i > 0 { " " } else { "" })?;
                }
                write!(f, "] top={top} durable={durable}")
            }
            Message::Truncate { view, at } => write!(f, "truncate view={view} at={at}"),
            Message::Prepare {
                view,
                seq,
                offset,
                bytes,
            } => {
                write!(f, "prepare view={view} seq={seq} offset=")?;
                match offset {
                    Some(o) => write!(f, "{o}")?,
                    None => write!(f, "?")?,
                }
                write!(f, " bytes={}", bytes.len())
            }
            Message::PrepareOk { boot, view, durable } => {
                write!(f, "prepare_ok boot={boot} view={view} durable={durable}")
            }
            Message::Pull { view, from } => write!(f, "pull view={view} from={from}"),
            Message::Forward { view, envelope } => write!(
                f,
                "forward view={view} client={} id={}",
                envelope.client, envelope.request_id
            ),
            Message::Fetch { view, seq } => write!(f, "fetch view={view} seq={seq}"),
            Message::Missing { view, seq } => write!(f, "missing view={view} seq={seq}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// A client request arrived.
    Request(RequestEnvelope),
    /// A message from another replica arrived.
    Peer { from: NodeId, message: Message },
    /// The read issued by `Output::Read { op }` finished with these bytes.
    ReadComplete { op: OpId, data: Vec<u8> },
    /// The bytes from `Output::Append { op, .. }` are durable.
    AppendComplete { op: OpId },
    /// The deadline from `next_deadline` arrived.
    Tick,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Read the whole log. Exactly one of these is ever issued, at start.
    Read {
        op: OpId,
    },
    /// Append bytes to the log at `offset` and make them durable.
    Append {
        op: OpId,
        offset: u64,
        bytes: Vec<u8>,
    },
    /// Send a reply to a client.
    Reply(ResponseEnvelope),
    /// Send a message to another replica.
    Send {
        to: NodeId,
        message: Message,
    },
    /// The log is damaged and there is no peer to repair it from. Stop
    /// driving this replica; it will not serve.
    Halt {
        damage: Damage,
    },
    PeerUp(NodeId),
    PeerDown(NodeId),
    /// The view or the status changed.
    Status {
        view: View,
        status: Status,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Waiting for the log to be read.
    Reading,
    /// Restarted with the log whole, waiting to hear which view the
    /// cluster is in before taking up a role.
    Joining,
    /// The log had a hole. Fetching the missing record from a peer and
    /// writing the log out whole again.
    Repairing,
    /// No peer had the missing record: erasing the hole and what follows
    /// it before joining.
    Erasing,
    /// Forwarding requests to the primary and appending what it sends.
    Backup,
    /// The primary of the view, not yet ready: gathering every replica's
    /// report, pulling the longest log, pushing it to every disk.
    Gathering,
    /// The primary of the view, serving.
    Primary,
    /// Damaged, with nobody to repair from.
    Halted,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Status::Reading => "reading",
            Status::Joining => "joining",
            Status::Repairing => "repairing",
            Status::Erasing => "erasing",
            Status::Backup => "backup",
            Status::Gathering => "gathering",
            Status::Primary => "primary",
            Status::Halted => "halted",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Client requests handled by this replica's node.
    pub requests: u64,
    /// Client requests forwarded to the primary.
    pub forwarded: u64,
    /// Client requests dropped: no primary to forward to, or a forward
    /// that arrived at a replica that is not serving.
    pub dropped: u64,
    pub prepares_sent: u64,
    pub prepares_accepted: u64,
    /// Prepares with a stale view, a sequence number already held, or a
    /// gap before them.
    pub prepares_ignored: u64,
    pub pulls_sent: u64,
    pub pulls_served: u64,
    /// Writes committed and applied by this replica as primary.
    pub commits: u64,
    pub view_changes: u64,
    /// Times this replica became the ready primary.
    pub promotions: u64,
    /// Times a serving primary heard a higher view.
    pub step_downs: u64,
    /// Logs found damaged at start.
    pub repairs: u64,
    /// Records kept from beyond a hole.
    pub salvaged: u64,
    /// Records fetched from a peer to fill a hole.
    pub fetched: u64,
    /// Holes no peer could fill, erased with what followed them.
    pub erasures: u64,
    /// Beats and acknowledgements older than one already applied.
    pub stragglers: u64,
    /// Tails erased because they disagreed with a newer log.
    pub truncations: u64,
}

/// What the newest beat from a peer said. Newest by `(boot, seq)`, not by
/// arrival: the network reorders.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PeerState {
    boot: u64,
    seq: u64,
    view: View,
    runs: Vec<(u32, u64)>,
    top: u64,
    durable: u64,
}

impl PeerState {
    /// Records in the peer's log.
    fn len(&self) -> u64 {
        runs_len(&self.runs)
    }

    /// The peer holds records beyond its log: it is repairing.
    fn is_repairing(&self) -> bool {
        self.top > self.len()
    }
}

/// Records in a log summed up as runs.
fn runs_len(runs: &[(u32, u64)]) -> u64 {
    runs.last().map_or(0, |&(_, until)| until)
}

/// The view of record `seq` in a log summed up as runs, if it has one.
fn view_at(runs: &[(u32, u64)], seq: u64) -> Option<u32> {
    runs.iter().find(|&&(_, until)| seq <= until).map(|&(v, _)| v)
}

/// The first record on which two logs disagree: one has it and the other
/// does not, or they were prepared in different views. One past the end
/// of both when they are the same log.
fn divergence(a: &[(u32, u64)], b: &[(u32, u64)]) -> u64 {
    let n = runs_len(a).max(runs_len(b));
    let mut seq = 1;
    while seq <= n {
        if view_at(a, seq) != view_at(b, seq) {
            return seq;
        }
        // Agreement holds to the end of the shorter of the two runs.
        let end_a = a.iter().find(|&&(_, u)| seq <= u).map_or(seq, |&(_, u)| u);
        let end_b = b.iter().find(|&&(_, u)| seq <= u).map_or(seq, |&(_, u)| u);
        seq = end_a.min(end_b) + 1;
    }
    n + 1
}

/// Which of two logs the cluster kept: at their first disagreement, the
/// record from the higher view wins, and a record beats no record. Equal
/// means the same log.
fn newer(a: &[(u32, u64)], b: &[(u32, u64)]) -> core::cmp::Ordering {
    let d = divergence(a, b);
    view_at(a, d).cmp(&view_at(b, d))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpKind {
    Read,
    /// The append of record `seq`, in the `generation` of the log it was
    /// part of. A truncation starts a new generation, and completions
    /// from an old one count for nothing.
    Append {
        seq: u64,
        generation: u64,
    },
    /// Zeros over a tail: after a hole, or after a truncation.
    Erase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingOp {
    op: OpId,
    kind: OpKind,
}

#[derive(Debug)]
pub struct Replica {
    me: NodeId,
    /// This incarnation, from the environment. A restart gets a new one.
    boot: u64,
    peers: Vec<NodeId>,
    status: Status,
    view: View,
    heartbeat: Heartbeat,
    /// The log as this replica knows it: every record durable or in
    /// flight, in order. A restart rebuilds it from the disk.
    log: Vec<u8>,
    /// Offset of record `i + 1`. Its length is the number of records.
    offsets: Vec<u64>,
    /// The view record `i + 1` was prepared in.
    views: Vec<u32>,
    /// Records known durable.
    durable: u64,
    /// Bumped by every truncation. An append from an older generation
    /// that completes afterwards is not durability of anything.
    generation: u64,
    /// Bytes ever written on this disk, at most. An erasure zeros up to
    /// here.
    disk_high: u64,
    /// This incarnation has taken up a role at least once. The first time
    /// a restarted node finds itself the primary, it moves to a view of
    /// its own, since it may have prepared records in this one before.
    joined: bool,
    /// What the disk read found at start.
    recovery: Option<Recovery>,
    /// Records found beyond a hole, by sequence number, until the log is
    /// whole again.
    retained: BTreeMap<u64, Vec<u8>>,
    /// Peers that said they do not have the record at `len + 1`.
    missing: BTreeSet<NodeId>,
    /// Storage operations in flight, oldest first.
    pending: VecDeque<PendingOp>,
    next_op: OpId,
    /// The store. Only a serving primary has one.
    node: Option<Node>,
    /// Writes the node submitted, awaiting every replica's `PrepareOk`:
    /// `(seq, node op)` in order.
    commits: VecDeque<(u64, OpId)>,
    peer_state: BTreeMap<NodeId, PeerState>,
    /// Requests held for a primary that is not ready yet.
    queued: VecDeque<RequestEnvelope>,
    /// The `(peer, from)` of the last pull sent, so that one tick sends
    /// one pull.
    last_pull: Option<(NodeId, u64)>,
    stats: Stats,
    buggify: Buggify,
    /// The node's fault points, carried from one node to the next.
    node_buggify: Buggify,
    bugs: Bugs,
    /// Planted bug: the map a stepped-down primary keeps answering from.
    stale_node: Option<Node>,
    last_now: Instant,
}

impl Replica {
    /// Create a replica with no fault point enabled. The production
    /// constructor. It asks for its log and sends its first beats.
    pub fn new(
        me: NodeId,
        boot: u64,
        peers: &[NodeId],
        config: heartbeat::Config,
        now: Instant,
        out: &mut Vec<Output>,
    ) -> Replica {
        Replica::with_buggify(
            Buggify::off(),
            Buggify::off(),
            Buggify::off(),
            me,
            boot,
            peers,
            config,
            now,
            out,
        )
    }

    /// Create a replica with fault points for itself, its node and its
    /// heartbeat. The simulator's constructor. `boot` must differ from
    /// every earlier incarnation of this node that may still have
    /// messages in flight, and be larger than each of them.
    #[allow(clippy::too_many_arguments)]
    pub fn with_buggify(
        buggify: Buggify,
        node_buggify: Buggify,
        heartbeat_buggify: Buggify,
        me: NodeId,
        boot: u64,
        peers: &[NodeId],
        config: heartbeat::Config,
        now: Instant,
        out: &mut Vec<Output>,
    ) -> Replica {
        let n = peers.len() + 1;
        let mut ids: Vec<NodeId> = peers.to_vec();
        ids.push(me);
        ids.sort_unstable();
        assert!(
            ids.iter().copied().eq(0..n as NodeId),
            "replica ids must be 0..n, got {ids:?}"
        );
        let mut hb_out = Vec::new();
        let heartbeat = Heartbeat::with_buggify(heartbeat_buggify, me, peers, config, now, &mut hb_out);
        let mut replica = Replica {
            me,
            boot,
            peers: peers.to_vec(),
            status: Status::Reading,
            view: 0,
            heartbeat,
            log: Vec::new(),
            offsets: Vec::new(),
            views: Vec::new(),
            durable: 0,
            generation: 0,
            disk_high: 0,
            joined: false,
            recovery: None,
            retained: BTreeMap::new(),
            missing: BTreeSet::new(),
            pending: VecDeque::new(),
            next_op: 0,
            node: None,
            commits: VecDeque::new(),
            peer_state: peers.iter().map(|&p| (p, PeerState::default())).collect(),
            queued: VecDeque::new(),
            last_pull: None,
            stats: Stats::default(),
            buggify,
            node_buggify,
            bugs: Bugs::none(),
            stale_node: None,
            last_now: now,
        };
        let op = replica.submit(OpKind::Read);
        out.push(Output::Read { op });
        replica.route_heartbeat(hb_out, out);
        replica.check_invariants();
        replica
    }

    // ---- views ----------------------------------------------------------------

    pub fn id(&self) -> NodeId {
        self.me
    }

    pub fn boot(&self) -> u64 {
        self.boot
    }

    /// Every replica in the cluster, this one included.
    pub fn cluster_size(&self) -> usize {
        self.peers.len() + 1
    }

    pub fn view(&self) -> View {
        self.view
    }

    pub fn status(&self) -> Status {
        self.status
    }

    /// The primary of `view`.
    pub fn primary_of(&self, view: View) -> NodeId {
        (view % self.cluster_size() as u64) as NodeId
    }

    /// The primary of the current view.
    pub fn primary(&self) -> NodeId {
        self.primary_of(self.view)
    }

    pub fn is_primary(&self) -> bool {
        self.status == Status::Primary
    }

    pub fn is_halted(&self) -> bool {
        self.status == Status::Halted
    }

    pub fn heartbeat(&self) -> &Heartbeat {
        &self.heartbeat
    }

    /// The store, while this replica is the serving primary.
    pub fn node(&self) -> Option<&Node> {
        self.node.as_ref()
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    // ---- the log -------------------------------------------------------------------

    /// Records in the log, durable or in flight.
    pub fn len(&self) -> u64 {
        self.offsets.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Records known durable.
    pub fn durable(&self) -> u64 {
        self.durable
    }

    /// The highest record this replica holds: the log, or a record kept
    /// from beyond a hole. What the beat reports while repairing, so that
    /// a gathering primary knows to wait for it.
    pub fn top(&self) -> u64 {
        self.retained
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .max(self.len())
    }

    /// Bytes of log, durable or in flight.
    pub fn log_len(&self) -> u64 {
        self.log.len() as u64
    }

    /// The log as runs of `(view, last seq)`: which view prepared each
    /// record. What beats carry, and what two logs are compared by.
    pub fn runs(&self) -> Vec<(u32, u64)> {
        let mut runs: Vec<(u32, u64)> = Vec::new();
        for (i, &view) in self.views.iter().enumerate() {
            let seq = i as u64 + 1;
            match runs.last_mut() {
                Some((v, until)) if *v == view => *until = seq,
                _ => runs.push((view, seq)),
            }
        }
        runs
    }

    /// The view record `seq` was prepared in.
    pub fn view_of(&self, seq: u64) -> u32 {
        self.views[(seq - 1) as usize]
    }

    /// The view as the record header carries it.
    fn view_u32(&self) -> u32 {
        u32::try_from(self.view).expect("view fits in a record header")
    }

    /// The log bytes, durable or in flight. The checker compares replicas.
    pub fn log_bytes(&self) -> &[u8] {
        &self.log
    }

    /// The records in the log, decoded.
    pub fn records(&self) -> Vec<Record> {
        log::recover(&self.log).records
    }

    /// What the disk read found at start, once it finished.
    pub fn recovery(&self) -> Option<Recovery> {
        self.recovery
    }

    /// Why the replica halted, if it did.
    pub fn damage(&self) -> Option<Damage> {
        match self.recovery?.tail {
            Tail::Damaged(d) if self.is_halted() => Some(d),
            _ => None,
        }
    }

    /// Storage operations in flight.
    pub fn pending_ops(&self) -> usize {
        self.pending.len()
    }

    /// Writes submitted by the node and not yet committed.
    pub fn uncommitted(&self) -> usize {
        self.commits.len()
    }

    /// When this replica next needs a `Tick`.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.heartbeat.next_deadline()
    }

    // ---- fault points and bugs -----------------------------------------------------

    pub fn buggify(&self) -> &Buggify {
        &self.buggify
    }

    pub fn buggify_mut(&mut self) -> &mut Buggify {
        &mut self.buggify
    }

    /// Disable every fault point in the replica, its node and its
    /// heartbeat. The quiet period of a run.
    pub fn disable_all_buggify(&mut self) {
        self.buggify.disable_all();
        self.node_buggify.disable_all();
        self.heartbeat.buggify_mut().disable_all();
        if let Some(node) = self.node.as_mut() {
            node.buggify_mut().disable_all();
        }
    }

    /// Fault points that fired since the last call, in the replica, its
    /// node and its heartbeat.
    pub fn take_buggify_log(&mut self) -> Vec<Site> {
        let mut fired = self.buggify.take_log();
        if let Some(node) = self.node.as_mut() {
            fired.extend(node.buggify_mut().take_log());
        }
        fired.extend(self.heartbeat.buggify_mut().take_log());
        fired
    }

    pub fn bugs(&self) -> &Bugs {
        &self.bugs
    }

    pub fn bugs_mut(&mut self) -> &mut Bugs {
        &mut self.bugs
    }

    // ---- inputs ----------------------------------------------------------------------

    /// Advance the state machine by one input, appending any effects to `out`.
    pub fn on_input(&mut self, now: Instant, input: Input, out: &mut Vec<Output>) {
        assert!(
            now >= self.last_now,
            "node {} saw time go backwards: {now} after {}",
            self.me,
            self.last_now
        );
        self.last_now = now;
        match input {
            Input::Tick => self.on_tick(now, out),
            Input::ReadComplete { op, data } => self.on_read_complete(op, &data, out),
            Input::AppendComplete { op } => self.on_append_complete(op, out),
            Input::Request(envelope) => self.on_request(envelope, out),
            Input::Peer { from, message } => self.on_peer(now, from, message, out),
        }
        self.check_invariants();
    }

    fn on_tick(&mut self, now: Instant, out: &mut Vec<Output>) {
        let mut hb_out = Vec::new();
        self.heartbeat.on_input(now, heartbeat::Input::Tick, &mut hb_out);
        self.route_heartbeat(hb_out, out);
        // One pull per tick while behind, however many beats arrive.
        self.last_pull = None;
        match self.status {
            Status::Gathering => self.progress(out),
            // Ask again for the record at the hole: the answer may have
            // been lost, or a peer may have it now.
            Status::Repairing => self.repair(out),
            _ => {}
        }
    }

    fn on_read_complete(&mut self, op: OpId, data: &[u8], out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Reading, "ReadComplete while not reading");
        let pending = self.pending.pop_front().expect("a read was pending");
        assert_eq!(
            pending,
            PendingOp {
                op,
                kind: OpKind::Read
            }
        );
        let recovered = log::recover(data);
        self.recovery = Some(Recovery {
            records: recovered.records.len() as u64,
            valid_len: recovered.valid_len as u64,
            tail: recovered.tail,
        });
        self.log = data[..recovered.valid_len].to_vec();
        self.offsets.clear();
        let mut offset = 0;
        for record in &recovered.records {
            self.offsets.push(offset);
            offset += record.encoded_len() as u64;
        }
        assert_eq!(offset as usize, recovered.valid_len);
        self.views = recovered.views.clone();
        self.durable = self.len();
        self.disk_high = data.len() as u64;
        if let Tail::Damaged(damage) = recovered.tail {
            if self.peers.is_empty() {
                // Serving on top of a hole would answer reads with state
                // that is missing an acknowledged write, and there is
                // nobody to ask for the record. Refuse.
                self.status = Status::Halted;
                self.queued.clear();
                out.push(Output::Halt { damage });
                out.push(Output::Status {
                    view: self.view,
                    status: self.status,
                });
                return;
            }
            // Keep every record found beyond the hole: they are real, and
            // this replica may hold the only copy still in anyone's
            // memory. Then ask the peers for the one that is missing.
            self.stats.repairs += 1;
            for (seq, view, bytes) in log::salvage(data, recovered.valid_len) {
                if seq <= self.len() {
                    continue;
                }
                // Two copies of one number: the one from the higher view
                // is the one the cluster kept, or the nearest to it.
                let newer = self
                    .retained
                    .get(&seq)
                    .is_none_or(|old| log::record_header(old).is_some_and(|(_, v)| v < view));
                if newer {
                    self.retained.insert(seq, bytes);
                    self.stats.salvaged += 1;
                }
            }
            self.status = Status::Repairing;
            out.push(Output::Status {
                view: self.view,
                status: self.status,
            });
            self.repair(out);
            return;
        }
        self.enter_view(out);
    }

    /// Make the log whole: write out every retained record in order, and
    /// at a hole ask every peer for the record. Called whenever something
    /// may have filled the hole.
    fn repair(&mut self, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Repairing);
        loop {
            let next = self.len() + 1;
            // Strays at or below the log are copies of what it has.
            while self.retained.keys().next().is_some_and(|&s| s < next) {
                self.retained.pop_first();
            }
            match self.retained.remove(&next) {
                // Back at its place, written again so the disk has it
                // where recovery looks.
                Some(bytes) => self.append(next, &bytes, out),
                None if self.retained.is_empty() => {
                    self.missing.clear();
                    self.enter_view(out);
                    return;
                }
                None => {
                    // A hole. Every peer is asked; the first answer fills
                    // it, and if every peer says no, nobody has it.
                    for peer in self.peers.clone() {
                        out.push(Output::Send {
                            to: peer,
                            message: Message::Fetch {
                                view: self.view,
                                seq: next,
                            },
                        });
                    }
                    return;
                }
            }
        }
    }

    /// No peer has the record at the hole. What this replica kept beyond
    /// it was never anywhere else, so it was never acknowledged, and the
    /// disk is erased from the hole on so that none of it can ever read
    /// back as current.
    fn erase(&mut self, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Repairing);
        self.stats.erasures += 1;
        self.retained.clear();
        self.missing.clear();
        self.status = Status::Erasing;
        self.submit_erase(out);
        out.push(Output::Status {
            view: self.view,
            status: self.status,
        });
    }

    /// Zeros from the end of the log to the end of everything ever
    /// written, so that nothing beyond the log can read back as a record.
    fn submit_erase(&mut self, out: &mut Vec<Output>) {
        let op = self.submit(OpKind::Erase);
        let from = self.log_len();
        out.push(Output::Append {
            op,
            offset: from,
            bytes: vec![0; (self.disk_high.max(from) - from) as usize],
        });
    }

    /// The log disagrees with a newer one from record `at` on: drop it
    /// from there, in memory now and on disk by an erasure that precedes
    /// every later append.
    fn truncate(&mut self, at: u64, out: &mut Vec<Output>) {
        assert!(at >= 1 && at <= self.len(), "truncate at {at} of {}", self.len());
        self.stats.truncations += 1;
        let keep = (at - 1) as usize;
        self.log.truncate(self.offsets[keep] as usize);
        self.offsets.truncate(keep);
        self.views.truncate(keep);
        self.durable = self.durable.min(at - 1);
        self.generation += 1;
        self.submit_erase(out);
    }

    fn on_append_complete(&mut self, op: OpId, out: &mut Vec<Output>) {
        let pending = self
            .pending
            .pop_front()
            .expect("AppendComplete with nothing pending");
        // Storage is required to complete operations in submission order.
        assert_eq!(pending.op, op, "appends completed out of order");
        match pending.kind {
            OpKind::Read => panic!("AppendComplete for a read"),
            OpKind::Erase => {
                if self.status == Status::Erasing {
                    self.enter_view(out);
                }
            }
            OpKind::Append { generation, .. } if generation != self.generation => {
                // Written for a log that was truncated since. The bytes
                // are on the disk, under zeros by now or soon.
            }
            OpKind::Append { seq, .. } => {
                assert_eq!(seq, self.durable + 1, "records become durable in order");
                self.durable = seq;
                match self.status {
                    Status::Backup => {
                        let to = self.primary();
                        out.push(Output::Send {
                            to,
                            message: Message::PrepareOk {
                                boot: self.boot,
                                view: self.view,
                                durable: self.durable,
                            },
                        });
                    }
                    Status::Gathering => self.progress(out),
                    Status::Primary => self.try_commit(out),
                    // A record in flight when the view changed, or one
                    // written out again by a repair. Its durability is
                    // reported with the next beat.
                    Status::Reading
                    | Status::Joining
                    | Status::Repairing
                    | Status::Erasing
                    | Status::Halted => {}
                }
            }
        }
    }

    fn on_request(&mut self, envelope: RequestEnvelope, out: &mut Vec<Output>) {
        match self.status {
            Status::Primary => self.serve(envelope, out),
            Status::Reading | Status::Joining | Status::Repairing | Status::Erasing | Status::Gathering => {
                self.queue(envelope)
            }
            Status::Backup => {
                // Planted bug: a stepped-down primary still answers reads
                // from the map it had. Writes go to the real primary, so
                // the map falls behind and the reads are stale.
                if let Some(node) = self.stale_node.as_mut() {
                    if matches!(envelope.request, Request::Get { .. }) {
                        let mut node_out = Vec::new();
                        node.on_input(node::Input::Request(envelope), &mut node_out);
                        for o in node_out {
                            if let node::Output::Reply(reply) = o {
                                out.push(Output::Reply(reply));
                            }
                        }
                        return;
                    }
                }
                self.forward(envelope, out);
            }
            Status::Halted => self.stats.dropped += 1,
        }
    }

    fn on_peer(&mut self, now: Instant, from: NodeId, message: Message, out: &mut Vec<Output>) {
        assert!(from != self.me, "node {from} received its own message");
        assert!(
            self.peer_state.contains_key(&from),
            "node {} heard from unknown node {from}",
            self.me
        );
        match message {
            Message::Beat {
                boot,
                seq,
                view,
                runs,
                top,
                durable,
            } => {
                let mut hb_out = Vec::new();
                self.heartbeat.on_input(
                    now,
                    heartbeat::Input::Message {
                        from,
                        message: heartbeat::Message::Beat { seq },
                    },
                    &mut hb_out,
                );
                self.route_heartbeat(hb_out, out);
                // Any beat is evidence of life, which the heartbeat took.
                // Only the newest beat is evidence about the log: a
                // straggler carries a length the peer has moved past, or
                // one from before a restart shrank it.
                let peer = self.peer_state.get_mut(&from).expect("known peer");
                if (boot, seq) <= (peer.boot, peer.seq) {
                    self.stats.stragglers += 1;
                    return;
                }
                *peer = PeerState {
                    boot,
                    seq,
                    view,
                    runs,
                    top,
                    durable,
                };
                if view > self.view {
                    self.adopt_view(view, out);
                }
                match self.status {
                    Status::Joining => self.enter_view(out),
                    Status::Gathering => {
                        self.sync_peer(from, out);
                        self.progress(out);
                    }
                    Status::Primary => {
                        self.sync_peer(from, out);
                        self.try_commit(out);
                    }
                    _ => {}
                }
            }
            Message::Truncate { view, at } => {
                if view < self.view {
                    return;
                }
                if view > self.view {
                    self.adopt_view(view, out);
                }
                if self.status != Status::Backup || from != self.primary() || at > self.len() {
                    return;
                }
                self.truncate(at, out);
            }
            Message::Prepare {
                view,
                seq,
                offset,
                bytes,
            } => {
                if view < self.view {
                    self.stats.prepares_ignored += 1;
                    return;
                }
                if view > self.view {
                    self.adopt_view(view, out);
                }
                let accept = match self.status {
                    Status::Backup => from == self.primary(),
                    Status::Gathering => self.last_pull.is_some_and(|(peer, _)| peer == from),
                    // The answer to a fetch, from whoever had it.
                    Status::Repairing => true,
                    _ => false,
                };
                if !accept || seq != self.len() + 1 {
                    self.stats.prepares_ignored += 1;
                    return;
                }
                // The invariant of the whole design: a record arrives at
                // the offset this replica has free. Anything else means
                // two logs diverged.
                if let Some(offset) = offset {
                    assert_eq!(
                        offset,
                        self.log_len(),
                        "node {} view {view}: record {seq} offered at offset {offset}, log ends at {}",
                        self.me,
                        self.log_len()
                    );
                }
                assert_eq!(
                    log::record_header(&bytes).map(|(s, _)| s),
                    Some(seq),
                    "node {} was offered bytes that are not record {seq}",
                    self.me
                );
                self.append(seq, &bytes, out);
                self.stats.prepares_accepted += 1;
                match self.status {
                    Status::Gathering => self.progress(out),
                    Status::Repairing => {
                        self.stats.fetched += 1;
                        self.missing.clear();
                        self.repair(out);
                    }
                    _ => {}
                }
            }
            Message::PrepareOk { boot, view, durable } => {
                if view != self.view {
                    return;
                }
                let peer = self.peer_state.get_mut(&from).expect("known peer");
                // From an incarnation this replica has since heard a beat
                // from a newer one of: its log is gone.
                if boot < peer.boot {
                    self.stats.stragglers += 1;
                    return;
                }
                if boot > peer.boot {
                    // A newer incarnation, before its first beat arrived.
                    // Its log is unknown until then.
                    *peer = PeerState {
                        boot,
                        seq: 0,
                        view,
                        runs: Vec::new(),
                        top: 0,
                        durable,
                    };
                }
                peer.durable = peer.durable.max(durable);
                match self.status {
                    Status::Gathering => self.progress(out),
                    Status::Primary => self.try_commit(out),
                    _ => {}
                }
            }
            Message::Pull { view, from: start } => {
                if view < self.view {
                    return;
                }
                if view > self.view {
                    self.adopt_view(view, out);
                }
                if matches!(
                    self.status,
                    Status::Reading | Status::Joining | Status::Erasing | Status::Halted
                ) {
                    return;
                }
                self.stats.pulls_served += 1;
                self.send_records(from, start, out);
            }
            Message::Fetch { view, seq } => {
                if view > self.view {
                    self.adopt_view(view, out);
                }
                if matches!(self.status, Status::Reading | Status::Halted) {
                    return;
                }
                if seq >= 1 && seq <= self.len() {
                    self.send_prepare(from, seq, out);
                } else if let Some(bytes) = self.retained.get(&seq) {
                    // Kept from beyond this replica's own hole. Real, but
                    // its place is not known here.
                    out.push(Output::Send {
                        to: from,
                        message: Message::Prepare {
                            view: self.view,
                            seq,
                            offset: None,
                            bytes: bytes.clone(),
                        },
                    });
                } else {
                    out.push(Output::Send {
                        to: from,
                        message: Message::Missing { view: self.view, seq },
                    });
                }
            }
            Message::Missing { view, seq } => {
                if view > self.view {
                    self.adopt_view(view, out);
                }
                if self.status != Status::Repairing || seq != self.len() + 1 {
                    return;
                }
                self.missing.insert(from);
                if self.peers.iter().all(|p| self.missing.contains(p)) {
                    self.erase(out);
                }
            }
            Message::Forward { view, envelope } => {
                if view > self.view {
                    self.adopt_view(view, out);
                }
                match self.status {
                    Status::Primary => self.serve(envelope, out),
                    Status::Gathering => self.queue(envelope),
                    // Not the primary any more, or not yet. The client
                    // retries, and the backup that forwarded it will know
                    // better by then.
                    _ => self.stats.dropped += 1,
                }
            }
        }
    }

    // ---- views ------------------------------------------------------------------

    /// Hear a higher view. A primary steps down; everyone starts over in
    /// the new view.
    fn adopt_view(&mut self, view: View, out: &mut Vec<Output>) {
        assert!(view > self.view);
        self.view = view;
        self.stats.view_changes += 1;
        match self.status {
            // The view takes effect once the log is whole.
            Status::Reading | Status::Joining | Status::Repairing | Status::Erasing | Status::Halted => {}
            Status::Backup | Status::Gathering | Status::Primary => self.enter_view(out),
        }
    }

    /// Take up the role the current view gives this replica.
    fn enter_view(&mut self, out: &mut Vec<Output>) {
        if self.status == Status::Primary {
            self.stats.step_downs += 1;
        }
        if let Some(mut node) = self.node.take() {
            self.node_buggify = node.buggify().clone();
            if self.bugs.has(Bug::StaleReadAfterStepDown) {
                // The stale primary believes its uncommitted tail went
                // through, and keeps the map to answer reads from. The
                // replies it would send for those writes are dropped: the
                // bug is a stale read, not a false acknowledgement.
                let mut sink = Vec::new();
                for &(_, op) in &self.commits {
                    node.on_input(node::Input::AppendComplete { op }, &mut sink);
                }
                self.stale_node = Some(node);
            }
        }
        self.commits.clear();
        self.last_pull = None;
        if !self.joined && self.boot > 0 && !self.peers.is_empty() {
            // A restarted node first hears which view the cluster is in,
            // so that it does not take the old primary's place back by
            // reflex. If that view makes it the primary, it may have
            // prepared records in that view before it crashed and lost
            // some of them, so it moves to a view of its own: their
            // numbers are never reused.
            if self.peer_state.values().all(|p| p.seq == 0) {
                if self.status != Status::Joining {
                    self.status = Status::Joining;
                    out.push(Output::Status {
                        view: self.view,
                        status: self.status,
                    });
                }
                return;
            }
            if self.primary() == self.me {
                self.view += self.cluster_size() as u64;
                self.stats.view_changes += 1;
            }
        }
        self.joined = true;
        if self.primary() == self.me {
            self.status = Status::Gathering;
            out.push(Output::Status {
                view: self.view,
                status: self.status,
            });
            self.progress(out);
        } else {
            self.status = Status::Backup;
            out.push(Output::Status {
                view: self.view,
                status: self.status,
            });
            while let Some(envelope) = self.queued.pop_front() {
                self.forward(envelope, out);
            }
        }
    }

    /// The failover rule: the primary is `Down`, so move to the next view
    /// whose primary is not.
    fn fail_over(&mut self, out: &mut Vec<Output>) {
        let n = self.cluster_size() as u64;
        let mut view = self.view + 1;
        while view < self.view + n {
            let primary = self.primary_of(view);
            if primary == self.me || self.heartbeat.liveness(primary) != Liveness::Down {
                break;
            }
            view += 1;
        }
        self.adopt_view(view, out);
    }

    /// A gathering primary's checklist. Called whenever something it is
    /// waiting for may have happened.
    fn progress(&mut self, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Gathering);
        // 1. Every replica has reported in this view. Until then, another
        //    primary may still be preparing.
        if !self.peer_state.values().all(|p| p.view == self.view) {
            return;
        }
        //    A replica still repairing holds records its beat cannot
        //    vouch for yet.
        if self.peer_state.values().any(PeerState::is_repairing) {
            return;
        }
        // 2. This log is the newest. If not, drop what disagrees with the
        //    newest and pull the rest from its holder.
        let mine = self.runs();
        let mut best: Option<(NodeId, Vec<(u32, u64)>)> = None;
        for (&id, p) in &self.peer_state {
            let better = best
                .as_ref()
                .is_none_or(|(_, b)| newer(&p.runs, b) == core::cmp::Ordering::Greater);
            if better {
                best = Some((id, p.runs.clone()));
            }
        }
        if let Some((holder, runs)) = best {
            if newer(&runs, &mine) == core::cmp::Ordering::Greater {
                let d = divergence(&mine, &runs);
                if d <= self.len() {
                    self.truncate(d, out);
                }
                let want = (holder, self.len() + 1);
                if self.last_pull != Some(want) {
                    self.last_pull = Some(want);
                    self.stats.pulls_sent += 1;
                    out.push(Output::Send {
                        to: holder,
                        message: Message::Pull {
                            view: self.view,
                            from: want.1,
                        },
                    });
                }
                return;
            }
        }
        // 3. Every disk has this log, exactly. A peer that disagrees or is
        //    behind is told so when its beat arrives, in `sync_peer`.
        let all_have_it = self
            .peer_state
            .values()
            .all(|p| p.runs == mine && p.durable >= self.len());
        if !all_have_it || self.durable < self.len() {
            return;
        }
        // 4. Serve.
        self.become_primary(out);
    }

    /// Bring a peer's log in line with this one, as far as its last beat
    /// shows: tell it to drop a tail that disagrees, or push what it
    /// lacks. For a gathering or serving primary.
    fn sync_peer(&mut self, peer: NodeId, out: &mut Vec<Output>) {
        let p = self.peer_state[&peer].clone();
        if p.view != self.view || p.is_repairing() {
            return;
        }
        let mine = self.runs();
        let d = divergence(&mine, &p.runs);
        if d <= p.len() && d <= self.len() {
            // The peer has a record this log disagrees with. Only the
            // newest log may say so; a gathering primary that is not the
            // newest finds out in `progress` and pulls instead.
            if newer(&mine, &p.runs) == core::cmp::Ordering::Greater {
                out.push(Output::Send {
                    to: peer,
                    message: Message::Truncate {
                        view: self.view,
                        at: d,
                    },
                });
            }
        } else if p.len() < self.len() && d > p.len() {
            self.send_records(peer, p.len() + 1, out);
        }
    }

    fn become_primary(&mut self, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Gathering);
        self.status = Status::Primary;
        self.stats.promotions += 1;
        self.stale_node = None;
        let mut node_out = Vec::new();
        let mut node = Node::with_buggify(self.node_buggify.clone(), &mut node_out);
        node.set_view(self.view_u32());
        for bug in self.bugs.planted() {
            node.bugs_mut().plant(bug);
        }
        assert_eq!(node_out, vec![node::Output::Read { op: 0 }]);
        node_out.clear();
        // The node recovers from the log this replica holds in memory. It
        // is the disk's content, already checked.
        node.on_input(
            node::Input::ReadComplete {
                op: 0,
                data: self.log.clone(),
            },
            &mut node_out,
        );
        assert!(node.is_running(), "the node refused a log the replica accepted");
        assert_eq!(node.log_seq(), self.len());
        assert_eq!(node.log_len(), self.log_len());
        self.node = Some(node);
        self.route_node(node_out, out);
        out.push(Output::Status {
            view: self.view,
            status: self.status,
        });
        while let Some(envelope) = self.queued.pop_front() {
            self.serve(envelope, out);
        }
    }

    // ---- requests ----------------------------------------------------------------

    fn serve(&mut self, envelope: RequestEnvelope, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Primary);
        self.stats.requests += 1;
        let mut node_out = Vec::new();
        self.node
            .as_mut()
            .expect("a primary has a node")
            .on_input(node::Input::Request(envelope), &mut node_out);
        self.route_node(node_out, out);
    }

    fn queue(&mut self, envelope: RequestEnvelope) {
        if self.queued.len() >= QUEUE_MAX {
            self.stats.dropped += 1;
            return;
        }
        self.queued.push_back(envelope);
    }

    fn forward(&mut self, envelope: RequestEnvelope, out: &mut Vec<Output>) {
        let primary = self.primary();
        if self.heartbeat.liveness(primary) == Liveness::Down {
            self.stats.dropped += 1;
            return;
        }
        self.stats.forwarded += 1;
        out.push(Output::Send {
            to: primary,
            message: Message::Forward {
                view: self.view,
                envelope,
            },
        });
    }

    /// Turn the node's effects into the replica's. An append goes to this
    /// disk and to every backup, and is answered when it is committed.
    fn route_node(&mut self, node_out: Vec<node::Output>, out: &mut Vec<Output>) {
        for o in node_out {
            match o {
                node::Output::Read { .. } => panic!("the node reads once, at construction"),
                node::Output::Halt { damage } => {
                    panic!("the node halted on a log the replica accepted: {damage}")
                }
                node::Output::Reply(reply) => out.push(Output::Reply(reply)),
                node::Output::Append { op, offset, bytes } => {
                    let seq = self.len() + 1;
                    assert_eq!(
                        offset,
                        self.log_len(),
                        "the node and the replica disagree on the log"
                    );
                    self.commits.push_back((seq, op));
                    self.append(seq, &bytes, out);
                    for peer in self.peers.clone() {
                        self.send_prepare(peer, seq, out);
                    }
                }
            }
        }
    }

    /// Commit every write that is on every disk, oldest first.
    fn try_commit(&mut self, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Primary);
        let all_durable = self
            .peer_state
            .values()
            .map(|p| p.durable)
            .fold(self.durable, u64::min);
        while let Some(&(seq, op)) = self.commits.front() {
            if seq > all_durable {
                break;
            }
            self.commits.pop_front();
            self.stats.commits += 1;
            let mut node_out = Vec::new();
            self.node
                .as_mut()
                .expect("a primary has a node")
                .on_input(node::Input::AppendComplete { op }, &mut node_out);
            self.route_node(node_out, out);
        }
    }

    // ---- the log -----------------------------------------------------------------------

    /// Add record `seq` to the log in memory and send it to the disk.
    fn append(&mut self, seq: u64, bytes: &[u8], out: &mut Vec<Output>) {
        assert_eq!(seq, self.len() + 1);
        let (_, view) = log::record_header(bytes).expect("a record");
        let offset = self.log_len();
        self.offsets.push(offset);
        self.views.push(view);
        self.log.extend_from_slice(bytes);
        self.disk_high = self.disk_high.max(self.log_len());
        let op = self.submit(OpKind::Append {
            seq,
            generation: self.generation,
        });
        out.push(Output::Append {
            op,
            offset,
            bytes: bytes.to_vec(),
        });
    }

    fn submit(&mut self, kind: OpKind) -> OpId {
        let op = self.next_op;
        self.next_op += 1;
        self.pending.push_back(PendingOp { op, kind });
        op
    }

    /// The bytes of record `seq`.
    fn record_bytes(&self, seq: u64) -> &[u8] {
        let i = (seq - 1) as usize;
        let start = self.offsets[i] as usize;
        let end = self.offsets.get(i + 1).map_or(self.log.len(), |&o| o as usize);
        &self.log[start..end]
    }

    fn send_prepare(&mut self, to: NodeId, seq: u64, out: &mut Vec<Output>) {
        // The message was never sent. The backup's next beat says how far
        // behind it is, and the primary pushes again.
        if crate::buggify!(self.buggify, Site::DropPrepare) {
            return;
        }
        self.stats.prepares_sent += 1;
        out.push(Output::Send {
            to,
            message: Message::Prepare {
                view: self.view,
                seq,
                offset: Some(self.offsets[(seq - 1) as usize]),
                bytes: self.record_bytes(seq).to_vec(),
            },
        });
    }

    /// Records `from` onward, at most a batch.
    fn send_records(&mut self, to: NodeId, from: u64, out: &mut Vec<Output>) {
        let from = from.max(1);
        let until = self.len().min(from + BATCH - 1);
        for seq in from..=until {
            self.send_prepare(to, seq, out);
        }
    }

    // ---- the heartbeat --------------------------------------------------------------

    /// Turn the heartbeat's effects into the replica's. A beat carries
    /// the view and the log; a `Down` primary triggers the failover rule.
    fn route_heartbeat(&mut self, hb_out: Vec<heartbeat::Output>, out: &mut Vec<Output>) {
        for o in hb_out {
            match o {
                heartbeat::Output::Send {
                    to,
                    message: heartbeat::Message::Beat { seq },
                } => out.push(Output::Send {
                    to,
                    message: Message::Beat {
                        boot: self.boot,
                        seq,
                        view: self.view,
                        runs: self.runs(),
                        top: self.top(),
                        durable: self.durable,
                    },
                }),
                heartbeat::Output::PeerUp(peer) => out.push(Output::PeerUp(peer)),
                heartbeat::Output::PeerDown(peer) => {
                    out.push(Output::PeerDown(peer));
                    if self.status == Status::Backup && peer == self.primary() {
                        self.fail_over(out);
                    }
                }
            }
        }
    }

    // ---- invariants ---------------------------------------------------------------------

    /// Everything that must hold between inputs.
    fn check_invariants(&self) {
        assert_eq!(
            self.offsets.len(),
            self.records_in(&self.log),
            "offsets match the log"
        );
        assert!(self.durable <= self.len(), "durable records are in the log");
        let mut last_op = None;
        assert_eq!(self.views.len(), self.offsets.len(), "a view per record");
        let mut seq = self.durable;
        for p in &self.pending {
            assert!(last_op.is_none_or(|last| p.op > last), "pending ops are in order");
            last_op = Some(p.op);
            if let OpKind::Append { seq: s, generation } = p.kind {
                if generation == self.generation {
                    seq += 1;
                    assert_eq!(s, seq, "pending appends are numbered in order");
                }
            }
        }
        assert_eq!(seq, self.len(), "every record is durable or in flight");
        match self.status {
            Status::Primary => {
                assert_eq!(self.primary(), self.me);
                let node = self.node.as_ref().expect("a primary has a node");
                assert!(node.is_running());
                assert_eq!(node.log_seq() + self.commits.len() as u64, self.len());
                assert!(self.queued.is_empty(), "a primary serves its queue");
                let mut last = node.log_seq();
                for &(s, _) in &self.commits {
                    assert_eq!(s, last + 1);
                    last = s;
                }
            }
            Status::Gathering => {
                assert_eq!(self.primary(), self.me);
                assert!(self.node.is_none() && self.commits.is_empty());
            }
            Status::Backup => {
                assert_ne!(self.primary(), self.me);
                assert!(self.node.is_none() && self.commits.is_empty());
                assert!(self.queued.is_empty(), "a backup forwards its queue");
            }
            Status::Reading | Status::Joining | Status::Erasing => {
                assert!(self.node.is_none() && self.commits.is_empty());
                assert!(self.retained.is_empty());
            }
            Status::Repairing => {
                assert!(self.node.is_none() && self.commits.is_empty());
                assert!(
                    self.retained.keys().all(|&s| s > self.len() + 1),
                    "a retained record at the log's end is written, not retained"
                );
                assert!(!self.retained.is_empty(), "nothing to repair");
            }
            Status::Halted => {
                assert!(self.peers.is_empty(), "only a cluster of one halts");
                assert!(self.damage().is_some(), "halted without damage");
            }
        }
        assert!(self.stale_node.is_none() || self.bugs.has(Bug::StaleReadAfterStepDown));
        assert!(self.status == Status::Repairing || self.retained.is_empty());
    }

    fn records_in(&self, log: &[u8]) -> usize {
        // Cheap: counts headers, trusting what this replica wrote.
        let mut n = 0;
        let mut pos = 0;
        while pos < log.len() {
            let payload_len = u32::from_le_bytes(log[pos + 4..pos + 8].try_into().expect("4 bytes")) as usize;
            pos += log::record_len(payload_len);
            n += 1;
        }
        assert_eq!(pos, log.len());
        n
    }
}
