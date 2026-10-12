//! The linearizability check.
//!
//! Herlihy and Wing: a history is linearizable if every operation can be
//! given one instant between its call and its return at which it took
//! effect, such that the model, applied in the order of those instants,
//! gives every operation the response the store gave. The search below
//! looks for such an order. It is Wing and Gong's algorithm with the one
//! change that makes it usable, Lowe's cache of configurations that have
//! already failed.
//!
//! ```text
//!   linearized: {}          eligible: every op that no un-linearized op precedes
//!        │
//!        ├─ try op a: model.apply(a) == a.response?  yes ─► linearized: {a} ─► ...
//!        │                                           no  ─► next candidate
//!        ├─ try op b ...
//!        │
//!        └─ nothing fits ─► remember (linearized, state), back up
//! ```
//!
//! A pending operation, one the client gave up on, has no return. It
//! never precedes anything, so it never blocks another operation, and the
//! search may place it anywhere after its call or leave it out. That is
//! Lowe's Definition 1: a history is linearizable if it can be extended by
//! appending returns for some of the pending operations and then
//! linearized. Knossos calls these `:info` operations and treats them the
//! same way. Porcupine does not treat them at all, and its users give a
//! pending operation a return at the end of time, which forces it in.
//!
//! The problem is NP-complete, so there is a budget. A search that runs
//! out returns [`Verdict::Unknown`], never a verdict it did not earn.

use crate::history::History;
use crate::model::Model;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use core::fmt;

/// A set of operation indices, small enough to compare and store by the
/// thousand.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(len: usize) -> Bits {
        Bits(alloc::vec![0; len.div_ceil(64)])
    }

    fn get(&self, i: usize) -> bool {
        self.0[i / 64] & (1 << (i % 64)) != 0
    }

    fn set(&mut self, i: usize, on: bool) {
        if on {
            self.0[i / 64] |= 1 << (i % 64);
        } else {
            self.0[i / 64] &= !(1 << (i % 64));
        }
    }
}

/// What the search did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub ops: usize,
    pub pending: usize,
    /// Candidate operations tried, across every branch.
    pub steps: u64,
    /// Configurations remembered as failed.
    pub cached: u64,
    /// Operations in the longest order found.
    pub depth: usize,
}

/// Why no order exists, as far as the search got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Illegal<Resp> {
    /// The longest sequence of operations the model agreed with, by index.
    pub linearized: Vec<usize>,
    /// At the end of it, the operations that could legally come next and
    /// what the model would have answered each. None of them matched.
    pub stuck: Vec<(usize, Resp)>,
    pub stats: Stats,
}

impl<Resp: fmt::Debug> fmt::Display for Illegal<Resp> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "not linearizable: after {} operations {:?}, none of these fit:",
            self.linearized.len(),
            self.linearized
        )?;
        for (index, expected) in &self.stuck {
            write!(f, " op {index} (model says {expected:?})")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict<Resp> {
    /// An order exists. The indices of the completed and included pending
    /// operations, in that order.
    Linearizable { order: Vec<usize>, stats: Stats },
    /// No order exists.
    Illegal(Illegal<Resp>),
    /// The budget ran out first.
    Unknown(Stats),
}

impl<Resp> Verdict<Resp> {
    pub fn is_linearizable(&self) -> bool {
        matches!(self, Verdict::Linearizable { .. })
    }

    pub fn stats(&self) -> Stats {
        match self {
            Verdict::Linearizable { stats, .. } | Verdict::Unknown(stats) => *stats,
            Verdict::Illegal(illegal) => illegal.stats,
        }
    }
}

struct Search<'a, M: Model> {
    history: &'a History<M::Request, M::Response>,
    linearized: Bits,
    /// Completed operations not yet placed. Zero means done.
    remaining: usize,
    seen: BTreeSet<(Bits, M)>,
    path: Vec<usize>,
    best: Illegal<M::Response>,
    stats: Stats,
    limit: u64,
    exhausted: bool,
}

impl<M: Model> Search<'_, M> {
    /// Operations that may take effect next: not yet placed, and not
    /// preceded by any operation that is not yet placed.
    fn eligible(&self) -> Vec<usize> {
        let n = self.history.len();
        let unplaced = || (0..n).filter(|&i| !self.linearized.get(i));
        let min_ret = self.history.min_ret_seq(unplaced());
        unplaced()
            .filter(|&i| min_ret.is_none_or(|r| self.history.called_before(i, r)))
            .collect()
    }

    fn dfs(&mut self, model: &M) -> bool {
        if self.remaining == 0 {
            return true;
        }
        let candidates = self.eligible();
        let mut stuck = Vec::new();
        for i in candidates {
            if self.stats.steps >= self.limit {
                self.exhausted = true;
                return false;
            }
            self.stats.steps += 1;
            let op = &self.history.ops()[i];
            let mut next = model.clone();
            let expected = next.apply(&op.request);
            if let Some(got) = &op.response {
                if *got != expected {
                    stuck.push((i, expected));
                    continue;
                }
            }
            self.linearized.set(i, true);
            if op.is_complete() {
                self.remaining -= 1;
            }
            self.path.push(i);
            let key = (self.linearized.clone(), next.clone());
            if !self.seen.contains(&key) {
                self.seen.insert(key);
                self.stats.cached += 1;
                if self.dfs(&next) {
                    return true;
                }
                if self.exhausted {
                    return false;
                }
            }
            self.path.pop();
            if op.is_complete() {
                self.remaining += 1;
            }
            self.linearized.set(i, false);
        }
        if self.path.len() >= self.best.linearized.len() {
            self.best.linearized.clone_from(&self.path);
            self.best.stuck = stuck;
        }
        false
    }
}

/// Search for an order of `history` that `M` agrees with, trying at most
/// `limit` candidates.
pub fn check<M: Model>(history: &History<M::Request, M::Response>, limit: u64) -> Verdict<M::Response> {
    let ops = history.len();
    let mut search = Search::<M> {
        history,
        linearized: Bits::new(ops),
        remaining: history.completed(),
        seen: BTreeSet::new(),
        path: Vec::new(),
        best: Illegal {
            linearized: Vec::new(),
            stuck: Vec::new(),
            stats: Stats::default(),
        },
        stats: Stats {
            ops,
            pending: history.pending(),
            ..Stats::default()
        },
        limit,
        exhausted: false,
    };
    let model = M::init();
    let found = search.dfs(&model);
    search.stats.depth = search.best.linearized.len().max(search.path.len());
    if found {
        Verdict::Linearizable {
            order: search.path,
            stats: search.stats,
        }
    } else if search.exhausted {
        Verdict::Unknown(search.stats)
    } else {
        search.best.stats = search.stats;
        Verdict::Illegal(search.best)
    }
}
