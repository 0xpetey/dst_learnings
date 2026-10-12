//! The client as a state machine: one request in flight, timeout, retry with
//! exponential backoff, and the same request id on every retry so the node
//! can deduplicate.
//!
//! The client never sleeps and never reads a clock. It is told the time
//! through `Input::Tick { now }` and says when it next needs waking through
//! [`Client::next_deadline`]. Module 4 will drive that from a virtual clock.

use crate::protocol::{ClientId, Request, RequestEnvelope, RequestId, Response, ResponseEnvelope};
use crate::time::{Duration, Instant};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Timeout for the first attempt.
    pub initial_timeout: Duration,
    /// Backoff cap.
    pub max_timeout: Duration,
    /// Give up after this many sends (including the first).
    pub max_attempts: u32,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            initial_timeout: Duration::from_millis(100),
            max_timeout: Duration::from_secs(5),
            max_attempts: 10,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Response(ResponseEnvelope),
    Tick { now: Instant },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Transmit this (possibly retransmitted) request.
    Send(RequestEnvelope),
    /// The operation finished.
    Done {
        request_id: RequestId,
        response: Response,
    },
    /// `max_attempts` sends went unanswered. The operation's fate is unknown:
    /// it may or may not have been applied.
    GaveUp { request_id: RequestId },
}

#[derive(Clone, Debug)]
struct InFlight {
    envelope: RequestEnvelope,
    attempts: u32,
    timeout: Duration,
    deadline: Instant,
}

#[derive(Clone, Debug)]
pub struct Client {
    id: ClientId,
    config: Config,
    next_request_id: RequestId,
    in_flight: Option<InFlight>,
    retries: u64,
}

impl Client {
    pub fn new(id: ClientId, config: Config) -> Client {
        assert!(config.max_attempts >= 1);
        assert!(config.initial_timeout > Duration::ZERO);
        assert!(config.max_timeout >= config.initial_timeout);
        Client {
            id,
            config,
            next_request_id: 1,
            in_flight: None,
            retries: 0,
        }
    }

    pub fn id(&self) -> ClientId {
        self.id
    }

    pub fn is_idle(&self) -> bool {
        self.in_flight.is_none()
    }

    /// Total retransmissions so far.
    pub fn retries(&self) -> u64 {
        self.retries
    }

    /// When the client next needs a `Tick`, if anything is in flight.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.in_flight.as_ref().map(|f| f.deadline)
    }

    /// Start an operation. The client is sequential: panics if one is already
    /// in flight, because a client that overlaps requests cannot be
    /// deduplicated by a single last-request-id.
    pub fn submit(&mut self, now: Instant, request: Request, out: &mut Vec<Output>) -> RequestId {
        assert!(
            self.in_flight.is_none(),
            "client {} submitted while a request was in flight",
            self.id
        );
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        let envelope = RequestEnvelope {
            client: self.id,
            request_id,
            request,
        };
        let timeout = self.config.initial_timeout;
        self.in_flight = Some(InFlight {
            envelope: envelope.clone(),
            attempts: 1,
            timeout,
            deadline: now + timeout,
        });
        out.push(Output::Send(envelope));
        request_id
    }

    pub fn on_input(&mut self, input: Input, out: &mut Vec<Output>) {
        match input {
            Input::Response(envelope) => self.on_response(envelope, out),
            Input::Tick { now } => self.on_tick(now, out),
        }
    }

    fn on_response(&mut self, envelope: ResponseEnvelope, out: &mut Vec<Output>) {
        assert_eq!(envelope.client, self.id, "response routed to the wrong client");
        assert!(
            envelope.request_id < self.next_request_id,
            "response to a request we never sent"
        );
        match &self.in_flight {
            Some(f) if f.envelope.request_id == envelope.request_id => {
                self.in_flight = None;
                out.push(Output::Done {
                    request_id: envelope.request_id,
                    response: envelope.response,
                });
            }
            // A late reply to an earlier attempt or an earlier request.
            _ => {}
        }
    }

    fn on_tick(&mut self, now: Instant, out: &mut Vec<Output>) {
        let Some(f) = self.in_flight.as_mut() else { return };
        if now < f.deadline {
            return;
        }
        if f.attempts >= self.config.max_attempts {
            let request_id = f.envelope.request_id;
            self.in_flight = None;
            out.push(Output::GaveUp { request_id });
            return;
        }
        f.attempts += 1;
        self.retries += 1;
        // Exponential backoff, capped. Deterministic: jitter, if wanted,
        // must come from a `Prng` handed in by the environment, never from
        // `rand::random` or the clock.
        f.timeout = f
            .timeout
            .checked_mul(2)
            .unwrap_or(self.config.max_timeout)
            .min(self.config.max_timeout);
        f.deadline = now + f.timeout;
        out.push(Output::Send(f.envelope.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn cfg() -> Config {
        Config {
            initial_timeout: Duration::from_millis(100),
            max_timeout: Duration::from_millis(350),
            max_attempts: 4,
        }
    }

    #[test]
    fn happy_path() {
        let mut c = Client::new(1, cfg());
        let mut out = Vec::new();
        let id = c.submit(Instant::ZERO, Request::Get { key: b"k".into() }, &mut out);
        assert_eq!(id, 1);
        assert!(matches!(&out[0], Output::Send(e) if e.request_id == 1));
        assert_eq!(c.next_deadline(), Some(Instant::from_nanos(100_000_000)));
        out.clear();
        c.on_input(
            Input::Response(ResponseEnvelope {
                client: 1,
                request_id: 1,
                response: Response::Value(None),
            }),
            &mut out,
        );
        assert_eq!(
            out,
            vec![Output::Done {
                request_id: 1,
                response: Response::Value(None)
            }]
        );
        assert!(c.is_idle());
    }

    #[test]
    fn backoff_doubles_caps_and_gives_up() {
        let mut c = Client::new(1, cfg());
        let mut out = Vec::new();
        let ms = Duration::from_millis;
        c.submit(Instant::ZERO, Request::Get { key: b"k".into() }, &mut out);
        let mut now = Instant::ZERO;
        let mut deadlines = Vec::new();
        // Tick before the deadline does nothing.
        c.on_input(Input::Tick { now: now + ms(50) }, &mut out);
        assert_eq!(out.len(), 1);
        while let Some(d) = c.next_deadline() {
            deadlines.push(d.saturating_duration_since(now));
            now = d;
            c.on_input(Input::Tick { now }, &mut out);
        }
        assert_eq!(deadlines, vec![ms(100), ms(200), ms(350), ms(350)]);
        let sends = out.iter().filter(|o| matches!(o, Output::Send(_))).count();
        assert_eq!(sends, 4);
        assert_eq!(c.retries(), 3);
        assert_eq!(out.last(), Some(&Output::GaveUp { request_id: 1 }));
        assert!(c.is_idle());
    }

    #[test]
    fn late_reply_to_old_request_is_ignored() {
        let mut c = Client::new(1, cfg());
        let mut out = Vec::new();
        c.submit(Instant::ZERO, Request::Get { key: b"k".into() }, &mut out);
        c.on_input(
            Input::Response(ResponseEnvelope {
                client: 1,
                request_id: 1,
                response: Response::Value(None),
            }),
            &mut out,
        );
        c.submit(Instant::ZERO, Request::Get { key: b"k".into() }, &mut out);
        out.clear();
        c.on_input(
            Input::Response(ResponseEnvelope {
                client: 1,
                request_id: 1,
                response: Response::Value(None),
            }),
            &mut out,
        );
        assert!(out.is_empty());
        assert!(!c.is_idle());
    }

    #[test]
    #[should_panic(expected = "never sent")]
    fn reply_to_unknown_request_is_a_bug() {
        let mut c = Client::new(1, cfg());
        let mut out = Vec::new();
        c.on_input(
            Input::Response(ResponseEnvelope {
                client: 1,
                request_id: 9,
                response: Response::PutOk,
            }),
            &mut out,
        );
    }
}
