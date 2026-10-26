# Removing adaptive access and scheduling overhead

Follow-up: [the longer broadcast stability study](BROADCAST_STABILITY_REPORT.md) repeats the 4,000-node anomaly with longer batches, balanced order, and an identical-binary control. The current library core remains unchanged; this report retains the original short-run results.

## Measured outcome

The accessor-layer regression is substantially reduced, with the dense benefit retained relative to pre-adaptive. This is not a claim that every case is faster. At 40,000 nodes:

| Case | Previous µs | Current µs | vs previous | vs pre-adaptive |
| --- | ---: | ---: | ---: | ---: |
| Sparse 1%, light | 42.57 | 42.77 | +0.5% | +2.4% |
| Sparse 1%, 64 rounds | 66.13 | 65.37 | -1.2% | +0.2% |
| Partitioned 10%, light | 348.65 | 332.80 | -4.5% | -3.0% |
| Broadcast 1%, light | 1337.43 | 1279.36 | -4.3% | -0.3% |
| Broadcast 10%, light | 1412.59 | 1372.02 | -2.9% | -2.3% |
| Full resources, light | 2079.76 | 1997.79 | -3.9% | -7.0% |
| Full resources, 64 rounds | 3931.95 | 3789.82 | -3.6% | -3.2% |

Remaining limits: the 40,000-node 1% light sparse case is +0.5% versus previous and +2.4% versus pre-adaptive; this run does not demonstrate its complete elimination. Full light updates at 400 nodes are 2.3–3.0% slower than the previous adaptive build, but still 6.0–6.5% faster than pre-adaptive. The 4,000-node 1% broadcast/light triple result is +4.0% versus previous (+6.3% versus pre-adaptive). All of these are retained in the tables.

That broadcast anomaly had current medians of 141.99 / 122.62 / 131.33 µs; its reconstruction control also varied from 563.31 to 808.11 µs. The original short diagnostic was rerun against the delivered source and passed (current 120.96 µs, reference 135.46 µs). This is consistent with substantial execution noise, not proof that the anomalous triple result should be discarded. The recheck has its own changed baseline timing and does not replace the full-matrix result. An isolated machine and longer runs would be needed to resolve the smallest remaining differences.

No comparison here reruns the older flat-snapshot implementation. The earlier ~12% gap against that implementation is a different baseline.

## Scope

The public API, published 16-slot snapshot layout, atomic publication, failure rollback, optimistic version checks, and owned/borrowed reads are unchanged. This revision removes strategy dispatch from dependency reads and selects scheduling before traversal. The detailed tables compare three freshly executed source trees, not historical timings.

## Diagnosis before implementation

A small differential probe used unchanged benchmark cases, three alternating passes, and a diagnostic 2% threshold for sparse/broadcast regressions. This threshold is a local investigation aid, not a portable CI test or statistical significance test. Raw probe data records every sample summary and core hash.

| Experiment (4,000 nodes, rounds=0) | Sparse 1% | Broadcast 1% | Full resources |
| --- | ---: | ---: | ---: |
| Previous adaptive vs pre-adaptive | −0.5% | +2.8% | −5.6% |
| Direct candidate access vs previous adaptive | −0.9% | −3.2% | +1.6% |
| Old scheduler alone vs previous adaptive | −0.4% | +0.5% | +1.6% |

The broadcast regression reproduced. Removing the accessor layer improved it; reverting scheduling alone did not give a measurable benefit in that small probe. Do not infer that scheduling was the sole cause. The accessor experiment removed both the read representation dispatch and the per-write strategy dispatch, so their individual costs are not separately established.

A larger 40,000-node probe of direct access still showed sparse +2.3%, broadcast +0.7%, and full −4.8% against pre-adaptive. Selecting scheduling once at entry gave sparse +1.4%, broadcast −1.4%, and full −6.4% in a subsequent probe. That was the first large probe to pass the diagnostic threshold. These sequential ablations guided the change; the full rotated comparison below is the performance evidence.

### Concrete overhead removed

1. `ValueView` made every candidate dependency read inspect a representation enum. `CandidateAccess` also dispatched each value write and allocated an extra block-reference directory for fully affected snapshots. Both types are removed. `Context` reads through the same `Snapshot` representation used before adaptive access was introduced.
2. `populate<const ALL: bool>` selects one of two statically specialized loops once per commit. Fully affected candidates retain direct empty-block construction, then write with checked `Arc::get_mut`. Partial candidates use the original copy-on-write setter. There is no unchecked mutation; private blocks cannot escape before publication. This trades the previous once-per-block exclusive borrow for per-write uniqueness checks, while removing access dispatch and its temporary directory.
3. The previous scheduler could traverse a broadcast root, reach the threshold, then traverse that root's outgoing edges again during the scan. It also checked the dynamic threshold on each reachable iteration. Selection now happens once: staged roots >= ceil(V / 4) use the topological scan; fewer roots use one complete reachable traversal plus rank sorting. The root count is a lower bound on affected nodes, not an estimate of final coverage. A one-root broadcast can therefore use reachable scheduling even if nearly every node is affected.

The 25% cutoff remains an internal heuristic. Neither scheduler invokes unaffected callbacks. Snapshot construction still uses the exact final affected count to decide whether all blocks can start empty. No public configuration, dependencies, or unsafe code were added.

## Benchmark method

Apple M2 Pro, 10 logical CPUs, 16 GiB, macOS 27.0.1, native Rust/Cargo 1.99.0. The machine was not CPU-pinned or isolated. Differences of a few percent can reflect scheduling, frequency and other background activity; results are not universal guarantees.

All three trees use identical benchmark source and Cargo.lock. For each of 36 cases, incremental/full_recompute/rebuild modes are unchanged. Every case validates leaf values against fresh reconstruction before timing. Each summary is the median of nine calibrated batch means (~20 ms minimum per batch). Each case runs in three passes; the side order rotates, with direction alternating by case, so each tree runs first once per case. Tables use the median of three run medians, with the final min–max range shown. P10/P90 batch data remain in the CSVs; none is a per-request tail latency or confidence interval.

- **Pre-adaptive** (`reference`): affected-only scheduling and chunked snapshots, before the adaptive revision.
- **Previous** (`before`): the adaptive implementation delivered immediately before this request.
- **Current** (`after`): this revision.

Full recomputation resubmits all resources and includes staging cost; reconstruction builds and destroys the graph. The synthetic performance graph has no business validators. Fresh complete before/after runs separately cover concurrent readers and retention.

## Incremental updates

µs/update. Negative delta means less time. Every matrix case is included.

### partitioned, rounds=0

| Nodes | Roots changed | Pre-adaptive | Previous | Current | Current range | vs previous | vs pre-adaptive |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 400 | 1% | 0.55 | 0.53 | 0.52 | 0.49–0.54 | -2.3% | -6.9% |
| 400 | 10% | 2.68 | 2.72 | 2.62 | 2.57–3.09 | -3.9% | -2.4% |
| 400 | 100% | 18.17 | 16.61 | 16.99 | 16.96–17.12 | +2.3% | -6.5% |
| 4000 | 1% | 3.30 | 3.38 | 3.29 | 3.29–3.82 | -2.5% | -0.0% |
| 4000 | 10% | 29.22 | 33.80 | 28.86 | 28.79–28.86 | -14.6% | -1.2% |
| 4000 | 100% | 201.80 | 188.14 | 190.00 | 187.61–190.32 | +1.0% | -5.8% |
| 40000 | 1% | 41.76 | 42.57 | 42.77 | 41.33–51.45 | +0.5% | +2.4% |
| 40000 | 10% | 343.12 | 348.65 | 332.80 | 327.90–365.33 | -4.5% | -3.0% |
| 40000 | 100% | 2148.73 | 2079.76 | 1997.79 | 1993.49–2033.39 | -3.9% | -7.0% |

### partitioned, rounds=64

| Nodes | Roots changed | Pre-adaptive | Previous | Current | Current range | vs previous | vs pre-adaptive |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 400 | 1% | 0.76 | 0.76 | 0.76 | 0.75–0.81 | -1.2% | -0.5% |
| 400 | 10% | 5.39 | 4.62 | 4.71 | 4.56–5.39 | +2.0% | -12.7% |
| 400 | 100% | 37.21 | 35.65 | 35.66 | 35.13–41.47 | +0.0% | -4.2% |
| 4000 | 1% | 5.37 | 5.34 | 5.26 | 5.25–5.31 | -1.5% | -2.0% |
| 4000 | 10% | 50.97 | 51.75 | 50.87 | 49.97–60.42 | -1.7% | -0.2% |
| 4000 | 100% | 385.30 | 373.60 | 367.41 | 365.91–372.36 | -1.7% | -4.6% |
| 40000 | 1% | 65.21 | 66.13 | 65.37 | 62.05–66.22 | -1.2% | +0.2% |
| 40000 | 10% | 568.79 | 580.07 | 567.49 | 547.66–606.93 | -2.2% | -0.2% |
| 40000 | 100% | 3913.95 | 3931.95 | 3789.82 | 3752.34–3796.34 | -3.6% | -3.2% |

### broadcast, rounds=0

| Nodes | Roots changed | Pre-adaptive | Previous | Current | Current range | vs previous | vs pre-adaptive |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 400 | 1% | 12.53 | 12.61 | 12.31 | 12.11–12.39 | -2.4% | -1.7% |
| 400 | 10% | 13.05 | 13.46 | 13.12 | 12.99–13.21 | -2.6% | +0.5% |
| 400 | 100% | 18.43 | 16.83 | 17.33 | 17.29–33.12 | +3.0% | -6.0% |
| 4000 | 1% | 123.56 | 126.26 | 131.33 | 122.62–141.99 | +4.0% | +6.3% |
| 4000 | 10% | 132.62 | 135.77 | 131.72 | 131.36–131.95 | -3.0% | -0.7% |
| 4000 | 100% | 202.07 | 188.67 | 188.12 | 186.07–190.45 | -0.3% | -6.9% |
| 40000 | 1% | 1282.81 | 1337.43 | 1279.36 | 1266.59–1354.39 | -4.3% | -0.3% |
| 40000 | 10% | 1404.79 | 1412.59 | 1372.02 | 1331.65–1382.49 | -2.9% | -2.3% |
| 40000 | 100% | 2157.99 | 2034.05 | 2019.74 | 1985.96–2026.72 | -0.7% | -6.4% |

### broadcast, rounds=64

| Nodes | Roots changed | Pre-adaptive | Previous | Current | Current range | vs previous | vs pre-adaptive |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 400 | 1% | 31.29 | 31.59 | 30.90 | 30.72–31.15 | -2.2% | -1.2% |
| 400 | 10% | 31.43 | 32.60 | 31.67 | 31.52–31.78 | -2.9% | +0.8% |
| 400 | 100% | 37.34 | 36.15 | 35.55 | 35.54–35.76 | -1.6% | -4.8% |
| 4000 | 1% | 305.71 | 306.82 | 303.50 | 302.65–305.91 | -1.1% | -0.7% |
| 4000 | 10% | 307.95 | 318.22 | 308.26 | 306.62–633.36 | -3.1% | +0.1% |
| 4000 | 100% | 383.98 | 375.08 | 367.98 | 366.35–370.94 | -1.9% | -4.2% |
| 40000 | 1% | 3044.78 | 3073.38 | 3067.89 | 3019.39–3075.52 | -0.2% | +0.8% |
| 40000 | 10% | 3145.39 | 3260.91 | 3163.42 | 3116.23–3194.95 | -3.0% | +0.6% |
| 40000 | 100% | 3919.91 | 3900.53 | 3812.33 | 3810.92–3813.85 | -2.3% | -2.7% |

## Full recomputation and reconstruction controls

40,000 nodes, partitioned graph, µs/update.

| Roots changed | Rounds | Mode | Pre-adaptive | Previous | Current |
| ---: | ---: | --- | ---: | ---: | ---: |
| 1% | 0 | full_recompute | 2125.95 | 2067.59 | 2076.65 |
| 1% | 0 | rebuild | 5794.72 | 5910.17 | 6475.49 |
| 1% | 64 | full_recompute | 3905.16 | 3893.84 | 3751.83 |
| 1% | 64 | rebuild | 7568.74 | 7694.18 | 7718.50 |
| 10% | 0 | full_recompute | 2140.73 | 2079.42 | 2014.02 |
| 10% | 0 | rebuild | 5807.48 | 5871.30 | 5989.09 |
| 10% | 64 | full_recompute | 3938.62 | 3940.55 | 3782.60 |
| 10% | 64 | rebuild | 7642.68 | 7679.44 | 7647.05 |
| 100% | 0 | full_recompute | 2128.82 | 2062.95 | 1998.11 |
| 100% | 0 | rebuild | 5851.36 | 5830.74 | 5890.34 |
| 100% | 64 | full_recompute | 3934.95 | 4485.18 | 3819.47 |
| 100% | 64 | rebuild | 7698.55 | 7688.99 | 7648.83 |

## Reader and retention controls

Snapshot acquisition and published storage did not change. The following fresh full runs are sequential controls, not rotated measurements. Read throughput must be interpreted together with writer throughput; changes can reflect background load.

| Writer | Readers | API | Previous M reads/s | Current M reads/s | Previous writes/s | Current writes/s |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| false | 1 | acquire | 71.14 | 69.99 | 0 | 0 |
| false | 1 | pinned | 115.99 | 115.03 | 0 | 0 |
| false | 1 | borrowed | 89.70 | 89.22 | 0 | 0 |
| false | 2 | acquire | 15.80 | 21.83 | 0 | 0 |
| false | 2 | pinned | 233.35 | 231.69 | 0 | 0 |
| false | 2 | borrowed | 180.34 | 181.27 | 0 | 0 |
| false | 4 | acquire | 9.09 | 9.28 | 0 | 0 |
| false | 4 | pinned | 462.23 | 465.23 | 0 | 0 |
| false | 4 | borrowed | 359.63 | 362.04 | 0 | 0 |
| false | 8 | acquire | 7.24 | 7.06 | 0 | 0 |
| false | 8 | pinned | 702.39 | 758.36 | 0 | 0 |
| false | 8 | borrowed | 542.38 | 594.43 | 0 | 0 |
| true | 1 | acquire | 23.50 | 34.04 | 796418 | 1077192 |
| true | 1 | pinned | 92.58 | 79.45 | 1641982 | 1461726 |
| true | 1 | borrowed | 33.79 | 44.48 | 911157 | 1066872 |
| true | 2 | acquire | 13.83 | 13.52 | 555997 | 591949 |
| true | 2 | pinned | 183.15 | 177.59 | 1422409 | 1272548 |
| true | 2 | borrowed | 92.07 | 103.17 | 632474 | 789407 |
| true | 4 | acquire | 10.27 | 9.79 | 403311 | 404648 |
| true | 4 | pinned | 351.10 | 292.83 | 1289082 | 955227 |
| true | 4 | borrowed | 199.72 | 209.95 | 402707 | 439306 |
| true | 8 | acquire | 8.15 | 7.62 | 217264 | 211402 |
| true | 8 | pinned | 476.97 | 431.80 | 972348 | 694253 |
| true | 8 | borrowed | 353.63 | 374.36 | 257074 | 234551 |

All 36 retention configurations have identical tracked live/released payload bytes before and after. Releasing history returns tracked payload storage to 3,276,800 bytes. RSS includes allocator behavior and is not equivalent to tracked payload storage. Values below are medians of three fresh processes, with 800 nodes and 4 KiB payloads.

| History kept | Roots changed | Live MiB | Previous commit µs | Current commit µs | Previous retained RSS MiB | Current retained RSS MiB | Previous release µs | Current release µs |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 1% | 3.125 | 1.46 | 1.42 | 5.73 | 5.75 | 0.12 | 0.08 |
| 0 | 10% | 3.125 | 15.25 | 13.58 | 6.17 | 6.17 | 0.12 | 0.12 |
| 0 | 100% | 3.125 | 127.04 | 130.21 | 9.78 | 9.75 | 0.21 | 0.17 |
| 1 | 1% | 3.156 | 1.54 | 1.42 | 5.81 | 5.78 | 1.00 | 1.62 |
| 1 | 10% | 3.438 | 13.79 | 13.21 | 6.52 | 6.47 | 10.04 | 6.04 |
| 1 | 100% | 6.250 | 134.29 | 133.29 | 12.98 | 12.98 | 32.83 | 31.54 |
| 10 | 1% | 3.438 | 1.58 | 1.54 | 6.06 | 6.09 | 7.04 | 5.67 |
| 10 | 10% | 6.250 | 15.25 | 14.58 | 9.42 | 9.47 | 37.17 | 37.25 |
| 10 | 100% | 34.375 | 224.58 | 206.29 | 41.81 | 41.77 | 1089.62 | 1132.62 |
| 100 | 1% | 6.250 | 1.92 | 1.88 | 9.11 | 9.08 | 52.29 | 47.21 |
| 100 | 10% | 34.375 | 29.25 | 28.17 | 39.09 | 39.08 | 1317.17 | 1362.12 |
| 100 | 100% | 315.625 | 258.33 | 192.67 | 329.22 | 329.20 | 33123.67 | 17430.88 |

## Validation and reproduction

**47 library/integration/rustdoc tests pass**, plus three README examples and the benchmark smoke check. The new broadcast scheduler regression test was run against the previous implementation and failed, then passed with the fix. Three new integration tests cover 37-node tail blocks and forward-reference joins through complete/partial updates and late failure, retained snapshot histories, consistent foreign/undeclared-dependency rejection, and full-candidate payload reclamation after the commit guard. Existing concurrent-reader, rollback, panic, epoch-wrap and destructor lock-release tests pass. Native cargo fmt/check/clippy (-D warnings)/test, doctests and config_reload passed. Rust 1.88, other platforms and exhaustive model checking remain untested.

- [Triple environment/hashes](benchmark-results/access-scheduler-triple/environment.json): **324 update summaries and 2,916 batch samples per tree**.
- [Pre-adaptive summaries](benchmark-results/access-scheduler-triple/reference-updates.csv), [previous summaries](benchmark-results/access-scheduler-triple/before-updates.csv), [current summaries](benchmark-results/access-scheduler-triple/after-updates.csv). Raw batches are alongside these files as `*-samples.csv`.
- [Fresh full before](benchmark-results/access-scheduler-before/environment.json), [fresh full after](benchmark-results/access-scheduler-after/environment.json): each contains 108 update summaries / 972 batches, 24 reader summaries / 120 samples, and 36 retention-process rows.
- [Diagnostic experiments](benchmark-results/access-scheduler-triple/diagnostics/README.md), [native check results](benchmark-results/access-scheduler-triple/validation/results.json).
- [Previous source archive](benchmark-results/access-scheduler-triple/previous-source.tar.gz), [pre-adaptive source archive](benchmark-results/access-scheduler-triple/reference-source.tar.gz).

Extract each baseline archive into a separate directory. Run from the current package:

```sh
python3 benches/compare.py --baseline /path/to/previous --reference /path/to/pre-adaptive --output benchmark-results/repeat-triple --passes 3
python3 benches/run.py --output benchmark-results/repeat-full
```

The benchmark body is unchanged. `compare.py` adds only an optional reference tree and rotates execution order; the existing two-tree invocation remains supported. Report tables, CSV counts, source hashes and retained-payload equality were checked. Earlier benchmark reports describe earlier revisions.
