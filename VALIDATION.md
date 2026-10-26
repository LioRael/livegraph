# LiveGraph MVP 验证报告

日期：2026-10-26（Asia/Shanghai）

项目位置：`<workspace>/outputs/livegraph`

已在该目录初始化独立 Git 仓库。未修改既有 Lenso 或其他项目，未推送或发布。

## 环境与命令

本地 macOS / aarch64，Rust 与 Cargo 1.99.0。依赖锁定为 arc-swap 1.9.3、rustversion 1.0.23。

本机 `cargo blaze --help` 返回 ra-jit 帮助，检查入口脚本确认它转发到 ra-jit。未通过它构建，也未安装、启用或修改任何持久加速配置。

最终验证直接调用已核实为原生可执行文件的：

```text
<home>/.rustup/toolchains/1.99.0-aarch64-apple-darwin/bin/cargo
```

该轮验证的子进程 PATH 临时优先指向同一工具链 bin，CARGO 临时指定上述原生路径，避免子命令重新经过本机命令包装器。未修改全局工具链或 Cargo 配置。

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --check` | 通过 |
| `cargo check --all-targets --all-features --locked` | 通过 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过，无警告 |
| `cargo test --all-features --locked` | 通过，24 项测试 |
| `cargo run --locked --example config_reload` | 通过 |

测试明细：4 项库内单元测试、13 项图行为测试、5 项并发测试、2 项文档测试（其中 1 项编译失败测试验证类型约束）。无跳过测试。

## 实际示例输出

```text
old v0: Routes { upstream: "https://localhost", max_connections: 100 }
new v1: Routes { upstream: "https://api.example.com", max_connections: 250 }
rejected: build parsed limit: invalid digit found in string
rejected: validation connection budget: connection limit exceeds 1000
still serving v1: Routes { upstream: "https://api.example.com", max_connections: 250 }
```

## 保证与验证边界

已验证受影响节点重算、拓扑顺序、循环拒绝、批量校验、失败不发布、旧快照可继续读取、旧资源正确释放，以及多个写者与读者并发工作时的快照一致性。

未进行最低 Rust 1.88 工具链实测、其他操作系统实测、模型检查或性能基准；没有执行文件监听或磁盘事务测试，因为本版没有文件 adapter。不可变性依赖调用者不通过内部可变性修改已发布 payload。回调及析构逻辑的外部副作用不属于事务回滚范围。详细语义见 README。

## 审查修复后的验证

新增 7 项回归测试（总数由 17 增至 24），另扩充派生节点赋值与析构路径断言。验证内容包括：

- 四个并发写者通过 `changes_from` 冲突重试完成 400 次累加，最终值和版本均为 400。
- 陈旧版本（包括空批次）与跨图快照被拒绝，冲突不触发校验或发布。
- 计算回调和校验回调递归提交立即失败，旧快照保留，后续合法提交可成功。
- 构建中失败的写者不把候选值泄漏给排队写者。
- 成功发布、候选校验失败、陈旧版本提前返回三条路径都在写锁及重入标记清除后释放库持有的值。
- 一次赋值准备错误使整个批次失效，不能误提交此前准备的部分资源。
- `Snapshot` / `LiveGraph` 的错误诊断无需 payload 实现 `Debug`。

详细审查发现和 API 兼容性说明见 [REVIEW.md](REVIEW.md)。

## 性能基准交付验证

新增独立、无第三方开发依赖的 benchmark 目标及 Python 标准库运行脚本。两轮完整性能测试均实际完成，结果见 [BENCHMARKS.md](BENCHMARKS.md)。每轮保存 108 行更新汇总、972 个更新样本、16 行读取汇总、80 个读取样本及 36 个独立进程的快照保留结果；行数已核对，第二轮源码摘要与交付代码一致。

新增目标后重新通过 `cargo fmt --check`、`cargo check --all-targets --all-features --locked`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --all-targets --all-features --locked` 和 `cargo test --doc --locked`。原有 24 项测试仍通过，benchmark 在测试模式下另运行轻量正确性冒烟检查。Python 运行脚本也通过语法检查并实际执行两轮。库核心没有为基准修改。

## Borrowed-read optimization validation

Added the opt-in `with_snapshot` API while preserving `snapshot() -> Arc<Snapshot>` and the commit path. Added seven integration tests and two rustdoc tests, bringing the library/integration/rustdoc total to **33 passing tests**. The three English README Rust examples also pass standalone rustdoc tests.

Final checks passed with the native Rust 1.99 toolchain: `cargo fmt --check`, `cargo check --all-targets --all-features --locked`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `cargo test --all-targets --all-features --locked`, `cargo test --doc --locked`, and the runnable `config_reload` example.

A fresh pre-change benchmark and two full post-change runs were completed serially. Each run includes 108 update rows / 972 raw update batches and 36 retention-process rows. The before run has 16 reader rows / 80 raw samples; each after run adds the borrowed mode for 24 rows / 120 raw samples. Row counts were verified. Both after-run source hashes match the delivered core/benchmark source and Cargo.lock. Live payload counts match the before run for all retention combinations.

The comparison, writer-throughput controls, variability, methodology and raw-file links are recorded in [BORROWED_READ_BENCHMARKS.md](BORROWED_READ_BENCHMARKS.md). README.md has been rewritten in English.

## Sparse-update optimization validation

Snapshots now use shared 16-slot blocks with copy-on-write for partial updates and empty blocks for wholly replaced regions. Scheduling collects only reachable affected nodes and orders them by cached topological rank, with reusable generation marks under the existing writer mutex. All public APIs and publication/rollback behavior are preserved.

Five additional tests bring the total to **38 passing library/integration/rustdoc tests**. They cover block boundaries/sharing, partial tails, generation wrap, independent DAG evaluation and exact callback counts across failures/retries, cross-block concurrency with owned and borrowed readers, and unaffected constants sharing changed blocks. The three English README code examples also pass standalone rustdoc testing.

Final native Cargo checks passed: fmt, check for all targets/features, clippy for all targets/features with `-D warnings`, tests for all targets/features, doc tests, and the config_reload example. The benchmark's test-mode smoke check also passed.

Two flat-baseline runs and two final-implementation runs used identical benchmark code; an intermediate 32-slot prototype run is retained separately. Every run contains 108 update summaries / 972 batch samples, 24 reader summaries / 120 reader samples, and 36 retention-process results. Counts and source hashes were verified. Both final source hashes match the delivered code; baseline hashes match each other. Live payload counts match baseline for all retention combinations. A baseline source archive accompanies the results.

See [SPARSE_UPDATE_BENCHMARKS.md](SPARSE_UPDATE_BENCHMARKS.md) for the measured sparse gains, dense-update regressions, reader variability, metadata costs, raw data, and reproduction instructions. The English README has been updated with the implementation and complexity boundaries.

## Adaptive-update optimization validation

Added adaptive scheduling: reachable-node traversal/sorting below 25% affected nodes, then a forward topological reachability pass. Fully affected snapshots skip block replacement counting and use one exclusive borrow per new block; partially affected snapshots retain lazy block copy-on-write. Public APIs and published snapshot storage are unchanged.

Five new tests bring the library/integration/rustdoc total to **43 passing tests**. Coverage includes exact scheduling against an independent reachability traversal on shuffled DAGs, threshold transitions, alternating sparse/dense history and callback counts, build errors/panics/validation failures followed by recovery, and concurrent owned/borrowed readers during alternating strategies. The existing destructor test now also checks computation failure cleanup after releasing the writer lock.

Native Rust 1.99 checks passed: `cargo fmt --check`, `cargo check --all-targets --all-features --locked`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `cargo test --all-targets --all-features --locked`, `cargo test --doc --locked`, and the `config_reload` example. All three English README Rust examples pass standalone rustdoc tests. The benchmark target's correctness smoke check also passes. The native toolchain was invoked directly with compiler-wrapper variables cleared; Blaze did not substitute for validation.

Performance results and limitations are recorded in [ADAPTIVE_UPDATE_BENCHMARKS.md](ADAPTIVE_UPDATE_BENCHMARKS.md). Historical reports above describe their respective earlier revisions; their timings do not describe this implementation.

The final three-pass paired run contains 324 update summaries and 2,916 batch samples per side, with identical benchmark/lockfile hashes. A final complete run contains 108 update summaries / 972 batches, 24 reader summaries / 120 samples, and 36 retention-process rows. Row counts and delivered source hashes were checked; tracked live/released payload bytes match the pre-change run in every retention configuration. Raw native validation logs are under `benchmark-results/adaptive-selected-paired/validation/`.

## Candidate-access and scheduler regression validation

Removed the private `ValueView`/`CandidateAccess` representation dispatch. Candidate dependency reads use `Snapshot` directly; the commit selects a statically specialized full/partial write loop. Fully affected snapshots still start with empty private blocks and use checked unique access. Scheduling now chooses once from the staged-root lower bound and never restarts an in-progress reachable traversal as a topological scan.

Four additional tests bring the library/integration/rustdoc total to **47 passing tests**. The new high-fanout scheduling test failed on the previous implementation before the fix, then passed. Integration coverage adds a 37-node forward-reference graph with a partial tail block, complete/partial replacements, late build failure and retained histories; identical foreign/undeclared-dependency checks through both candidate paths; and full-candidate payload destruction after releasing the commit guard. Existing alternating-strategy tests were adjusted to exercise the new staged-root threshold.

Final checks passed using the native Rust 1.99 toolchain with compiler wrappers cleared: `cargo fmt --check`, `cargo check --all-targets --all-features --locked`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `cargo test --all-targets --all-features --locked`, `cargo test --doc --locked`, the runnable `config_reload` example, and all three README Rust examples. The benchmark smoke check also passes. No Blaze configuration or toolchain settings were changed.

The three-tree comparison runs the pre-adaptive, previous adaptive and current implementations with identical benchmark/lockfile hashes and rotating order, three passes each. It contains 324 update summaries and 2,916 batch samples per tree. Fresh complete before/after runs each contain 108 update summaries / 972 batches, 24 reader summaries / 120 samples, and 36 retention-process rows. Delivered source hashes, row counts and retained-payload equality are checked when generating the report. Diagnostic probes, including the original failure and final passing recheck, are preserved.

See [ACCESS_SCHEDULER_BENCHMARKS.md](ACCESS_SCHEDULER_BENCHMARKS.md) for the ablations, all results, the noisy broadcast case, remaining differences and reproduction archives. Native validation logs are in `benchmark-results/access-scheduler-triple/validation/`. Earlier sections and reports describe historical revisions.


## Longer broadcast stability investigation

The library source is unchanged from the access/scheduler fix (`73fd69138139afbd65a97a8840712a4656849eb5d976dc661d23bac8b5e26dd4`). Only benchmark configuration, orchestration, metadata and documentation changed. The target broadcast case ran for 12 balanced blocks with 11 batches targeted at 150 ms; neighboring cases used four blocks and nine 100 ms-targeted batches. An identical current executable was repeated as an A/A control. The long experiment recorded 96 processes, 288 mode summaries and 2,880 batch samples. A separate CPU/wall probe recorded 16 processes, 48 summaries and 528 samples. Source/binary identities, counts and balanced order were verified.

Native Rust 1.99 fmt/check/clippy (`-D warnings`)/test, doctests, the config_reload example, all 47 library/integration/rustdoc tests and three README examples passed. Additional harness checks verified default nine-batch/mode-order compatibility, configurable samples/rotation, rejection of invalid settings before measurement, Python syntax and CLI help. Validation logs and check results are under `benchmark-results/broadcast-long/validation/`.

The longer data does not establish a stable incremental broadcast regression on this shared host. Large identical-binary differences and CPU/wall discrepancies demonstrate execution variability; small true overhead remains unresolved. The full-recomputation control is reported separately rather than dismissed as noise. The decision is to retain the current implementation. See [BROADCAST_STABILITY_REPORT.md](BROADCAST_STABILITY_REPORT.md) for every block, uncertainty, CPU records and reproduction instructions. No further library optimization or full-matrix/reader/retention rerun is claimed in this follow-up.


## 0.1.0 release preparation

The library core is unchanged by release preparation. Release metadata, an explicit crate-file allowlist, installation instructions, changelog and GitHub CI were added. Local home/workspace paths in public benchmark text artifacts were redacted; timing samples and source hashes were retained.

Rust 1.88.0 now passes native check, all-target/all-feature tests, doctests and the runnable example, confirming the declared minimum version. Rust 1.99.0 additionally supplies fmt and strict clippy validation. Earlier sections stating that the minimum version was untested describe their historical runs. CI is configured for Ubuntu (1.88 and stable), macOS stable and Windows stable; actual hosted results are reported separately from local validation.
