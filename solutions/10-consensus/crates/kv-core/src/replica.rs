//! The replicated store: Viewstamped Replication over the Module 9 replica.
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
//! Module 9 answered a write once every disk had it, and a new primary
//! served once every replica had reported. That is safe and it is not
//! live: one replica down stops the cluster. Module 10 keeps the shape
//! and changes the counting. A cluster of `n = 2f + 1` replicas needs
//! `f + 1` of anything.
//!
//! * The primary of view `v` is replica `v mod n`, as before. A write is
//!   committed, applied and answered once it is durable on `f + 1`
//!   disks, the primary's among them. Every beat and every `PrepareOk`
//!   says how far the sender's log is durable, and the primary commits
//!   the `(f + 1)`-th largest of those numbers.
//! * A view change is Liskov and Cowling's three steps. A replica that
//!   suspects the primary sends `StartViewChange` for the next view. One
//!   that has `f` of those from others sends `DoViewChange` to the new
//!   primary with its log summed up as runs and the last view it was
//!   normal in. The new primary, with `f` of those plus its own, picks
//!   the log from the highest such view, longest among equals, pulls it
//!   if it is not its own, and sends `StartView`. It serves once the
//!   whole of that log is durable on `f + 1` disks in the new view.
//! * Every record carries the checksum of the record before it, so a
//!   log is a hash chain. A replica appends a record only if the chain
//!   says it belongs at the end of its log. A backup whose log disagrees
//!   with the primary's finds out from the primary's beat and truncates
//!   itself to the point of disagreement. No record is ever truncated
//!   below the commit number, and the replica asserts it.
//! * A read is answered from the primary's map, but not before `f`
//!   peers have sent a beat in this view since the read arrived. A
//!   primary that was replaced without hearing of it never gets those
//!   beats, and never answers. Leases would be cheaper, and the planted
//!   bug `Bug::LeaseRead` shows what a slow clock does to a lease.
//! * A damaged log keeps what it finds beyond the hole, and asks every
//!   peer for the record at the hole. Any answer, from a log or from
//!   another replica's hole, is checked against the chain before it is
//!   written. Only when every peer says it has nothing is the hole
//!   erased with what follows it. A cluster of one halts, as Module 6
//!   decided.
//! * A restarted replica knows nothing but its disk. Its claim for the
//!   view change is the view of its last record, which is never more
//!   than the truth. It waits for a beat before taking up a role, and
//!   if that beat says it is the primary of the current view, it starts
//!   a view change instead of resuming: it may have numbered records in
//!   that view that are now gone.
//!
//! Fault points: `Site::DropPrepare` makes the primary skip one
//! `Prepare`. Planted bugs: `Bug::StaleReadAfterStepDown` makes a
//! primary that stepped down keep answering reads from its old map, and
//! `Bug::LeaseRead` answers a read at once on a lease of two detector
//! timeouts since a majority last beat.

use crate::buggify::{Buggify, Site};
use crate::bugs::{Bug, Bugs};
use crate::heartbeat::{self, Heartbeat, Liveness, NodeId};
use crate::log::{self, Damage, Record, Tail};
use crate::node::{self, Node, OpId, Recovery};
use crate::protocol::{Request, RequestEnvelope, ResponseEnvelope};
use crate::superblock::{self, Superblock};
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
/// Reads a primary holds while it waits for its peers' beats.
pub const READS_MAX: usize = 256;

/// The log as runs of `(view, last seq)`: which view prepared each record.
pub type Runs = Vec<(u32, u64)>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// "I am alive, in this view, with this log." Sent every interval to
    /// every peer. `boot` names the sender's incarnation and `seq` is the
    /// heartbeat's own counter, so a receiver can tell a straggler.
    Beat {
        boot: u64,
        seq: u64,
        view: View,
        /// What the sender is. Only a serving primary's log is the
        /// view's log, and only a backup's durable count is of it.
        status: Status,
        /// The last view the sender was normal in: a backup or the
        /// primary, with a log the primary of that view vouched for.
        log_view: View,
        runs: Runs,
        /// The highest record held, in the log or kept from beyond a hole.
        top: u64,
        /// Records known durable.
        durable: u64,
        /// Records the sender knows committed.
        commit: u64,
        /// The log has lost nothing since a primary last vouched for it.
        whole: bool,
    },
    /// "The primary of my view is gone. Let us move to `view`."
    StartViewChange { view: View },
    /// "Here is my log, for you to start `view` with." To the primary
    /// of `view`. `whole` says the log has lost nothing since a primary
    /// last vouched for it: a replica that restarted since then cannot
    /// tell a record it never had from one its disk lost.
    DoViewChange {
        view: View,
        log_view: View,
        runs: Runs,
        commit: u64,
        whole: bool,
    },
    /// "View `view` begins, with this log." From its primary.
    StartView { view: View, runs: Runs, commit: u64 },
    /// Record `seq`. From the primary to a backup, from the holder of a
    /// log to a primary that pulled it, or in answer to a `Fetch`. The
    /// record carries its own place: the checksum of the record before
    /// it. `commit` is what the sender knows committed.
    Prepare {
        view: View,
        seq: u64,
        commit: u64,
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
    /// "No." Under the fault atlas this is the truth about the sender's
    /// disk: a committed record is intact on all but `f` of the disks
    /// that have it, so a hole that every peer calls missing was never
    /// committed.
    Missing { view: View, seq: u64 },
}

fn write_runs(f: &mut fmt::Formatter<'_>, runs: &[(u32, u64)]) -> fmt::Result {
    write!(f, "[")?;
    for (i, (v, until)) in runs.iter().enumerate() {
        write!(f, "{}{v}..{until}", if i > 0 { " " } else { "" })?;
    }
    write!(f, "]")
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Message::Beat {
                boot,
                seq,
                view,
                status,
                log_view,
                runs,
                top,
                durable,
                commit,
                whole,
            } => {
                write!(
                    f,
                    "beat boot={boot} seq={seq} view={view} {status} log_view={log_view} runs="
                )?;
                write_runs(f, runs)?;
                write!(f, " top={top} durable={durable} commit={commit} whole={whole}")
            }
            Message::StartViewChange { view } => write!(f, "start_view_change view={view}"),
            Message::DoViewChange {
                view,
                log_view,
                runs,
                commit,
                whole,
            } => {
                write!(f, "do_view_change view={view} log_view={log_view} runs=")?;
                write_runs(f, runs)?;
                write!(f, " commit={commit} whole={whole}")
            }
            Message::StartView { view, runs, commit } => {
                write!(f, "start_view view={view} runs=")?;
                write_runs(f, runs)?;
                write!(f, " commit={commit}")
            }
            Message::Prepare {
                view,
                seq,
                commit,
                bytes,
            } => {
                write!(
                    f,
                    "prepare view={view} seq={seq} commit={commit} bytes={}",
                    bytes.len()
                )
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
    /// Between views: the primary of the last one is gone, and the
    /// primary of this one has not started it.
    ViewChange,
    /// Forwarding requests to the primary and appending what it sends.
    Backup,
    /// The primary of the view, not yet ready: collecting the logs of a
    /// majority, pulling the newest, pushing it to a majority of disks.
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
            Status::ViewChange => "view_change",
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
    /// Prepares with a stale view, a sequence number already held, a gap
    /// before them, or a parent that is not the end of this log.
    pub prepares_ignored: u64,
    pub pulls_sent: u64,
    pub pulls_served: u64,
    /// Writes committed and applied by this replica as primary.
    pub commits: u64,
    /// Reads answered after the beats that prove the view.
    pub reads: u64,
    pub view_changes: u64,
    /// View changes this replica started on its own detector or timer.
    pub view_changes_started: u64,
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
    /// Kept records that turned out not to chain to the log, and were
    /// dropped.
    pub strays: u64,
    /// Holes no peer could fill, erased with what followed them.
    pub erasures: u64,
    /// Beats and acknowledgements older than one already applied.
    pub stragglers: u64,
    /// Tails erased because they disagreed with a newer log.
    pub truncations: u64,
}

/// What the newest beat from a peer said. Newest by `(boot, seq)`, not by
/// arrival: the network reorders.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PeerState {
    boot: u64,
    seq: u64,
    view: View,
    status: Status,
    log_view: View,
    runs: Runs,
    top: u64,
    durable: u64,
    commit: u64,
    whole: bool,
}

impl Default for PeerState {
    fn default() -> PeerState {
        PeerState {
            boot: 0,
            seq: 0,
            view: 0,
            status: Status::Reading,
            log_view: 0,
            runs: Vec::new(),
            top: 0,
            durable: 0,
            commit: 0,
            whole: false,
        }
    }
}

impl PeerState {
    /// The peer is a backup in `view`: its log is a prefix of the
    /// view's log, and its durable count is of that log.
    fn is_backup_in(&self, view: View) -> bool {
        self.view == view && self.status == Status::Backup
    }

    /// Records in the peer's log.
    fn len(&self) -> u64 {
        runs_len(&self.runs)
    }
}

/// What a `DoViewChange` said.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Offer {
    log_view: View,
    runs: Runs,
    commit: u64,
    whole: bool,
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
pub fn divergence(a: &[(u32, u64)], b: &[(u32, u64)]) -> u64 {
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
pub fn newer(a: &[(u32, u64)], b: &[(u32, u64)]) -> core::cmp::Ordering {
    let d = divergence(a, b);
    view_at(a, d).cmp(&view_at(b, d))
}

/// Which of two offers a new primary takes: the one from the higher
/// normal view, and among equals the newer log.
fn better(a: &Offer, b: &Offer) -> core::cmp::Ordering {
    a.log_view.cmp(&b.log_view).then_with(|| newer(&a.runs, &b.runs))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpKind {
    Read,
    /// The append of record `seq`. A truncation at or below `seq` makes
    /// it `stale`: the completion is of a record the log no longer has
    /// and counts for nothing.
    Append {
        seq: u64,
        stale: bool,
    },
    /// Zeros over a tail: after a hole, or after a truncation.
    Erase,
    /// The superblock, carrying a promise for `view`.
    Superblock {
        view: View,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingOp {
    op: OpId,
    kind: OpKind,
}

/// A read waiting for proof that this replica is still the primary: a
/// beat from `f` peers newer than the ones it had when the read arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingRead {
    envelope: RequestEnvelope,
    /// Each peer's `(boot, seq)` when the read arrived.
    since: BTreeMap<NodeId, (u64, u64)>,
}

#[derive(Debug)]
pub struct Replica {
    me: NodeId,
    /// This incarnation, from the environment. A restart gets a new one.
    boot: u64,
    peers: Vec<NodeId>,
    status: Status,
    view: View,
    /// The last view this replica was normal in. A restarted replica
    /// claims the view of its last record, which is never too high.
    log_view: View,
    /// Records known committed by the cluster. On a backup this can run
    /// ahead of the log.
    commit: u64,
    /// The highest commit number this replica has heard from anyone, in
    /// any view. A view never starts with a log shorter than this.
    commit_heard: u64,
    /// The log has lost nothing since a primary last vouched for it.
    /// False from a restart until this replica holds a view's log
    /// again: a disk can lose a record it acknowledged, and a restarted
    /// replica cannot tell such a record from one it never had. Seed
    /// 5884 started a view from two such logs.
    whole: bool,
    /// The superblock as last written or read.
    superblock: Superblock,
    /// The highest view the disk holds a promise for. Nothing is said in
    /// a view until the promise for it is durable.
    promised: View,
    /// The primary of this view is serving and this replica has agreed
    /// to follow it, once its promise is durable.
    follow_pending: bool,
    heartbeat: Heartbeat,
    /// The log as this replica knows it: every record durable or in
    /// flight, in order. A restart rebuilds it from the disk.
    log: Vec<u8>,
    /// Offset of record `i + 1`. Its length is the number of records.
    offsets: Vec<u64>,
    /// The view record `i + 1` was prepared in.
    views: Vec<u32>,
    /// The checksum of record `i + 1`: the parent of record `i + 2`.
    checksums: Vec<u32>,
    /// Records known durable.
    durable: u64,
    /// Bytes ever written on this disk, at most. An erasure zeros up to
    /// here.
    disk_high: u64,
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
    /// Writes the node submitted, awaiting a majority's `PrepareOk`:
    /// `(seq, node op)` in order.
    commits: VecDeque<(u64, OpId)>,
    /// Reads waiting for the beats that prove this replica is still the
    /// primary.
    reads: VecDeque<PendingRead>,
    peer_state: BTreeMap<NodeId, PeerState>,
    /// Peers that sent `StartViewChange` for the current view.
    view_change_from: BTreeSet<NodeId>,
    /// The `DoViewChange` of each peer for the current view.
    offers: BTreeMap<NodeId, Offer>,
    /// This replica has sent its `DoViewChange`, or counted its own.
    offered: bool,
    /// The primary of this view has sent `StartView`.
    started: bool,
    /// Records in the primary's log when this backup first heard from
    /// it in this view: the view's log, or more.
    start_len: u64,
    /// When the view change last made progress. Silence past the
    /// detector's limit moves to the next view.
    view_change_at: Instant,
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
            log_view: 0,
            commit: 0,
            commit_heard: 0,
            whole: boot == 0,
            superblock: Superblock::default(),
            promised: 0,
            follow_pending: false,
            heartbeat,
            log: Vec::new(),
            offsets: Vec::new(),
            views: Vec::new(),
            checksums: Vec::new(),
            durable: 0,
            disk_high: 0,
            recovery: None,
            retained: BTreeMap::new(),
            missing: BTreeSet::new(),
            pending: VecDeque::new(),
            next_op: 0,
            node: None,
            commits: VecDeque::new(),
            reads: VecDeque::new(),
            peer_state: peers.iter().map(|&p| (p, PeerState::default())).collect(),
            view_change_from: BTreeSet::new(),
            offers: BTreeMap::new(),
            offered: false,
            started: false,
            start_len: 0,
            view_change_at: now,
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

    /// Failures the cluster tolerates: `(n - 1) / 2`.
    pub fn f(&self) -> usize {
        (self.cluster_size() - 1) / 2
    }

    pub fn view(&self) -> View {
        self.view
    }

    /// The last view this replica was normal in.
    pub fn log_view(&self) -> View {
        self.log_view
    }

    /// Records this replica knows committed.
    pub fn commit(&self) -> u64 {
        self.commit
    }

    /// The highest view the disk holds a promise for.
    pub fn promised(&self) -> View {
        self.promised
    }

    /// The log has lost nothing since a primary last vouched for it.
    pub fn is_whole(&self) -> bool {
        self.whole
    }

    /// The superblock as last written or read.
    pub fn superblock(&self) -> Superblock {
        self.superblock
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
    /// from beyond a hole.
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
    pub fn runs(&self) -> Runs {
        let mut runs: Runs = Vec::new();
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

    /// The checksum of the last record: what the next one must name as
    /// its parent.
    pub fn chain(&self) -> u32 {
        self.checksums.last().copied().unwrap_or(0)
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

    /// Reads waiting for the beats that prove the view.
    pub fn pending_reads(&self) -> usize {
        self.reads.len()
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
            Status::ViewChange | Status::Gathering => {
                // Silence from everyone this view change depends on, for
                // as long as the detector takes to call a peer down: the
                // primary of this view is not coming. Try the next one.
                let limit = self.heartbeat.config().suspect_after;
                if now.saturating_duration_since(self.view_change_at) >= limit {
                    self.fail_over(out);
                    return;
                }
                // Messages get lost. Say it again.
                if self.promised >= self.view {
                    self.on_promised(out);
                }
            }
            // Ask again for the record at the hole: the answer may have
            // been lost, or a peer may have it now.
            Status::Repairing => self.repair(out),
            Status::Primary => self.serve_reads(out),
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
        if let Some(sb) = superblock::read(data) {
            self.superblock = sb;
            self.promised = sb.view;
            self.view = self.view.max(sb.view).max(sb.log_view);
        }
        let data = superblock::log_bytes(data);
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
        self.checksums = recovered.checksums.clone();
        self.durable = self.len();
        self.disk_high = data.len() as u64;
        self.claim_last_view();
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
            for (header, bytes) in log::salvage(data, recovered.valid_len) {
                if header.seq <= self.len() {
                    continue;
                }
                // Two copies of one number: the one from the higher view
                // is the one the cluster kept, or the nearest to it.
                let newer = self
                    .retained
                    .get(&header.seq)
                    .is_none_or(|old| log::record_header(old).is_some_and(|h| h.view < header.view));
                if newer {
                    self.retained.insert(header.seq, bytes);
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
        self.join(out);
    }

    /// Make the log whole: write out every retained record that chains
    /// to it, and at a hole ask every peer for the record. Called
    /// whenever something may have filled the hole.
    fn repair(&mut self, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Repairing);
        loop {
            let next = self.len() + 1;
            // Strays at or below the log are copies of what it has.
            while self.retained.keys().next().is_some_and(|&s| s < next) {
                self.retained.pop_first();
            }
            match self.retained.get(&next) {
                // Back at its place, written again so the disk has it
                // where recovery looks.
                Some(bytes) if log::record_header(bytes).is_some_and(|h| h.parent == self.chain()) => {
                    let bytes = self.retained.remove(&next).expect("present");
                    self.append(next, &bytes, out);
                }
                // The record with this number was replaced after this
                // copy was written. The chain says so.
                Some(_) => {
                    self.retained.remove(&next);
                    self.stats.strays += 1;
                }
                None if self.retained.is_empty() => {
                    self.missing.clear();
                    self.join(out);
                    return;
                }
                None => {
                    // A hole. Every peer is asked; the first answer that
                    // chains fills it, and if every peer says no, nobody
                    // has it.
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
            offset: superblock::LOG_OFFSET as u64 + from,
            bytes: vec![0; (self.disk_high.max(from) - from) as usize],
        });
    }

    /// The log disagrees with the one the cluster kept from record `at`
    /// on: drop it from there, in memory now and on disk by an erasure
    /// that precedes every later append. Never below the commit number:
    /// that would be the protocol losing an acknowledged write, and the
    /// assertion is the check for it.
    fn truncate(&mut self, at: u64, out: &mut Vec<Output>) {
        assert!(at >= 1 && at <= self.len(), "truncate at {at} of {}", self.len());
        assert!(
            at > self.commit,
            "node {} view {}: told to truncate record {at}, committed through {}",
            self.me,
            self.view,
            self.commit
        );
        self.stats.truncations += 1;
        let keep = (at - 1) as usize;
        self.log.truncate(self.offsets[keep] as usize);
        self.offsets.truncate(keep);
        self.views.truncate(keep);
        self.checksums.truncate(keep);
        self.durable = self.durable.min(at - 1);
        // Appends of records below the cut are still of this log. Seed 19
        // found a truncation that forgot them all, and the durable count
        // never caught up with the log again.
        for p in &mut self.pending {
            if let OpKind::Append { seq, stale } = &mut p.kind {
                if *seq >= at {
                    *stale = true;
                }
            }
        }
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
                    self.join(out);
                }
            }
            OpKind::Superblock { view } => {
                self.promised = self.promised.max(view);
                if view == self.view {
                    self.on_promised(out);
                }
            }
            OpKind::Append { stale: true, .. } => {
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
                    | Status::ViewChange
                    | Status::Halted => {}
                }
            }
        }
    }

    fn on_request(&mut self, envelope: RequestEnvelope, out: &mut Vec<Output>) {
        match self.status {
            Status::Primary => self.serve(envelope, out),
            Status::Reading
            | Status::Joining
            | Status::Repairing
            | Status::Erasing
            | Status::ViewChange
            | Status::Gathering => self.queue(envelope),
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
                status,
                log_view,
                runs,
                top,
                durable,
                commit,
                whole,
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
                    status,
                    log_view,
                    runs: runs.clone(),
                    top,
                    durable,
                    commit,
                    whole,
                };
                self.commit_heard = self.commit_heard.max(commit);
                if view > self.view {
                    self.hear_view(view, now, out);
                }
                if view < self.view {
                    return;
                }
                // A beat from a peer between views is its offer for this
                // one, whether or not its `DoViewChange` arrives. Seed 525
                // started a view from a quorum that lacked the one replica
                // whose log had the last two committed records, with that
                // replica's beat in hand.
                if status == Status::ViewChange && self.primary() == self.me {
                    self.offer(
                        from,
                        Offer {
                            log_view,
                            runs,
                            commit,
                            whole,
                        },
                        now,
                        out,
                    );
                }
                let normal_primary = from == self.primary() && status == Status::Primary;
                // The primary this replica follows is reading its disk: it
                // restarted, and the view is over whether or not the
                // detector noticed. Only for a backup. A replica between
                // views hears the first beat of a primary that has not
                // read its disk yet, and that is a start, not a restart.
                let primary_restarted = from == self.primary()
                    && matches!(
                        status,
                        Status::Reading | Status::Joining | Status::Repairing | Status::Erasing
                    );
                if primary_restarted && self.status == Status::Backup {
                    self.fail_over(out);
                    return;
                }
                match self.status {
                    Status::Joining => self.take_up(now, out),
                    Status::ViewChange => {
                        if normal_primary {
                            self.follow(from, out);
                        } else if from == self.primary() {
                            self.view_change_at = now;
                        }
                    }
                    Status::Backup => {
                        if normal_primary {
                            self.follow(from, out);
                        }
                    }
                    Status::Repairing => self.adopt_newer(from, out),
                    Status::Gathering => {
                        self.view_change_at = now;
                        self.sync_peer(from, out);
                        self.progress(out);
                    }
                    Status::Primary => {
                        self.sync_peer(from, out);
                        self.try_commit(out);
                        self.serve_reads(out);
                    }
                    Status::Reading | Status::Erasing | Status::Halted => {}
                }
            }
            Message::StartViewChange { view } => {
                if view < self.view {
                    return;
                }
                if view > self.view {
                    self.hear_view(view, now, out);
                }
                if self.status == Status::ViewChange || self.status == Status::Gathering {
                    self.view_change_from.insert(from);
                    self.view_change_at = now;
                    self.send_view_change(out);
                }
            }
            Message::DoViewChange {
                view,
                log_view,
                runs,
                commit,
                whole,
            } => {
                if view < self.view {
                    return;
                }
                if view > self.view {
                    self.hear_view(view, now, out);
                }
                self.commit_heard = self.commit_heard.max(commit);
                if self.primary() != self.me {
                    return;
                }
                self.offer(
                    from,
                    Offer {
                        log_view,
                        runs,
                        commit,
                        whole,
                    },
                    now,
                    out,
                );
            }
            Message::StartView { view, runs, commit } => {
                if view < self.view {
                    return;
                }
                if view > self.view {
                    self.hear_view(view, now, out);
                }
                if from != self.primary() {
                    return;
                }
                // The same as a beat from a serving primary, and it comes
                // sooner.
                let peer = self.peer_state.get_mut(&from).expect("known peer");
                peer.view = view;
                peer.status = Status::Primary;
                peer.log_view = view;
                peer.runs = runs;
                peer.commit = peer.commit.max(commit);
                self.commit_heard = self.commit_heard.max(commit);
                match self.status {
                    Status::Joining => self.take_up(now, out),
                    Status::ViewChange | Status::Backup => self.follow(from, out),
                    Status::Repairing => self.adopt_newer(from, out),
                    _ => {}
                }
            }
            Message::Prepare {
                view,
                seq,
                commit,
                bytes,
            } => {
                if view < self.view {
                    self.stats.prepares_ignored += 1;
                    return;
                }
                if view > self.view {
                    self.hear_view(view, now, out);
                }
                self.commit_heard = self.commit_heard.max(commit);
                let accept = match self.status {
                    Status::Backup => from == self.primary(),
                    Status::Gathering => self.last_pull.is_some_and(|(peer, _)| peer == from),
                    // The answer to a fetch, from whoever had it.
                    Status::Repairing => true,
                    _ => false,
                };
                let header = log::record_header(&bytes);
                let chains = header.is_some_and(|h| h.seq == seq && h.parent == self.chain());
                if !accept || seq != self.len() + 1 || !chains {
                    self.stats.prepares_ignored += 1;
                    return;
                }
                self.append(seq, &bytes, out);
                self.stats.prepares_accepted += 1;
                match self.status {
                    Status::Backup => {
                        self.commit = self.commit.max(commit);
                        self.note_caught_up(out);
                    }
                    Status::Gathering => {
                        self.view_change_at = now;
                        self.progress(out);
                    }
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
                        status: Status::Backup,
                        log_view: view,
                        runs: Vec::new(),
                        top: 0,
                        durable,
                        commit: 0,
                        whole: false,
                    };
                }
                // Only a backup that took this view's log acknowledges.
                // What its beat said before it took the view's log was
                // durability of a different log. Seed 563 carried an old
                // uncommitted tail of 69 records into view 26 as an
                // acknowledgement of three records only the primary had.
                if peer.is_backup_in(view) {
                    peer.durable = peer.durable.max(durable);
                } else {
                    peer.view = view;
                    peer.status = Status::Backup;
                    peer.log_view = view;
                    peer.durable = durable;
                }
                match self.status {
                    Status::Gathering => {
                        self.view_change_at = now;
                        self.progress(out);
                    }
                    Status::Primary => self.try_commit(out),
                    _ => {}
                }
            }
            Message::Pull { view, from: start } => {
                if view < self.view {
                    return;
                }
                if view > self.view {
                    self.hear_view(view, now, out);
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
                    self.hear_view(view, now, out);
                }
                if matches!(self.status, Status::Reading | Status::Halted) {
                    return;
                }
                if seq >= 1 && seq <= self.len() {
                    self.send_prepare(from, seq, out);
                } else if let Some(bytes) = self.retained.get(&seq) {
                    // Kept from beyond this replica's own hole. Real, and
                    // the chain lets the asker tell whether it fits.
                    out.push(Output::Send {
                        to: from,
                        message: Message::Prepare {
                            view: self.view,
                            seq,
                            commit: self.commit,
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
                    self.hear_view(view, now, out);
                }
                if self.status != Status::Repairing || seq != self.len() + 1 {
                    return;
                }
                // Every peer, whatever its state: a peer that is down
                // may hold the only intact copy, and the erase waits for
                // it. Seed 619 erased two committed records when two
                // disks had lost the same one; the atlas forbids that
                // now. Seed 353 waited for a word only a whole peer could
                // give, from a peer that could never be whole again.
                self.missing.insert(from);
                if self.peers.iter().all(|p| self.missing.contains(p)) {
                    self.erase(out);
                }
            }
            Message::Forward { view, envelope } => {
                if view > self.view {
                    self.hear_view(view, now, out);
                }
                match self.status {
                    Status::Primary => self.serve(envelope, out),
                    Status::Gathering | Status::ViewChange => self.queue(envelope),
                    // Not the primary any more, or not yet. The client
                    // retries, and the backup that forwarded it will know
                    // better by then.
                    _ => self.stats.dropped += 1,
                }
            }
        }
    }

    // ---- views ------------------------------------------------------------------

    /// Hear of a higher view. A replica with a role joins the view
    /// change for it; one still reading or repairing its disk only notes
    /// the number.
    fn hear_view(&mut self, view: View, now: Instant, out: &mut Vec<Output>) {
        assert!(view > self.view);
        match self.status {
            Status::Reading | Status::Joining | Status::Repairing | Status::Erasing | Status::Halted => {
                self.view = view;
            }
            Status::ViewChange | Status::Backup | Status::Gathering | Status::Primary => {
                self.start_view_change(view, now, out);
            }
        }
    }

    /// The claim for a view change, for a replica that remembers nothing
    /// but its disk: the view of the last record. This replica was
    /// normal in that view at least, and may have been in a later one,
    /// which is the safe side to err on. Made again once a repair has
    /// put the records beyond a hole back: seed 36 offered a log of
    /// ninety-two records under the view of the twenty-four before the
    /// hole, and lost to a shorter log.
    ///
    /// The superblock does better when the log is as long as it was
    /// when the claim in it was made. A log that is shorter lost
    /// records, and the claim in the superblock is void.
    fn claim_last_view(&mut self) {
        let last = self.views.last().map_or(0, |&v| u64::from(v));
        let sb = self.superblock;
        self.log_view = if self.len() >= sb.log_view_len {
            last.max(sb.log_view)
        } else {
            last
        };
        self.view = self.view.max(self.log_view);
    }

    /// Write the superblock: the view this replica agrees to, and the
    /// view and length of the log it last held for a primary. Nothing is
    /// said in a view before its promise is durable, and the disk
    /// completes writes in order, so an acknowledgement for any record
    /// appended after this write implies it.
    fn write_superblock(&mut self, out: &mut Vec<Output>) {
        let next = Superblock {
            seq: self.superblock.seq + 1,
            view: self.view,
            log_view: self.log_view,
            log_view_len: if self.log_view == self.view {
                self.len()
            } else {
                self.superblock.log_view_len
            },
        };
        // Nothing new to say: the last write, durable or in the queue
        // ahead of everything later, already says it.
        let same = (next.view, next.log_view, next.log_view_len)
            == (
                self.superblock.view,
                self.superblock.log_view,
                self.superblock.log_view_len,
            );
        if same && self.superblock.seq > 0 {
            return;
        }
        self.superblock = next;
        let sb = self.superblock;
        let op = self.submit(OpKind::Superblock { view: sb.view });
        out.push(Output::Append {
            op,
            offset: sb.offset() as u64,
            bytes: sb.encode(),
        });
    }

    /// The promise for the current view is on the disk. Say what was
    /// held back for it.
    fn on_promised(&mut self, out: &mut Vec<Output>) {
        match self.status {
            Status::ViewChange => {
                for peer in self.peers.clone() {
                    out.push(Output::Send {
                        to: peer,
                        message: Message::StartViewChange { view: self.view },
                    });
                }
                self.send_view_change(out);
                if self.follow_pending {
                    self.become_backup(out);
                }
            }
            Status::Gathering => self.progress(out),
            _ => {}
        }
    }

    /// The log is whole. Take up a role, or wait to hear which view the
    /// cluster is in.
    fn join(&mut self, out: &mut Vec<Output>) {
        let now = self.last_now;
        self.claim_last_view();
        if self.boot == 0 || self.peers.is_empty() {
            // A fresh cluster starts in view 0, and a cluster of one is
            // its own primary at once.
            self.start_view_change(self.view, now, out);
            return;
        }
        // A restarted node first hears which view the cluster is in, so
        // that it does not take the old primary's place back by reflex.
        if self.peer_state.values().all(|p| p.seq == 0) {
            self.status = Status::Joining;
            out.push(Output::Status {
                view: self.view,
                status: self.status,
            });
            return;
        }
        self.take_up(now, out);
    }

    /// A restarted replica has heard a beat. If the view it names makes
    /// this replica the primary, it may have prepared records in that
    /// view before it crashed and lost some of them, so it moves on to a
    /// view of someone else's: those numbers are never reused. Otherwise
    /// it follows the primary if the primary is normal, and joins the
    /// view change if not.
    fn take_up(&mut self, now: Instant, out: &mut Vec<Output>) {
        assert!(
            matches!(
                self.status,
                Status::Reading | Status::Joining | Status::Repairing | Status::Erasing
            ),
            "take_up as {}",
            self.status
        );
        let view = self
            .peer_state
            .values()
            .map(|p| p.view)
            .max()
            .unwrap_or(0)
            .max(self.view);
        self.view = view;
        if self.primary() == self.me {
            self.fail_over(out);
            return;
        }
        self.start_view_change(view, now, out);
        let primary = self.primary();
        let serving =
            self.peer_state[&primary].view == view && self.peer_state[&primary].status == Status::Primary;
        if serving {
            self.follow(primary, out);
        }
    }

    /// Move to `view` and ask the others to. A primary steps down;
    /// everyone starts over in the new view.
    fn start_view_change(&mut self, view: View, now: Instant, out: &mut Vec<Output>) {
        assert!(
            view > self.view
                || !matches!(
                    self.status,
                    Status::ViewChange | Status::Backup | Status::Gathering | Status::Primary
                ),
            "node {} view {}: start_view_change({view}) as {}",
            self.me,
            self.view,
            self.status
        );
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
        if view > self.view {
            self.stats.view_changes += 1;
        }
        self.view = view;
        self.commits.clear();
        self.reads.clear();
        self.last_pull = None;
        self.view_change_from.clear();
        self.offers.clear();
        self.offered = false;
        self.started = false;
        self.view_change_at = now;
        self.follow_pending = false;
        self.status = Status::ViewChange;
        out.push(Output::Status {
            view: self.view,
            status: self.status,
        });
        // The promise first. Seed 562 had a replica vote, crash, forget,
        // and rejoin the view it had voted to leave.
        if self.promised >= view {
            self.on_promised(out);
        } else {
            self.write_superblock(out);
        }
    }

    /// A peer's log, offered to this replica as the primary of the view.
    /// An offer is also a vote for the view.
    fn offer(&mut self, from: NodeId, offer: Offer, now: Instant, out: &mut Vec<Output>) {
        assert_eq!(self.primary(), self.me);
        if self.status != Status::ViewChange && self.status != Status::Gathering {
            return;
        }
        self.view_change_from.insert(from);
        self.view_change_at = now;
        self.offers.insert(from, offer);
        self.send_view_change(out);
        if self.status == Status::Gathering {
            self.progress(out);
        }
    }

    /// With `f` peers agreed on the view, offer the log to its primary.
    /// The primary of the view counts its own offer and gathers.
    fn send_view_change(&mut self, out: &mut Vec<Output>) {
        if self.view_change_from.len() < self.f() || self.promised < self.view {
            return;
        }
        if self.primary() == self.me {
            if self.status == Status::ViewChange {
                self.status = Status::Gathering;
                self.offered = true;
                out.push(Output::Status {
                    view: self.view,
                    status: self.status,
                });
                self.progress(out);
            }
            return;
        }
        // Sent again on every tick until the primary's `StartView`.
        self.offered = true;
        out.push(Output::Send {
            to: self.primary(),
            message: Message::DoViewChange {
                view: self.view,
                log_view: self.log_view,
                runs: self.runs(),
                commit: self.commit,
                whole: self.whole,
            },
        });
    }

    /// The failover rule: the primary is gone, so move to the next view
    /// whose primary is not known to be.
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
        self.stats.view_changes_started += 1;
        let now = self.last_now;
        self.start_view_change(view, now, out);
    }

    /// The primary of this view is normal in it, and its last beat or
    /// `StartView` says what its log is. Become its backup, if not one
    /// already, and make this log agree with its: drop a tail that
    /// disagrees, and learn what is committed.
    fn follow(&mut self, primary: NodeId, out: &mut Vec<Output>) {
        assert_eq!(primary, self.primary());
        let p = self.peer_state[&primary].clone();
        assert_eq!((p.view, p.status), (self.view, Status::Primary));
        let first = self.status != Status::Backup;
        if first {
            self.start_len = p.len();
        }
        // Drop what disagrees before taking the primary's commit number:
        // that number is of the primary's log, and the guard inside
        // `truncate` compares against what this replica knew committed of
        // its own. Seed 13 had an old primary adopt a commit of 10 and
        // then try to drop its own uncommitted record 7.
        //
        // On the first word from the primary, a tail it does not have is
        // dropped too: those records are from older views and were never
        // committed. After that the primary's log only grows, and a beat
        // that is merely shorter than this log is older than the prepares
        // that made this log. Seed 24 had a beat arrive 130 ms late and a
        // backup cut three committed records to match it.
        let d = divergence(&self.runs(), &p.runs);
        if d <= self.len() && (first || d <= p.len()) {
            self.truncate(d, out);
        }
        self.commit = self.commit.max(p.commit);
        if first {
            // A backup only once the promise for the view is durable: the
            // primary counts a backup's disk towards a majority, and a
            // backup that forgot the view could let an old primary
            // overwrite what it counted.
            if self.promised >= self.view {
                self.become_backup(out);
            } else {
                self.follow_pending = true;
            }
            return;
        }
        self.note_caught_up(out);
    }

    fn become_backup(&mut self, out: &mut Vec<Output>) {
        assert!(self.promised >= self.view);
        self.follow_pending = false;
        self.status = Status::Backup;
        out.push(Output::Status {
            view: self.view,
            status: self.status,
        });
        self.note_caught_up(out);
        // The primary has not heard what this disk holds since the view
        // began.
        let to = self.primary();
        out.push(Output::Send {
            to,
            message: Message::PrepareOk {
                boot: self.boot,
                view: self.view,
                durable: self.durable,
            },
        });
        while let Some(envelope) = self.queued.pop_front() {
            self.forward(envelope, out);
        }
    }

    /// A backup claims the view for its log only once it holds the
    /// view's log. Seed 7 found a backup that had taken the view on the
    /// primary's word and offered its short log under the view's number
    /// at the next view change, where it beat a longer log from an
    /// older view and would have cost six committed records.
    ///
    /// The log covers the view's log once it is as long as the primary's
    /// was at the first word, or once it holds any record prepared in
    /// this view: such a record chains back through the whole of the
    /// view's log. Seed 22 had the target move with the primary's every
    /// append, and a backup one record behind that never claimed the
    /// view lost the next one to a shorter log that had.
    fn note_caught_up(&mut self, out: &mut Vec<Output>) {
        let holds_the_view = self.len() >= self.start_len || self.views.last() == Some(&self.view_u32());
        if self.status == Status::Backup && holds_the_view && self.log_view < self.view {
            self.log_view = self.view;
            // The primary vouched for every record up to here, and every
            // record from here on is appended by this incarnation, which
            // knows what it appended.
            self.whole = true;
            self.write_superblock(out);
        }
    }

    /// A repairing replica makes its prefix agree with any peer whose
    /// log is newer at the first disagreement, so that the record it
    /// fetches next has somewhere to chain to.
    fn adopt_newer(&mut self, peer: NodeId, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Repairing);
        let p = &self.peer_state[&peer];
        let mine = self.runs();
        let d = divergence(&mine, &p.runs);
        if d <= self.len() && newer(&p.runs, &mine) == core::cmp::Ordering::Greater {
            self.truncate(d, out);
            self.repair(out);
        }
    }

    /// The new primary's checklist. Called whenever something it is
    /// waiting for may have happened.
    fn progress(&mut self, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Gathering);
        let f = self.f();
        // 1. `f` offers, which with this replica's own make a majority.
        //    Any committed record is on one of these logs, if the logs
        //    are whole. A replica that restarted since a primary last
        //    vouched for its log may have lost a record its disk
        //    acknowledged, and cannot say so. Its records count, its
        //    silence does not: a majority of whole logs is needed, or
        //    every log there is.
        if self.offers.len() < f {
            return;
        }
        let whole = self.offers.values().filter(|o| o.whole).count() + usize::from(self.whole);
        if whole < f + 1 && self.offers.len() < self.peers.len() {
            return;
        }
        // 2. The log to start with: from the highest normal view, longest
        //    among equals. If it is not this one, pull it from its holder
        //    after dropping what disagrees with it.
        let mine = Offer {
            log_view: self.log_view,
            runs: self.runs(),
            commit: self.commit,
            whole: self.whole,
        };
        let mut best: Option<(NodeId, Offer)> = None;
        for (&id, offer) in &self.offers {
            let is_better = best
                .as_ref()
                .is_none_or(|(_, b)| better(offer, b) == core::cmp::Ordering::Greater);
            if is_better {
                best = Some((id, offer.clone()));
            }
        }
        if let Some((holder, offer)) = best {
            if better(&offer, &mine) == core::cmp::Ordering::Greater && offer.runs != mine.runs {
                let d = divergence(&mine.runs, &offer.runs);
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
        // 3. This is the newest log offered. If it is shorter than a
        //    commit number this replica has heard, a committed record is
        //    on a log that has not been offered yet: a replica that is
        //    slow, cut off or down. Wait for it. A disk that lost a write
        //    after acknowledging it makes this real: seed 525.
        if self.len() < self.commit_heard {
            return;
        }
        //    Everything any offer called committed is in it, and the
        //    assertion is the check.
        if !self.started {
            let commit = self.offers.values().map(|o| o.commit).fold(self.commit, u64::max);
            assert!(
                commit <= self.len(),
                "node {} view {}: the chosen log has {} records, {commit} are committed",
                self.me,
                self.view,
                self.len()
            );
            self.commit = commit;
            self.log_view = self.view;
            self.whole = true;
            self.started = true;
            self.write_superblock(out);
            for peer in self.peers.clone() {
                self.send_start_view(peer, out);
            }
        }
        // 4. The whole log is durable on a majority of disks in this
        //    view. A backup that disagrees or is behind is told so when
        //    its beat arrives, in `sync_peer`.
        let have_it = self
            .peer_state
            .values()
            .filter(|p| p.is_backup_in(self.view) && p.durable >= self.len())
            .count();
        if have_it < f || self.durable < self.len() {
            return;
        }
        // 5. Serve.
        self.commit = self.len();
        self.become_primary(out);
    }

    fn send_start_view(&mut self, to: NodeId, out: &mut Vec<Output>) {
        out.push(Output::Send {
            to,
            message: Message::StartView {
                view: self.view,
                runs: self.runs(),
                commit: self.commit,
            },
        });
    }

    /// Bring a peer in line with this log, as far as its last beat shows:
    /// tell it the view started, or push what it lacks. A peer that has a
    /// record this log disagrees with drops it on its own when it sees
    /// this log's runs. For a gathering primary that has started the
    /// view, or a serving one.
    fn sync_peer(&mut self, peer: NodeId, out: &mut Vec<Output>) {
        if !self.started {
            return;
        }
        let p = self.peer_state[&peer].clone();
        if p.view != self.view {
            return;
        }
        match p.status {
            Status::Backup => {}
            Status::ViewChange | Status::Joining => {
                self.send_start_view(peer, out);
                return;
            }
            _ => return,
        }
        let d = divergence(&self.runs(), &p.runs);
        if d > p.len() && p.len() < self.len() {
            self.send_records(peer, p.len() + 1, out);
        }
    }

    fn become_primary(&mut self, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Gathering);
        assert_eq!(self.commit, self.len(), "a new primary serves a committed log");
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
        // is the view's log, committed in full, already checked.
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
        if matches!(envelope.request, Request::Get { .. }) {
            // Planted bug: "I heard from a majority a moment ago, so I am
            // still the primary." A moment by whose clock, and how long
            // does the moment last?
            let lease = self.bugs.has(Bug::LeaseRead) && {
                let limit = self.heartbeat.config().suspect_after * 2;
                let now = self.last_now;
                self.peers
                    .iter()
                    .filter(|&&p| {
                        self.heartbeat
                            .last_heard(p)
                            .is_some_and(|at| now.saturating_duration_since(at) <= limit)
                    })
                    .count()
                    >= self.f()
            };
            if !lease {
                if self.reads.len() >= READS_MAX {
                    self.stats.dropped += 1;
                    return;
                }
                let since = self
                    .peer_state
                    .iter()
                    .map(|(&id, p)| (id, (p.boot, p.seq)))
                    .collect();
                self.reads.push_back(PendingRead { envelope, since });
                self.serve_reads(out);
                return;
            }
        }
        self.answer(envelope, out);
    }

    /// Hand a request to the node.
    fn answer(&mut self, envelope: RequestEnvelope, out: &mut Vec<Output>) {
        let mut node_out = Vec::new();
        self.node
            .as_mut()
            .expect("a primary has a node")
            .on_input(node::Input::Request(envelope), &mut node_out);
        self.route_node(node_out, out);
    }

    /// Answer every waiting read that `f` peers have beaten past, in
    /// this view, since it arrived. Those beats say a majority still
    /// follows this primary, so nothing newer than its map has been
    /// committed anywhere.
    fn serve_reads(&mut self, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Primary);
        while let Some(read) = self.reads.front() {
            let proven = self
                .peer_state
                .iter()
                .filter(|(id, p)| p.is_backup_in(self.view) && (p.boot, p.seq) > read.since[id])
                .count();
            if proven < self.f() {
                return;
            }
            let read = self.reads.pop_front().expect("front");
            self.stats.reads += 1;
            self.answer(read.envelope, out);
        }
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

    /// Commit every write that is on a majority of disks, oldest first.
    fn try_commit(&mut self, out: &mut Vec<Output>) {
        assert_eq!(self.status, Status::Primary);
        let majority = self.majority_durable();
        while let Some(&(seq, op)) = self.commits.front() {
            if seq > majority {
                break;
            }
            self.commits.pop_front();
            self.stats.commits += 1;
            self.commit = seq;
            let mut node_out = Vec::new();
            self.node
                .as_mut()
                .expect("a primary has a node")
                .on_input(node::Input::AppendComplete { op }, &mut node_out);
            self.route_node(node_out, out);
        }
    }

    /// The highest record durable on `f + 1` disks, this one among them,
    /// counting only peers that are normal in this view: their logs are
    /// prefixes of this one, so their durable counts are of these
    /// records.
    fn majority_durable(&self) -> u64 {
        let mut durables: Vec<u64> = self
            .peer_state
            .values()
            .filter(|p| p.is_backup_in(self.view))
            .map(|p| p.durable.min(self.durable))
            .collect();
        durables.sort_unstable_by(|a, b| b.cmp(a));
        durables
            .get(self.f().wrapping_sub(1))
            .copied()
            .unwrap_or(if self.f() == 0 { self.durable } else { 0 })
    }

    // ---- the log -----------------------------------------------------------------------

    /// Add record `seq` to the log in memory and send it to the disk. The
    /// caller checked that the record chains to the log.
    fn append(&mut self, seq: u64, bytes: &[u8], out: &mut Vec<Output>) {
        assert_eq!(seq, self.len() + 1);
        let header = log::record_header(bytes).expect("a record");
        assert_eq!(header.seq, seq);
        assert_eq!(
            header.parent,
            self.chain(),
            "a record chains to the one before it"
        );
        let offset = self.log_len();
        self.offsets.push(offset);
        self.views.push(header.view);
        self.checksums.push(header.checksum);
        self.log.extend_from_slice(bytes);
        self.disk_high = self.disk_high.max(self.log_len());
        let op = self.submit(OpKind::Append { seq, stale: false });
        out.push(Output::Append {
            op,
            offset: superblock::LOG_OFFSET as u64 + offset,
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
                commit: self.commit,
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
                        status: self.status,
                        log_view: self.log_view,
                        runs: self.runs(),
                        top: self.top(),
                        durable: self.durable,
                        commit: self.commit,
                        whole: self.whole,
                    },
                }),
                heartbeat::Output::PeerUp(peer) => out.push(Output::PeerUp(peer)),
                heartbeat::Output::PeerDown(peer) => {
                    out.push(Output::PeerDown(peer));
                    let waiting_on_it = matches!(self.status, Status::Backup | Status::ViewChange);
                    if waiting_on_it && peer == self.primary() {
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
        assert_eq!(self.views.len(), self.offsets.len(), "a view per record");
        assert_eq!(self.checksums.len(), self.offsets.len(), "a checksum per record");
        assert!(
            self.views.windows(2).all(|w| w[0] <= w[1]),
            "views never go down along a log"
        );
        let mut last_op = None;
        let mut seq = self.durable;
        for p in &self.pending {
            assert!(last_op.is_none_or(|last| p.op > last), "pending ops are in order");
            last_op = Some(p.op);
            if let OpKind::Append { seq: s, stale: false } = p.kind {
                seq += 1;
                assert_eq!(s, seq, "pending appends are numbered in order");
            }
        }
        assert_eq!(seq, self.len(), "every record is durable or in flight");
        assert!(self.log_view <= self.view, "normal in a view not yet reached");
        match self.status {
            Status::Primary => {
                assert_eq!(self.primary(), self.me);
                assert_eq!(self.log_view, self.view);
                assert!(self.promised >= self.view, "a primary has promised its view");
                let node = self.node.as_ref().expect("a primary has a node");
                assert!(node.is_running());
                assert_eq!(node.log_seq() + self.commits.len() as u64, self.len());
                assert_eq!(node.log_seq(), self.commit, "the map is the committed log");
                assert!(self.queued.is_empty(), "a primary serves its queue");
                let mut last = node.log_seq();
                for &(s, _) in &self.commits {
                    assert_eq!(s, last + 1);
                    last = s;
                }
            }
            Status::Gathering => {
                assert_eq!(self.primary(), self.me);
                assert!(self.node.is_none() && self.commits.is_empty() && self.reads.is_empty());
                assert!(self.offered);
            }
            Status::Backup => {
                assert_ne!(self.primary(), self.me);
                assert!(self.promised >= self.view, "a backup has promised its view");
                assert!(self.node.is_none() && self.commits.is_empty() && self.reads.is_empty());
                assert!(self.queued.is_empty(), "a backup forwards its queue");
            }
            Status::ViewChange => {
                assert!(self.node.is_none() && self.commits.is_empty() && self.reads.is_empty());
            }
            Status::Reading | Status::Joining | Status::Erasing => {
                assert!(self.node.is_none() && self.commits.is_empty());
                assert!(self.retained.is_empty());
            }
            Status::Repairing => {
                assert!(self.node.is_none() && self.commits.is_empty());
                assert!(
                    self.retained.keys().all(|&s| s > self.len() + 1),
                    "a retained record at the log's end is written or dropped, not retained"
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
