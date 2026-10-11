//! The real environment: system clock, a log file with `fsync`, and TCP.
//!
//! Single threaded on purpose. The server loop polls a non-blocking listener
//! and its connections, feeds complete frames to the node, and writes replies
//! back. Storage is synchronous underneath (write, `fsync`, done) but still
//! reports through `poll_completions`, so the node sees exactly the same
//! shape of events it sees under the fake.
//!
//! Everything in this file that touches the OS is the *only* place in the
//! workspace allowed to. Module 3 turns that sentence into a lint.

use kv_core::client::{self, Client};
use kv_core::env::{Driver, Network, Observer, Storage, Time};
use kv_core::node::{Input, OpId, Output};
use kv_core::protocol::{decode_frame, encode_frame, DecodeError, Reader};
use kv_core::{ClientId, Duration, Instant, Request, RequestEnvelope, Response, ResponseEnvelope};
use std::collections::{BTreeMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;

/// Nanoseconds since the server started, from the monotonic clock.
pub struct RealTime {
    start: std::time::Instant,
}

impl RealTime {
    pub fn new() -> RealTime {
        RealTime {
            start: std::time::Instant::now(),
        }
    }

    fn convert(&self, t: std::time::Instant) -> Instant {
        Instant::from_nanos(
            u64::try_from(t.duration_since(self.start).as_nanos()).expect("uptime fits in u64 ns"),
        )
    }
}

impl Default for RealTime {
    fn default() -> Self {
        RealTime::new()
    }
}

impl Time for RealTime {
    fn now(&self) -> Instant {
        self.convert(std::time::Instant::now())
    }
}

/// An append-only file, synced on every append.
pub struct FileStorage {
    file: File,
    completions: VecDeque<Input>,
}

impl FileStorage {
    pub fn open(path: &Path) -> io::Result<FileStorage> {
        let file = OpenOptions::new()
            .read(true)
            .append(false)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        Ok(FileStorage {
            file,
            completions: VecDeque::new(),
        })
    }
}

impl Storage for FileStorage {
    fn read_all(&mut self, op: OpId) {
        let mut data = Vec::new();
        self.file.seek(SeekFrom::Start(0)).expect("seek log");
        self.file.read_to_end(&mut data).expect("read log");
        self.completions.push_back(Input::ReadComplete { op, data });
    }

    fn append(&mut self, op: OpId, offset: u64, bytes: &[u8]) {
        // Writing at the node's offset rather than O_APPEND means a torn tail
        // left by a crash is overwritten, not appended after.
        self.file.seek(SeekFrom::Start(offset)).expect("seek log");
        self.file.write_all(bytes).expect("write log");
        self.file
            .set_len(offset + bytes.len() as u64)
            .expect("truncate torn tail");
        self.file.sync_data().expect("fsync log");
        self.completions.push_back(Input::AppendComplete { op });
    }

    fn poll_completions(&mut self, inputs: &mut Vec<Input>) {
        inputs.extend(self.completions.drain(..));
    }
}

struct Conn {
    stream: TcpStream,
    inbuf: Vec<u8>,
    outbuf: Vec<u8>,
    closed: bool,
}

/// Non-blocking TCP server side.
pub struct TcpNetwork {
    listener: TcpListener,
    conns: Vec<Conn>,
    /// Which connection last spoke for each client, so replies find their way.
    routes: BTreeMap<ClientId, usize>,
}

impl TcpNetwork {
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<TcpNetwork> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(TcpNetwork {
            listener,
            conns: Vec::new(),
            routes: BTreeMap::new(),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    fn accept_all(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true).expect("nonblocking");
                    stream.set_nodelay(true).ok();
                    self.conns.push(Conn {
                        stream,
                        inbuf: Vec::new(),
                        outbuf: Vec::new(),
                        closed: false,
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) => {
                    eprintln!("accept failed: {e}");
                    return;
                }
            }
        }
    }

    fn read_frames(&mut self, inputs: &mut Vec<Input>) {
        for (i, conn) in self.conns.iter_mut().enumerate() {
            let mut buf = [0u8; 64 * 1024];
            loop {
                match conn.stream.read(&mut buf) {
                    Ok(0) => {
                        conn.closed = true;
                        break;
                    }
                    Ok(n) => conn.inbuf.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        conn.closed = true;
                        break;
                    }
                }
            }
            loop {
                match decode_frame(&conn.inbuf) {
                    Ok((payload, used)) => {
                        match RequestEnvelope::decode(&mut Reader::new(payload)) {
                            Ok(envelope) => {
                                self.routes.insert(envelope.client, i);
                                inputs.push(Input::Request(envelope));
                            }
                            Err(e) => {
                                eprintln!("bad request, closing connection: {e}");
                                conn.closed = true;
                            }
                        }
                        conn.inbuf.drain(..used);
                    }
                    Err(DecodeError::Incomplete) => break,
                    Err(e) => {
                        eprintln!("bad frame, closing connection: {e}");
                        conn.closed = true;
                        break;
                    }
                }
            }
        }
    }

    fn flush_and_prune(&mut self) {
        for conn in &mut self.conns {
            while !conn.outbuf.is_empty() {
                match conn.stream.write(&conn.outbuf) {
                    Ok(0) => {
                        conn.closed = true;
                        break;
                    }
                    Ok(n) => {
                        conn.outbuf.drain(..n);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        conn.closed = true;
                        break;
                    }
                }
            }
        }
        // Remove closed connections; indices shift, so rebuild the routes.
        if self.conns.iter().any(|c| c.closed) {
            let mut kept = Vec::new();
            let mut remap = vec![None; self.conns.len()];
            for (old, conn) in self.conns.drain(..).enumerate() {
                if !conn.closed {
                    remap[old] = Some(kept.len());
                    kept.push(conn);
                }
            }
            self.conns = kept;
            self.routes = self
                .routes
                .iter()
                .filter_map(|(c, &i)| remap[i].map(|n| (*c, n)))
                .collect();
        }
    }
}

impl Network for TcpNetwork {
    fn send(&mut self, reply: ResponseEnvelope) {
        let Some(&i) = self.routes.get(&reply.client) else {
            // Client went away between request and reply. Nothing to do; it
            // will retry on a new connection if it still cares.
            return;
        };
        encode_frame(&mut self.conns[i].outbuf, |o| reply.encode(o));
    }

    fn poll_requests(&mut self, inputs: &mut Vec<Input>) {
        self.accept_all();
        self.read_frames(inputs);
        self.flush_and_prune();
    }
}

/// Prints one line per input and output to stderr.
pub struct StderrObserver;

impl Observer for StderrObserver {
    fn input(&mut self, now: Instant, input: &Input) {
        match input {
            Input::Request(r) => eprintln!("{now} in  request client={} id={}", r.client, r.request_id),
            Input::ReadComplete { op, data } => {
                eprintln!("{now} in  read complete op={op} bytes={}", data.len())
            }
            Input::AppendComplete { op } => eprintln!("{now} in  append complete op={op}"),
        }
    }
    fn output(&mut self, now: Instant, output: &Output) {
        match output {
            Output::Reply(r) => eprintln!("{now} out reply client={} id={}", r.client, r.request_id),
            Output::Append { op, offset, bytes } => {
                eprintln!("{now} out append op={op} offset={offset} bytes={}", bytes.len())
            }
            Output::Read { op } => eprintln!("{now} out read op={op}"),
        }
    }
}

/// Run the server forever. `on_ready` is called once with the bound address.
pub fn serve(
    addr: &str,
    data_dir: &Path,
    verbose: bool,
    on_ready: impl FnOnce(SocketAddr),
) -> io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let storage = FileStorage::open(&data_dir.join("kv.log"))?;
    let network = TcpNetwork::bind(addr)?;
    on_ready(network.local_addr()?);
    if verbose {
        run_loop(Driver::new(RealTime::new(), storage, network, StderrObserver))
    } else {
        run_loop(Driver::new(
            RealTime::new(),
            storage,
            network,
            kv_core::env::Silent,
        ))
    }
}

fn run_loop<O: Observer>(mut driver: Driver<RealTime, FileStorage, TcpNetwork, O>) -> io::Result<()> {
    loop {
        if driver.step() == 0 {
            // Nothing happened. A production server would block in poll(2);
            // a short sleep keeps this teaching version simple.
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

/// Errors a one-shot client can hit.
#[derive(Debug)]
pub enum CallError {
    Io(io::Error),
    Decode(DecodeError),
    /// Retries exhausted. The request may or may not have been applied.
    GaveUp,
}

impl From<io::Error> for CallError {
    fn from(e: io::Error) -> Self {
        CallError::Io(e)
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Io(e) => write!(f, "io error: {e}"),
            CallError::Decode(e) => write!(f, "bad reply: {e}"),
            CallError::GaveUp => write!(f, "no reply after all retries"),
        }
    }
}

/// Perform one request against a server, driving the `Client` state machine
/// with the real clock. Retries on timeout with the same request id, so a
/// slow server never applies a write twice.
pub fn call(
    addr: &str,
    client_id: ClientId,
    request: Request,
    config: client::Config,
) -> Result<Response, CallError> {
    let time = RealTime::new();
    let mut stream = TcpStream::connect(addr)?;
    stream.set_nodelay(true).ok();
    let mut client = Client::new(client_id, config);
    let mut out = Vec::new();
    client.submit(time.now(), request, &mut out);
    let mut inbuf = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        for o in out.drain(..) {
            match o {
                client::Output::Send(envelope) => {
                    let mut frame = Vec::new();
                    encode_frame(&mut frame, |w| envelope.encode(w));
                    stream.write_all(&frame)?;
                }
                client::Output::Done { response, .. } => return Ok(response),
                client::Output::GaveUp { .. } => return Err(CallError::GaveUp),
            }
        }
        let deadline = client.next_deadline().expect("in flight");
        let wait = deadline
            .saturating_duration_since(time.now())
            .max(Duration::from_millis(1));
        stream.set_read_timeout(Some(wait))?;
        match stream.read(&mut buf) {
            Ok(0) => {
                return Err(CallError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "server closed connection",
                )))
            }
            Ok(n) => inbuf.extend_from_slice(&buf[..n]),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.into()),
        }
        loop {
            match decode_frame(&inbuf) {
                Ok((payload, used)) => {
                    let reply =
                        ResponseEnvelope::decode(&mut Reader::new(payload)).map_err(CallError::Decode)?;
                    inbuf.drain(..used);
                    client.on_input(client::Input::Response(reply), &mut out);
                }
                Err(DecodeError::Incomplete) => break,
                Err(e) => return Err(CallError::Decode(e)),
            }
        }
        client.on_input(client::Input::Tick { now: time.now() }, &mut out);
    }
}
