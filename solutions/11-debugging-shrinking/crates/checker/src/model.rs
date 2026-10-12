//! The reference model: a sequential specification of the store.
//!
//! A model is a state and a function from a request to a response that
//! changes the state. It is the simplest correct implementation there is,
//! and it is the oracle for everything else. If a history has no overlap,
//! the check is a loop: apply each request to the model and compare the
//! model's response with the store's. That is what the Module 2 to 7
//! tests did inline, and it is a complete check for a sequential history.
//!
//! With overlap, the order in which the model should see the operations
//! is not given, and the check becomes a search. See [`crate::linearize`].

use crate::history::History;
use core::fmt;

/// A sequential specification. `Ord` and `Clone` are what the search
/// needs: it copies the state at every branch and remembers the states it
/// has already failed from.
pub trait Model: Clone + Ord {
    type Request;
    type Response: PartialEq + Clone + fmt::Debug;

    /// The initial state.
    fn init() -> Self;

    /// Apply one request and return the response the specification gives.
    fn apply(&mut self, request: &Self::Request) -> Self::Response;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SequentialError<Resp> {
    /// The history has overlapping operations. The order the model should
    /// see is not given, so this check does not apply.
    NotSequential { first: usize, second: usize },
    /// The store answered an operation differently from the model.
    Mismatch { index: usize, expected: Resp, got: Resp },
}

impl<Resp: fmt::Debug> fmt::Display for SequentialError<Resp> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SequentialError::NotSequential { first, second } => {
                write!(f, "operations {first} and {second} overlap")
            }
            SequentialError::Mismatch { index, expected, got } => {
                write!(f, "operation {index}: expected {expected:?}, got {got:?}")
            }
        }
    }
}

/// Check a history with no overlap against `M`. Every completed operation
/// must get the model's answer. A pending operation can only be the last
/// one, and it is applied without a comparison, because it may well have
/// happened.
pub fn check_sequential<M: Model>(
    history: &History<M::Request, M::Response>,
) -> Result<M, SequentialError<M::Response>> {
    if let Some((first, second)) = history.first_overlap() {
        return Err(SequentialError::NotSequential { first, second });
    }
    let mut model = M::init();
    for (index, op) in history.ops().iter().enumerate() {
        let expected = model.apply(&op.request);
        if let Some(got) = &op.response {
            if *got != expected {
                return Err(SequentialError::Mismatch {
                    index,
                    expected,
                    got: got.clone(),
                });
            }
        }
    }
    Ok(model)
}
