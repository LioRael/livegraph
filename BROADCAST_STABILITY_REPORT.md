# Longer broadcast measurements: regression or noise?

## Decision

**Keep the delivered library implementation unchanged.** The target incremental case did not reproduce a stable +4% regression against the preceding adaptive build. Its median paired difference is **−0.13%**, with a descriptive 95% whole-block bootstrap interval of **−2.54% to +3.83%**. Against pre-adaptive it is **+0.41%** (−5.34% to +2.79%).

The identical-binary A/A control has a median difference of +0.21%, an interval of −4.12% to +5.12%, and individual block differences from **−10.84% to +16.65%**. This directly demonstrates execution variability greater than the previously reported +4% point estimate. Both code comparisons include zero and overlap the A/A variability. These observations do not support tuning the implementation to that earlier point estimate.

This is **not proof that all of the earlier difference was noise**, nor an equivalence result within ±2%. A small real overhead remains unresolved on this shared host. A longer run did not make environmental disturbance disappear. No favorable rerun or outlier exclusion is substituted for these results.

The controls also prevent a blanket “everything is noise” conclusion: main-case full recomputation is **+3.79% versus the preceding adaptive build** (descriptive interval +3.15% to +6.34%). Its point estimate remains −4.59% versus pre-adaptive, but that interval crosses zero. This is a separate full-update tradeoff, consistent with why the earlier report retained small dense-case differences; it is not evidence of an incremental broadcast regression. Fresh reconstruction also varies. All modes are reported below.

The library source is byte-for-byte identical to the prior delivery; the sparse/dense algorithms, public API, snapshot representation, atomic publication and rollback logic have not been changed. No new optimization is justified by the target-case evidence from this experiment.

## Experiment held constant

The library implementation is unchanged. Its source SHA-256 matches the previous delivered revision. This investigation changes only benchmark controls and documentation. The same typed graph, update operations, snapshot publication, and correctness checks run in all trees.

Three frozen versions are measured: pre-adaptive (`reference`), the earlier adaptive build (`previous`), and the delivered access/scheduler fix (`current`). `current_repeat` invokes the **identical current executable** a second time, providing an A/A execution-noise control. The primary comparison always uses the preassigned `current` label; it is not selected or averaged with the faster duplicate after seeing the results.

## Protocol

- Main case: **4,000 nodes, broadcast graph, 1% changed roots, zero synthetic compute rounds**. Twelve fresh-process blocks per label; each mode uses eleven batches calibrated to **150 ms**, versus the former nine ~20 ms batches and three passes.
- Every invocation measures incremental update, full recomputation, and fresh reconstruction. Full recomputation stages all resources; reconstruction includes graph construction and destruction. Calibration itself warms the workload but is not included in the retained batch means.
- Four-side balanced orders repeat three times: reference / previous / duplicate / current; previous / current / reference / duplicate; current / duplicate / previous / reference; duplicate / reference / current / previous. Each label occupies each position three times, and adjacent ordering is balanced. The starting mode rotates each four-block cycle, so all three modes occur first for every label/position combination.
- Neighbor controls: broadcast 10%/light, broadcast 1%/64 rounds, and partitioned 1%/light, all with 4,000 nodes. Each has four balanced blocks, nine batches targeted at 100 ms, and the same A/A duplicate. These controls are descriptive and have less repetition than the main case.
- No outliers, blocks, or processes are removed. Load averages and process durations are recorded. The host is not CPU-pinned or isolated, and other processes are not stopped. A/A measures execution variability, not every possible environmental bias.

Measured host: macOS-27.0.1-arm64-arm-64bit, 10 logical CPUs, rustc 1.99.0 (b940084d7 2026-09-28). Elapsed measurement time: **683.6 seconds**, excluding compilation. All three benchmark and lockfile hashes match; only the intended core revisions differ.

## Main result

Times below are medians of the twelve process medians, in µs/update. The duplicate is shown independently. Detailed paired differences follow; ratios of aggregate medians can differ from medians of within-block ratios.

| Mode | Pre-adaptive | Earlier adaptive | Current | Same-binary duplicate |
| --- | ---: | ---: | ---: | ---: |
| incremental | 132.75 | 133.89 | 133.71 | 132.63 |
| full_recompute | 215.80 | 198.14 | 205.68 | 204.42 |
| rebuild | 591.52 | 588.83 | 601.37 | 604.47 |

Paired percentage changes use log(current/baseline) within each complete block and then the median. Negative means faster. Bootstrap intervals resample **whole blocks**, with a fixed seed and 20,000 resamples; batch samples are not treated as independent experiments. Intervals are descriptive sensitivity estimates under an exchangeable-block assumption. Serial drift and dependence can invalidate formal coverage; twelve blocks cannot establish exact equality or universality. A practical 2% difference is the investigation threshold inherited from the earlier local diagnostic, not a product guarantee.

| Mode | Comparison | Median change | 95% block-bootstrap interval | Block range | Faster blocks |
| --- | --- | ---: | ---: | ---: | ---: |
| incremental | vs_reference | +0.41% | -5.34% … +2.79% | -63.17% … +14.07% | 5/12 |
| incremental | vs_previous | -0.13% | -2.54% … +3.83% | -66.03% … +11.39% | 6/12 |
| incremental | aa | +0.21% | -4.12% … +5.12% | -10.84% … +16.65% | 5/12 |
| full_recompute | vs_reference | -4.59% | -6.27% … +1.92% | -35.16% … +362.12% | 9/12 |
| full_recompute | vs_previous | +3.79% | +3.15% … +6.34% | +1.72% … +405.46% | 0/12 |
| full_recompute | aa | -0.88% | -6.68% … +0.75% | -79.61% … +275.25% | 8/12 |
| rebuild | vs_reference | +2.64% | -4.91% … +14.97% | -72.79% … +274.53% | 3/12 |
| rebuild | vs_previous | +2.01% | +0.67% … +9.84% | -29.39% … +274.75% | 1/12 |
| rebuild | aa | -0.09% | -4.69% … +3.25% | -72.81% … +73.07% | 6/12 |

## Every main block

Incremental-update process medians, µs/update. The final column compares the two labels that run the same binary; it must not be interpreted as a code change.

| Block | Run order | Pre-adaptive | Earlier adaptive | Current | Duplicate | Current vs pre-adaptive | Duplicate vs current |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | reference → previous → current_repeat → current | 131.94 | 133.78 | 132.05 | 132.56 | +0.09% | +0.38% |
| 2 | previous → current → reference → current_repeat | 387.55 | 135.28 | 142.75 | 159.84 | -63.17% | +11.98% |
| 3 | current → current_repeat → previous → reference | 149.01 | 133.58 | 140.82 | 130.36 | -5.49% | -7.43% |
| 4 | current_repeat → reference → current → previous | 132.81 | 136.01 | 151.50 | 135.08 | +14.07% | -10.84% |
| 5 | reference → previous → current_repeat → current | 132.70 | 396.70 | 134.78 | 157.22 | +1.57% | +16.65% |
| 6 | previous → current → reference → current_repeat | 137.63 | 134.65 | 130.48 | 139.00 | -5.19% | +6.53% |
| 7 | current → current_repeat → previous → reference | 134.14 | 138.52 | 139.96 | 130.29 | +4.34% | -6.91% |
| 8 | current_repeat → reference → current → previous | 130.82 | 133.36 | 129.92 | 130.75 | -0.69% | +0.64% |
| 9 | reference → previous → current_repeat → current | 129.83 | 132.86 | 130.79 | 130.75 | +0.74% | -0.03% |
| 10 | previous → current → reference → current_repeat | 129.37 | 133.66 | 130.32 | 128.70 | +0.74% | -1.24% |
| 11 | current → current_repeat → previous → reference | 168.84 | 131.07 | 132.65 | 132.70 | -21.43% | +0.04% |
| 12 | current_repeat → reference → current → previous | 131.74 | 134.01 | 137.05 | 142.18 | +4.03% | +3.74% |

## Order and time sensitivity

The groups below each contain only four or six paired observations; they are checks for large ordering effects, not independently powered experiments.

| Main incremental grouping | Blocks | Median current vs pre-adaptive | Median duplicate vs current |
| --- | ---: | ---: | ---: |
| mode rotation 1 | 4 | -2.74% | -3.60% |
| mode rotation 2 | 4 | +0.43% | +3.54% |
| mode rotation 3 | 4 | +0.74% | +0.00% |
| current runs before reference | 6 | -5.34% | -0.60% |
| current runs after reference | 6 | +1.15% | +0.51% |

Recorded one-minute load average ranged from 6.29 to 19.79; it is context, not a CPU-temperature or per-process interference measurement. The experiment cannot determine which background activity caused a specific slow sample.

## Neighbor and dense controls

Four blocks per label, process medians in µs/update. All three modes are included. Changes are paired medians, and the A/A range shows all four duplicate/current block ratios.

| Case | Mode | Pre-adaptive | Earlier adaptive | Current | vs pre-adaptive | vs earlier adaptive | A/A range |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1000:true:10:0 | incremental | 156.36 | 202.58 | 146.86 | -6.58% | -26.87% | -4.89% … +122.99% |
| 1000:true:10:0 | full_recompute | 220.70 | 214.17 | 222.80 | -5.32% | +1.40% | -48.23% … +3.55% |
| 1000:true:10:0 | rebuild | 645.84 | 613.62 | 615.22 | -6.17% | -1.78% | -2.79% … +80.25% |
| 1000:true:1:64 | incremental | 329.21 | 337.65 | 321.73 | -1.44% | -2.11% | -82.41% … +1.31% |
| 1000:true:1:64 | full_recompute | 411.80 | 401.53 | 414.63 | -3.38% | +3.38% | -75.99% … +1.15% |
| 1000:true:1:64 | rebuild | 784.26 | 788.47 | 812.66 | +7.17% | +7.38% | -72.66% … +412.90% |
| 1000:false:1:0 | incremental | 3.55 | 3.64 | 3.59 | +1.01% | -2.28% | -24.65% … +1.55% |
| 1000:false:1:0 | full_recompute | 215.23 | 194.13 | 196.81 | -7.17% | +0.81% | -7.47% … +5.53% |
| 1000:false:1:0 | rebuild | 581.72 | 582.62 | 592.63 | +1.88% | +1.76% | -16.56% … +0.02% |

## CPU/wall-time follow-up

After the long run finished, a separate four-block probe ran the same four labels and identical executables, with eleven 150 ms-targeted batches per mode. Parent-side `getrusage(RUSAGE_CHILDREN)` differences record child user/system CPU time and context switches. No CPU probe overlapped the long experiment. These are **whole-process totals**, including setup, calibration and all three modes. They cannot be used to subtract scheduling time from an individual incremental batch or normalize its latency.

| Label | Median CPU/wall | Range |
| --- | ---: | ---: |
| reference | 98.9% | 52.1%–99.0% |
| previous | 98.5% | 66.3%–98.8% |
| current | 97.7% | 93.2%–99.0% |
| current_repeat | 98.7% | 97.3%–99.3% |

Every process is retained below. A low CPU/wall fraction establishes time not accounted for as child CPU execution, but does not identify the responsible process or separate all scheduling, launch, page-fault and I/O effects. Frequency/cache effects may also change CPU time itself. The supplemental probe has only four blocks and cannot certify equivalence.

| Block | Position | Label | Wall seconds | Child CPU seconds | CPU/wall |
| ---: | ---: | --- | ---: | ---: | ---: |
| 1 | 1 | reference | 11.881 | 6.184 | 52.1% |
| 1 | 2 | previous | 8.191 | 8.088 | 98.7% |
| 1 | 3 | current_repeat | 8.213 | 8.096 | 98.6% |
| 1 | 4 | current | 10.371 | 10.055 | 97.0% |
| 2 | 1 | previous | 10.057 | 9.881 | 98.2% |
| 2 | 2 | current | 8.188 | 8.064 | 98.5% |
| 2 | 3 | reference | 10.196 | 10.092 | 99.0% |
| 2 | 4 | current_repeat | 9.870 | 9.796 | 99.3% |
| 3 | 1 | current | 11.356 | 10.585 | 93.2% |
| 3 | 2 | current_repeat | 8.506 | 8.279 | 97.3% |
| 3 | 3 | previous | 15.902 | 10.548 | 66.3% |
| 3 | 4 | reference | 10.143 | 10.043 | 99.0% |
| 4 | 1 | current_repeat | 10.105 | 9.994 | 98.9% |
| 4 | 2 | reference | 10.140 | 10.027 | 98.9% |
| 4 | 3 | current | 10.076 | 9.979 | 99.0% |
| 4 | 4 | previous | 10.027 | 9.905 | 98.8% |

Probe duration excluding compilation: 163.3 seconds. [Probe metadata](benchmark-results/broadcast-cpu-probe/environment.json), [all 16 CPU/wall records](benchmark-results/broadcast-cpu-probe/runs.csv), [48 summaries](benchmark-results/broadcast-cpu-probe/summaries.csv), [528 batches](benchmark-results/broadcast-cpu-probe/samples.csv), [CPU summary](benchmark-results/broadcast-cpu-probe/cpu-summary.json).

The original long run predates CPU accounting; its exact runner is archived as [runner-used.py](benchmark-results/broadcast-long/runner-used.py). The delivered runner adds CPU accounting to future long runs and supports `--cpu-probe` for the shorter follow-up. Measurement operations, graph inputs and compiled executables were identical across both experiments.

## Evidence and reproduction

- [Environment, protocol and source/executable hashes](benchmark-results/broadcast-long/environment.json). Current and duplicate executable hashes are identical.
- [All 288 process/mode summaries](benchmark-results/broadcast-long/summaries.csv), [all 2,880 batch means](benchmark-results/broadcast-long/samples.csv), [all 96 process records](benchmark-results/broadcast-long/runs.csv), [paired analysis](benchmark-results/broadcast-long/analysis.json).
- [Pre-adaptive source archive](benchmark-results/broadcast-long/reference-source.tar.gz), [earlier adaptive source archive](benchmark-results/broadcast-long/previous-source.tar.gz). Both contain the same extended benchmark harness as the current package.

```sh
python3 benches/broadcast_long.py --baseline /path/to/earlier-adaptive --reference /path/to/pre-adaptive --output benchmark-results/broadcast-repeat
```

Use a new output directory. Defaults in the original benchmark remain nine batches targeted at 20 ms and the same mode order. The new runner explicitly sets `LIVEGRAPH_BENCH_BATCH_MS`, `LIVEGRAPH_BENCH_SAMPLES`, and `LIVEGRAPH_BENCH_MODE_ROTATION` before launching each process. All settings are parsed outside timed operations. The library and workload body are unchanged. This is a focused follow-up; it does not rerun the entire 400/4,000/40,000-node matrix or the reader/retention families. Historical measurements remain in [the previous investigation](ACCESS_SCHEDULER_BENCHMARKS.md).

## Validation

The library core SHA-256 remains `73fd69138139afbd65a97a8840712a4656849eb5d976dc661d23bac8b5e26dd4`, identical to the prior delivery. Native Rust 1.99 fmt/check/clippy (`-D warnings`)/test, doctests, and `config_reload` pass. All 47 library/integration/rustdoc tests and three README examples pass. No new library tests were needed because the implementation did not change.

Benchmark-specific checks confirm legacy nine-batch/default-order behavior, configurable batch count and mode rotation, rejection of out-of-range settings before timing, and Python syntax/CLI help. The complete long-run counts, source/binary identity, balanced main-block positions/mode rotations, and CPU-probe counts were verified. See [native validation results](benchmark-results/broadcast-long/validation/native-cargo.json) and [harness checks](benchmark-results/broadcast-long/validation/harness.json).

The original generic runners now record the configured sample count, target duration and mode rotation rather than assuming default timing settings; the comparison runner checks raw-row counts against the configured sample count. Their default workloads remain unchanged. Analysis uses only Python's standard library. [Analysis/report script](benchmark-results/broadcast-long/analysis-and-report-used.py) and [CPU analysis script](benchmark-results/broadcast-long/cpu-analysis-used.py) are archived as executed and include paths relative to the original task workspace.
