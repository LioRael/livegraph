# Changelog

## 0.1.0

Initial public release.

- Typed, graph-scoped resource and derived-node handles with declared dependencies.
- Fixed DAG validation, forward references, cycle checks and dependency-ordered updates.
- Affected-node recomputation with shared 16-slot snapshot blocks.
- Atomic publication after successful computation and validation; failed candidates retain the previous snapshot.
- Owned snapshots and short synchronous borrowed reads for concurrent readers.
- Optimistic version checks for read-modify-write batches.
- Examples, concurrency/rollback tests and reproducible performance investigations.

This is an early in-memory release. It does not provide dynamic topology, async computation, persistence, file watching or atomic multi-file disk updates. Published payloads must be treated as logically immutable. Performance gains vary by workload; see the README and benchmark reports.
