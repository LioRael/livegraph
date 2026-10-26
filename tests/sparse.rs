//! Chunk boundary and affected-only scheduling checks against an independent evaluator.
use livegraph::{GraphBuilder, Result};
use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

fn dependencies(i: usize, count: usize) -> Vec<usize> {
    let mut deps = Vec::new();
    if !i.is_multiple_of(4) && i + 1 < count {
        deps.push(i + 1);
    }
    if i.is_multiple_of(3) && i + 7 < count {
        deps.push(i + 7);
    }
    deps
}

fn reference(roots: &[u64], count: usize) -> Vec<u64> {
    let mut values = vec![0_u64; count];
    for i in (0..count).rev() {
        values[i] = roots[i % roots.len()];
        for dependency in dependencies(i, count) {
            values[i] = values[i].wrapping_add(values[dependency]);
        }
    }
    values
}

#[test]
fn sparse_forward_dag_matches_full_evaluator_across_failures_and_retries() -> Result<()> {
    const COUNT: usize = 137;
    let mut builder = GraphBuilder::new();
    let mut root_values = vec![1_u64; 8];
    let roots: Vec<_> = (0..8)
        .map(|i| builder.resource(format!("root-{i}"), 1_u64))
        .collect();
    let nodes: Vec<_> = (0..COUNT)
        .map(|i| builder.reserve::<u64>(format!("derived-{i}")))
        .collect();
    let calls = Arc::new((0..COUNT).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
    for i in 0..COUNT {
        let root = roots[i % roots.len()];
        let deps: Vec<_> = dependencies(i, COUNT).iter().map(|&d| nodes[d]).collect();
        let declared: Vec<_> = std::iter::once(root.dep())
            .chain(deps.iter().map(|n| n.dep()))
            .collect();
        let calls = calls.clone();
        builder.define_derived(nodes[i], declared, move |cx| {
            calls[i].fetch_add(1, Ordering::Relaxed);
            let mut value = *cx.get(root)?;
            if value == u64::MAX {
                return Err("invalid resource".into());
            }
            for &dependency in &deps {
                value = value.wrapping_add(*cx.get(dependency)?);
            }
            Ok(value)
        })?;
    }
    let validate_root = roots[7];
    builder.validate("sentinel", move |s| {
        if *s.get(validate_root)? == 7777 {
            Err("rejected snapshot".into())
        } else {
            Ok(())
        }
    });
    let graph = builder.build()?;
    for step in 0..24 {
        let old = graph.snapshot();
        let old_expected = reference(&root_values, COUNT);
        if step == 5 || step == 9 {
            let mut bad = graph.changes();
            bad.set(
                roots[if step == 5 { 0 } else { 7 }],
                if step == 5 { u64::MAX } else { 7777 },
            )?;
            assert!(graph.commit(bad).is_err());
            assert!(Arc::ptr_eq(&old, &graph.snapshot()));
        }
        let mut changed_roots = [false; 8];
        let mut changes = graph.changes();
        for i in [step % 8, (step + 3) % 8] {
            changed_roots[i] = true;
            root_values[i] += step as u64 + 1;
            changes.set(roots[i], root_values[i])?;
        }
        let previous_calls: Vec<_> = calls.iter().map(|c| c.load(Ordering::Relaxed)).collect();
        let mut affected = [false; COUNT];
        for i in (0..COUNT).rev() {
            affected[i] =
                changed_roots[i % 8] || dependencies(i, COUNT).iter().any(|&d| affected[d]);
        }
        let new = graph.commit(changes)?;
        let expected = reference(&root_values, COUNT);
        for i in 0..COUNT {
            assert_eq!(*new.get(nodes[i])?, expected[i], "node {i}, step {step}");
            assert_eq!(*old.get(nodes[i])?, old_expected[i]);
            assert_eq!(
                calls[i].load(Ordering::Relaxed) - previous_calls[i],
                usize::from(affected[i]),
                "callback {i}, step {step}"
            );
        }
        assert_eq!(new.version(), step as u64 + 1);
    }
    Ok(())
}

#[test]
fn multiple_chunks_are_atomic_for_owned_and_borrowed_readers() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let nodes: Vec<_> = (0..200)
        .map(|i| builder.resource(format!("root-{i}"), 0_u64))
        .collect();
    let a = nodes[0];
    let b = nodes[199];
    let independent = nodes[198];
    let sum = builder.derived("sum", [a.dep(), b.dep()], move |cx| {
        Ok(cx.get(a)? + cx.get(b)?)
    })?;
    let graph = builder.build()?;
    let old = graph.snapshot();
    let barrier = Barrier::new(6);
    thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            for i in 1..=500 {
                let mut changes = graph.changes();
                changes.set(a, i).unwrap().set(b, i * 2).unwrap();
                graph.commit(changes).unwrap();
                thread::yield_now();
            }
        });
        scope.spawn(|| {
            barrier.wait();
            for i in 1..=500 {
                let mut changes = graph.changes();
                changes.set(independent, i).unwrap();
                graph.commit(changes).unwrap();
                thread::yield_now();
            }
        });
        for _ in 0..4 {
            scope.spawn(|| {
                barrier.wait();
                let mut previous = 0;
                let mut check = |s: &livegraph::Snapshot| {
                    assert_eq!(*s.get(b).unwrap(), s.get(a).unwrap() * 2);
                    assert_eq!(*s.get(sum).unwrap(), s.get(a).unwrap() * 3);
                    assert!(s.version() >= previous);
                    previous = s.version();
                };
                for i in 0..3000 {
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
    let new = graph.snapshot();
    assert_eq!(
        (*new.get(sum)?, *new.get(independent)?, new.version()),
        (1500, 500, 1000)
    );
    assert_eq!((*old.get(a)?, *old.get(b)?, *old.get(sum)?), (0, 0, 0));
    Ok(())
}

#[test]
fn untouched_constant_in_a_changed_block_survives_dense_updates() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let anchor = builder.derived("constant", [], move |_| {
        counted.fetch_add(1, Ordering::Relaxed);
        Ok(String::from("unchanged"))
    })?;
    let roots: Vec<_> = (0..67)
        .map(|i| builder.resource(format!("resource-{i}"), i as u64))
        .collect();
    let first = roots[0];
    let last = roots[66];
    let sum = builder.derived("sum", [first.dep(), last.dep()], move |cx| {
        Ok(cx.get(first)? + cx.get(last)?)
    })?;
    let graph = builder.build()?;
    let old = graph.snapshot();
    let mut changes = graph.changes();
    for (i, &root) in roots.iter().enumerate() {
        changes.set(root, 100 + i as u64)?;
    }
    let new = graph.commit(changes)?;
    assert_eq!(new.get(anchor)?, "unchanged");
    assert!(std::ptr::eq(old.get(anchor)?, new.get(anchor)?));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(*old.get(sum)?, 66);
    assert_eq!(*new.get(sum)?, 266);
    Ok(())
}
