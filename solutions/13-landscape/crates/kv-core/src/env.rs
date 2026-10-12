//! The environment contract and the driver that connects a [`Replica`] to it.
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
//! including never, out of order (which the replica asserts against), or
//! after a crash.
//!
//! Module 9 made the network two-sided. It carries replies to clients, as
//! before, and messages to peers. The driver also feeds the replica a
//! `Tick` whenever its deadline has passed, so no environment needs a
//! timer of its own: the fake world wakes the driver at the right instant,
//! the real server polls.

use crate::buggify::Buggify;
use crate::heartbeat::{self, NodeId};
use crate::node::OpId;
use crate::protocol::ResponseEnvelope;
use crate::replica::{Input, Message, Output, Replica};
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
    /// Begin writing `bytes` at `offset` and making them durable; completes
    /// with `Input::AppendComplete`. Implementations must complete
    /// operations in submission order.
    fn append(&mut self, op: OpId, offset: u64, bytes: &[u8]);
    /// Move any finished operations into `inputs`.
    fn poll_completions(&mut self, inputs: &mut Vec<Input>);
}

/// Messages to and from clients and peers.
pub trait Network {
    /// Queue a reply for delivery to its client.
    fn send(&mut self, reply: ResponseEnvelope);
    /// Queue a message for delivery to a peer replica.
    fn send_peer(&mut self, to: NodeId, message: Message);
    /// Move any received requests and peer messages into `inputs`.
    fn poll(&mut self, inputs: &mut Vec<Input>);
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

/// Owns a replica and its environment and moves events between them.
pub struct Driver<T, S, N, O = Silent> {
    pub replica: Replica,
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
    /// Create the replica and submit its initial log read. No fault point
    /// is enabled: this is the production constructor. `boot` names this
    /// incarnation of the node; a restart must pass a larger one.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        me: NodeId,
        boot: u64,
        peers: &[NodeId],
        config: heartbeat::Config,
        time: T,
        storage: S,
        network: N,
        observer: O,
    ) -> Self {
        let off = || Buggify::off();
        Driver::with_buggify(
            [off(), off(), off()],
            me,
            boot,
            peers,
            config,
            time,
            storage,
            network,
            observer,
        )
    }

    /// Create the replica with fault points for the replica, its node and
    /// its heartbeat, for the simulator.
    #[allow(clippy::too_many_arguments)]
    pub fn with_buggify(
        [replica, node, heartbeat]: [Buggify; 3],
        me: NodeId,
        boot: u64,
        peers: &[NodeId],
        config: heartbeat::Config,
        time: T,
        storage: S,
        network: N,
        observer: O,
    ) -> Self {
        let mut outputs = Vec::new();
        let now = time.now();
        let replica = Replica::with_buggify(
            replica,
            node,
            heartbeat,
            me,
            boot,
            peers,
            config,
            now,
            &mut outputs,
        );
        let mut driver = Driver {
            replica,
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

    /// One iteration: a tick if the deadline passed, then every input the
    /// environment has, each fed to the replica with its outputs
    /// dispatched. Returns the number of inputs processed; zero means
    /// idle. A halted driver is always idle.
    pub fn step(&mut self) -> usize {
        if self.halted {
            return 0;
        }
        self.steps += 1;
        debug_assert!(self.inputs.is_empty());
        let now = self.time.now();
        if self.replica.next_deadline().is_some_and(|d| d <= now) {
            self.inputs.push(Input::Tick);
        }
        self.storage.poll_completions(&mut self.inputs);
        self.network.poll(&mut self.inputs);
        let count = self.inputs.len();
        let inputs = core::mem::take(&mut self.inputs);
        for input in inputs {
            let now = self.time.now();
            self.observer.input(now, &input);
            self.replica.on_input(now, input, &mut self.outputs);
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

    /// The replica asked to be stopped. See [`Output::Halt`].
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
                Output::Send { to, message } => self.network.send_peer(to, message),
                Output::Halt { .. } => self.halted = true,
                Output::PeerUp(_) | Output::PeerDown(_) | Output::Status { .. } => {}
            }
        }
    }
}
