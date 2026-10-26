//! Candidate access across complete and partial snapshot replacement.
use livegraph::{Error, GraphBuilder, Result};
use std::sync::Arc;

#[test]
fn full_and_partial_candidates_preserve_a_tail_block_and_forward_join_on_failure() -> Result<()> {
    let mut b = GraphBuilder::new();
    let total = b.reserve::<i64>("forward join");
    let roots: Vec<_> = (0..18)
        .map(|i| b.resource(format!("r{i}"), 0_i64))
        .collect();
    let mut leaves = Vec::new();
    for (i, &root) in roots.iter().enumerate() {
        leaves.push(b.derived(format!("d{i}"), [root.dep()], move |cx| {
            Ok(*cx.get(root)? * 3)
        })?);
    }
    let inputs = leaves.clone();
    b.define_derived(total, leaves.iter().map(|n| n.dep()), move |cx| {
        let sum = inputs
            .iter()
            .try_fold(0, |sum, &node| -> Result<_> { Ok(sum + cx.get(node)?) })?;
        if sum < 0 {
            Err("late failure".into())
        } else {
            Ok(sum)
        }
    })?;
    let graph = b.build()?; // 37 nodes: the last block has only five real slots.
    let mut expected = [0_i64; 18];
    let mut history = Vec::new();
    for (step, count) in [18, 1, 18, 9, 1, 18].into_iter().enumerate() {
        let old = graph.snapshot();
        history.push((old.clone(), expected));
        let mut bad = graph.changes_from(&old)?;
        for &root in roots.iter().take(count) {
            bad.set(root, -100_000)?;
        }
        assert!(matches!(graph.commit(bad), Err(Error::Build { .. })));
        assert!(Arc::ptr_eq(&old, &graph.snapshot()));
        let mut changes = graph.changes_from(&old)?;
        for i in 0..count {
            expected[i] = (step + i + 1) as i64;
            changes.set(roots[i], expected[i])?;
        }
        let current = graph.commit(changes)?;
        assert_eq!(*current.get(total)?, expected.iter().sum::<i64>() * 3);
        for (snapshot, values) in &history {
            for (i, &root) in roots.iter().enumerate() {
                assert_eq!(*snapshot.get(root)?, values[i]);
                assert_eq!(*snapshot.get(leaves[i])?, values[i] * 3);
            }
            assert_eq!(*snapshot.get(total)?, values.iter().sum::<i64>() * 3);
        }
        graph.with_snapshot(|snapshot| {
            assert_eq!(
                *snapshot.get(total).unwrap(),
                expected.iter().sum::<i64>() * 3
            );
        });
    }
    Ok(())
}

#[test]
fn dependency_checks_remain_identical_for_full_and_partial_candidates() -> Result<()> {
    let mut other = GraphBuilder::new();
    let foreign = other.resource("foreign", 0_i32);
    let mut b = GraphBuilder::new();
    let trigger = b.resource("trigger", 0_i32);
    let hidden = b.resource("hidden", 0_i32);
    let value = b.derived("checked", [trigger.dep()], move |cx| {
        assert!(matches!(cx.get(foreign), Err(Error::ForeignGraph)));
        assert!(matches!(
            cx.get(hidden),
            Err(Error::UndeclaredDependency(_))
        ));
        Ok(*cx.get(trigger)?)
    })?;
    let graph = b.build()?;
    for (i, full) in [true, false, true, false].into_iter().enumerate() {
        let mut changes = graph.changes();
        changes.set(trigger, i as i32 + 1)?;
        if full {
            changes.set(hidden, 100)?;
        }
        let current = graph.commit(changes)?;
        assert_eq!(*current.get(value)?, i as i32 + 1);
    }
    Ok(())
}

#[test]
fn failed_full_candidate_releases_payloads_after_the_commit_guard() -> Result<()> {
    use std::sync::{
        OnceLock, Weak,
        atomic::{AtomicUsize, Ordering},
    };
    struct Probe {
        graph: Arc<OnceLock<Weak<livegraph::LiveGraph>>>,
        dropped: Arc<AtomicUsize>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            if let Some(graph) = self.graph.get().and_then(Weak::upgrade) {
                // Empty commits still acquire the writer lock and recursion guard.
                assert!(graph.commit(graph.changes()).is_ok());
                self.dropped.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
    let holder = Arc::new(OnceLock::new());
    let dropped = Arc::new(AtomicUsize::new(0));
    let probe = || Probe {
        graph: holder.clone(),
        dropped: dropped.clone(),
    };
    let mut b = GraphBuilder::new();
    let payload = b.resource("payload", probe());
    let fail = b.resource("fail", false);
    b.derived("derived", [fail.dep()], move |cx| {
        if *cx.get(fail)? {
            Err("late failure".into())
        } else {
            Ok(())
        }
    })?;
    let graph = Arc::new(b.build()?);
    holder.set(Arc::downgrade(&graph)).unwrap();
    let old = graph.snapshot();
    let mut changes = graph.changes();
    changes.set(payload, probe())?.set(fail, true)?; // Every node is affected.
    assert!(matches!(graph.commit(changes), Err(Error::Build { .. })));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&old, &graph.snapshot()));
    let mut recovery = graph.changes();
    recovery.set(payload, probe())?.set(fail, false)?;
    graph.commit(recovery)?;
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    drop(old);
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
    Ok(())
}
