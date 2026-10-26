# Sparse updates: chunked snapshots and affected-only scheduling

## Outcome

The existing public API is unchanged. Snapshots now share 16-slot blocks, and commits schedule only reachable affected nodes using precomputed topological ranks. Candidate validation, atomic publication, optimistic version checks, rollback, owned snapshots, and borrowed reads keep their previous semantics.

Two baseline runs and two runs of the final implementation used the **identical benchmark source** on Apple M2 Pro, 10 logical CPUs, 16 GiB, macOS 27.0.1, Rust/Cargo 1.99.0, optimized native Cargo builds. An earlier 32-slot prototype was also measured and retained separately.

At 40,000 nodes with independent branches, 1% resource changes improve by about **4.0× for light computation** and **3.0× for 64-round computation**. At 10% changes the difference is roughly neutral, and dense/all-descendant updates generally incur additional overhead. This is a sparse-update optimization, not a universal speedup.

## Implementation

- A snapshot holds its logical length and a directory of `Arc` blocks, each containing 16 optional value slots. The last block may be partial. Unchanged blocks are shared between versions.
- Partially modified blocks use `Arc::make_mut`, cloning references within that block once before modification. Payloads still do not require `Clone`.
- Before building a candidate, the affected set is counted per block. If every real slot in a block will be replaced, that block starts empty rather than copying old values. Resource replacements are staged first; derived values are then populated in dependency order. Validators run only after all affected slots are populated. A failed candidate is never published.
- Each graph stores a node-to-topological-rank map. The serialized writer walks reverse edges from changed roots, deduplicates with generation marks, sorts only the collected ranks, and executes those nodes. It does not scan the entire node order for sparse updates.
- Generation marks and the worklist are writer-local reusable scratch storage. Every attempt uses a new generation, so failed builds cannot leave valid-looking marks for the next commit. On 64-bit generation wrap, the mark array is cleared once.
- Retired and rejected data is still released after leaving the writer lock and recursion guard. Borrowed readers keep their original immutable version during concurrent publication.

### Remaining costs and tradeoffs

Let V be total nodes, C = ceil(V / 16), A the affected set, E_A the outgoing edges visited, P partially modified blocks, and F fully replaced blocks. Internal work is O(C + E_A + A log A + 16(P + F)), plus callback and validator work. Copying/scanning the block directory remains O(C); this is not a persistent tree with O(log V) updates. Global validators still all run.
The writer retains O(V) marking/rank metadata and worklist capacity up to its largest affected set. Dense updates do not benefit from skipping nodes and pay for block bookkeeping and affected-set scheduling. Reading values adds a block lookup. Chunk size is private and may change.

## Benchmark method

All benchmark families ran serially. The update matrix still covers 400/4,000/40,000 nodes, partitioned/broadcast shapes, 1%/10%/100% resource changes, 0/64 rounds of synthetic derived computation, and incremental/full-recompute/rebuild modes. No business validators are installed in the performance graph. Each scenario verifies all leaf values against a fresh reconstruction before timing.
Each update result is the median of nine calibrated batch means (at least ~20 ms per batch). Reader configurations retain their five ~200 ms samples, and retention configurations run in three fresh processes each. P10/P90 values are preserved in CSV and describe samples/batch means, not single-request tail latency.
Tables below use the median of the two per-run medians on each side (with two points this is their midpoint). This avoids choosing the faster final run, but is not a confidence interval. Before-time / after-time greater than 1 means faster. The machine was not isolated or CPU-pinned.

| Run | Role | Duration excluding build |
| --- | --- | ---: |
| sparse-before | Baseline | 69.8 s |
| sparse-after-1 | Exploratory 32-slot prototype; not final | 66.8 s |
| sparse-after-2 | Final 16-slot implementation | 67.3 s |
| sparse-after-3 | Final 16-slot implementation | 67.3 s |
| sparse-before-repeat | Baseline | 64.3 s |

The baseline repeat ran from a separate temporary copy with the exact same core/benchmark/lockfile hashes as the first baseline. It did not replace the delivered implementation. A [baseline source archive](benchmark-results/sparse-before/baseline-source.tar.gz) is provided for reproduction.

### Why the prototype changed

The 32-slot prototype reduced the 40,000-node, 1%-change light-compute case from 178.98 to 48.87 µs, but 10% updates rose from 350.05 to 379.55 µs and 100% from 2,013.91 to 2,352.30 µs in that initial pair. Uniformly spaced changes touched many blocks. The final implementation uses 16-slot blocks and skips old-value copying for wholly replaced blocks. The prototype data is retained, but excluded from final before/after ratios.

## Incremental-update results

Units: µs per update. “Range” lists the two final-run medians, not an uncertainty bound.

### Independent branches, rounds=0

| Nodes | Resource changes | Before | Final | Final-run range | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 400 | 1% | 1.52 | 0.51 | 0.51–0.51 | 2.99× |
| 400 | 10% | 2.65 | 2.60 | 2.59–2.60 | 1.02× |
| 400 | 100% | 16.14 | 18.21 | 18.18–18.24 | 0.89× |
| 4,000 | 1% | 19.79 | 3.32 | 3.31–3.34 | 5.95× |
| 4,000 | 10% | 31.34 | 29.86 | 29.07–30.65 | 1.05× |
| 4,000 | 100% | 183.45 | 205.43 | 202.56–208.30 | 0.89× |
| 40,000 | 1% | 173.84 | 42.94 | 42.27–43.60 | 4.05× |
| 40,000 | 10% | 333.17 | 331.21 | 330.08–332.34 | 1.01× |
| 40,000 | 100% | 1927.26 | 2156.92 | 2141.41–2172.42 | 0.89× |

### Independent branches, rounds=64

| Nodes | Resource changes | Before | Final | Final-run range | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 400 | 1% | 1.77 | 0.73 | 0.73–0.73 | 2.42× |
| 400 | 10% | 4.79 | 4.57 | 4.53–4.60 | 1.05× |
| 400 | 100% | 34.06 | 37.03 | 36.20–37.87 | 0.92× |
| 4,000 | 1% | 20.13 | 5.28 | 5.24–5.32 | 3.81× |
| 4,000 | 10% | 51.74 | 50.84 | 50.78–50.90 | 1.02× |
| 4,000 | 100% | 367.67 | 382.76 | 381.30–384.22 | 0.96× |
| 40,000 | 1% | 199.74 | 67.28 | 65.83–68.73 | 2.97× |
| 40,000 | 10% | 547.44 | 567.62 | 564.85–570.39 | 0.96× |
| 40,000 | 100% | 3602.06 | 3937.84 | 3930.89–3944.80 | 0.91× |

### Shared root (all derived nodes affected), rounds=0

| Nodes | Resource changes | Before | Final | Final-run range | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 40,000 | 1% | 1156.85 | 1287.45 | 1281.16–1293.73 | 0.90× |
| 40,000 | 10% | 1242.00 | 1400.85 | 1397.78–1403.93 | 0.89× |
| 40,000 | 100% | 1968.39 | 2154.34 | 2124.09–2184.60 | 0.91× |

### Shared root (all derived nodes affected), rounds=64

| Nodes | Resource changes | Before | Final | Final-run range | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 40,000 | 1% | 2861.57 | 3070.55 | 3050.40–3090.70 | 0.93× |
| 40,000 | 10% | 2983.37 | 3203.83 | 3182.86–3224.79 | 0.93× |
| 40,000 | 100% | 3670.22 | 3961.59 | 3955.54–3967.64 | 0.93× |

### Full recomputation and graph reconstruction controls

Both baseline modes remain in the same benchmark. Full recomputation resubmits **all resources**, including unchanged ones, so it also includes all-resource staging cost; it is not an idealized compute-only full scheduler. Rebuild includes graph definition, topology, initial computation and destruction.

40,000 nodes, independent branches, 64 rounds. Units: µs/update.

| Resource changes | Mode | Before | Final | Before/final |
| ---: | --- | ---: | ---: | ---: |
| 1% | incremental | 199.74 | 67.28 | 2.97× |
| 1% | full_recompute | 4096.18 | 3939.48 | 1.04× |
| 1% | rebuild | 7325.82 | 8057.21 | 0.91× |
| 10% | incremental | 547.44 | 567.62 | 0.96× |
| 10% | full_recompute | 3634.54 | 3958.22 | 0.92× |
| 10% | rebuild | 7277.50 | 7791.08 | 0.93× |
| 100% | incremental | 3602.06 | 3937.84 | 0.91× |
| 100% | full_recompute | 3611.94 | 3957.23 | 0.91× |
| 100% | rebuild | 7243.12 | 7757.92 | 0.93× |

The final implementation preserves the large advantage of incremental work over full reconstruction for sparse changes. It does not eliminate the dense-update tradeoff: compared with the flat implementation, all-node updates in this table are slower. The benefit depends on actual affected nodes and block locality, not just the number of changed resources.

## Reader controls

The same owned-acquire, pinned and borrowed modes were rerun. Tables show aggregate throughput ranges across each pair of runs in millions of reads/s. No fixed writer arrival rate was imposed, so writer-active configurations are not equal-write-load experiments.

| Mode | Writer | Readers | Before range, M/s | Final range, M/s |
| --- | --- | ---: | ---: | ---: |
| acquire | None | 1 | 71.68–72.42 | 70.96–72.24 |
| acquire | None | 8 | 7.14–14.22 | 7.20–7.80 |
| acquire | Continuous | 1 | 17.18–33.43 | 27.63–27.98 |
| acquire | Continuous | 8 | 7.27–9.61 | 8.10–10.07 |
| borrowed | None | 1 | 90.21–92.93 | 93.26–94.05 |
| borrowed | None | 8 | 315.72–603.91 | 521.29–554.72 |
| borrowed | Continuous | 1 | 28.19–30.73 | 44.50–47.24 |
| borrowed | Continuous | 8 | 234.00–343.75 | 244.04–350.11 |
| pinned | None | 1 | 120.24–123.33 | 115.97–116.52 |
| pinned | None | 8 | 337.65–811.06 | 689.59–711.31 |
| pinned | Continuous | 1 | 76.63–120.42 | 95.83–99.05 |
| pinned | Continuous | 8 | 361.48–701.12 | 487.58–529.72 |

High-concurrency readings vary substantially: the unchanged baseline borrowed/no-writer eight-reader case ranges from 315.72 to 603.91 million reads/s, while final runs range from 521.29 to 554.72. With a writer, final borrowed eight-reader results range from 244.04 to 350.11 million reads/s. These overlapping ranges do not establish a new reader-speed improvement or a precise regression bound. The previous borrowed-read fast path remains available; raw files include writer throughput as well.

## Snapshot retention and memory

The same 800-node, 4 KiB/payload workload ran with 0/1/10/100 retained historical versions and 1%/10%/100% changes. For **every combination**, live payload byte counts match the flat baseline exactly; the optimization affects snapshot metadata/reference copying rather than the number of logically live payloads. Releasing history returns payloads to 3.125 MiB; dropping the graph returns the counter to zero, checked at runtime.

The table focuses on 100 retained versions. Values aggregate the three independent processes in each of two runs (six observations per side). Commit time is each process’s steady-state median. RSS is sampled resident memory, not peak allocation or a live-object count.

| Changes | Live payload MiB | Before commit µs | Final commit µs | Before RSS MiB | Final RSS MiB | Before bulk release ms | Final bulk release ms |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1% | 6.250 | 5.27 | 2.40 | 10.41 | 9.13 | 0.14 | 0.11 |
| 10% | 34.375 | 29.02 | 34.75 | 39.52 | 39.10 | 1.45 | 2.20 |
| 100% | 315.625 | 325.29 | 458.23 | 329.95 | 329.32 | 27.56 | 44.56 |

A flat 800-slot reference vector holds roughly 12.8 KiB of slot storage per version before allocator overhead. A 16-slot block directory has 50 pointers (400 bytes on this 64-bit host), plus shared or copied blocks. Sparse histories can share most block metadata; dense histories allocate most blocks anew and can have more per-block overhead. RSS measurements remain allocator/OS dependent and should not be treated as exact bookkeeping savings.

## Tests and guarantees

Five new tests supplement the previous 33:

- Verify that only touched blocks are replaced, untouched blocks are pointer-shared, old values stay unchanged, and boundaries/partial tail slots work.
- Force the generation counter to wrap and verify stale marks do not suppress computation or break topological order.
- Compare a 137-derived-node forward-reference DAG with an independent full evaluator over 24 successful batches, including build/validation failures followed by retries. Check every leaf and exact per-node callback counts.
- Read cross-block resource/derived invariants through both owned and borrowed APIs while two writers update shared and disjoint blocks; retain and verify the original snapshot.
- Preserve an unaffected constant derived value inside a partially updated block during a dense resource update.

Existing tests continue to cover cycle rejection, diamonds, rollback, optimistic conflicts, borrowed lifetimes, panic cleanup, concurrent publication and cleanup outside the writer lock. No public API changed and no dependency was added. The English README describes the new layout, complexity, scratch memory, and remaining dense-workload tradeoff.

## Reproduce and inspect

```sh
python3 benches/run.py --output benchmark-results/new-run
cargo test --all-targets --all-features --locked
cargo test --doc --locked
```

Each run contains 108 update summaries, 972 update batch samples, 24 reader summaries, 120 reader samples and 36 retention-process rows. Counts were verified. Both final-run hashes match delivered core/benchmark code and Cargo.lock; baseline core hashes match each other. The benchmark file hash is identical across all five runs.

### sparse-before

[Update summary](benchmark-results/sparse-before/updates.csv) · [Update samples](benchmark-results/sparse-before/updates-samples.csv) · [Reader summary](benchmark-results/sparse-before/readers.csv) · [Reader samples](benchmark-results/sparse-before/readers-samples.csv) · [Retention](benchmark-results/sparse-before/retention.csv) · [Environment and hashes](benchmark-results/sparse-before/environment.json)

### sparse-before-repeat

[Update summary](benchmark-results/sparse-before-repeat/updates.csv) · [Update samples](benchmark-results/sparse-before-repeat/updates-samples.csv) · [Reader summary](benchmark-results/sparse-before-repeat/readers.csv) · [Reader samples](benchmark-results/sparse-before-repeat/readers-samples.csv) · [Retention](benchmark-results/sparse-before-repeat/retention.csv) · [Environment and hashes](benchmark-results/sparse-before-repeat/environment.json)

### sparse-after-2

[Update summary](benchmark-results/sparse-after-2/updates.csv) · [Update samples](benchmark-results/sparse-after-2/updates-samples.csv) · [Reader summary](benchmark-results/sparse-after-2/readers.csv) · [Reader samples](benchmark-results/sparse-after-2/readers-samples.csv) · [Retention](benchmark-results/sparse-after-2/retention.csv) · [Environment and hashes](benchmark-results/sparse-after-2/environment.json)

### sparse-after-3

[Update summary](benchmark-results/sparse-after-3/updates.csv) · [Update samples](benchmark-results/sparse-after-3/updates-samples.csv) · [Reader summary](benchmark-results/sparse-after-3/readers.csv) · [Reader samples](benchmark-results/sparse-after-3/readers-samples.csv) · [Retention](benchmark-results/sparse-after-3/retention.csv) · [Environment and hashes](benchmark-results/sparse-after-3/environment.json)

### sparse-after-1

[Update summary](benchmark-results/sparse-after-1/updates.csv) · [Update samples](benchmark-results/sparse-after-1/updates-samples.csv) · [Reader summary](benchmark-results/sparse-after-1/readers.csv) · [Reader samples](benchmark-results/sparse-after-1/readers-samples.csv) · [Retention](benchmark-results/sparse-after-1/retention.csv) · [Environment and hashes](benchmark-results/sparse-after-1/environment.json)
