#!/usr/bin/env python3
"""Run identical update cases in alternating baseline/candidate order."""
import argparse
import csv
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import subprocess
import time

CANDIDATE = Path(__file__).resolve().parents[1]


def build(root, cargo):
    result = subprocess.run([cargo, "bench", "--bench", "performance", "--no-run", "--locked",
                             "--message-format=json"], cwd=root, text=True,
                            stdout=subprocess.PIPE, check=True)
    items = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
    return next(item["executable"] for item in items
                if item.get("reason") == "compiler-artifact"
                and item.get("target", {}).get("name") == "performance" and item.get("executable"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--reference", type=Path, help="Optional third source tree")
    parser.add_argument("--passes", default=2, type=int)
    args = parser.parse_args()
    if args.passes < 1:
        parser.error("--passes must be positive")
    roots = {"before": args.baseline.resolve(), "after": CANDIDATE}
    if args.reference:
        roots["reference"] = args.reference.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    cargo = os.environ.get("CARGO", "cargo")
    hashes = {name: {file: hashlib.sha256((root / file).read_bytes()).hexdigest()
                     for file in ["src/lib.rs", "benches/performance.rs", "Cargo.lock"]}
              for name, root in roots.items()}
    if len({files["benches/performance.rs"] for files in hashes.values()}) != 1:
        parser.error("all source trees must contain the same benchmark file")
    sample_count = int(os.environ.get("LIVEGRAPH_BENCH_SAMPLES", "9"))
    executables = {name: build(root, cargo) for name, root in roots.items()}
    metadata = {"source_sha256": hashes, "platform": platform.platform(),
                "cargo": subprocess.check_output([cargo, "--version"], text=True).strip(),
                "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
                "update_samples": sample_count,
                "update_batch_ms": int(os.environ.get("LIVEGRAPH_BENCH_BATCH_MS", "20")),
                "update_mode_rotation": int(os.environ.get("LIVEGRAPH_BENCH_MODE_ROTATION", "1")),
                "passes": args.passes, "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                "method": "same cases; alternating pair order or rotating triple order by case/pass; batch settings recorded above"}
    summaries = {name: [] for name in roots}
    samples = {name: [] for name in roots}
    cases = [f"{n}:{str(broadcast).lower()}:{percent}:{rounds}"
             for n in [100, 1000, 10000] for broadcast in [False, True]
             for percent in [1, 10, 100] for rounds in [0, 64]]
    started = time.monotonic()
    for run in range(args.passes):
        for i, case in enumerate(cases):
            if len(roots) == 2:
                order = ["before", "after"] if (i + run) % 2 == 0 else ["after", "before"]
            else:
                order = list(roots)
                offset = (i + run) % len(order)
                order = order[offset:] + order[:offset]
                if i % 2:
                    order.reverse()
            for side in order:
                env = dict(os.environ, LIVEGRAPH_BENCH_CASE=case)
                result = subprocess.run([executables[side], "updates"], cwd=roots[side], env=env,
                                        text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
                summary = list(csv.DictReader(io.StringIO(result.stdout)))
                raw = list(csv.DictReader(io.StringIO(result.stderr)))
                assert len(summary) == 3 and len(raw) == 3 * sample_count, (side, case)
                summaries[side].extend(dict(run=run, **row) for row in summary)
                samples[side].extend(dict(run=run, **row) for row in raw)
            if (i + 1) % 6 == 0:
                print(f"Pass {run + 1}/{args.passes}: {i + 1}/{len(cases)} cases", flush=True)
        # Save completed passes before starting the next one.
        for side in roots:
            for suffix, rows in [("updates", summaries[side]), ("samples", samples[side])]:
                with (output / f"{side}-{suffix}.csv").open("w", newline="") as f:
                    writer = csv.DictWriter(f, fieldnames=list(rows[0]))
                    writer.writeheader()
                    writer.writerows(rows)
    metadata["elapsed_seconds"] = time.monotonic() - started
    metadata["completed_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    (output / "environment.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(f"Saved {output} ({metadata['elapsed_seconds']:.1f}s)", flush=True)


if __name__ == "__main__":
    main()
