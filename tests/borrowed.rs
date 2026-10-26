//! Borrowed-read lifetime, publication, rollback, and reclamation regressions.
use livegraph::{Error, GraphBuilder, LiveGraph, Node, Result, Snapshot};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Barrier, mpsc},
    thread,
    time::Duration,
};

#[test]
fn callback_runs_once_accepts_fn_once_and_returns_owned_results() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let count = builder.resource("count", 7_u64);
    let graph = builder.build()?;
    let mut calls = 0;
    let owned = String::from("moved result");
    let result = graph.with_snapshot(|snapshot| -> Result<_> {
        calls += 1;
        Ok((owned, *snapshot.get(count)?))
    })?;
    assert_eq!(calls, 1);
    assert_eq!(result, (String::from("moved result"), 7));
    let owned_snapshot: Arc<Snapshot> = graph.snapshot();
    assert_eq!(*owned_snapshot.get(count)?, 7);
    let foreign = GraphBuilder::new().resource("foreign", 0_u64);
    assert_eq!(
        graph.with_snapshot(|s| s.get(foreign).copied()),
        Err(Error::ForeignGraph)
    );
    Ok(())
}

#[test]
fn writer_publishes_while_borrowed_references_stay_valid_and_then_reclaim() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let initial = Arc::new(String::from("old"));
    let weak = Arc::downgrade(&initial);
    let text = builder.resource("text", initial);
    let length = builder.derived("length", [text.dep()], move |cx| Ok(cx.get(text)?.len()))?;
    let graph = Arc::new(builder.build()?);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let read_graph = graph.clone();
    let reader = thread::spawn(move || {
        read_graph.with_snapshot(|snapshot| -> Result<()> {
            let before = snapshot.get(text)?.as_str();
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(before, "old");
            assert_eq!(*snapshot.get(length)?, 3);
            assert_eq!(snapshot.version(), 0);
            Ok(())
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (published_tx, published_rx) = mpsc::channel();
    let write_graph = graph.clone();
    let writer = thread::spawn(move || {
        let mut changes = write_graph.changes();
        changes
            .set(text, Arc::new(String::from("new content")))
            .unwrap();
        published_tx.send(write_graph.commit(changes)).unwrap();
    });
    let result = published_rx.recv_timeout(Duration::from_secs(2));
    let retained = weak.upgrade().is_some();
    // Always release the reader before asserting, so a lock regression cannot strand it.
    release_tx.send(()).unwrap();
    reader.join().unwrap()?;
    writer.join().unwrap();
    let new = result.expect("writer blocked by borrowed reader")?;
    assert!(retained);
    assert!(weak.upgrade().is_none());
    assert_eq!(new.version(), 1);
    assert_eq!(*new.get(length)?, 11);
    assert_eq!(graph.with_snapshot(|s| s.get(length).copied())?, 11);
    Ok(())
}

#[test]
fn failed_build_and_validation_leave_borrowed_and_owned_versions_unchanged() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let input = builder.resource("input", 1_i32);
    let derived = builder.derived("derived", [input.dep()], move |cx| {
        if *cx.get(input)? < 0 {
            return Err("negative input".into());
        }
        Ok(cx.get(input)? * 2)
    })?;
    builder.validate("limit", move |s| {
        if *s.get(input)? > 10 {
            Err("too large".into())
        } else {
            Ok(())
        }
    });
    let graph = builder.build()?;
    let old = graph.snapshot();
    graph.with_snapshot(|snapshot| -> Result<()> {
        for bad in [-1, 11] {
            let mut changes = graph.changes();
            changes.set(input, bad)?;
            assert!(graph.commit(changes).is_err());
            assert_eq!(snapshot.version(), 0);
            assert_eq!((*snapshot.get(input)?, *snapshot.get(derived)?), (1, 2));
            assert!(Arc::ptr_eq(&old, &graph.snapshot()));
        }
        Ok(())
    })?;
    Ok(())
}

#[test]
fn panic_releases_borrow_without_poisoning_writers() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let value = Arc::new(String::from("old"));
    let weak = Arc::downgrade(&value);
    let input = builder.resource("input", value);
    let graph = builder.build()?;
    let panic = catch_unwind(AssertUnwindSafe(|| {
        graph.with_snapshot(|snapshot| {
            assert_eq!(snapshot.get(input).unwrap().as_str(), "old");
            panic!("deliberate read panic");
        })
    }));
    assert!(panic.is_err());
    assert_eq!(graph.snapshot().version(), 0);
    let mut changes = graph.changes();
    changes.set(input, Arc::new(String::from("new")))?;
    graph.commit(changes)?;
    assert!(weak.upgrade().is_none());
    assert_eq!(graph.with_snapshot(|s| s.get(input).unwrap().len()), 3);
    Ok(())
}

#[test]
fn nested_borrows_remain_consistent_when_fast_slots_are_exhausted() -> Result<()> {
    fn nested(graph: &LiveGraph, input: Node<u64>, depth: usize) -> Result<()> {
        graph.with_snapshot(|snapshot| {
            assert_eq!(snapshot.version(), 0);
            if depth > 0 {
                nested(graph, input, depth - 1)?;
            } else {
                let mut changes = graph.changes();
                changes.set(input, 2)?;
                graph.commit(changes)?;
            }
            assert_eq!(*snapshot.get(input)?, 1);
            assert_eq!(snapshot.version(), 0);
            Ok(())
        })
    }
    let mut builder = GraphBuilder::new();
    let input = builder.resource("input", 1_u64);
    let graph = builder.build()?;
    nested(&graph, input, 64)?;
    assert_eq!(graph.with_snapshot(|s| s.get(input).copied())?, 2);
    Ok(())
}

#[test]
fn checked_batch_can_leave_borrow_but_rejects_stale_values() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let input = builder.resource("input", 1_u64);
    let graph = builder.build()?;
    let stale = graph.with_snapshot(|snapshot| -> Result<_> {
        let mut changes = graph.changes_from(snapshot)?;
        changes.set(input, snapshot.get(input)? + 1)?;
        Ok(changes)
    })?;
    let mut intervening = graph.changes();
    intervening.set(input, 10)?;
    graph.commit(intervening)?;
    assert_eq!(
        graph.commit(stale).unwrap_err(),
        Error::Conflict {
            expected: 0,
            actual: 1
        }
    );
    assert_eq!(graph.with_snapshot(|s| s.get(input).copied())?, 10);
    Ok(())
}

#[test]
fn borrowed_readers_never_tear_with_two_writers() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let a = builder.resource("a", 0_u64);
    let b = builder.resource("b", 0_u64);
    let sum = builder.derived("sum", [a.dep(), b.dep()], move |cx| {
        Ok(cx.get(a)? + cx.get(b)?)
    })?;
    let graph = builder.build()?;
    let barrier = Barrier::new(6);
    thread::scope(|scope| {
        for node in [a, b] {
            let graph = &graph;
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                for i in 1..=500 {
                    let mut changes = graph.changes();
                    changes.set(node, i).unwrap();
                    graph.commit(changes).unwrap();
                    thread::yield_now();
                }
            });
        }
        for _ in 0..4 {
            scope.spawn(|| {
                barrier.wait();
                let mut previous = 0;
                for _ in 0..3000 {
                    graph.with_snapshot(|s| {
                        assert_eq!(*s.get(sum).unwrap(), s.get(a).unwrap() + s.get(b).unwrap());
                        assert!(s.version() >= previous);
                        previous = s.version();
                    });
                    thread::yield_now();
                }
            });
        }
    });
    assert_eq!(
        graph.with_snapshot(|s| (*s.get(sum).unwrap(), s.version())),
        (1000, 1000)
    );
    Ok(())
}
