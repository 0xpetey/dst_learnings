//! The request/response protocol and its wire encoding.
//!
//! The encoding is hand-rolled and tiny: little-endian integers, length
//! prefixed byte strings, one tag byte per variant. It needs no dependency,
//! it is trivially deterministic, and the same bytes go over TCP in the real
//! environment and through a `VecDeque` in the simulated one.
//!
//! Framing: every message on a stream is `[u32 LE length][payload]`.

use std::fmt;

/// Chosen by the client, unique per client process or simulated client.
pub type ClientId = u64;
/// Monotonically increasing per client. Together with [`ClientId`] it
/// identifies one logical operation across any number of retries.
pub type RequestId = u64;

/// Largest key the store accepts.
pub const MAX_KEY_LEN: usize = 1024;
/// Largest value the store accepts.
pub const MAX_VALUE_LEN: usize = 1 << 20;
/// Largest frame on the wire, derived from the limits above with headroom.
pub const MAX_FRAME_LEN: usize = MAX_KEY_LEN + MAX_VALUE_LEN + 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Get { key: Vec<u8> },
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    /// Result of a `Get`.
    Value(Option<Vec<u8>>),
    /// A `Put` is durable and applied.
    PutOk,
    /// A `Delete` is durable and applied; `existed` says whether the key was there.
    DeleteOk { existed: bool },
    /// The request violated a limit. Never retried by a well-behaved client.
    Rejected(Rejection),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    KeyTooLong,
    ValueTooLong,
    EmptyKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestEnvelope {
    pub client: ClientId,
    pub request_id: RequestId,
    pub request: Request,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseEnvelope {
    pub client: ClientId,
    pub request_id: RequestId,
    pub response: Response,
}

impl Request {
    pub fn key(&self) -> &[u8] {
        match self {
            Request::Get { key } | Request::Put { key, .. } | Request::Delete { key } => key,
        }
    }

    /// Check the limits. Done once, at the node's edge, so every later layer
    /// can assert them instead of re-checking.
    pub fn validate(&self) -> Result<(), Rejection> {
        let key = self.key();
        if key.is_empty() {
            return Err(Rejection::EmptyKey);
        }
        if key.len() > MAX_KEY_LEN {
            return Err(Rejection::KeyTooLong);
        }
        if let Request::Put { value, .. } = self {
            if value.len() > MAX_VALUE_LEN {
                return Err(Rejection::ValueTooLong);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Not enough bytes yet; wait for more.
    Incomplete,
    /// Bytes are not a valid message. The connection should be dropped.
    Malformed(&'static str),
    /// Frame claims a length above `MAX_FRAME_LEN`.
    FrameTooLarge(usize),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Incomplete => write!(f, "incomplete message"),
            DecodeError::Malformed(why) => write!(f, "malformed message: {why}"),
            DecodeError::FrameTooLarge(n) => write!(f, "frame of {n} bytes exceeds limit"),
        }
    }
}

/// A cursor over a byte slice for decoding. Every read is bounds checked and
/// returns `Incomplete` rather than panicking: the bytes come from the
/// network, which is to say from an adversary.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Reader<'a> {
        Reader { buf, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        let b = *self.buf.get(self.pos).ok_or(DecodeError::Incomplete)?;
        self.pos += 1;
        Ok(b)
    }

    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().expect("4 bytes")))
    }

    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().expect("8 bytes")))
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(DecodeError::Malformed("length overflow"))?;
        let s = self.buf.get(self.pos..end).ok_or(DecodeError::Incomplete)?;
        self.pos = end;
        Ok(s)
    }

    /// A `u32` length followed by that many bytes, capped at `max`.
    pub fn blob(&mut self, max: usize) -> Result<&'a [u8], DecodeError> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(DecodeError::Malformed("blob too long"));
        }
        self.bytes(n)
    }
}

pub fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_blob(out: &mut Vec<u8>, b: &[u8]) {
    put_u32(out, u32::try_from(b.len()).expect("blob length fits in u32"));
    out.extend_from_slice(b);
}

const TAG_GET: u8 = 1;
const TAG_PUT: u8 = 2;
const TAG_DELETE: u8 = 3;

const TAG_VALUE_NONE: u8 = 10;
const TAG_VALUE_SOME: u8 = 11;
const TAG_PUT_OK: u8 = 12;
const TAG_DELETE_OK: u8 = 13;
const TAG_REJECTED: u8 = 14;

impl Request {
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Request::Get { key } => {
                out.push(TAG_GET);
                put_blob(out, key);
            }
            Request::Put { key, value } => {
                out.push(TAG_PUT);
                put_blob(out, key);
                put_blob(out, value);
            }
            Request::Delete { key } => {
                out.push(TAG_DELETE);
                put_blob(out, key);
            }
        }
    }

    pub fn decode(r: &mut Reader<'_>) -> Result<Request, DecodeError> {
        match r.u8()? {
            TAG_GET => Ok(Request::Get {
                key: r.blob(MAX_KEY_LEN)?.to_vec(),
            }),
            TAG_PUT => Ok(Request::Put {
                key: r.blob(MAX_KEY_LEN)?.to_vec(),
                value: r.blob(MAX_VALUE_LEN)?.to_vec(),
            }),
            TAG_DELETE => Ok(Request::Delete {
                key: r.blob(MAX_KEY_LEN)?.to_vec(),
            }),
            _ => Err(DecodeError::Malformed("unknown request tag")),
        }
    }
}

impl Response {
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Response::Value(None) => out.push(TAG_VALUE_NONE),
            Response::Value(Some(v)) => {
                out.push(TAG_VALUE_SOME);
                put_blob(out, v);
            }
            Response::PutOk => out.push(TAG_PUT_OK),
            Response::DeleteOk { existed } => {
                out.push(TAG_DELETE_OK);
                out.push(u8::from(*existed));
            }
            Response::Rejected(why) => {
                out.push(TAG_REJECTED);
                out.push(match why {
                    Rejection::KeyTooLong => 1,
                    Rejection::ValueTooLong => 2,
                    Rejection::EmptyKey => 3,
                });
            }
        }
    }

    pub fn decode(r: &mut Reader<'_>) -> Result<Response, DecodeError> {
        match r.u8()? {
            TAG_VALUE_NONE => Ok(Response::Value(None)),
            TAG_VALUE_SOME => Ok(Response::Value(Some(r.blob(MAX_VALUE_LEN)?.to_vec()))),
            TAG_PUT_OK => Ok(Response::PutOk),
            TAG_DELETE_OK => match r.u8()? {
                0 => Ok(Response::DeleteOk { existed: false }),
                1 => Ok(Response::DeleteOk { existed: true }),
                _ => Err(DecodeError::Malformed("bad bool")),
            },
            TAG_REJECTED => Ok(Response::Rejected(match r.u8()? {
                1 => Rejection::KeyTooLong,
                2 => Rejection::ValueTooLong,
                3 => Rejection::EmptyKey,
                _ => return Err(DecodeError::Malformed("unknown rejection")),
            })),
            _ => Err(DecodeError::Malformed("unknown response tag")),
        }
    }
}

impl RequestEnvelope {
    pub fn encode(&self, out: &mut Vec<u8>) {
        put_u64(out, self.client);
        put_u64(out, self.request_id);
        self.request.encode(out);
    }

    pub fn decode(r: &mut Reader<'_>) -> Result<RequestEnvelope, DecodeError> {
        Ok(RequestEnvelope {
            client: r.u64()?,
            request_id: r.u64()?,
            request: Request::decode(r)?,
        })
    }
}

impl ResponseEnvelope {
    pub fn encode(&self, out: &mut Vec<u8>) {
        put_u64(out, self.client);
        put_u64(out, self.request_id);
        self.response.encode(out);
    }

    pub fn decode(r: &mut Reader<'_>) -> Result<ResponseEnvelope, DecodeError> {
        Ok(ResponseEnvelope {
            client: r.u64()?,
            request_id: r.u64()?,
            response: Response::decode(r)?,
        })
    }
}

/// Append `[u32 length][payload]` to `out`, where `payload` is produced by `f`.
pub fn encode_frame(out: &mut Vec<u8>, f: impl FnOnce(&mut Vec<u8>)) {
    let start = out.len();
    put_u32(out, 0);
    f(out);
    let len = out.len() - start - 4;
    assert!(len <= MAX_FRAME_LEN, "encoded a frame larger than MAX_FRAME_LEN");
    out[start..start + 4].copy_from_slice(&(len as u32).to_le_bytes());
}

/// Try to take one frame off the front of `buf`. On success returns the
/// payload and the total number of bytes consumed (header included).
pub fn decode_frame(buf: &[u8]) -> Result<(&[u8], usize), DecodeError> {
    let mut r = Reader::new(buf);
    let len = r.u32()? as usize;
    if len > MAX_FRAME_LEN {
        return Err(DecodeError::FrameTooLarge(len));
    }
    let payload = r.bytes(len)?;
    Ok((payload, r.position()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_request(req: Request) {
        let env = RequestEnvelope {
            client: 7,
            request_id: 99,
            request: req,
        };
        let mut buf = Vec::new();
        encode_frame(&mut buf, |o| env.encode(o));
        let (payload, used) = decode_frame(&buf).unwrap();
        assert_eq!(used, buf.len());
        let mut r = Reader::new(payload);
        assert_eq!(RequestEnvelope::decode(&mut r).unwrap(), env);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn requests_roundtrip() {
        roundtrip_request(Request::Get { key: b"k".to_vec() });
        roundtrip_request(Request::Put {
            key: b"k".to_vec(),
            value: vec![0; 1000],
        });
        roundtrip_request(Request::Delete { key: b"k".to_vec() });
    }

    #[test]
    fn responses_roundtrip() {
        for resp in [
            Response::Value(None),
            Response::Value(Some(b"v".to_vec())),
            Response::PutOk,
            Response::DeleteOk { existed: true },
            Response::DeleteOk { existed: false },
            Response::Rejected(Rejection::ValueTooLong),
        ] {
            let env = ResponseEnvelope {
                client: 1,
                request_id: 2,
                response: resp,
            };
            let mut buf = Vec::new();
            env.encode(&mut buf);
            assert_eq!(ResponseEnvelope::decode(&mut Reader::new(&buf)).unwrap(), env);
        }
    }

    #[test]
    fn partial_frames_are_incomplete_not_errors() {
        let env = RequestEnvelope {
            client: 1,
            request_id: 1,
            request: Request::Get { key: b"abc".to_vec() },
        };
        let mut buf = Vec::new();
        encode_frame(&mut buf, |o| env.encode(o));
        for cut in 0..buf.len() {
            assert_eq!(
                decode_frame(&buf[..cut]).unwrap_err(),
                DecodeError::Incomplete,
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn garbage_is_rejected_without_panicking() {
        assert!(matches!(
            decode_frame(&[0xff, 0xff, 0xff, 0x7f]),
            Err(DecodeError::FrameTooLarge(_))
        ));
        let mut r = Reader::new(&[42]);
        assert_eq!(
            Request::decode(&mut r).unwrap_err(),
            DecodeError::Malformed("unknown request tag")
        );
        let mut r = Reader::new(&[TAG_GET, 0xff, 0xff, 0, 0]);
        assert_eq!(
            Request::decode(&mut r).unwrap_err(),
            DecodeError::Malformed("blob too long")
        );
    }

    #[test]
    fn validation() {
        assert_eq!(Request::Get { key: vec![] }.validate(), Err(Rejection::EmptyKey));
        assert_eq!(
            Request::Get {
                key: vec![0; MAX_KEY_LEN + 1]
            }
            .validate(),
            Err(Rejection::KeyTooLong)
        );
        assert_eq!(
            Request::Put {
                key: vec![1],
                value: vec![0; MAX_VALUE_LEN + 1]
            }
            .validate(),
            Err(Rejection::ValueTooLong)
        );
        assert_eq!(
            Request::Put {
                key: vec![1],
                value: vec![]
            }
            .validate(),
            Ok(())
        );
    }
}
