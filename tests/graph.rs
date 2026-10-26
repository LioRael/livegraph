//! Graph semantics and failure isolation.

use livegraph::{Error, GraphBuilder, Result};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn diamond_recomputes_once_and_reuses_unrelated_values() -> Result<()> {
    let mut b = GraphBuilder::new();
    let root = b.resource("root", 2);
    let other = b.resource("other", String::from("untouched"));
    let other_calls = Arc::new(AtomicUsize::new(0));
    let c = other_calls.clone();
    let independent = b.derived("independent", [other.dep()], move |cx| {
        c.fetch_add(1, Ordering::SeqCst);
        Ok(cx.get(other)?.len())
    })?;
    let left = b.derived("left", [root.dep(), root.dep()], move |cx| {
        Ok(cx.get(root)? + 1)
    })?;
    let right = b.derived("right", [root.dep()], move |cx| Ok(cx.get(root)? * 3))?;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let join = b.derived("join", [left.dep(), right.dep()], move |cx| {
        c.fetch_add(1, Ordering::SeqCst);
        Ok(cx.get(left)? + cx.get(right)?)
    })?;
    let g = b.build()?;
    let old = g.snapshot();
    let mut update = g.changes();
    update.set(root, 5)?;
    let new = g.commit(update)?;
    assert_eq!(*old.get(join)?, 9);
    assert_eq!(*new.get(join)?, 21);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(other_calls.load(Ordering::SeqCst), 1);
    assert!(std::ptr::eq(old.get(other)?, new.get(other)?));
    assert!(std::ptr::eq(old.get(independent)?, new.get(independent)?));
    assert_eq!(new.version(), 1);
    Ok(())
}

#[test]
fn batch_is_validated_as_a_whole_and_empty_batch_is_noop() -> Result<()> {
    let mut b = GraphBuilder::new();
    let min = b.resource("min", 1);
    let max = b.resource("max", 2);
    b.validate("range", move |s| {
        if s.get(min)? > s.get(max)? {
            Err("min exceeds max".into())
        } else {
            Ok(())
        }
    });
    let g = b.build()?;
    let old = g.snapshot();
    assert!(Arc::ptr_eq(&old, &g.commit(g.changes())?));
    let mut bad = g.changes();
    bad.set(min, 10)?;
    assert!(matches!(g.commit(bad), Err(Error::Validation { .. })));
    assert!(Arc::ptr_eq(&old, &g.snapshot()));
    let mut good = g.changes();
    good.set(min, 5)?.set(min, 10)?.set(max, 20)?;
    let new = g.commit(good)?;
    assert_eq!((*new.get(min)?, *new.get(max)?), (10, 20));
    assert_eq!(new.version(), 1);
    Ok(())
}

#[test]
fn failed_build_rolls_back_all_resources_and_intermediate_nodes() -> Result<()> {
    let mut b = GraphBuilder::new();
    let x = b.resource("x", 2);
    let y = b.resource("y", 3);
    let intermediate = b.derived("intermediate", [x.dep()], move |cx| Ok(cx.get(x)? * 2))?;
    let end = b.derived("end", [intermediate.dep(), y.dep()], move |cx| {
        if *cx.get(y)? == 0 {
            return Err("zero forbidden".into());
        }
        Ok(cx.get(intermediate)? / cx.get(y)?)
    })?;
    let g = b.build()?;
    let old = g.snapshot();
    let mut update = g.changes();
    update.set(x, 99)?.set(y, 0)?;
    assert!(matches!(g.commit(update), Err(Error::Build { node, .. }) if node == "end"));
    assert!(Arc::ptr_eq(&old, &g.snapshot()));
    let mut update = g.changes();
    update.set(y, 2)?;
    let new = g.commit(update)?;
    assert_eq!(*new.get(x)?, 2);
    assert_eq!(*new.get(end)?, 2);
    Ok(())
}

#[test]
fn callback_panic_keeps_old_snapshot_and_writer_usable() -> Result<()> {
    let mut b = GraphBuilder::new();
    let x = b.resource("x", 1);
    b.derived("panicker", [x.dep()], move |cx| {
        let n = *cx.get(x)?;
        assert_ne!(n, 0, "deliberate test panic");
        Ok(n)
    })?;
    b.validate("validator", move |s| {
        assert_ne!(*s.get(x)?, -1, "deliberate validator panic");
        Ok(())
    });
    let g = b.build()?;
    let old = g.snapshot();
    for n in [0, -1] {
        let mut changes = g.changes();
        changes.set(x, n)?;
        assert!(matches!(g.commit(changes), Err(Error::CallbackPanicked(_))));
        assert!(Arc::ptr_eq(&old, &g.snapshot()));
    }
    let mut changes = g.changes();
    changes.set(x, 2)?;
    assert_eq!(g.commit(changes)?.version(), 1);
    Ok(())
}

#[test]
fn forward_dependencies_are_sorted() -> Result<()> {
    let mut b = GraphBuilder::new();
    let result = b.reserve::<i32>("result");
    let middle = b.reserve::<i32>("middle");
    let input = b.reserve::<i32>("input");
    b.define_derived(result, [middle.dep()], move |cx| Ok(cx.get(middle)? + 1))?;
    b.define_derived(middle, [input.dep()], move |cx| Ok(cx.get(input)? * 2))?;
    b.define_resource(input, 5)?;
    let g = b.build()?;
    assert_eq!(*g.snapshot().get(result)?, 11);
    let mut changes = g.changes();
    changes.set(input, 6)?;
    assert_eq!(*g.commit(changes)?.get(result)?, 13);
    Ok(())
}

#[test]
fn cycles_are_rejected_before_callbacks_run() -> Result<()> {
    for self_cycle in [false, true] {
        let mut b = GraphBuilder::new();
        let a = b.reserve::<i32>("a");
        let z = if self_cycle { a } else { b.reserve::<i32>("z") };
        b.define_derived(a, [z.dep()], |_| panic!("must not execute"))?;
        if !self_cycle {
            b.define_derived(z, [a.dep()], |_| panic!("must not execute"))?;
        }
        assert!(matches!(b.build(), Err(Error::Cycle(_))));
    }
    Ok(())
}

#[test]
fn missing_duplicate_and_foreign_definitions_rejected() -> Result<()> {
    let mut b = GraphBuilder::new();
    let x = b.reserve::<i32>("missing");
    assert!(matches!(b.build(), Err(Error::Undefined(_))));
    let mut b = GraphBuilder::new();
    assert_eq!(b.define_resource(x, 1), Err(Error::ForeignGraph));
    let x = b.resource("x", 1);
    assert!(matches!(
        b.define_resource(x, 2),
        Err(Error::AlreadyDefined(_))
    ));
    let mut foreign = GraphBuilder::new();
    assert!(matches!(
        foreign.derived("foreign", [x.dep()], |_| Ok(1)),
        Err(Error::ForeignGraph)
    ));
    // Failed derived registration does not leave an undefined node behind.
    foreign.build()?;
    Ok(())
}

#[test]
fn handles_changes_and_dependency_access_are_graph_scoped() -> Result<()> {
    let mut b = GraphBuilder::new();
    let x = b.resource("x", 1);
    let derived = b.derived("derived", [x.dep()], move |cx| Ok(*cx.get(x)?))?;
    let g = b.build()?;
    let mut b2 = GraphBuilder::new();
    let foreign = b2.resource("foreign", 2);
    let g2 = b2.build()?;
    assert_eq!(g.snapshot().get(foreign), Err(Error::ForeignGraph));
    assert!(matches!(
        g.changes().set(foreign, 2),
        Err(Error::ForeignGraph)
    ));
    assert!(matches!(g.commit(g2.changes()), Err(Error::ForeignGraph)));
    let old = g.snapshot();
    let mut changes = g.changes();
    changes.set(x, 10)?;
    assert!(matches!(
        changes.set(derived, 42),
        Err(Error::NotResource(_))
    ));
    assert!(matches!(changes.set(x, 20), Err(Error::NotResource(_))));
    assert!(matches!(g.commit(changes), Err(Error::NotResource(_))));
    assert!(Arc::ptr_eq(&old, &g.snapshot()));
    let mut b = GraphBuilder::new();
    let hidden = b.resource("hidden", 1);
    b.derived("invalid", [], move |cx| Ok(*cx.get(hidden)?))?;
    assert!(matches!(b.build(), Err(Error::Build { reason, .. }) if reason.contains("undeclared")));
    Ok(())
}

#[test]
fn initial_validation_and_build_errors_are_returned() -> Result<()> {
    let mut b = GraphBuilder::new();
    b.derived::<i32, _>("bad", [], |_| Err("invalid initial value".into()))?;
    assert!(matches!(b.build(), Err(Error::Build { .. })));
    let mut b = GraphBuilder::new();
    b.validate("reject", |_| Err("invalid initial snapshot".into()));
    assert!(matches!(b.build(), Err(Error::Validation { .. })));
    assert_eq!(GraphBuilder::new().build()?.snapshot().version(), 0);
    Ok(())
}

#[test]
fn multiple_roots_recompute_shared_descendant_once_per_commit() -> Result<()> {
    let mut b = GraphBuilder::new();
    let a = b.resource("a", 1);
    let z = b.resource("z", 2);
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let sum = b.derived("sum", [a.dep(), z.dep()], move |cx| {
        c.fetch_add(1, Ordering::SeqCst);
        Ok(cx.get(a)? + cx.get(z)?)
    })?;
    let g = b.build()?;
    let mut changes = g.changes();
    changes.set(a, 10)?.set(z, 20)?;
    assert_eq!(*g.commit(changes)?.get(sum)?, 30);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn checked_batches_reject_stale_and_foreign_snapshots_before_callbacks() -> Result<()> {
    let mut b = GraphBuilder::new();
    let x = b.resource("x", 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    b.validate("counter", move |_| {
        c.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let g = b.build()?;
    let old = g.snapshot();
    let empty = g.changes_from(&old)?;
    let mut stale = g.changes_from(&old)?;
    stale.set(x, 99)?;
    let mut good = g.changes_from(&old)?;
    good.set(x, 2)?;
    let new = g.commit(good)?;
    for changes in [stale, empty] {
        assert_eq!(
            g.commit(changes).unwrap_err(),
            Error::Conflict {
                expected: 0,
                actual: 1
            }
        );
        assert!(Arc::ptr_eq(&new, &g.snapshot()));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let foreign = GraphBuilder::new().build()?;
    assert!(matches!(
        g.changes_from(&foreign.snapshot()),
        Err(Error::ForeignGraph)
    ));
    Ok(())
}

#[test]
fn failed_set_invalidates_prior_staged_values() -> Result<()> {
    let mut b = GraphBuilder::new();
    let x = b.resource("x", 1);
    let g = b.build()?;
    let foreign = GraphBuilder::new().resource("foreign", 3);
    let old = g.snapshot();
    let mut changes = g.changes();
    changes.set(x, 2)?;
    assert!(matches!(changes.set(foreign, 4), Err(Error::ForeignGraph)));
    assert_eq!(g.commit(changes).unwrap_err(), Error::ForeignGraph);
    assert!(Arc::ptr_eq(&old, &g.snapshot()));
    Ok(())
}

#[test]
fn diagnostics_do_not_require_payload_debug() -> Result<()> {
    struct PrivateValue;
    let mut b = GraphBuilder::new();
    b.resource("private", PrivateValue);
    let graph = b.build()?;
    assert!(format!("{:?}", graph.snapshot()).contains("Snapshot"));
    assert!(format!("{graph:?}").contains("LiveGraph"));
    let mut b = GraphBuilder::new();
    b.reserve::<PrivateValue>("undefined");
    assert_eq!(b.build().unwrap_err(), Error::Undefined("undefined".into()));
    Ok(())
}
