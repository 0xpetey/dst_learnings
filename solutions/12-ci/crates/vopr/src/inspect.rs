//! The trace, read back: the cluster at any instant, one replica's
//! lines, one view's lines.
//!
//! A trace is the run. Every message, every disk completion, every
//! status change, on one line with a timestamp. A failing seed with
//! `--trace` prints it up to the assertion. This module parses those
//! lines and answers the three questions a bug hunt asks of them:
//!
//! - What did every replica believe at time T? ([`Timeline::snapshot`])
//! - What did replica N see and do? (`--replica N`)
//! - What happened in view V? (`--view V`)
//!
//! No state is reconstructed that the trace does not carry. A replica's
//! status and view come from its `status` lines. Its log's top, durable
//! count, commit number and `whole` flag come from the beats it sent.
//! Its suspicions come from its `peer up` and `peer down` lines. The
//! snapshot is the newest of each before T, and nothing more.

use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;

/// One trace line, with its timestamp and emitter parsed out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// Nanoseconds since the start of the run.
    pub at: u64,
    /// The replica the line is about: the emitter of a host line, the
    /// receiver of a delivery, the sender of a send.
    pub node: Option<u8>,
    /// The view the line belongs to: a `view=` token, or the emitting
    /// replica's view at the time.
    pub view: Option<u64>,
    /// The line, after the timestamp.
    pub text: String,
}

/// What the trace says one replica believed at an instant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaState {
    pub id: u8,
    pub up: bool,
    pub paused: bool,
    pub status: String,
    pub view: u64,
    pub log_view: u64,
    /// Records in the log, from the last beat.
    pub top: u64,
    pub durable: u64,
    pub commit: u64,
    pub whole: bool,
    /// Peers this replica's detector marked `Down`.
    pub suspects: BTreeSet<u8>,
    /// The last line this replica emitted, if any.
    pub last_seen: Option<u64>,
}

impl ReplicaState {
    fn new(id: u8) -> ReplicaState {
        ReplicaState {
            id,
            up: true,
            paused: false,
            status: "starting".into(),
            view: 0,
            log_view: 0,
            top: 0,
            durable: 0,
            commit: 0,
            whole: true,
            suspects: BTreeSet::new(),
            last_seen: None,
        }
    }
}

impl fmt::Display for ReplicaState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.up {
            return write!(f, "n{} down", self.id);
        }
        write!(
            f,
            "n{} {} view={} log_view={} top={} durable={} commit={} whole={}",
            self.id, self.status, self.view, self.log_view, self.top, self.durable, self.commit, self.whole
        )?;
        if !self.suspects.is_empty() {
            write!(f, " suspects=[")?;
            for (i, p) in self.suspects.iter().enumerate() {
                write!(f, "{}n{p}", if i > 0 { " " } else { "" })?;
            }
            write!(f, "]")?;
        }
        if self.paused {
            write!(f, " paused")?;
        }
        Ok(())
    }
}

/// The cluster at an instant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub at: u64,
    pub replicas: Vec<ReplicaState>,
    /// The partition in force, as the trace printed it.
    pub partition: Option<String>,
    pub faults_stopped: bool,
    /// Trace lines up to and including `at`.
    pub lines: usize,
}

impl Snapshot {
    /// The replicas that call themselves primary. More than one is a
    /// split, which the protocol allows only across views.
    pub fn primaries(&self) -> Vec<u8> {
        self.replicas
            .iter()
            .filter(|r| r.up && r.status == "primary")
            .map(|r| r.id)
            .collect()
    }
}

impl fmt::Display for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "at {} after {} lines:", fmt_time(self.at), self.lines)?;
        for r in &self.replicas {
            writeln!(f, "  {r}")?;
        }
        match &self.partition {
            Some(p) => writeln!(f, "  net partition {p}")?,
            None => writeln!(f, "  net connected")?,
        }
        if self.faults_stopped {
            writeln!(f, "  faults stopped")?;
        }
        Ok(())
    }
}

/// What to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub file: String,
    /// The snapshot's instant. The end of the trace if not given.
    pub at: Option<u64>,
    pub since: Option<u64>,
    pub until: Option<u64>,
    pub replica: Option<u8>,
    pub view: Option<u64>,
}

impl Query {
    pub fn new(file: &str) -> Query {
        Query {
            file: file.to_string(),
            at: None,
            since: None,
            until: None,
            replica: None,
            view: None,
        }
    }

    /// Is any line filter set? Without one, only the snapshot and the
    /// lines just before it are shown.
    pub fn filters(&self) -> bool {
        self.since.is_some() || self.until.is_some() || self.replica.is_some() || self.view.is_some()
    }
}

/// The parsed trace.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    pub events: Vec<Event>,
    /// Replicas seen, by the highest id.
    pub nodes: u8,
}

/// Lines just before the snapshot, when no filter asks for more.
const TAIL: usize = 12;

impl Timeline {
    pub fn parse<'a>(lines: impl IntoIterator<Item = &'a str>) -> Timeline {
        let mut t = Timeline::default();
        // Views per node, tracked while parsing, so that a line
        // without a `view=` token still knows its view.
        let mut views: Vec<u64> = Vec::new();
        for line in lines {
            let Some((at, text)) = split_time(line) else {
                continue;
            };
            let node = node_of(text);
            if let Some(n) = node {
                t.nodes = t.nodes.max(n + 1);
                if views.len() <= usize::from(n) {
                    views.resize(usize::from(n) + 1, 0);
                }
            }
            if let (Some(n), Some(rest)) = (
                node,
                text.strip_prefix(&format!("n{} status view=", node.unwrap_or(0))),
            ) {
                if let Some(v) = rest.split(' ').next().and_then(|v| v.parse().ok()) {
                    views[usize::from(n)] = v;
                }
            }
            let view = field(text, "view").or_else(|| node.map(|n| views[usize::from(n)]));
            t.events.push(Event {
                at,
                node,
                view,
                text: text.to_string(),
            });
        }
        t
    }

    /// The last instant in the trace.
    pub fn end(&self) -> u64 {
        self.events.last().map_or(0, |e| e.at)
    }

    /// The cluster at `at`, from every line up to and including it.
    pub fn snapshot(&self, at: u64) -> Snapshot {
        let mut replicas: Vec<ReplicaState> = (0..self.nodes).map(ReplicaState::new).collect();
        let mut partition = None;
        let mut faults_stopped = false;
        let mut lines = 0;
        for e in self.events.iter().take_while(|e| e.at <= at) {
            lines += 1;
            let text = e.text.as_str();
            if let Some(rest) = text.strip_prefix("net partition ") {
                partition = Some(rest.to_string());
                continue;
            }
            if text.starts_with("net heal") {
                partition = None;
                continue;
            }
            if text == "faults stopped" {
                faults_stopped = true;
                continue;
            }
            let Some(n) = e.node else { continue };
            let r = &mut replicas[usize::from(n)];
            let Some(rest) = text.strip_prefix(&format!("n{n} ")) else {
                continue;
            };
            r.last_seen = Some(e.at);
            if let Some(rest) = rest.strip_prefix("status view=") {
                let mut it = rest.split(' ');
                r.view = it.next().and_then(|v| v.parse().ok()).unwrap_or(r.view);
                r.status = it.next().unwrap_or("?").to_string();
            } else if rest.starts_with("crash") {
                r.up = false;
                r.paused = false;
                r.suspects.clear();
            } else if rest == "restart" {
                *r = ReplicaState::new(n);
                r.last_seen = Some(e.at);
            } else if rest == "pause" {
                r.paused = true;
            } else if rest.starts_with("resume") {
                r.paused = false;
            } else if rest.starts_with("halted") {
                r.status = "halted".into();
            } else if let Some(peer) = rest.strip_prefix("peer down n") {
                if let Ok(p) = peer.parse() {
                    r.suspects.insert(p);
                }
            } else if let Some(peer) = rest.strip_prefix("peer up n") {
                if let Ok(p) = peer.parse::<u8>() {
                    r.suspects.remove(&p);
                }
            } else if rest.starts_with("out Send to=") && rest.contains(" beat ") {
                // The beat says what the replica believes about its log.
                if let Some(v) = field(rest, "view") {
                    r.view = v;
                }
                r.log_view = field(rest, "log_view").unwrap_or(r.log_view);
                r.top = field(rest, "top").unwrap_or(r.top);
                r.durable = field(rest, "durable").unwrap_or(r.durable);
                r.commit = field(rest, "commit").unwrap_or(r.commit);
                if let Some(w) = text_field(rest, "whole") {
                    r.whole = w == "true";
                }
            }
        }
        Snapshot {
            at,
            replicas,
            partition,
            faults_stopped,
            lines,
        }
    }

    /// The lines a query selects: inside `since..=until`, about the
    /// replica, in the view.
    pub fn select(&self, q: &Query) -> Vec<&Event> {
        let since = q.since.unwrap_or(0);
        let until = q.until.or(q.at).unwrap_or(u64::MAX);
        self.events
            .iter()
            .filter(|e| e.at >= since && e.at <= until)
            .filter(|e| {
                q.replica
                    .is_none_or(|n| e.node == Some(n) || mentions(&e.text, n))
            })
            .filter(|e| q.view.is_none_or(|v| e.view == Some(v)))
            .collect()
    }

    /// The answer: a snapshot, and the lines that led to it.
    pub fn render(&self, q: &Query) -> String {
        let at = q.at.or(q.until).unwrap_or_else(|| self.end());
        let snapshot = self.snapshot(at);
        let mut out = String::new();
        if q.filters() {
            let selected = self.select(q);
            for e in &selected {
                let _ = writeln!(out, "{} {}", fmt_time(e.at), e.text);
            }
            let _ = writeln!(out, "{} lines selected", selected.len());
        } else {
            let upto: Vec<&Event> = self.events.iter().filter(|e| e.at <= at).collect();
            for e in upto.iter().rev().take(TAIL).rev() {
                let _ = writeln!(out, "{} {}", fmt_time(e.at), e.text);
            }
        }
        out.push_str(&snapshot.to_string());
        let primaries = snapshot.primaries();
        if primaries.len() > 1 {
            let _ = writeln!(
                out,
                "  {} primaries: {}",
                primaries.len(),
                primaries
                    .iter()
                    .map(|p| format!("n{p}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        out
    }
}

/// `t=1234ns rest` to `(1234, "rest")`.
fn split_time(line: &str) -> Option<(u64, &str)> {
    let rest = line.strip_prefix("t=")?;
    let (ns, rest) = rest.split_once("ns ")?;
    Some((ns.parse().ok()?, rest))
}

/// The replica a line is about.
fn node_of(text: &str) -> Option<u8> {
    if let Some(rest) = text.strip_prefix('n') {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() && rest[digits.len()..].starts_with(' ') {
            return digits.parse().ok();
        }
    }
    if let Some(rest) = text.strip_prefix("timer node=") {
        return rest.split(' ').next()?.parse().ok();
    }
    // `net deliver n0->n1 ...`: the receiver. `net send n0->n1`: the sender.
    if let Some(rest) = text.strip_prefix("net deliver ") {
        let (_, to) = rest.split_once("->")?;
        return to.strip_prefix('n')?.split(' ').next()?.parse().ok();
    }
    if let Some(rest) = text.strip_prefix("net send ") {
        return rest.strip_prefix('n')?.split("->").next()?.parse().ok();
    }
    None
}

/// Does the line name `nN` anywhere?
fn mentions(text: &str, n: u8) -> bool {
    let name = format!("n{n}");
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w == name)
}

/// The value of `key=` in a line, as a number.
fn field(text: &str, key: &str) -> Option<u64> {
    text_field(text, key)?.parse().ok()
}

fn text_field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!(" {key}=");
    let start = text.find(&pat)? + pat.len();
    let rest = &text[start..];
    Some(rest.split(' ').next().unwrap_or(rest))
}

/// `12.5s`, `350ms`, `20us`, `1234ns`, `t=1234ns`, or a bare number of
/// nanoseconds.
pub fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim().strip_prefix("t=").unwrap_or(s.trim());
    let (num, unit) = match s.find(|c: char| c.is_ascii_alphabetic()) {
        Some(i) => s.split_at(i),
        None => (s, "ns"),
    };
    let scale: f64 = match unit {
        "ns" => 1.0,
        "us" => 1e3,
        "ms" => 1e6,
        "s" => 1e9,
        _ => return None,
    };
    let value: f64 = num.parse().ok()?;
    if value < 0.0 {
        return None;
    }
    Some((value * scale).round() as u64)
}

/// Nanoseconds as seconds, with the zeros trimmed.
pub fn fmt_time(ns: u64) -> String {
    let secs = ns / 1_000_000_000;
    let frac = ns % 1_000_000_000;
    if frac == 0 {
        return format!("{secs}s");
    }
    let mut f = format!("{frac:09}");
    while f.ends_with('0') {
        f.pop();
    }
    format!("{secs}.{f}s")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACE: &str = "\
t=0ns config seed=1 net=[reliable latency=1ms]
t=0ns n0 out Read op=0
t=1000000ns n1 status view=0 view_change
t=2000000ns n0 status view=0 primary
t=2000000ns n0 out Send to=n1 beat boot=0 seq=3 view=0 primary log_view=0 runs=[0..4] top=4 durable=4 commit=3 whole=true
t=3000000ns net partition bridge n2 [n0] | [n1]
t=4000000ns n1 peer down n0
t=5000000ns n1 status view=1 view_change
t=5000000ns n1 out Send to=n2 start_view_change view=1
t=6000000ns net deliver n1->n2 m9 start_view_change view=1
t=7000000ns n0 crash landed=0 torn=0 lost=0
t=8000000ns net heal
t=9000000ns n0 restart
t=9500000ns n0 pause
t=10000000ns faults stopped
";

    #[test]
    fn times_parse_and_print() {
        assert_eq!(parse_time("12.5s"), Some(12_500_000_000));
        assert_eq!(parse_time("350ms"), Some(350_000_000));
        assert_eq!(parse_time("20us"), Some(20_000));
        assert_eq!(parse_time("t=1234ns"), Some(1234));
        assert_eq!(parse_time("99"), Some(99));
        assert_eq!(parse_time("-1s"), None);
        assert_eq!(parse_time("1h"), None);
        assert_eq!(fmt_time(12_500_000_000), "12.5s");
        assert_eq!(fmt_time(0), "0s");
        assert_eq!(fmt_time(1234), "0.000001234s");
    }

    #[test]
    fn lines_know_their_node_and_view() {
        let t = Timeline::parse(TRACE.lines());
        // n2 is only ever a receiver, and still counts.
        assert_eq!(t.nodes, 3);
        assert_eq!(t.events.len(), 15);
        let e = &t.events[4];
        assert_eq!((e.node, e.view), (Some(0), Some(0)));
        let e = &t.events[8];
        assert_eq!((e.node, e.view), (Some(1), Some(1)));
        // A delivery is about the receiver, and its view is the message's.
        let e = &t.events[9];
        assert_eq!((e.node, e.view), (Some(2), Some(1)));
        // A world line has no node and no view.
        assert_eq!((t.events[5].node, t.events[5].view), (None, None));
        // A node line without a `view=` carries the node's view then.
        assert_eq!(t.events[6].view, Some(0));
    }

    #[test]
    fn the_snapshot_is_the_newest_of_everything_before() {
        let t = Timeline::parse(TRACE.lines());
        let s = t.snapshot(4_500_000);
        assert_eq!(s.lines, 7);
        assert_eq!(s.partition.as_deref(), Some("bridge n2 [n0] | [n1]"));
        let n0 = &s.replicas[0];
        assert_eq!(
            (n0.status.as_str(), n0.view, n0.top, n0.durable, n0.commit),
            ("primary", 0, 4, 4, 3)
        );
        let n1 = &s.replicas[1];
        assert_eq!(n1.status, "view_change");
        assert_eq!(n1.suspects.iter().copied().collect::<Vec<_>>(), vec![0]);
        assert_eq!(s.primaries(), vec![0]);

        let s = t.snapshot(7_000_000);
        assert!(!s.replicas[0].up);
        assert_eq!(s.replicas[1].view, 1);
        assert_eq!(s.to_string().lines().nth(1).unwrap().trim(), "n0 down");

        let s = t.snapshot(t.end());
        assert!(s.replicas[0].up && s.replicas[0].paused);
        assert_eq!(s.replicas[0].status, "starting");
        assert!(s.partition.is_none() && s.faults_stopped);
    }

    #[test]
    fn selection_by_replica_view_and_window() {
        let t = Timeline::parse(TRACE.lines());
        let mut q = Query::new("-");
        q.replica = Some(1);
        let lines: Vec<&str> = t.select(&q).iter().map(|e| e.text.as_str()).collect();
        // Its own lines, the beat sent to it, the partition that names
        // it, and the delivery it sent.
        assert_eq!(lines.len(), 7, "{lines:?}");
        assert!(lines.iter().all(|l| l.contains("n1")));
        q.view = Some(1);
        assert_eq!(t.select(&q).len(), 3);
        q.replica = None;
        q.since = Some(6_000_000);
        q.until = Some(8_000_000);
        let lines: Vec<&str> = t.select(&q).iter().map(|e| e.text.as_str()).collect();
        assert_eq!(lines, ["net deliver n1->n2 m9 start_view_change view=1"]);
        let text = t.render(&q);
        assert!(text.contains("1 lines selected"), "{text}");
        assert!(text.contains("at 0.008s after 12 lines"), "{text}");
    }
}
