//! Adaptive strategy transitions retain exact invalidation and rollback semantics.
use livegraph::{Error, GraphBuilder, Result};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn alternating_sparse_dense_and_threshold_updates_preserve_history_and_callback_counts()
-> Result<()> {
    let mut builder = GraphBuilder::new();
    let constant = builder.derived("constant", [], |_| Ok(String::from("stable")))?;
    let roots: Vec<_> = (0..64)
        .map(|i| builder.resource(format!("r{i}"), 0_i64))
        .collect();
    let calls = Arc::new((0..64).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
    let mut derived = Vec::new();
    for (i, &root) in roots.iter().enumerate() {
        let calls = calls.clone();
        derived.push(builder.derived(format!("d{i}"), [root.dep()], move |cx| {
            calls[i].fetch_add(1, Ordering::Relaxed);
            Ok(cx.get(root)? * 2)
        })?);
    }
    let graph = builder.build()?;
    let mut expected = [0_i64; 64];
    let mut history = Vec::new();
    for (step, count) in [1, 32, 33, 64, 1, 64, 33, 32, 1].into_iter().enumerate() {
        history.push((graph.snapshot(), expected));
        let old_calls: Vec<_> = calls.iter().map(|c| c.load(Ordering::Relaxed)).collect();
        let mut changes = graph.changes();
        for i in 0..count {
            expected[i] = step as i64 * 100 + i as i64 + 1;
            changes.set(roots[i], expected[i])?;
        }
        let new = graph.commit(changes)?;
        assert_eq!(new.version(), step as u64 + 1);
        for i in 0..64 {
            assert_eq!(*new.get(derived[i])?, expected[i] * 2);
            assert_eq!(
                calls[i].load(Ordering::Relaxed) - old_calls[i],
                usize::from(i < count)
            );
        }
        for (old, values) in &history {
            assert_eq!(old.get(constant)?, "stable");
            assert!(std::ptr::eq(old.get(constant)?, new.get(constant)?));
            for i in 0..64 {
                assert_eq!(*old.get(roots[i])?, values[i]);
                assert_eq!(*old.get(derived[i])?, values[i] * 2);
            }
        }
    }
    Ok(())
}

#[test]
fn build_panic_and_validation_failures_roll_back_both_strategies_and_recover() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let roots: Vec<_> = (0..64)
        .map(|i| builder.resource(format!("r{i}"), 0_i64))
        .collect();
    let mut derived = Vec::new();
    for (i, &root) in roots.iter().enumerate() {
        derived.push(builder.derived(format!("d{i}"), [root.dep()], move |cx| {
            let value = *cx.get(root)?;
            if value == -1 {
                return Err("build failed".into());
            }
            assert_ne!(value, -2, "deliberate callback panic");
            Ok(value * 2)
        })?);
    }
    let checked = roots.clone();
    builder.validate("sentinel", move |s| {
        for &root in &checked {
            if *s.get(root)? == 7777 {
                return Err("validation failed".into());
            }
        }
        Ok(())
    });
    let graph = builder.build()?;
    for count in [1, 64, 1] {
        for bad in [-1, -2, 7777] {
            let old = graph.snapshot();
            let mut changes = graph.changes();
            for &root in roots.iter().take(count) {
                changes.set(root, 123)?;
            }
            changes.set(roots[count - 1], bad)?;
            let error = graph.commit(changes).unwrap_err();
            match bad {
                -1 => assert!(matches!(error, Error::Build { .. })),
                -2 => assert!(matches!(error, Error::CallbackPanicked(_))),
                _ => assert!(matches!(error, Error::Validation { .. })),
            }
            assert!(Arc::ptr_eq(&old, &graph.snapshot()));
            let mut recovery = graph.changes_from(&old)?;
            recovery.set(roots[0], old.version() as i64 + 1)?;
            let new = graph.commit(recovery)?;
            for i in 0..64 {
                assert_eq!(*new.get(derived[i])?, new.get(roots[i])? * 2);
                if i > 0 {
                    assert_eq!(new.get(roots[i])?, old.get(roots[i])?);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn readers_remain_consistent_while_writer_alternates_strategies() -> Result<()> {
    use std::{sync::Barrier, thread};
    let mut builder = GraphBuilder::new();
    let roots: Vec<_> = (0..64)
        .map(|i| builder.resource(format!("r{i}"), 0_u64))
        .collect();
    let derived: Vec<_> = roots
        .iter()
        .enumerate()
        .map(|(i, &root)| {
            builder
                .derived(format!("d{i}"), [root.dep()], move |cx| {
                    Ok(cx.get(root)? * 2)
                })
                .unwrap()
        })
        .collect();
    let graph = builder.build()?;
    let original = graph.snapshot();
    let barrier = Barrier::new(5);
    thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            for generation in 1..=300 {
                let count = if generation % 2 == 0 { 64 } else { 1 };
                let mut changes = graph.changes();
                for &root in roots.iter().take(count) {
                    changes.set(root, generation).unwrap();
                }
                graph.commit(changes).unwrap();
                thread::yield_now();
            }
        });
        for _ in 0..4 {
            scope.spawn(|| {
                barrier.wait();
                let mut previous = 0;
                let mut check = |snapshot: &livegraph::Snapshot| {
                    for i in 0..64 {
                        assert_eq!(
                            *snapshot.get(derived[i]).unwrap(),
                            snapshot.get(roots[i]).unwrap() * 2
                        );
                    }
                    assert!(snapshot.version() >= previous);
                    previous = snapshot.version();
                };
                for i in 0..1000 {
                    if i % 2 == 0 {
                        graph.with_snapshot(&mut check);
                    } else {
                        check(&graph.snapshot());
                    }
                    thread::yield_now();
                }
            });
        }
    });
    let final_snapshot = graph.snapshot();
    assert_eq!(final_snapshot.version(), 300);
    for &node in &derived {
        assert_eq!(*final_snapshot.get(node)?, 600);
        assert_eq!(*original.get(node)?, 0);
    }
    Ok(())
}
