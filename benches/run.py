#!/usr/bin/env python3
"""Build once, run benchmark families serially, preserve CSV samples and metadata."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def capture(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "benchmark-results")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    cargo = os.environ.get("CARGO", "cargo")
    metadata = {
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "platform": platform.platform(),
        "cargo": capture(cargo, "--version"),
        "rustc": capture("rustc", "--version"),
        "cargo_executable": cargo,
        "logical_cpus": os.cpu_count(),
        "source_sha256": {
            name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
            for name in ["src/lib.rs", "benches/performance.rs", "Cargo.lock"]
        },
        "update_samples": int(os.environ.get("LIVEGRAPH_BENCH_SAMPLES", "9")),
        "update_min_batch_ms": int(os.environ.get("LIVEGRAPH_BENCH_BATCH_MS", "20")),
        "update_mode_rotation": int(os.environ.get("LIVEGRAPH_BENCH_MODE_ROTATION", "1")),
        "reader_modes": ["acquire", "pinned", "borrowed"],
        "reader_samples": 5,
        "reader_sample_ms": 200,
        "retention_process_repeats": 3,
    }
    if platform.system() == "Darwin":
        metadata["cpu"] = capture("sysctl", "-n", "machdep.cpu.brand_string")
        metadata["memory_bytes"] = int(capture("sysctl", "-n", "hw.memsize"))
        metadata["os_version"] = capture("sw_vers", "-productVersion")
    print("Building optimized benchmark (no additional dependencies)...", flush=True)
    result = subprocess.run(
        [cargo, "bench", "--bench", "performance", "--no-run", "--locked", "--message-format=json"],
        cwd=ROOT, text=True, stdout=subprocess.PIPE, check=True,
    )
    artifacts = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
    executable = next(item["executable"] for item in artifacts
                      if item.get("reason") == "compiler-artifact"
                      and item.get("target", {}).get("name") == "performance"
                      and item.get("executable"))
    started = time.monotonic()
    for family in ["updates", "readers"]:
        print(f"Running {family} serially...", flush=True)
        with (output / f"{family}.csv").open("w") as summary, (output / f"{family}-samples.csv").open("w") as raw:
            process = subprocess.Popen([executable, family], cwd=ROOT, text=True,
                                       stdout=subprocess.PIPE, stderr=raw)
            rows = 0
            for line in process.stdout:
                summary.write(line)
                summary.flush()
                rows += 1
                if rows % 18 == 0:
                    print(f"  {family}: {rows - 1} rows complete", flush=True)
            if process.wait() != 0:
                raise RuntimeError(f"{family} failed; see sample file for diagnostics")
        print(f"Finished {family} ({time.monotonic() - started:.1f}s elapsed)", flush=True)
    print("Running retention in fresh processes (3 repetitions each)...", flush=True)
    with (output / "retention.csv").open("w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["repeat", "keep", "changed_percent", "changed_resources", "nodes", "payload_bytes",
                         "baseline_rss_kib", "graph_rss_kib", "retained_rss_kib", "released_rss_kib",
                         "live_payload_bytes", "released_payload_bytes", "median_commit_us",
                         "p10_commit_us", "p90_commit_us", "release_us"])
        for repeat in range(3):
            for keep in [0, 1, 10, 100]:
                for percent in [1, 10, 100]:
                    line = capture(executable, "retention", str(keep), str(percent))
                    writer.writerow([repeat, *next(csv.reader([line]))])
                    f.flush()
            print(f"  Retention repeat {repeat + 1}/3 complete", flush=True)
    metadata["elapsed_seconds"] = time.monotonic() - started
    metadata["completed_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    (output / "environment.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(f"Results: {output} ({metadata['elapsed_seconds']:.1f}s)", flush=True)


if __name__ == "__main__":
    main()
