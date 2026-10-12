//! The oracles, bound to the KV store.
//!
//! [`KvModel`] is the specification: a map. Applying a request to it gives
//! the response the store owes. [`check`] is the linearizability check
//! for a KV history, made tractable by the fact that keys are independent.
//! A history of the store is linearizable if and only if the history of
//! each key is, so each key is searched on its own. Porcupine calls this
//! `Partition`. Horn and Kroening call it P-compositionality, and it is
//! what turns a hopeless search into a quick one.
//!
//! [`replay`] rebuilds the model from log records. It is the white-box
//! oracle: the node's map must equal the log replayed, at every moment the
//! node is idle.

use crate::history::History;
use crate::linearize::{self, Stats, Verdict};
use crate::model::Model;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use kv_core::log::{Entry, Record};
use kv_core::protocol::{Request, Response};

/// The specification of the store: a map, and nothing else.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct KvModel {
    map: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl KvModel {
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.map.get(key).map(Vec::as_slice)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.map.iter().map(|(k, v)| (k.as_slice(), v.as_slice()))
    }

    /// Apply a durable entry, as the node does on recovery.
    pub fn apply_entry(&mut self, entry: &Entry) -> Response {
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
}

impl Model for KvModel {
    type Request = Request;
    type Response = Response;

    fn init() -> KvModel {
        KvModel::default()
    }

    fn apply(&mut self, request: &Request) -> Response {
        if let Err(why) = request.validate() {
            return Response::Rejected(why);
        }
        match request {
            Request::Get { key } => Response::Value(self.map.get(key).cloned()),
            Request::Put { key, value } => self.apply_entry(&Entry::Put {
                key: key.clone(),
                value: value.clone(),
            }),
            Request::Delete { key } => self.apply_entry(&Entry::Delete { key: key.clone() }),
        }
    }
}

/// The state a log describes.
pub fn replay(records: &[Record]) -> KvModel {
    let mut model = KvModel::default();
    for record in records {
        model.apply_entry(&record.entry);
    }
    model
}

/// One history per key, in key order. A pending read is dropped: it
/// constrains nothing, since no answer was seen, and it would only widen
/// the search.
pub fn partition(history: &History<Request, Response>) -> Vec<(Vec<u8>, History<Request, Response>)> {
    let mut keys: Vec<Vec<u8>> = history.ops().iter().map(|op| op.request.key().to_vec()).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .map(|key| {
            let part = history.filter(|op| {
                op.request.key() == key.as_slice()
                    && !(op.is_pending() && matches!(op.request, Request::Get { .. }))
            });
            (key, part)
        })
        .collect()
}

/// One key's history and what the search made of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub key: Vec<u8>,
    pub verdict: Verdict<Response>,
    /// The history the verdict's indices refer to.
    pub history: History<Request, Response>,
}

/// The verdict for a KV history: every key's history must be
/// linearizable. The first key that is not, or that ran out of budget,
/// decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub keys: usize,
    /// Summed over the keys.
    pub stats: Stats,
    /// The first key that was not linearizable.
    pub failed: Option<Failure>,
}

impl Report {
    pub fn is_linearizable(&self) -> bool {
        self.failed.is_none()
    }
}

/// Check a KV history, key by key, with `limit` candidates per key.
pub fn check(history: &History<Request, Response>, limit: u64) -> Report {
    let mut report = Report {
        keys: 0,
        stats: Stats::default(),
        failed: None,
    };
    for (key, part) in partition(history) {
        report.keys += 1;
        let verdict = linearize::check::<KvModel>(&part, limit);
        let s = verdict.stats();
        report.stats.ops += s.ops;
        report.stats.pending += s.pending;
        report.stats.steps += s.steps;
        report.stats.cached += s.cached;
        report.stats.depth = report.stats.depth.max(s.depth);
        if !verdict.is_linearizable() {
            report.failed = Some(Failure {
                key,
                verdict,
                history: part,
            });
            return report;
        }
    }
    report
}
