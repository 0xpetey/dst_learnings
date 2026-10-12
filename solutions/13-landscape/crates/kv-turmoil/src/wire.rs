//! A wire format for the replica's peer messages, and a datagram envelope.
//!
//! The fake network of Modules 5 to 12 carried `replica::Message` by
//! value. The real environment never dialled a peer, so the protocol
//! between replicas had no bytes until this module. A protocol with no
//! wire format is a protocol that was never finished, and turmoil's
//! network carries bytes and nothing else.
//!
//! The encoding follows `kv_core::protocol`: little-endian integers,
//! length-prefixed byte strings, one tag byte per variant, no dependency.
//! One datagram carries one [`Datagram`], and a datagram is a message, so
//! there is no stream framing here.

use kv_core::protocol::{put_u32, put_u64, DecodeError, Reader, MAX_FRAME_LEN};
use kv_core::replica::{Message, Status};
use kv_core::{NodeId, RequestEnvelope, ResponseEnvelope};

/// What one datagram carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Datagram {
    /// A client's request, to a replica.
    Request(RequestEnvelope),
    /// A replica's reply, to a client.
    Reply(ResponseEnvelope),
    /// A replica's message to a peer. The sender names itself, because
    /// the replica wants a `NodeId` and the network knows an address.
    Peer { from: NodeId, message: Message },
}

const TAG_REQUEST: u8 = 1;
const TAG_REPLY: u8 = 2;
const TAG_PEER: u8 = 3;

const TAG_BEAT: u8 = 20;
const TAG_START_VIEW_CHANGE: u8 = 21;
const TAG_DO_VIEW_CHANGE: u8 = 22;
const TAG_START_VIEW: u8 = 23;
const TAG_PREPARE: u8 = 24;
const TAG_PREPARE_OK: u8 = 25;
const TAG_PULL: u8 = 26;
const TAG_FORWARD: u8 = 27;
const TAG_FETCH: u8 = 28;
const TAG_MISSING: u8 = 29;

/// Every status, in a fixed order, so that a beat can carry one.
const STATUSES: [Status; 9] = [
    Status::Reading,
    Status::Joining,
    Status::Repairing,
    Status::Erasing,
    Status::ViewChange,
    Status::Backup,
    Status::Gathering,
    Status::Primary,
    Status::Halted,
];

fn put_status(out: &mut Vec<u8>, status: Status) {
    let index = STATUSES
        .iter()
        .position(|&s| s == status)
        .expect("every status is in the table");
    out.push(index as u8);
}

fn status(r: &mut Reader<'_>) -> Result<Status, DecodeError> {
    let index = usize::from(r.u8()?);
    STATUSES
        .get(index)
        .copied()
        .ok_or(DecodeError::Malformed("unknown status"))
}

fn put_bool(out: &mut Vec<u8>, b: bool) {
    out.push(u8::from(b));
}

fn boolean(r: &mut Reader<'_>) -> Result<bool, DecodeError> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DecodeError::Malformed("bad bool")),
    }
}

fn put_runs(out: &mut Vec<u8>, runs: &[(u32, u64)]) {
    put_u32(out, u32::try_from(runs.len()).expect("runs fit in u32"));
    for &(view, until) in runs {
        put_u32(out, view);
        put_u64(out, until);
    }
}

fn runs(r: &mut Reader<'_>) -> Result<Vec<(u32, u64)>, DecodeError> {
    let n = r.u32()? as usize;
    // Each run is twelve bytes. A count that cannot fit in the datagram
    // is a lie, and the loop below must not allocate for it.
    if n > r.remaining() / 12 {
        return Err(DecodeError::Malformed("too many runs"));
    }
    (0..n).map(|_| Ok((r.u32()?, r.u64()?))).collect()
}

impl Datagram {
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Datagram::Request(envelope) => {
                out.push(TAG_REQUEST);
                envelope.encode(out);
            }
            Datagram::Reply(envelope) => {
                out.push(TAG_REPLY);
                envelope.encode(out);
            }
            Datagram::Peer { from, message } => {
                out.push(TAG_PEER);
                out.push(*from);
                encode_message(message, out);
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Datagram, DecodeError> {
        let mut r = Reader::new(bytes);
        let datagram = match r.u8()? {
            TAG_REQUEST => Datagram::Request(RequestEnvelope::decode(&mut r)?),
            TAG_REPLY => Datagram::Reply(ResponseEnvelope::decode(&mut r)?),
            TAG_PEER => Datagram::Peer {
                from: r.u8()?,
                message: decode_message(&mut r)?,
            },
            _ => return Err(DecodeError::Malformed("unknown datagram tag")),
        };
        if r.remaining() != 0 {
            return Err(DecodeError::Malformed("trailing bytes"));
        }
        Ok(datagram)
    }

    /// The bytes of this datagram, freshly allocated.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode(&mut out);
        out
    }
}

pub fn encode_message(message: &Message, out: &mut Vec<u8>) {
    match message {
        Message::Beat {
            boot,
            seq,
            view,
            status: s,
            log_view,
            runs,
            top,
            durable,
            commit,
            whole,
        } => {
            out.push(TAG_BEAT);
            put_u64(out, *boot);
            put_u64(out, *seq);
            put_u64(out, *view);
            put_status(out, *s);
            put_u64(out, *log_view);
            put_runs(out, runs);
            put_u64(out, *top);
            put_u64(out, *durable);
            put_u64(out, *commit);
            put_bool(out, *whole);
        }
        Message::StartViewChange { view } => {
            out.push(TAG_START_VIEW_CHANGE);
            put_u64(out, *view);
        }
        Message::DoViewChange {
            view,
            log_view,
            runs,
            commit,
            whole,
        } => {
            out.push(TAG_DO_VIEW_CHANGE);
            put_u64(out, *view);
            put_u64(out, *log_view);
            put_runs(out, runs);
            put_u64(out, *commit);
            put_bool(out, *whole);
        }
        Message::StartView { view, runs, commit } => {
            out.push(TAG_START_VIEW);
            put_u64(out, *view);
            put_runs(out, runs);
            put_u64(out, *commit);
        }
        Message::Prepare {
            view,
            seq,
            commit,
            bytes,
        } => {
            out.push(TAG_PREPARE);
            put_u64(out, *view);
            put_u64(out, *seq);
            put_u64(out, *commit);
            kv_core::protocol::put_blob(out, bytes);
        }
        Message::PrepareOk { boot, view, durable } => {
            out.push(TAG_PREPARE_OK);
            put_u64(out, *boot);
            put_u64(out, *view);
            put_u64(out, *durable);
        }
        Message::Pull { view, from } => {
            out.push(TAG_PULL);
            put_u64(out, *view);
            put_u64(out, *from);
        }
        Message::Forward { view, envelope } => {
            out.push(TAG_FORWARD);
            put_u64(out, *view);
            envelope.encode(out);
        }
        Message::Fetch { view, seq } => {
            out.push(TAG_FETCH);
            put_u64(out, *view);
            put_u64(out, *seq);
        }
        Message::Missing { view, seq } => {
            out.push(TAG_MISSING);
            put_u64(out, *view);
            put_u64(out, *seq);
        }
    }
}

pub fn decode_message(r: &mut Reader<'_>) -> Result<Message, DecodeError> {
    Ok(match r.u8()? {
        TAG_BEAT => Message::Beat {
            boot: r.u64()?,
            seq: r.u64()?,
            view: r.u64()?,
            status: status(r)?,
            log_view: r.u64()?,
            runs: runs(r)?,
            top: r.u64()?,
            durable: r.u64()?,
            commit: r.u64()?,
            whole: boolean(r)?,
        },
        TAG_START_VIEW_CHANGE => Message::StartViewChange { view: r.u64()? },
        TAG_DO_VIEW_CHANGE => Message::DoViewChange {
            view: r.u64()?,
            log_view: r.u64()?,
            runs: runs(r)?,
            commit: r.u64()?,
            whole: boolean(r)?,
        },
        TAG_START_VIEW => Message::StartView {
            view: r.u64()?,
            runs: runs(r)?,
            commit: r.u64()?,
        },
        TAG_PREPARE => Message::Prepare {
            view: r.u64()?,
            seq: r.u64()?,
            commit: r.u64()?,
            bytes: r.blob(MAX_FRAME_LEN)?.to_vec(),
        },
        TAG_PREPARE_OK => Message::PrepareOk {
            boot: r.u64()?,
            view: r.u64()?,
            durable: r.u64()?,
        },
        TAG_PULL => Message::Pull {
            view: r.u64()?,
            from: r.u64()?,
        },
        TAG_FORWARD => Message::Forward {
            view: r.u64()?,
            envelope: RequestEnvelope::decode(r)?,
        },
        TAG_FETCH => Message::Fetch {
            view: r.u64()?,
            seq: r.u64()?,
        },
        TAG_MISSING => Message::Missing {
            view: r.u64()?,
            seq: r.u64()?,
        },
        _ => return Err(DecodeError::Malformed("unknown message tag")),
    })
}
