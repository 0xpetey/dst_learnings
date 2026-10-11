//! The environment contract and the driver that connects a [`Node`] to it.
//!
//! Three traits, one per thing the outside world provides. Each has a
//! *real* implementation (system clock, TCP, files) and a *fake* one
//! (counter, `VecDeque`, `Vec<u8>`) in the `kv-env` crate. The driver here
//! is generic over them and contains the only event loop the server has.
//!
//! Why generics and not `dyn Trait`? Monomorphisation gives the fake
//! environment zero overhead, which matters when a simulator runs millions
//! of events; and a mismatched environment is a compile error rather than a
//! runtime one. The cost is one more type parameter, which the driver hides.
//!
//! The style is completion based, like io_uring: the driver *submits* an
//! append and later *polls* for its completion. Nothing blocks inside the
//! core, so the fake environment can complete operations whenever it likes,
//! including never, out of order (which the node asserts against), or after
//! a crash.

use crate::node::{Input, Node, OpId, Output};
use crate::protocol::ResponseEnvelope;
use crate::time::Instant;
use alloc::vec::Vec;

/// A monotonic clock.
pub trait Time {
    fn now(&self) -> Instant;
}

/// An append-only log with asynchronous completion.
pub trait Storage {
    /// Begin reading the entire log; completes with `Input::ReadComplete`.
    fn read_all(&mut self, op: OpId);
    /// Begin appending `bytes` at `offset`; completes with `Input::AppendComplete`.
    /// Implementations must complete appends in submission order.
    fn append(&mut self, op: OpId, offset: u64, bytes: &[u8]);
    /// Move any finished operations into `inputs`.
    fn poll_completions(&mut self, inputs: &mut Vec<Input>);
}

/// Messages to and from clients.
pub trait Network {
    /// Queue a reply for delivery.
    fn send(&mut self, reply: ResponseEnvelope);
    /// Move any received requests into `inputs`.
    fn poll_requests(&mut self, inputs: &mut Vec<Input>);
}

/// Anything that wants to observe every input and output. The real server
/// logs; the simulator records a `Trace`.
pub trait Observer {
    fn input(&mut self, now: Instant, input: &Input);
    fn output(&mut self, now: Instant, output: &Output);
}

/// An observer that does nothing.
pub struct Silent;

impl Observer for Silent {
    fn input(&mut self, _: Instant, _: &Input) {}
    fn output(&mut self, _: Instant, _: &Output) {}
}

/// Owns a node and its environment and moves events between them.
pub struct Driver<T, S, N, O = Silent> {
    pub node: Node,
    pub time: T,
    pub storage: S,
    pub network: N,
    pub observer: O,
    inputs: Vec<Input>,
    outputs: Vec<Output>,
    steps: u64,
    halted: bool,
}

impl<T: Time, S: Storage, N: Network, O: Observer> Driver<T, S, N, O> {
    /// Create the node and submit its initial log read.
    pub fn new(time: T, storage: S, network: N, observer: O) -> Self {
        let mut outputs = Vec::new();
        let node = Node::new(&mut outputs);
        let mut driver = Driver {
            node,
            time,
            storage,
            network,
            observer,
            inputs: Vec::new(),
            outputs,
            steps: 0,
            halted: false,
        };
        driver.dispatch_outputs();
        driver
    }

    /// One iteration: gather inputs, feed them to the node, dispatch outputs.
    /// Returns the number of inputs processed; zero means idle. A halted
    /// driver is always idle.
    pub fn step(&mut self) -> usize {
        if self.halted {
            return 0;
        }
        self.steps += 1;
        debug_assert!(self.inputs.is_empty());
        self.storage.poll_completions(&mut self.inputs);
        self.network.poll_requests(&mut self.inputs);
        let count = self.inputs.len();
        let inputs = core::mem::take(&mut self.inputs);
        for input in inputs {
            self.observer.input(self.time.now(), &input);
            self.node.on_input(input, &mut self.outputs);
            self.dispatch_outputs();
        }
        count
    }

    /// Step until an iteration processes nothing.
    pub fn run_until_idle(&mut self) {
        while self.step() > 0 {}
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// The node asked to be stopped. See [`Output::Halt`].
    pub fn is_halted(&self) -> bool {
        self.halted
    }

    fn dispatch_outputs(&mut self) {
        let outputs = core::mem::take(&mut self.outputs);
        for output in outputs {
            self.observer.output(self.time.now(), &output);
            match output {
                Output::Read { op } => self.storage.read_all(op),
                Output::Append { op, offset, bytes } => self.storage.append(op, offset, &bytes),
                Output::Reply(reply) => self.network.send(reply),
                Output::Halt { .. } => self.halted = true,
            }
        }
    }
}
