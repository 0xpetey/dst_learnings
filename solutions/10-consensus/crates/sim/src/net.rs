//! The simulated network.
//!
//! ```text
//!   send(now, from, to, prng)                        deliver(id)
//!   ──────────────────────►  ┌────────────────┐  ──────────────────►
//!                            │ link state     │   ledger check
//!     Err(Dropped)  ◄──────  │ fault dice     │   cut check
//!     Ok([Delivery])  ─────► │ in-flight      │   Ok((from, to))
//!                            └────────────────┘   Err(Partition)
//! ```
//!
//! The network does not own a queue. The scheduler is the only queue in the
//! simulator. [`Net::send`] *decides* the fate of a message: dropped, or
//! delivered at one or more instants. The caller schedules one event per
//! instant and, when the event fires, calls [`Net::deliver`]. That call
//! checks the message against a ledger of everything that was sent and
//! against the link state at the moment of delivery.
//!
//! The ledger is what makes the first assertion of the module hold: the
//! network never delivers a message that nobody sent. A forged id, or an id
//! delivered one time too many, panics.
//!
//! The faults are the usual suspects, each a probability per message:
//! latency drawn uniformly from a range, loss, duplication, and reordering.
//! Reordering is implemented the way `tc netem` does it, as a delay applied
//! to some messages and not others. Partitions are a set of cut directed
//! links, which covers symmetric, asymmetric and partial partitions with one
//! mechanism. [`Shape`] names the shapes that Jepsen found worth testing.
//!
//! Every random choice comes from the `Prng` the caller passes in. The model
//! holds no randomness of its own, so the same seed drives the same network.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::fmt;
pub use determinism::Chance;
use determinism::Prng;
use kv_core::{Duration, Instant};

/// The fault model. The defaults are a perfect network with 1 ms latency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetConfig {
    /// One-way latency is drawn uniformly from `latency_min..=latency_max`.
    pub latency_min: Duration,
    pub latency_max: Duration,
    /// Per message: silently lost on the way.
    pub loss: Chance,
    /// Per message: a second copy arrives, with its own latency draw.
    pub duplicate: Chance,
    /// Per message: held back for an extra `0..=reorder_delay_max`, so that
    /// later messages on the same link overtake it.
    pub reorder: Chance,
    pub reorder_delay_max: Duration,
}

impl NetConfig {
    /// No faults, constant latency. Delivery on every link is first in,
    /// first out, because every message takes exactly as long as the last.
    pub const fn reliable(latency: Duration) -> NetConfig {
        NetConfig {
            latency_min: latency,
            latency_max: latency,
            loss: Chance::NEVER,
            duplicate: Chance::NEVER,
            reorder: Chance::NEVER,
            reorder_delay_max: Duration::ZERO,
        }
    }

    /// A fault configuration drawn from the seed. The draws happen in a
    /// fixed order, so a seed means the same faults on every run.
    ///
    /// The ranges are wide on purpose. Module 7 turns this into swarm
    /// testing, where each seed also chooses *which* faults to enable.
    pub fn random(prng: &mut Prng) -> NetConfig {
        let latency_min = Duration::from_millis(prng.range(0, 5));
        let latency_max = latency_min + Duration::from_millis(prng.range(0, 50));
        let loss = Chance::percent(prng.below(20) as u32);
        let duplicate = Chance::percent(prng.below(10) as u32);
        let reorder = Chance::percent(prng.below(20) as u32);
        let reorder_delay_max = Duration::from_millis(prng.range(10, 200));
        NetConfig {
            latency_min,
            latency_max,
            loss,
            duplicate,
            reorder,
            reorder_delay_max,
        }
    }

    pub fn is_reliable(&self) -> bool {
        self.latency_min == self.latency_max
            && self.loss.is_never()
            && self.duplicate.is_never()
            && self.reorder.is_never()
    }

    /// Panics on a configuration that cannot mean anything.
    pub fn validate(&self) {
        assert!(
            self.latency_min <= self.latency_max,
            "latency_min {:?} above latency_max {:?}",
            self.latency_min,
            self.latency_max
        );
        assert!(
            self.reorder.is_never() || !self.reorder_delay_max.is_zero(),
            "reordering with a zero delay reorders nothing"
        );
    }
}

impl Default for NetConfig {
    fn default() -> NetConfig {
        NetConfig::reliable(Duration::from_millis(1))
    }
}

impl fmt::Display for NetConfig {
    /// One line, for the configuration header of a run.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_reliable() {
            return write!(f, "reliable latency={:?}", self.latency_min);
        }
        write!(
            f,
            "latency={:?}..{:?} loss={} dup={} reorder={}+{:?}",
            self.latency_min,
            self.latency_max,
            self.loss,
            self.duplicate,
            self.reorder,
            self.reorder_delay_max
        )
    }
}

/// Identifies one sent message. Every copy of a duplicated message shares
/// the id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MessageId(u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m{}", self.0)
    }
}

/// One planned arrival. `copy` is 0 for the original and counts up for
/// duplicates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub id: MessageId,
    pub at: Instant,
    pub copy: u32,
}

/// Why a message did not arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dropped {
    /// The link was cut, at send time or at delivery time.
    Partition,
    /// The dice said so, or a test asked for it with [`Net::drop_next`].
    Loss,
}

impl fmt::Display for Dropped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Dropped::Partition => write!(f, "partition"),
            Dropped::Loss => write!(f, "loss"),
        }
    }
}

/// Counters, for tests and the end-of-run summary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetStats {
    /// Calls to `send`, whatever happened next.
    pub sent: u64,
    /// Copies that reached their destination.
    pub delivered: u64,
    /// Extra copies scheduled by the duplication fault.
    pub duplicated: u64,
    /// Copies given an extra delay by the reordering fault.
    pub reordered: u64,
    /// Dropped by loss, at send time.
    pub lost: u64,
    /// Dropped by a cut link, at send time or at delivery time.
    pub partitioned: u64,
}

/// A partition shape from the Jepsen catalogue. Apply one with
/// [`Net::apply`]. Every shape is a set of cut links, so shapes compose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape<A> {
    /// One node loses contact with everyone, both ways.
    Isolate { node: A, rest: Vec<A> },
    /// Two sides that cannot reach each other.
    Halves { a: Vec<A>, b: Vec<A> },
    /// Two sides that cannot reach each other, plus one node that both
    /// sides can still reach.
    Bridge { bridge: A, a: Vec<A>, b: Vec<A> },
    /// Nothing from `from` reaches `to`. The other direction works. This is
    /// the asymmetric partition: a node that hears everyone and is heard by
    /// nobody.
    OneWay { from: Vec<A>, to: Vec<A> },
    /// Each node reaches only its two neighbours in the ring. With four or
    /// more nodes every node sees a majority, and no two see the same one.
    Ring { order: Vec<A> },
}

impl<A: Copy + Ord> Shape<A> {
    /// A random shape over `nodes`. Needs at least two nodes. `Bridge`
    /// needs three and `Ring` needs four, so smaller clusters never draw
    /// them.
    pub fn random(prng: &mut Prng, nodes: &[A]) -> Shape<A> {
        assert!(nodes.len() >= 2, "a partition needs at least two nodes");
        let mut order: Vec<A> = nodes.to_vec();
        prng.shuffle(&mut order);
        let n = order.len();
        let kinds = match n {
            2 => 3,
            3 => 4,
            _ => 5,
        };
        match prng.below(kinds) {
            0 => Shape::Isolate {
                node: order[0],
                rest: order[1..].to_vec(),
            },
            1 => {
                let k = prng.range(1, n as u64) as usize;
                Shape::Halves {
                    a: order[..k].to_vec(),
                    b: order[k..].to_vec(),
                }
            }
            2 => {
                let k = prng.range(1, n as u64) as usize;
                Shape::OneWay {
                    from: order[..k].to_vec(),
                    to: order[k..].to_vec(),
                }
            }
            3 => {
                let k = prng.range(2, n as u64) as usize;
                Shape::Bridge {
                    bridge: order[0],
                    a: order[1..k].to_vec(),
                    b: order[k..].to_vec(),
                }
            }
            _ => Shape::Ring { order },
        }
    }
}

impl<A: fmt::Display> fmt::Display for Shape<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn list<A: fmt::Display>(f: &mut fmt::Formatter<'_>, items: &[A]) -> fmt::Result {
            write!(f, "[")?;
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    write!(f, " ")?;
                }
                write!(f, "{item}")?;
            }
            write!(f, "]")
        }
        match self {
            Shape::Isolate { node, .. } => write!(f, "isolate {node}"),
            Shape::Halves { a, b } => {
                write!(f, "halves ")?;
                list(f, a)?;
                write!(f, " | ")?;
                list(f, b)
            }
            Shape::Bridge { bridge, a, b } => {
                write!(f, "bridge {bridge} ")?;
                list(f, a)?;
                write!(f, " | ")?;
                list(f, b)
            }
            Shape::OneWay { from, to } => {
                write!(f, "one-way ")?;
                list(f, from)?;
                write!(f, " -x-> ")?;
                list(f, to)
            }
            Shape::Ring { order } => {
                write!(f, "ring ")?;
                list(f, order)
            }
        }
    }
}

struct InFlight<A> {
    from: A,
    to: A,
    copies: u32,
}

/// The network model. `A` is the address type. The simulator uses one
/// address space for nodes and clients.
pub struct Net<A> {
    config: NetConfig,
    /// Directed links that drop everything.
    cut: BTreeSet<(A, A)>,
    /// Test knob: the next `n` messages on a link are lost.
    drop_next: BTreeMap<(A, A), u32>,
    /// Every message with a copy still on the wire.
    in_flight: BTreeMap<u64, InFlight<A>>,
    next_id: u64,
    stats: NetStats,
}

impl<A: Copy + Ord + fmt::Debug> Net<A> {
    pub fn new(config: NetConfig) -> Net<A> {
        config.validate();
        Net {
            config,
            cut: BTreeSet::new(),
            drop_next: BTreeMap::new(),
            in_flight: BTreeMap::new(),
            next_id: 0,
            stats: NetStats::default(),
        }
    }

    pub fn config(&self) -> &NetConfig {
        &self.config
    }

    /// Change the faults mid-run. Module 7 uses this to turn them off for
    /// the quiet period in which the cluster must recover.
    pub fn set_config(&mut self, config: NetConfig) {
        config.validate();
        self.config = config;
    }

    pub fn stats(&self) -> NetStats {
        self.stats
    }

    /// Messages with at least one copy not yet delivered or dropped.
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    // ---- link state -------------------------------------------------------

    pub fn is_connected(&self, from: A, to: A) -> bool {
        !self.cut.contains(&(from, to))
    }

    /// Cut one direction. Messages from `from` to `to` are dropped. The
    /// other direction is untouched.
    pub fn cut(&mut self, from: A, to: A) {
        assert!(from != to, "cannot cut a node off from itself");
        self.cut.insert((from, to));
    }

    /// Undo [`Net::cut`].
    pub fn restore(&mut self, from: A, to: A) {
        self.cut.remove(&(from, to));
    }

    /// Cut both directions between `a` and `b`.
    pub fn partition(&mut self, a: A, b: A) {
        self.cut(a, b);
        self.cut(b, a);
    }

    /// Undo [`Net::partition`].
    pub fn heal(&mut self, a: A, b: A) {
        self.restore(a, b);
        self.restore(b, a);
    }

    /// Cut every link between the two sides, both ways.
    pub fn split(&mut self, side_a: &[A], side_b: &[A]) {
        for &a in side_a {
            for &b in side_b {
                self.partition(a, b);
            }
        }
    }

    /// Cut `node` off from every address in `rest`, both ways.
    pub fn isolate(&mut self, node: A, rest: &[A]) {
        self.split(&[node], rest);
    }

    /// Reconnect everything.
    pub fn heal_all(&mut self) {
        self.cut.clear();
    }

    /// The cut links, in address order.
    pub fn cuts(&self) -> impl Iterator<Item = (A, A)> + '_ {
        self.cut.iter().copied()
    }

    /// Add the cuts that `shape` describes.
    pub fn apply(&mut self, shape: &Shape<A>) {
        match shape {
            Shape::Isolate { node, rest } => self.isolate(*node, rest),
            Shape::Halves { a, b } => self.split(a, b),
            Shape::Bridge { a, b, .. } => self.split(a, b),
            Shape::OneWay { from, to } => {
                for &f in from {
                    for &t in to {
                        self.cut(f, t);
                    }
                }
            }
            Shape::Ring { order } => {
                let n = order.len();
                for i in 0..n {
                    for j in 0..n {
                        let distance = (j + n - i) % n;
                        let neighbour = distance == 0 || distance == 1 || distance == n - 1;
                        if !neighbour {
                            self.cut(order[i], order[j]);
                        }
                    }
                }
            }
        }
    }

    /// Lose the next message sent from `from` to `to`. Calling it twice
    /// loses the next two. For tests that want one specific loss.
    pub fn drop_next(&mut self, from: A, to: A) {
        *self.drop_next.entry((from, to)).or_insert(0) += 1;
    }

    // ---- the decision -----------------------------------------------------

    /// Decide what happens to a message sent now. `Ok` holds one `Delivery`
    /// per copy that will arrive. Schedule each one, and call
    /// [`Net::deliver`] when it fires.
    pub fn send(&mut self, now: Instant, from: A, to: A, prng: &mut Prng) -> Result<Vec<Delivery>, Dropped> {
        assert!(from != to, "{from:?} used the network to talk to itself");
        self.stats.sent += 1;
        if !self.is_connected(from, to) {
            self.stats.partitioned += 1;
            return Err(Dropped::Partition);
        }
        if let Some(n) = self.drop_next.get_mut(&(from, to)) {
            *n -= 1;
            if *n == 0 {
                self.drop_next.remove(&(from, to));
            }
            self.stats.lost += 1;
            return Err(Dropped::Loss);
        }
        if self.config.loss.roll(prng) {
            self.stats.lost += 1;
            return Err(Dropped::Loss);
        }
        let id = MessageId(self.next_id);
        self.next_id += 1;
        let mut deliveries = Vec::with_capacity(2);
        deliveries.push(Delivery {
            id,
            at: now + self.delay(prng),
            copy: 0,
        });
        if self.config.duplicate.roll(prng) {
            self.stats.duplicated += 1;
            deliveries.push(Delivery {
                id,
                at: now + self.delay(prng),
                copy: 1,
            });
        }
        self.in_flight.insert(
            id.0,
            InFlight {
                from,
                to,
                copies: deliveries.len() as u32,
            },
        );
        Ok(deliveries)
    }

    /// A copy of `id` has reached its destination. Returns the endpoints,
    /// or `Err(Partition)` if the link was cut while the copy was in
    /// flight. Panics if `id` was never sent or has no copy in flight: the
    /// network never delivers what nobody sent.
    pub fn deliver(&mut self, id: MessageId) -> Result<(A, A), Dropped> {
        let entry = self
            .in_flight
            .get_mut(&id.0)
            .unwrap_or_else(|| panic!("network delivered {id}, which was never sent or already arrived"));
        let (from, to) = (entry.from, entry.to);
        entry.copies -= 1;
        if entry.copies == 0 {
            self.in_flight.remove(&id.0);
        }
        if !self.is_connected(from, to) {
            self.stats.partitioned += 1;
            return Err(Dropped::Partition);
        }
        self.stats.delivered += 1;
        Ok((from, to))
    }

    /// One latency draw, plus the reordering delay when the dice say so.
    fn delay(&mut self, prng: &mut Prng) -> Duration {
        let mut d = uniform(prng, self.config.latency_min, self.config.latency_max);
        if self.config.reorder.roll(prng) {
            self.stats.reordered += 1;
            d += uniform(prng, Duration::ZERO, self.config.reorder_delay_max);
        }
        d
    }
}

/// Uniform in `lo..=hi`. Spends no randomness when the range is a point.
fn uniform(prng: &mut Prng, lo: Duration, hi: Duration) -> Duration {
    debug_assert!(lo <= hi);
    if lo == hi {
        return lo;
    }
    let span = u64::try_from((hi - lo).as_nanos()).expect("latency span fits u64");
    lo + Duration::from_nanos(prng.below(span + 1))
}
