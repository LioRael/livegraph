# Borrowed snapshot reads: implementation and measured comparison

## Change

Added `LiveGraph::with_snapshot<R>(&self, read: impl FnOnce(&Snapshot) -> R) -> R`. It loads one arc-swap guard, invokes the callback exactly once, then releases that guard. The guard stays private; references borrowed from the snapshot cannot escape the callback. `snapshot() -> Arc<Snapshot>` is unchanged, as are writer serialization, validation, rollback and atomic publication.

This is an opt-in fast path for short synchronous reads, not an automatic acceleration of existing `snapshot()` callers. It acquires a version on every invocation rather than caching a historical version. Concurrent publication does not change the snapshot already borrowed by a callback. No new dependency or unsafe code was introduced.

arc-swap 1.9.3 documents `load()` as a temporary borrow with limited per-thread fast slots. It can fall back to owned protection, and writers may promote active guards. The change reduces the need for each reader to increment/decrement the same owned Arc reference count; it does not promise that every read avoids such operations. Long-lived/cross-thread ownership should continue to use `snapshot()`.

## Experimental setup

- Fresh before run followed by two after runs, all on the same Apple M2 Pro, 10 logical CPUs, 16 GiB, macOS 27.0.1, Rust/Cargo 1.99.0, optimized Cargo bench build.
- No benchmark families ran concurrently. No CPU affinity, isolated machine, or fixed writer rate was used. Desktop background load and scheduling can affect results.
- The existing 2-node reader workload is unchanged: read the source and its doubled derived value, assert their relationship, and black-box the values. Same 1/2/4/8 readers, with/without one continuously committing writer, one warm-up and five ~200 ms samples per configuration. Thread creation is outside timing.
- `acquire` keeps the original per-operation owned Arc path; `pinned` keeps the original once-per-thread owned snapshot path. `borrowed` uses one new callback per operation. A compile-time branch selects the new path, leaving the original operation sequence intact in the other specialization.
- The after runs retain both old controls. `pinned` is not an equivalent freshness API: it intentionally reads one old version for the entire sample. Both `acquire` and `borrowed` acquire a committed version per operation.
- The complete existing update and retention matrices were also rerun unchanged, not only the reader subset. Before: 108 update rows, 16 reader rows, 36 retention-process rows. Each after run: 108 update rows, 24 reader rows, 36 retention-process rows. Raw update batches and reader samples are saved.

| Run | Measured elapsed time (excluding build) | Source/environment record |
| --- | ---: | --- |
| borrow-before | 55.9 s | [environment](benchmark-results/borrow-before/environment.json) |
| borrow-after-1 | 65.7 s | [environment](benchmark-results/borrow-after-1/environment.json) |
| borrow-after-2 | 65.5 s | [environment](benchmark-results/borrow-after-2/environment.json) |

## Read throughput

Values are aggregate millions of read operations per second, using sample medians. Before is the fresh pre-change run; after columns use after-2. The ranges use both after-run medians divided by the same before median, not confidence intervals.

| Writer | Readers | Before: Arc | After: Arc control | After: borrowed | Borrowed / before | Two-after-run range |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| None | 1 | 70.03 | 70.81 | 89.84 | 1.28× | 1.28–1.28× |
| None | 2 | 18.64 | 16.88 | 178.01 | 9.55× | 9.55–9.64× |
| None | 4 | 10.35 | 9.20 | 358.46 | 34.65× | 34.60–34.65× |
| None | 8 | 7.38 | 7.11 | 594.62 | 80.59× | 76.42–80.59× |
| Continuous | 1 | 29.23 | 28.17 | 39.30 | 1.34× | 1.34–1.34× |
| Continuous | 2 | 14.38 | 14.37 | 87.00 | 6.05× | 6.05–6.17× |
| Continuous | 4 | 9.77 | 9.43 | 195.57 | 20.01× | 20.01–20.20× |
| Continuous | 8 | 8.27 | 7.53 | 334.23 | 40.40× | 40.40–42.01× |

At eight readers without a writer, the new API reaches 563.86–594.62 million reads/s across the two after runs, compared with 7.38 million for the before Arc path. With a continuous writer, it reaches 334.23–347.51 million reads/s compared with 8.27 million before. Against the same-binary Arc control, after-2 improves by 83.68× without a writer and 44.40× with one.
Single-reader gains are smaller: about 1.28× without a writer and 1.34× with one versus before. The large multi-reader effect is consistent with reducing shared reference-count contention, but no hardware-counter profiling was performed.

### Writer throughput and pinned control

Unthrottled writers complete different numbers of commits under each read mode. The comparison is therefore not a controlled experiment at an identical write arrival rate. These after-2 medians make that distinction visible.

| Readers | Arc reads, M/s | Borrowed reads, M/s | Pinned reads, M/s | Arc-mode writes, M/s | Borrowed-mode writes, M/s | Pinned-mode writes, M/s |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 28.168 | 39.301 | 95.562 | 0.817 | 0.910 | 1.334 |
| 2 | 14.369 | 87.004 | 192.563 | 0.618 | 0.595 | 1.202 |
| 4 | 9.428 | 195.567 | 345.857 | 0.329 | 0.357 | 1.018 |
| 8 | 7.527 | 334.230 | 458.144 | 0.190 | 0.203 | 0.691 |

Borrowed reads do not obtain their gain simply by stopping the writer: at eight readers, after-2 records 0.203 million writes/s for borrowed versus 0.190 million for the Arc control. This difference is not a claim that the writer itself was optimized. The pinned control avoids repeated acquisition but intentionally permits indefinitely stale reads.
Variability remains: after-2 eight-reader borrowed throughput with a writer has P10/P90 sample rates of 273.36/358.70 million reads/s. The Arc control also varies between runs (for example, its four-reader no-writer result is 10.35 million before and 9.20 million after-2). Raw samples are preserved; these are synthetic throughput measurements, not latency guarantees.

## Update and retention controls

The update graph shapes, sizes, change ratios, synthetic compute work, and three baselines are unchanged. The following ratios compare each matched scenario’s median time to before, then summarize all 36 scenarios per mode. A ratio above 1 means slower. This aggregate is a regression screen, not proof that every workload has identical performance.

| Mode | After-1 median time ratio | After-2 median time ratio | After-2 min–max across scenarios |
| --- | ---: | ---: | ---: |
| incremental | 1.002 | 1.008 | 0.973–1.041 |
| full_recompute | 1.004 | 1.007 | 0.967–1.052 |
| rebuild | 1.010 | 1.012 | 0.535–1.054 |

Median matched update costs differ by roughly 0–1.3% in these runs; no systematic large update regression appeared. The unusually low rebuild minimum reflects a slow before sample, not an optimization to graph construction. Full CSVs retain all scenarios.

The retention test still uses owned historical snapshots: 800 nodes with 4 KiB payloads, retaining 0/1/10/100 previous versions, three fresh processes per combination. Its live-payload counts match before exactly for every combination. With 100 retained versions, 1% updates retain 6.25 MiB and 100% updates retain 315.625 MiB; releasing history returns to 3.125 MiB, and dropping the graph returns the payload counter to zero.
This does not claim zero memory overhead for borrowed guards. Their guard lifetime is bounded by the callback, and a writer may keep the old data alive until that callback returns. A deterministic regression test verifies both survival during concurrent publication and reclamation after callback exit. As before, process RSS is not a live-object counter.

## Correctness and API tests

Added seven integration tests and two rustdoc tests:

- The callback runs exactly once, accepts FnOnce, returns owned results, and preserves foreign-handle errors; the Arc API retains its original type.
- A writer completes publication while a borrowed reader retains references to the old values; old payloads are reclaimed after callback exit.
- Both computation failure and validation failure preserve borrowed and owned versions.
- An unwinding read panic releases protection and does not poison future writes.
- 65 nested reads remain consistent across a commit, including when arc-swap’s fast borrow slots are exhausted.
- A checked batch can leave the callback, but stale replacements still conflict at commit.
- Four borrowed readers and two writers check values and version monotonicity under concurrent publication.
- A working documentation example and a compile-fail example demonstrate valid owned returns and rejection of escaping snapshot references.

The read callback’s panic is not swallowed. A read callback is not a rollback transaction for explicit commits or external effects it performs. Logical immutability of payloads remains required. No public arc-swap guard type is exposed, and no deep-freeze claim is made.

## Reproduce and inspect

```sh
python3 benches/run.py --output benchmark-results/new-run
cargo test --all-targets --all-features --locked
cargo test --doc --locked
```

The README is now fully in English and documents both read APIs, conflict retries, rollback boundaries and benchmark interpretation. The existing owned API, update semantics and optional file-adapter boundary are preserved.

### borrow-before

[Update summary](benchmark-results/borrow-before/updates.csv) · [Update samples](benchmark-results/borrow-before/updates-samples.csv) · [Reader summary](benchmark-results/borrow-before/readers.csv) · [Reader samples](benchmark-results/borrow-before/readers-samples.csv) · [Retention](benchmark-results/borrow-before/retention.csv) · [Environment and hashes](benchmark-results/borrow-before/environment.json)

### borrow-after-1

[Update summary](benchmark-results/borrow-after-1/updates.csv) · [Update samples](benchmark-results/borrow-after-1/updates-samples.csv) · [Reader summary](benchmark-results/borrow-after-1/readers.csv) · [Reader samples](benchmark-results/borrow-after-1/readers-samples.csv) · [Retention](benchmark-results/borrow-after-1/retention.csv) · [Environment and hashes](benchmark-results/borrow-after-1/environment.json)

### borrow-after-2

[Update summary](benchmark-results/borrow-after-2/updates.csv) · [Update samples](benchmark-results/borrow-after-2/updates-samples.csv) · [Reader summary](benchmark-results/borrow-after-2/readers.csv) · [Reader samples](benchmark-results/borrow-after-2/readers-samples.csv) · [Retention](benchmark-results/borrow-after-2/retention.csv) · [Environment and hashes](benchmark-results/borrow-after-2/environment.json)
