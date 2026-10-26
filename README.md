# LiveGraph

A Rust library for typed resources and derived values in a fixed dependency graph. A batch of resource replacements recomputes only affected derived nodes. The graph publishes one complete snapshot after all computations and validators succeed. Failed builds leave the previous snapshot available.

Useful for read-heavy applications such as configuration → parsed settings → routing tables, or game data → indexes → query views. LiveGraph is not a database, an async task runtime, or a filesystem transaction manager.

## Run

```sh
cargo run --example config_reload
cargo test
cargo fmt --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

LiveGraph 0.1.0 is an early release with a fixed-graph, synchronous in-memory core. It uses Rust 2024 edition and requires Rust 1.88 or newer. The release is tested locally with Rust 1.88.0 and 1.99.0. The only direct dependency is `arc-swap`. API changes may occur in later 0.x releases; the limitations and performance tradeoffs below are part of the current contract.

## Install

```sh
cargo add livegraph
```

```toml
[dependencies]
livegraph = "0.1"
```

[API documentation](https://docs.rs/livegraph) · [Source and issues](https://github.com/LioRael/livegraph)


## Define and update a graph

```rust
use livegraph::{GraphBuilder, Result};

fn main() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let price = builder.resource("price", 100_u32);
    let quantity = builder.resource("quantity", 2_u32);
    let total = builder.derived("total", [price.dep(), quantity.dep()], move |cx| {
        cx.get(price)?.checked_mul(*cx.get(quantity)?)
            .ok_or_else(|| "total overflow".into())
    })?;
    builder.validate("budget", move |snapshot| {
        if *snapshot.get(total)? > 10_000 {
            Err("budget exceeded".into())
        } else {
            Ok(())
        }
    });
    let graph = builder.build()?;
    let old = graph.snapshot();

    let mut changes = graph.changes();
    changes.set(price, 150)?.set(quantity, 3)?;
    let new = graph.commit(changes)?;

    assert_eq!(*old.get(total)?, 200);
    assert_eq!(*new.get(total)?, 450);
    assert_eq!(new.version(), 1);
    Ok(())
}
```

`Node<T>` is a copyable, graph-scoped handle with private fields. Reads and assignments must match its value type at compile time. Handles from a different graph return an error. `dep()` erases the type only for heterogeneous dependency lists.

A derived callback can read only its declared direct dependencies through `Context::get`. Undeclared reads return an error. Do not capture external mutable state or another snapshot to bypass the dependency graph. Use `reserve::<T>` followed by `define_resource` or `define_derived` for forward references. `build` rejects undefined nodes, self-cycles, and indirect cycles. Duplicate dependency entries are deduplicated; duplicate definitions fail. Names are diagnostic labels and need not be unique.

## Read a consistent snapshot

Choose an API according to the required lifetime:

| API | Lifetime | Typical use |
| --- | --- | --- |
| `snapshot() -> Arc<Snapshot>` | Owned; may outlive the graph or cross threads | Long operations, async work, retained history |
| `with_snapshot(|snapshot| ...)` | Borrowed for one synchronous callback | Short reads of one or more related values |

Both acquire one committed version without taking the writer mutex. Related values must be read through the same snapshot. Separate calls can observe different versions.

```rust
use livegraph::{GraphBuilder, Result};

fn main() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let count = builder.resource("count", 7_u64);
    let doubled = builder.derived("doubled", [count.dep()], move |cx| {
        Ok(cx.get(count)? * 2)
    })?;
    let graph = builder.build()?;

    let (version, count, doubled) = graph.with_snapshot(|snapshot| -> Result<_> {
        Ok((snapshot.version(), *snapshot.get(count)?, *snapshot.get(doubled)?))
    })?;
    assert_eq!((version, count, doubled), (0, 7, 14));
    Ok(())
}
```

`with_snapshot` uses `ArcSwap::load` and keeps its guard private. The callback runs exactly once and can return owned values, including `Result`, but references borrowed from that snapshot cannot escape. This avoids forcing an owned `Arc` acquisition for every short read. It does not cache a snapshot between calls. A concurrent commit may publish a newer version while the callback continues reading its original version.

Keep callbacks short: do not block or hold work across an async boundary. Deeply nested reads can exhaust arc-swap's limited fast borrowing slots and fall back to a more expensive path; correctness is unchanged. Writers may also promote active guards to owned references. There is no promise of zero allocation or zero reference-count traffic in every case. Use `snapshot()` for long-lived access.

A panic in a read callback propagates normally, releasing the guard during unwinding. This read API is not a transaction: any explicit commit or external side effect performed inside the callback is not undone by that panic.

**Logical immutability:** readers receive only `&T`, but `Send + Sync` does not forbid mutexes, atomics, or other interior mutation. Treat published payloads as immutable. Changing them through another channel bypasses snapshot isolation; LiveGraph does not deeply freeze arbitrary Rust values.

## Commit semantics and failure isolation

1. Serialize writers and start from the latest committed snapshot. Check the batch's expected version, if present.
2. Apply all staged resource replacements to a private candidate and find affected descendants through reverse edges.
3. Select scheduling once from the staged resource count, a lower bound on affected nodes. If it reaches ceil(V / 4), propagate reachability in one forward topological scan. Otherwise collect reachable nodes and sort their cached ranks, even if a broadcast root eventually affects most of the graph. Discovery never restarts as a full scan. Both paths recompute only affected derived nodes, at most once per batch. A diamond join sees the new values from both branches.
4. Run every snapshot validator after computation succeeds. Initial construction also runs all validators.
5. Publish the complete snapshot with one `ArcSwap::store` and increment its version. The returned snapshot is the version published by this commit, even if another writer subsequently commits.

A build error, validation error, or unwinding panic in a build/validation callback discards the candidate. The previous snapshot and version remain unchanged. The panic hook may still print a message. `panic=abort`, process termination, and allocation failure are outside this recovery guarantee. The library contains no unsafe code.

Snapshots share unchanged 16-slot blocks through `Arc`. Partially modified blocks use copy-on-write; blocks whose real slots will all be replaced start empty. When every node is affected, create an empty block directory directly and use checked unique access to its private blocks. Partial updates retain lazy copy-on-write, even when scheduling uses a dense scan. The commit chooses one of two statically specialized write loops; dependency reads always use the same snapshot representation, without a per-read strategy branch or temporary block-access directory. This avoids copying unchanged resources in broadcast graphs. Unaffected blocks remain shared. Unchanged payloads remain shared and do not require `T: Clone`. No block is mutated in a previously published snapshot. Readers may retain old snapshots; their data can be reclaimed after the final reader releases it. The graph releases retired snapshots and rejected candidates after leaving the writer lock and recursion guard. User destructors must not panic: a destructor panic after publication does not undo that publication. Temporary objects created inside a user callback remain that callback's responsibility.

`Changes::set` rejects foreign handles and derived-node assignments immediately. Any staging error invalidates the entire batch: subsequent `set` calls and `commit` return that error, preventing accidental partial publication. Create a new batch after correcting the input. `Snapshot` and `LiveGraph` implement metadata-only `Debug`, so `unwrap_err()` and `expect_err()` do not require payloads to implement `Debug`.

Empty batches are no-ops after checking their expected version; stale checked empty batches still conflict. Repeated assignments in a batch use the last value. Equal replacements still invalidate descendants; this MVP does not perform equality pruning.

## Concurrent read-modify-write

`changes()` creates an unconditional replacement batch. It preserves unrelated updates by applying to the latest committed version, but it cannot prevent two callers from reading the same old value and overwriting each other's increments.

Use `changes_from(&snapshot)` when replacements depend on what you read. The whole-graph version is checked under the writer lock; an intervening successful commit produces `Error::Conflict { expected, actual }` before computation or publication. Retry by reading and calculating again, not by resubmitting stale replacements unconditionally.

```rust
use livegraph::{Error, GraphBuilder, Result};

fn main() -> Result<()> {
    let mut builder = GraphBuilder::new();
    let count = builder.resource("count", 0_u64);
    let graph = builder.build()?;

    loop {
        let changes = graph.with_snapshot(|snapshot| -> Result<_> {
            let mut changes = graph.changes_from(snapshot)?;
            changes.set(count, snapshot.get(count)? + 1)?;
            Ok(changes)
        })?;
        // The short-lived read guard has already been released.
        match graph.commit(changes) {
            Ok(_) => break,
            Err(Error::Conflict { .. }) => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
```

Conflict detection is at graph granularity, so even an unrelated resource update can invalidate a checked batch. LiveGraph does not automatically retry user operations.

## Constraints and costs

- Build and validation callbacks should be pure, synchronous functions. External I/O and shared-state changes cannot be rolled back.
- Same-thread recursive commits to the same graph return `ReentrantCommit`. Callbacks must not wait for another thread to commit to that graph or create cross-thread/cross-graph cyclic waits. Reading the published snapshot is allowed, but cannot substitute for reading candidate dependencies.
- Initial topology construction and cycle checks are O(V + E) and iterative. Cycle errors list blocked nodes, which may include descendants; they are not exact cycle paths.
- Updates copy a directory of O(ceil(V / 16)) block references, not all V value references. Only partially changed blocks clone their value references; fully overwritten blocks start empty. The block size is an internal implementation detail.
- Scheduling uses the number of staged resources as a lower bound on affected nodes. At ceil(V / 4) staged resources it selects a topological scan before visiting edges; otherwise it completes one reachable traversal and rank sort. A low-root-count broadcast does not switch midway, avoiding duplicate high-fanout edge traversal and per-node threshold checks. This is an internal heuristic, not a universally optimal crossover. Writer-local generation marks avoid clearing a V-sized array on each sparse update; counter wrap clears marks once.
- With C blocks, A affected nodes, E_A outgoing edges visited, P partially modified blocks and F fully replaced blocks, reachable traversal/building takes O(C + E_A + A log A + 16(P + F)). The topological scan/build path takes O(V + E_A + 16(P + F)). Fully affected snapshots skip the per-block replacement-counting pass. No path uses a temporary access directory or repacks a contiguous snapshot. These bounds exclude callbacks; every global validator still runs. Sparse updates still depend on the block-directory size, and speedups depend on the workload.
- The writer retains O(V) marking/rank metadata plus a worklist whose capacity follows the largest affected set seen so far. Historical snapshots share unchanged blocks, but each snapshot still owns its block-reference directory. Partial blocks may retain otherwise unchanged payload references.
- The graph is fixed after construction. Runtime node changes, dynamic dependencies, async/parallel computation, cancellation, notifications, persistence, and distributed consistency are not implemented.
- Snapshot history and long-lived readers retain memory. Borrowing does not remove the cost of retaining old versions while writers publish new ones.

## Boundaries: arc-swap, notify, and LiveGraph

| Component | Responsibility | Outside its scope |
| --- | --- | --- |
| [arc-swap](https://docs.rs/arc-swap/latest/arc_swap/) | Atomic shared-version publication and protected reads | Graph scheduling, invalidation, business validation |
| [notify](https://docs.rs/notify/latest/notify/) | Filesystem change events | Dependency graphs, multi-file transactions, in-memory publication |
| LiveGraph | Typed nodes, dependency checks, affected-node computation, candidate validation and publication | Filesystem event reliability and atomic multi-file disk updates |

This MVP uses arc-swap 1.9.3 and has no notify dependency. A future optional adapter could coalesce events, reread and parse resources, then submit one `Changes` batch. Events are signals to reread, not proof of a complete transaction. Consistent multi-file input needs an application-level protocol, such as versioned directories or a generation manifest. Atomic in-memory publication does **not** make multi-file disk updates atomic.

## Tests and benchmarks

Tests cover dependency ordering, diamonds, unaffected branches, cycle checks, batch validation, failed-build and panic rollback, foreign handles, optimistic updates, recursive commits, reclamation, and concurrent readers/writers. Sparse-update tests verify block sharing and boundaries, partial tail blocks, generation wrap, a forward-reference DAG against an independent full evaluator, exact affected callback counts after failures, and cross-block atomic reads with concurrent writers. Adaptive-update tests compare scheduling against an independent reachability oracle on permuted DAGs, cross the staged-root threshold, ensure broadcast traversal does not restart, and exercise error/panic/validation rollback, destructor lock release, and concurrent readers across sparse and dense commits. Candidate-access tests also cover partial tail blocks, a forward-reference join that fails late, retained histories, and identical foreign/undeclared-dependency checks in both write paths. Borrowed reads additionally cover publication during an active read, failure isolation, panic cleanup, nested guards, and compile-time lifetime restrictions. These tests are not exhaustive interleaving model checks.

```sh
python3 benches/run.py --output benchmark-results/new-run
```

Benchmark runners, reports and raw results are available in the GitHub checkout; they are not bundled in the crate download. The benchmark runner uses only the Python standard library. It records CSV samples and environment/source hashes. It measures incremental updates versus full recomputation and graph reconstruction, concurrent reads, and retained snapshots. Use a fresh output directory to preserve earlier measurements.

See [the original performance report](https://github.com/LioRael/livegraph/blob/main/BENCHMARKS.md), [the borrowed-read comparison](https://github.com/LioRael/livegraph/blob/main/BORROWED_READ_BENCHMARKS.md), [the sparse-update comparison](https://github.com/LioRael/livegraph/blob/main/SPARSE_UPDATE_BENCHMARKS.md), [the adaptive-update comparison](https://github.com/LioRael/livegraph/blob/main/ADAPTIVE_UPDATE_BENCHMARKS.md), [the access/scheduler regression investigation](https://github.com/LioRael/livegraph/blob/main/ACCESS_SCHEDULER_BENCHMARKS.md), [the longer broadcast stability study](https://github.com/LioRael/livegraph/blob/main/BROADCAST_STABILITY_REPORT.md), and [validation results](https://github.com/LioRael/livegraph/blob/main/VALIDATION.md). Measurements are local synthetic workloads, not cross-platform validation or production latency guarantees. The latest report compares the pre-adaptive implementation, the previous adaptive revision, and the current implementation with identical timed workloads, including any remaining regressions.

To compare two source trees with identical update workloads and alternating execution order:

```sh
python3 benches/compare.py --baseline /path/to/baseline --output benchmark-results/paired --passes 3
```

Add `--reference /path/to/pre-adaptive` for a three-way comparison with rotating execution order. All trees must contain the same `benches/performance.rs`. The optional `LIVEGRAPH_BENCH_CASE` selector filters cases before timing; it does not change the default full matrix or timed operations.

For longer measurements of the 4,000-node broadcast case, with balanced execution order and an identical-binary A/A control:

```sh
python3 benches/broadcast_long.py --baseline /path/to/earlier-adaptive --reference /path/to/pre-adaptive --output benchmark-results/broadcast-repeat
```

The focused main case uses 12 blocks, 11 calibrated batches targeted at 150 ms per mode, and four execution labels (including a repeat of the current binary). Neighbor controls use four blocks and 100 ms batches. Add `--cpu-probe` for four focused blocks with process CPU/wall-time accounting and no neighbor matrix. The runner records every sample, order, load average, and source/binary hash; it rejects an existing output directory. CPU accounting covers the entire child invocation, including setup and calibration, and is not an incremental-operation CPU measurement.

`LIVEGRAPH_BENCH_BATCH_MS`, `LIVEGRAPH_BENCH_SAMPLES`, and `LIVEGRAPH_BENCH_MODE_ROTATION` configure the raw benchmark outside timed operations. Unset values retain the original 20 ms / nine-batch defaults and mode order. The long runner sets these explicitly. Interpret repeated blocks as the experimental units; many batches within one process do not establish independent statistical evidence.

The longer broadcast study did not establish a stable +4% incremental regression: the delivered fix versus the preceding adaptive build has a paired median of −0.13%, with a descriptive whole-block bootstrap interval of −2.54% to +3.83%. The identical-binary control varies more widely. The library implementation is retained unchanged; small real differences remain unresolved on the shared host. Full-recomputation controls are reported separately and must not be dismissed as noise. CPU accounting in the Python runner uses Unix `resource` APIs; the measured platform is macOS.
