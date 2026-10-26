//! Concurrent publication and reader isolation.

use livegraph::{GraphBuilder, Result};
use std::{
    sync::{Arc, Barrier, Mutex, mpsc},
    thread,
    time::Duration,
};

#[test]
fn readers_keep_old_snapshot_while_candidate_is_blocked() -> Result<()> {
    let mut b = GraphBuilder::new();
    let input = b.resource("input", 1);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let double = b.derived("double", [input.dep()], move |cx| {
        let n = *cx.get(input)?;
        if n == 2 {
            entered_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        Ok(n * 2)
    })?;
    let graph = Arc::new(b.build()?);
    let pinned = graph.snapshot();
    let writer_graph = graph.clone();
    let writer = thread::spawn(move || {
        let mut changes = writer_graph.changes();
        changes.set(input, 2)?;
        writer_graph.commit(changes)
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let reader_graph = graph.clone();
    let (read_tx, read_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let s = reader_graph.snapshot();
        read_tx
            .send((*s.get(input).unwrap(), *s.get(double).unwrap(), s.version()))
            .unwrap();
    });
    // Release even if the read times out, so a regression cannot strand the writer.
    let seen = read_rx.recv_timeout(Duration::from_secs(2));
    release_tx.send(()).unwrap();
    reader.join().unwrap();
    let new = writer.join().unwrap()?;
    assert_eq!(seen.unwrap(), (1, 2, 0));
    assert_eq!((*new.get(input)?, *new.get(double)?), (2, 4));
    assert_eq!((*pinned.get(input)?, *pinned.get(double)?), (1, 2));
    Ok(())
}

#[test]
fn concurrent_writers_do_not_lose_disjoint_updates_and_readers_never_tear() -> Result<()> {
    let mut b = GraphBuilder::new();
    let a = b.resource("a", 0_u64);
    let z = b.resource("z", 0_u64);
    let sum = b.derived("sum", [a.dep(), z.dep()], move |cx| {
        Ok(cx.get(a)? + cx.get(z)?)
    })?;
    let graph = Arc::new(b.build()?);
    let start = Arc::new(Barrier::new(6));
    thread::scope(|scope| {
        for node in [a, z] {
            let graph = graph.clone();
            let start = start.clone();
            scope.spawn(move || {
                start.wait();
                for i in 1..=500 {
                    let mut changes = graph.changes();
                    changes.set(node, i).unwrap();
                    graph.commit(changes).unwrap();
                    thread::yield_now();
                }
            });
        }
        for _ in 0..4 {
            let graph = graph.clone();
            let start = start.clone();
            scope.spawn(move || {
                start.wait();
                let mut previous = 0;
                for _ in 0..3000 {
                    let s = graph.snapshot();
                    assert_eq!(*s.get(sum).unwrap(), s.get(a).unwrap() + s.get(z).unwrap());
                    assert!(s.version() >= previous);
                    previous = s.version();
                    thread::yield_now();
                }
            });
        }
    });
    let s = graph.snapshot();
    assert_eq!((*s.get(a)?, *s.get(z)?, *s.get(sum)?), (500, 500, 1000));
    assert_eq!(s.version(), 1000);
    Ok(())
}

#[test]
fn checked_read_modify_write_retries_without_losing_increments() -> Result<()> {
    use livegraph::Error;
    let mut b = GraphBuilder::new();
    let count = b.resource("count", 0_u64);
    let g = b.build()?;
    let start = Barrier::new(4);
    thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                // Force all writers to stage the same first version.
                let initial = g.snapshot();
                start.wait();
                let mut snapshot = initial;
                for _ in 0..100 {
                    loop {
                        let mut changes = g.changes_from(&snapshot).unwrap();
                        changes
                            .set(count, snapshot.get(count).unwrap() + 1)
                            .unwrap();
                        match g.commit(changes) {
                            Ok(new) => {
                                snapshot = new;
                                break;
                            }
                            Err(Error::Conflict { .. }) => snapshot = g.snapshot(),
                            Err(e) => panic!("unexpected error: {e}"),
                        }
                    }
                }
            });
        }
    });
    let snapshot = g.snapshot();
    assert_eq!(*snapshot.get(count)?, 400);
    assert_eq!(snapshot.version(), 400);
    Ok(())
}

#[test]
fn recursive_compute_and_validation_commits_return_errors_instead_of_hanging() {
    use livegraph::{Error, LiveGraph};
    use std::sync::{OnceLock, Weak};
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || -> Result<()> {
        for validation in [false, true] {
            let mut b = GraphBuilder::new();
            let x = b.resource("x", 0);
            let holder = Arc::new(OnceLock::<Weak<LiveGraph>>::new());
            let h = holder.clone();
            let recurse = move || -> Result<()> {
                let graph = h.get().unwrap().upgrade().unwrap();
                graph.commit(graph.changes()).map(|_| ())
            };
            if validation {
                b.validate("reentrant validator", move |s| {
                    if *s.get(x)? == 1 {
                        recurse()?;
                    }
                    Ok(())
                });
            } else {
                b.derived("reentrant compute", [x.dep()], move |cx| {
                    if *cx.get(x)? == 1 {
                        recurse()?;
                    }
                    Ok(())
                })?;
            }
            let graph = Arc::new(b.build()?);
            holder.set(Arc::downgrade(&graph)).unwrap();
            let old = graph.snapshot();
            let mut changes = graph.changes();
            changes.set(x, 1)?;
            let error = graph.commit(changes).unwrap_err();
            match error {
                Error::Build { reason, .. } | Error::Validation { reason, .. } => {
                    assert!(reason.contains("recursive commit"));
                }
                e => panic!("unexpected: {e}"),
            }
            assert!(Arc::ptr_eq(&old, &graph.snapshot()));
            let mut changes = graph.changes();
            changes.set(x, 2)?;
            assert_eq!(graph.commit(changes)?.version(), 1);
        }
        done_tx.send(()).unwrap();
        Ok(())
    });
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("recursive commit hung");
    worker.join().unwrap().unwrap();
}

#[test]
fn failed_writer_does_not_leak_candidates_into_next_writer() -> Result<()> {
    let mut b = GraphBuilder::new();
    let x = b.resource("x", 1);
    let y = b.resource("y", 2);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let sum = b.derived("sum", [x.dep(), y.dep()], move |cx| {
        if *cx.get(x)? == 99 {
            entered_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            return Err("reject candidate".into());
        }
        Ok(cx.get(x)? + cx.get(y)?)
    })?;
    let graph = Arc::new(b.build()?);
    let old = graph.snapshot();
    let g = graph.clone();
    let failing = thread::spawn(move || {
        let mut changes = g.changes();
        changes.set(x, 99).unwrap();
        g.commit(changes).unwrap_err()
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let g = graph.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let succeeding = thread::spawn(move || {
        let mut changes = g.changes();
        changes.set(y, 10).unwrap();
        started_tx.send(()).unwrap();
        g.commit(changes).unwrap()
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(Arc::ptr_eq(&old, &graph.snapshot()));
    release_tx.send(()).unwrap();
    assert!(matches!(
        failing.join().unwrap(),
        livegraph::Error::Build { .. }
    ));
    let new = succeeding.join().unwrap();
    assert_eq!((*new.get(x)?, *new.get(y)?, *new.get(sum)?), (1, 10, 11));
    assert_eq!(new.version(), 1);
    Ok(())
}
