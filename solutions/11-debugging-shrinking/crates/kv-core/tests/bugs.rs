//! Module 8 acceptance tests: each planted bug does what its name says,
//! on a bare node, and does nothing when it is not planted.

#![cfg(feature = "bugs")]

use kv_core::node::{Input, Node, Output};
use kv_core::{Bug, Request, RequestEnvelope, Response};

fn req(client: u64, id: u64, request: Request) -> Input {
    Input::Request(RequestEnvelope {
        client,
        request_id: id,
        request,
    })
}
fn put(k: &str, v: &str) -> Request {
    Request::Put {
        key: k.into(),
        value: v.into(),
    }
}
fn del(k: &str) -> Request {
    Request::Delete { key: k.into() }
}
fn replies(out: &[Output]) -> Vec<Response> {
    out.iter()
        .filter_map(|o| match o {
            Output::Reply(r) => Some(r.response.clone()),
            _ => None,
        })
        .collect()
}
fn log_of(outputs: &[Output]) -> Vec<u8> {
    let mut log = Vec::new();
    for o in outputs {
        if let Output::Append { offset, bytes, .. } = o {
            let offset = *offset as usize;
            log.resize(offset.max(log.len()), 0);
            log.truncate(offset);
            log.extend_from_slice(bytes);
        }
    }
    log
}
fn started(bug: Option<Bug>) -> (Node, Vec<Output>) {
    let mut out = Vec::new();
    let mut node = Node::new(&mut out);
    if let Some(bug) = bug {
        node.bugs_mut().plant(bug);
        assert!(node.bugs().has(bug));
        assert_eq!(node.bugs().planted(), vec![bug]);
    }
    out.clear();
    node.on_input(Input::ReadComplete { op: 0, data: vec![] }, &mut out);
    (node, out)
}

#[test]
fn lost_update_never_applies_a_write_submitted_behind_another() {
    for bug in [None, Some(Bug::LostUpdate)] {
        let (mut node, mut out) = started(bug);
        node.on_input(req(1, 1, put("a", "1")), &mut out);
        node.on_input(req(2, 1, put("a", "2")), &mut out);
        out.clear();
        node.on_input(Input::AppendComplete { op: 1 }, &mut out);
        assert_eq!(node.get(b"a"), Some(&b"1"[..]), "the first write lands");
        node.on_input(Input::AppendComplete { op: 2 }, &mut out);
        assert_eq!(
            replies(&out),
            vec![Response::PutOk, Response::PutOk],
            "acknowledged either way"
        );
        let expected = if bug.is_some() { "1" } else { "2" };
        assert_eq!(node.get(b"a"), Some(expected.as_bytes()), "{bug:?}");
        assert_eq!(node.log_seq(), 2, "the log is right in both cases");

        // A delete behind a put: the key stays.
        node.on_input(req(1, 2, put("b", "1")), &mut out);
        node.on_input(req(2, 2, del("b")), &mut out);
        out.clear();
        node.on_input(Input::AppendComplete { op: 3 }, &mut out);
        node.on_input(Input::AppendComplete { op: 4 }, &mut out);
        assert_eq!(
            replies(&out),
            vec![Response::PutOk, Response::DeleteOk { existed: true }],
            "{bug:?}"
        );
        assert_eq!(node.get(b"b").is_some(), bug.is_some(), "{bug:?}");

        // One client never has two writes in flight, so it never sees it.
        node.on_input(req(1, 3, put("c", "1")), &mut out);
        node.on_input(Input::AppendComplete { op: 5 }, &mut out);
        node.on_input(req(1, 4, put("c", "2")), &mut out);
        node.on_input(Input::AppendComplete { op: 6 }, &mut out);
        assert_eq!(node.get(b"c"), Some(&b"2"[..]));
    }
}

#[test]
fn stale_read_after_restart_leaves_the_last_record_unapplied() {
    let (mut node, mut out) = started(None);
    node.on_input(req(1, 1, put("a", "1")), &mut out);
    node.on_input(Input::AppendComplete { op: 1 }, &mut out);
    node.on_input(req(1, 2, put("b", "2")), &mut out);
    node.on_input(Input::AppendComplete { op: 2 }, &mut out);
    let log = log_of(&out);

    for bug in [None, Some(Bug::StaleReadAfterRestart)] {
        let mut out = Vec::new();
        let mut node = Node::new(&mut out);
        if let Some(bug) = bug {
            node.bugs_mut().plant(bug);
        }
        node.on_input(
            Input::ReadComplete {
                op: 0,
                data: log.clone(),
            },
            &mut out,
        );
        assert_eq!(node.stats().recovered_entries, 2);
        assert_eq!(node.get(b"a"), Some(&b"1"[..]));
        let expected = if bug.is_some() { None } else { Some(&b"2"[..]) };
        assert_eq!(node.get(b"b"), expected, "{bug:?}");
        // The session came back, so the retry is answered as if applied.
        out.clear();
        node.on_input(req(1, 2, put("b", "2")), &mut out);
        assert_eq!(replies(&out), vec![Response::PutOk]);
        assert_eq!(node.stats().duplicates, 1);
        // And the next append goes after the record that was skipped.
        out.clear();
        node.on_input(req(1, 3, put("c", "3")), &mut out);
        assert!(matches!(&out[0], Output::Append { offset, .. } if *offset == 1024));
    }
}

#[test]
fn duplicate_apply_on_retry_runs_a_finished_request_again() {
    for bug in [None, Some(Bug::DuplicateApplyOnRetry)] {
        let (mut node, mut out) = started(bug);
        node.on_input(req(1, 1, put("k", "v")), &mut out);
        node.on_input(Input::AppendComplete { op: 1 }, &mut out);
        node.on_input(req(1, 2, del("k")), &mut out);
        node.on_input(Input::AppendComplete { op: 2 }, &mut out);
        out.clear();
        // The reply was lost; the client resends.
        node.on_input(req(1, 2, del("k")), &mut out);
        assert_eq!(node.stats().duplicates, 1);
        if bug.is_some() {
            assert_eq!(node.pending_writes(), 1, "appended again");
            node.on_input(Input::AppendComplete { op: 3 }, &mut out);
            assert_eq!(replies(&out), vec![Response::DeleteOk { existed: false }]);
            assert_eq!(node.log_seq(), 3);
        } else {
            assert_eq!(replies(&out), vec![Response::DeleteOk { existed: true }]);
            assert_eq!(node.log_seq(), 2);
        }
        // A retry of a request still in flight is absorbed either way.
        node.on_input(req(1, 3, put("k", "w")), &mut out);
        out.clear();
        node.on_input(req(1, 3, put("k", "w")), &mut out);
        assert!(out.is_empty());
        assert_eq!(node.pending_writes(), 1);
    }
}
