//! Heartbeats and a failure detector, as a state machine.
//!
//! ```text
//!   inputs                              outputs
//!   Message { from, Beat { seq } } ──► ┌──────────┐ ──► Send { to, Beat { seq } }
//!   Tick                           ──► │ Heartbeat│ ──► PeerUp(id)
//!                                      └──────────┘ ──► PeerDown(id)
//! ```
//!
//! Every `interval`, a node sends `Beat { seq }` to every peer. A peer that
//! has not been heard from for `suspect_after` is marked `Down`. The first
//! beat heard from it marks it `Up` again. This is the simplest failure
//! detector there is, and it is wrong in exactly the ways the lesson wants
//! to show: a partition looks like a crash, a slow link looks like a crash,
//! and an asymmetric cut makes two nodes disagree about who is alive.
//!
//! Like the client, the heartbeat never reads a clock. Every input carries
//! `now`, because a received beat is itself evidence about time. It says
//! when it next needs waking through [`Heartbeat::next_deadline`].
//!
//! Module 9 uses the liveness view to choose a primary. Until then the view
//! is only observed by tests.
//!
//! One fault point: `Site::SkipBeat` makes a tick send nothing, as a
//! process does when it stalls across a round. The sequence numbers stay
//! dense, because a beat that was never sent has no number.

use crate::buggify::{Buggify, Site};
use crate::time::{Duration, Instant};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// A node's position in the cluster. Small on purpose: a cluster has a
/// handful of nodes, and a `u8` makes "every peer" a cheap loop.
pub type NodeId = u8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Time between beats.
    pub interval: Duration,
    /// Silence after which a peer is `Down`. Must exceed `interval`, or a
    /// perfect network produces false suspicions.
    pub suspect_after: Duration,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            interval: Duration::from_millis(100),
            suspect_after: Duration::from_millis(350),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    /// "I am alive." `seq` increases by one per beat, so a receiver can
    /// tell a duplicate from a straggler.
    Beat { seq: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Message {
        from: NodeId,
        message: Message,
    },
    /// The deadline from `next_deadline` arrived.
    Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Send { to: NodeId, message: Message },
    PeerUp(NodeId),
    PeerDown(NodeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Liveness {
    /// Never heard from, and not yet silent for long enough to be `Down`.
    Unknown,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug)]
struct Peer {
    liveness: Liveness,
    /// The last beat's arrival, or `None` before the first.
    last_heard: Option<Instant>,
    /// Highest `seq` seen. Beats can arrive twice or out of order.
    max_seq: u64,
    received: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Beats sent, counting one per peer.
    pub sent: u64,
    pub received: u64,
    /// Received beats whose `seq` was already seen.
    pub duplicates: u64,
    /// Received beats whose `seq` was below one already seen.
    pub reordered: u64,
    /// Transitions to `Up`.
    pub ups: u64,
    /// Transitions to `Down`.
    pub downs: u64,
}

#[derive(Clone, Debug)]
pub struct Heartbeat {
    me: NodeId,
    config: Config,
    peers: BTreeMap<NodeId, Peer>,
    /// `seq` of the last beat sent.
    seq: u64,
    next_beat: Instant,
    /// When this node started. Silence is measured from here until the
    /// first beat from a peer.
    started: Instant,
    /// Inputs must not go back in time.
    last_now: Instant,
    stats: Stats,
    buggify: Buggify,
}

impl Heartbeat {
    /// Create the detector and send the first round of beats into `out`.
    /// No fault point is enabled.
    pub fn new(
        me: NodeId,
        peers: &[NodeId],
        config: Config,
        now: Instant,
        out: &mut Vec<Output>,
    ) -> Heartbeat {
        Heartbeat::with_buggify(Buggify::off(), me, peers, config, now, out)
    }

    /// Create the detector with fault points. The simulator's constructor.
    pub fn with_buggify(
        buggify: Buggify,
        me: NodeId,
        peers: &[NodeId],
        config: Config,
        now: Instant,
        out: &mut Vec<Output>,
    ) -> Heartbeat {
        assert!(
            config.interval > Duration::ZERO,
            "heartbeat interval must be positive"
        );
        assert!(
            config.suspect_after > config.interval,
            "suspect_after {:?} must exceed interval {:?}",
            config.suspect_after,
            config.interval
        );
        let mut map = BTreeMap::new();
        for &peer in peers {
            assert!(peer != me, "node {me} listed itself as a peer");
            let prev = map.insert(
                peer,
                Peer {
                    liveness: Liveness::Unknown,
                    last_heard: None,
                    max_seq: 0,
                    received: 0,
                },
            );
            assert!(prev.is_none(), "node {me} listed peer {peer} twice");
        }
        let mut hb = Heartbeat {
            me,
            config,
            peers: map,
            seq: 0,
            next_beat: now,
            started: now,
            last_now: now,
            stats: Stats::default(),
            buggify,
        };
        hb.on_tick(now, out);
        hb.check_invariants();
        hb
    }

    pub fn id(&self) -> NodeId {
        self.me
    }

    pub fn config(&self) -> Config {
        self.config
    }

    pub fn peers(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.peers.keys().copied()
    }

    /// What this node believes about `peer`. Panics for a non-peer.
    pub fn liveness(&self, peer: NodeId) -> Liveness {
        self.peers
            .get(&peer)
            .unwrap_or_else(|| panic!("node {} has no peer {peer}", self.me))
            .liveness
    }

    /// When the last beat from `peer` arrived, if one ever did.
    pub fn last_heard(&self, peer: NodeId) -> Option<Instant> {
        self.peers.get(&peer).and_then(|p| p.last_heard)
    }

    /// Peers currently believed `Up`.
    pub fn up_peers(&self) -> usize {
        self.peers.values().filter(|p| p.liveness == Liveness::Up).count()
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The fault points and their counts.
    pub fn buggify(&self) -> &Buggify {
        &self.buggify
    }

    pub fn buggify_mut(&mut self) -> &mut Buggify {
        &mut self.buggify
    }

    /// When this node next needs a `Tick`: the next beat, or the moment a
    /// peer becomes suspect, whichever is first. `None` with no peers.
    pub fn next_deadline(&self) -> Option<Instant> {
        if self.peers.is_empty() {
            return None;
        }
        let mut deadline = self.next_beat;
        for peer in self.peers.values() {
            if let Some(at) = self.suspect_at(peer) {
                deadline = deadline.min(at);
            }
        }
        Some(deadline)
    }

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
            Input::Message { from, message } => match message {
                Message::Beat { seq } => self.on_beat(now, from, seq, out),
            },
        }
        self.check_invariants();
    }

    /// The instant at which `peer` becomes `Down` if it stays silent.
    /// `None` once it is `Down`.
    fn suspect_at(&self, peer: &Peer) -> Option<Instant> {
        match peer.liveness {
            Liveness::Down => None,
            Liveness::Up | Liveness::Unknown => {
                Some(peer.last_heard.unwrap_or(self.started) + self.config.suspect_after)
            }
        }
    }

    fn on_tick(&mut self, now: Instant, out: &mut Vec<Output>) {
        if now >= self.next_beat {
            // The process stalled across this round and sent nothing. The
            // round is gone; the next one is numbered as if it never was.
            if !crate::buggify!(self.buggify, Site::SkipBeat) {
                self.seq += 1;
                for &to in self.peers.keys() {
                    out.push(Output::Send {
                        to,
                        message: Message::Beat { seq: self.seq },
                    });
                }
                self.stats.sent += self.peers.len() as u64;
            }
            // A tick that arrives late (a paused node, or one whose clock
            // jumped) sends one beat, not one per missed interval.
            while self.next_beat <= now {
                self.next_beat = self.next_beat + self.config.interval;
            }
        }
        let suspect_after = self.config.suspect_after;
        let started = self.started;
        for (&id, peer) in &mut self.peers {
            if peer.liveness == Liveness::Down {
                continue;
            }
            let silent_since = peer.last_heard.unwrap_or(started);
            if silent_since + suspect_after <= now {
                peer.liveness = Liveness::Down;
                self.stats.downs += 1;
                out.push(Output::PeerDown(id));
            }
        }
    }

    fn on_beat(&mut self, now: Instant, from: NodeId, seq: u64, out: &mut Vec<Output>) {
        assert!(from != self.me, "node {from} received its own beat");
        let me = self.me;
        let peer = self
            .peers
            .get_mut(&from)
            .unwrap_or_else(|| panic!("node {me} heard a beat from unknown node {from}"));
        self.stats.received += 1;
        peer.received += 1;
        if peer.received == 1 || seq > peer.max_seq {
            peer.max_seq = seq;
        } else if seq == peer.max_seq {
            self.stats.duplicates += 1;
        } else {
            self.stats.reordered += 1;
        }
        // Any beat, even a straggler, is evidence that the peer was alive
        // recently. The simplest rule: it resets the silence.
        peer.last_heard = Some(now);
        if peer.liveness != Liveness::Up {
            peer.liveness = Liveness::Up;
            self.stats.ups += 1;
            out.push(Output::PeerUp(from));
        }
    }

    /// Everything that must hold between inputs.
    fn check_invariants(&self) {
        assert!(self.next_beat > self.last_now, "next beat is not in the future");
        assert_eq!(
            self.stats.sent,
            self.seq * self.peers.len() as u64,
            "every beat went to every peer"
        );
        let mut received = 0;
        for (&id, peer) in &self.peers {
            assert!(id != self.me);
            match peer.liveness {
                Liveness::Unknown => assert!(peer.last_heard.is_none(), "unknown peer {id} was heard"),
                Liveness::Up => assert!(peer.last_heard.is_some(), "up peer {id} was never heard"),
                Liveness::Down => {}
            }
            if let Some(at) = peer.last_heard {
                assert!(at <= self.last_now);
            }
            received += peer.received;
        }
        assert_eq!(received, self.stats.received);
    }
}
