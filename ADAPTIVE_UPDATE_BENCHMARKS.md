# Adaptive updates: scheduling and candidate construction

## Measured outcome

Full-resource, light-compute updates improve by **9.1% / 6.2% / 3.8%** at 400 / 4,000 / 40,000 nodes in the partitioned graph. At 40,000 nodes, the median falls from **2,102.37 to 2,021.70 µs**. The 64-round case is **1.0% slower** (3,768.98 → 3,806.39 µs). This is a partial reduction in dense-update overhead, **not evidence that the historical ~12% gap versus flat snapshots has been eliminated**; the flat implementation was not rerun in this comparison.

The sparse algorithm and shared-block benefits remain, but the new strategy dispatch is not free. For 40,000 nodes and 1% independent resource changes, medians move from **40.91 to 42.81 µs (+4.6%)** for light computation and **60.74 to 64.01 µs (+5.4%)** for 64 rounds. Those costs remain far below full recomputation, but do not claim zero sparse regression. Broadcast cases with 1%/10% root changes are 1.2–5.4% slower at 40,000 nodes; topological scanning is not automatically faster than sorting the existing reachable set in these graphs. The heuristic is a tradeoff, not a universal optimization.

Background load was observed and the after ranges overlap some before measurements. Three alternating passes reduce order bias but do not provide isolation or statistical significance. All regressions and raw samples are included below.

## Implementation and guarantees

Public APIs and published snapshot layout are unchanged. Sparse scheduling walks only reachable nodes and sorts their topological ranks. At ceil(V / 4) discovered nodes, scheduling switches to a forward topological pass that finishes exact reachability. Unaffected callbacks never execute. The heuristic counts affected nodes, not changed resources.

Snapshot construction makes a separate decision: when A = V, create an empty directory directly, skip per-block affected counting, and borrow each new block exclusively once. Writes then avoid per-node Arc ownership checks; dependency reads have no shared/exclusive block branch. When A < V, retain the previous lazy copy-on-write path, including for broadcast updates. This preserves sharing of unaffected resources. The two strategy decisions deliberately have different thresholds.

Every candidate stays private until computation and all validators succeed. Failure/panic preserves the old version; retired/rejected payload owners outlive the writer lock. No unsafe code or new dependencies were added.

## Method

Apple M2 Pro, 10 logical CPUs, 16 GiB, macOS 27.0.1; native Rust/Cargo 1.99.0 release benchmark. This machine was not isolated: other CPU-intensive work was observed during the experiments. Treat small differences and outliers cautiously; these results do not establish a universal speedup.

The primary comparison runs the same 36 update cases, each with incremental/full_recompute/rebuild, in three passes. Before/after order alternates by case and pass. Both trees use identical benchmark and lockfile hashes. A case selector runs outside timing; timed operations and the default matrix are unchanged. Each result is nine calibrated batch means (~20 ms minimum per batch), with correctness checked against a fresh graph before timing. Table values are medians of three run medians, not individual request latency or confidence intervals. The after range shows run-to-run variability.

A separate final full run covers readers and retention using the unchanged matrix. Its core hash matches the paired after build and delivered source. The fresh pre-change full run supplies reader/retention controls; these are sequential observations, not synchronized statistical controls. Full recomputation resubmits all resources; rebuild includes graph construction and destruction. No business validators are installed in this synthetic graph.

## Incremental updates

Microseconds/update; negative change is faster. All cases are shown, including regressions.

### partitioned, rounds=0

| Nodes | Changed roots | Before | After | After range | Change |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 400 | 1% | 0.53 | 0.53 | 0.50–0.76 | -0.0% |
| 400 | 10% | 2.70 | 2.69 | 2.66–2.75 | -0.6% |
| 400 | 100% | 18.47 | 16.80 | 16.72–16.82 | -9.1% |
| 4000 | 1% | 3.35 | 3.51 | 3.32–3.66 | +4.9% |
| 4000 | 10% | 29.13 | 29.52 | 29.42–29.80 | +1.4% |
| 4000 | 100% | 200.87 | 188.46 | 185.72–190.47 | -6.2% |
| 40000 | 1% | 40.91 | 42.81 | 39.87–43.91 | +4.6% |
| 40000 | 10% | 312.91 | 324.21 | 318.19–330.99 | +3.6% |
| 40000 | 100% | 2102.37 | 2021.70 | 1938.66–2034.80 | -3.8% |

### partitioned, rounds=64

| Nodes | Changed roots | Before | After | After range | Change |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 400 | 1% | 0.76 | 0.76 | 0.75–0.76 | -0.7% |
| 400 | 10% | 4.61 | 4.69 | 4.63–4.71 | +1.7% |
| 400 | 100% | 36.97 | 36.09 | 35.91–36.19 | -2.4% |
| 4000 | 1% | 5.30 | 5.37 | 5.36–5.49 | +1.3% |
| 4000 | 10% | 50.38 | 50.97 | 50.10–52.60 | +1.2% |
| 4000 | 100% | 383.00 | 373.83 | 372.46–374.03 | -2.4% |
| 40000 | 1% | 60.74 | 64.01 | 63.52–65.97 | +5.4% |
| 40000 | 10% | 538.13 | 549.61 | 523.29–553.04 | +2.1% |
| 40000 | 100% | 3768.98 | 3806.39 | 3724.24–3873.24 | +1.0% |

### broadcast, rounds=0

| Nodes | Changed roots | Before | After | After range | Change |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 400 | 1% | 12.45 | 12.72 | 12.71–12.76 | +2.2% |
| 400 | 10% | 13.11 | 13.49 | 13.45–15.36 | +2.9% |
| 400 | 100% | 18.35 | 16.73 | 16.73–16.74 | -8.8% |
| 4000 | 1% | 123.19 | 125.65 | 125.52–127.53 | +2.0% |
| 4000 | 10% | 131.08 | 134.61 | 132.85–134.99 | +2.7% |
| 4000 | 100% | 202.27 | 186.09 | 185.99–188.36 | -8.0% |
| 40000 | 1% | 1213.89 | 1231.09 | 1226.50–1266.83 | +1.4% |
| 40000 | 10% | 1295.87 | 1365.22 | 1331.12–1493.54 | +5.4% |
| 40000 | 100% | 2102.84 | 2037.14 | 2007.01–2054.21 | -3.1% |

### broadcast, rounds=64

| Nodes | Changed roots | Before | After | After range | Change |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 400 | 1% | 31.21 | 32.28 | 31.80–32.92 | +3.4% |
| 400 | 10% | 31.50 | 32.24 | 32.17–33.03 | +2.4% |
| 400 | 100% | 37.13 | 35.88 | 35.42–35.92 | -3.4% |
| 4000 | 1% | 302.04 | 307.02 | 306.21–312.81 | +1.7% |
| 4000 | 10% | 307.44 | 315.75 | 311.48–320.28 | +2.7% |
| 4000 | 100% | 382.26 | 369.24 | 366.45–379.81 | -3.4% |
| 40000 | 1% | 2912.53 | 3036.28 | 2964.68–3038.06 | +4.2% |
| 40000 | 10% | 3041.20 | 3078.24 | 3074.96–3223.47 | +1.2% |
| 40000 | 100% | 3876.56 | 3817.93 | 3763.02–3882.53 | -1.5% |

## Full recomputation and reconstruction controls

40,000 nodes, partitioned graph; medians of the same three passes, µs/update.

| Changed roots | Rounds | Mode | Before | After | Change |
| ---: | ---: | --- | ---: | ---: | ---: |
| 1% | 0 | full_recompute | 2101.16 | 2049.62 | -2.5% |
| 1% | 0 | rebuild | 5775.41 | 5829.86 | +0.9% |
| 1% | 64 | full_recompute | 3829.28 | 3808.14 | -0.6% |
| 1% | 64 | rebuild | 7484.56 | 7514.67 | +0.4% |
| 10% | 0 | full_recompute | 2055.96 | 1998.05 | -2.8% |
| 10% | 0 | rebuild | 5740.69 | 5723.92 | -0.3% |
| 10% | 64 | full_recompute | 3813.20 | 3817.35 | +0.1% |
| 10% | 64 | rebuild | 7459.62 | 7506.93 | +0.6% |
| 100% | 0 | full_recompute | 2090.54 | 2018.70 | -3.4% |
| 100% | 0 | rebuild | 5725.38 | 5674.78 | -0.9% |
| 100% | 64 | full_recompute | 3771.29 | 3800.86 | +0.8% |
| 100% | 64 | rebuild | 7417.27 | 7436.12 | +0.3% |

## Concurrent reads

Million reads/second from the fresh full before and final full after runs. The published block layout and snapshot acquisition methods did not change. Background load and writer throughput can affect these results.

| Writer | Readers | API | Before M/s | After M/s | Before writes/s | After writes/s |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| false | 1 | acquire | 70.58 | 73.33 | 0 | 0 |
| false | 1 | pinned | 114.71 | 119.44 | 0 | 0 |
| false | 1 | borrowed | 91.81 | 92.98 | 0 | 0 |
| false | 2 | acquire | 17.42 | 21.37 | 0 | 0 |
| false | 2 | pinned | 230.58 | 235.35 | 0 | 0 |
| false | 2 | borrowed | 183.62 | 183.07 | 0 | 0 |
| false | 4 | acquire | 9.02 | 10.04 | 0 | 0 |
| false | 4 | pinned | 452.28 | 469.10 | 0 | 0 |
| false | 4 | borrowed | 364.43 | 361.88 | 0 | 0 |
| false | 8 | acquire | 7.25 | 7.18 | 0 | 0 |
| false | 8 | pinned | 726.42 | 732.36 | 0 | 0 |
| false | 8 | borrowed | 366.32 | 575.42 | 0 | 0 |
| true | 1 | acquire | 26.57 | 32.02 | 930707 | 1030039 |
| true | 1 | pinned | 102.40 | 83.96 | 1726101 | 1451957 |
| true | 1 | borrowed | 41.63 | 54.87 | 1039080 | 1351062 |
| true | 2 | acquire | 14.10 | 13.50 | 628374 | 612694 |
| true | 2 | pinned | 200.78 | 169.08 | 1545649 | 1174877 |
| true | 2 | borrowed | 92.93 | 85.88 | 707033 | 635548 |
| true | 4 | acquire | 9.69 | 9.72 | 366072 | 403163 |
| true | 4 | pinned | 376.57 | 302.12 | 1401612 | 950729 |
| true | 4 | borrowed | 207.27 | 193.27 | 420003 | 379873 |
| true | 8 | acquire | 7.45 | 7.46 | 185680 | 207130 |
| true | 8 | pinned | 547.51 | 452.44 | 1070890 | 782114 |
| true | 8 | borrowed | 359.47 | 370.76 | 247394 | 223340 |

## Retained snapshots

800 nodes, 4 KiB payloads, three fresh processes per configuration. Live payload bytes agree exactly before/after for all 36 rows; releasing history returns tracked payload storage to 3,276,800 bytes. RSS includes allocator retention and is not a payload-size measurement. Columns below are medians across processes.

| Kept | Changed roots | Live payload MiB | Before commit µs | After commit µs | Before retained RSS MiB | After retained RSS MiB | Before release µs | After release µs |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 1% | 3.125 | 1.46 | 1.50 | 5.80 | 5.81 | 0.21 | 0.21 |
| 0 | 10% | 3.125 | 15.46 | 14.00 | 6.09 | 6.22 | 0.17 | 0.17 |
| 0 | 100% | 3.125 | 142.71 | 113.75 | 9.08 | 9.80 | 0.29 | 0.17 |
| 1 | 1% | 3.156 | 1.54 | 1.46 | 5.83 | 5.81 | 3.33 | 1.29 |
| 1 | 10% | 3.438 | 14.79 | 15.00 | 6.47 | 6.48 | 17.71 | 5.12 |
| 1 | 100% | 6.250 | 147.62 | 123.63 | 12.30 | 13.00 | 81.92 | 56.58 |
| 10 | 1% | 3.438 | 1.62 | 1.54 | 6.16 | 6.14 | 17.21 | 5.88 |
| 10 | 10% | 6.250 | 15.67 | 14.79 | 9.41 | 9.47 | 60.00 | 93.71 |
| 10 | 100% | 34.375 | 237.58 | 237.79 | 41.06 | 41.84 | 1400.96 | 1216.83 |
| 100 | 1% | 6.250 | 2.00 | 2.00 | 9.16 | 9.11 | 67.04 | 52.33 |
| 100 | 10% | 34.375 | 34.62 | 27.54 | 39.09 | 39.16 | 2042.54 | 1424.33 |
| 100 | 100% | 315.625 | 631.21 | 384.29 | 262.81 | 330.05 | 57658.62 | 26397.00 |

## Validation

43 library/integration/rustdoc tests pass, plus three README examples and the benchmark smoke check. New tests cover threshold transitions, exact scheduling against an independent traversal on permuted DAGs, unchanged callback counts/history, failure/panic/validation rollback with recovery, destructor execution after unlocking, and concurrent owned/borrowed readers while strategies alternate. Native cargo fmt/check/clippy (-D warnings)/test, doctests and config_reload all pass. Minimum Rust 1.88, other platforms and exhaustive interleaving model checks remain untested.

## Evidence and reproduction

- [Paired environment and source hashes](benchmark-results/adaptive-selected-paired/environment.json): 324 summaries and 2,916 raw batch samples **per side**.
- [Before summaries](benchmark-results/adaptive-selected-paired/before-updates.csv), [after summaries](benchmark-results/adaptive-selected-paired/after-updates.csv), [before batches](benchmark-results/adaptive-selected-paired/before-samples.csv), [after batches](benchmark-results/adaptive-selected-paired/after-samples.csv).
- [Final full environment](benchmark-results/adaptive-selected-full/environment.json), [updates](benchmark-results/adaptive-selected-full/updates.csv), [readers](benchmark-results/adaptive-selected-full/readers.csv), [retention](benchmark-results/adaptive-selected-full/retention.csv): 108 update summaries / 972 batches, 24 reader summaries / 120 samples, 36 retention processes.
- [Fresh baseline environment](benchmark-results/adaptive-before/environment.json) and [baseline source archive](benchmark-results/adaptive-selected-paired/baseline-source.tar.gz).

Exploratory data is retained separately and excluded from the final comparison: adaptive-after-1 used a contiguous staging prototype; adaptive-paired and adaptive-after-2 used mixed shared/exclusive block references; adaptive-paired-final added inlining to that discarded design (despite its directory name, it is not the delivered implementation). Those paths added access/conversion overhead and showed substantial timing noise. See source hashes to distinguish builds.

Extract the baseline archive into a separate directory, then run from this package:

```sh
python3 benches/compare.py --baseline /path/to/extracted-baseline --output benchmark-results/repeat-paired --passes 3
python3 benches/run.py --output benchmark-results/repeat-full
```

The baseline archive includes the same case-selecting benchmark as the final package. The earlier adaptive-before full run predates this untimed selector; the core and timed workloads are identical. Existing historical sparse/borrowed reports remain historical measurements, not results for this revision.
