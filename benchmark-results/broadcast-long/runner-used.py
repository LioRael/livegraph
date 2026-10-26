#!/usr/bin/env python3
"""Long repeated broadcast measurements, with balanced order and an A/A control."""
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
from compare import build

ROOT = Path(__file__).resolve().parents[1]
# Balanced four-treatment orders: each side occupies every position once,
# and every directed adjacent pair occurs once in each four-block cycle.
ORDERS = [(0, 1, 3, 2), (1, 2, 0, 3), (2, 3, 1, 0), (3, 0, 2, 1)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', required=True, type=Path)
    parser.add_argument('--reference', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    roots = dict(reference=args.reference.resolve(), previous=args.baseline.resolve(), current=ROOT)
    names = [*roots, 'current_repeat']
    hashes = {s: {f: hashlib.sha256((r/f).read_bytes()).hexdigest()
                  for f in ['src/lib.rs', 'benches/performance.rs', 'Cargo.lock']}
              for s, r in roots.items()}
    if len({h['benches/performance.rs'] for h in hashes.values()}) != 1:
        parser.error('all trees must have the same benchmark source')
    cargo = os.environ.get('CARGO', 'cargo')
    executables = {s: build(r, cargo) for s, r in roots.items()}
    executables['current_repeat'] = executables['current']
    roots['current_repeat'] = roots['current']
    phases = [dict(name='main', blocks=12, batch_ms=150, samples=11,
                   cases=['1000:true:1:0']),
              dict(name='controls', blocks=4, batch_ms=100, samples=9,
                   cases=['1000:true:10:0', '1000:true:1:64', '1000:false:1:0'])]
    metadata = dict(started_at=time.strftime('%Y-%m-%dT%H:%M:%S%z'),
                    source_sha256=hashes, phases=phases, order_indices=ORDERS,
                    names=names, platform=platform.platform(), logical_cpus=os.cpu_count(),
                    cargo=subprocess.check_output([cargo, '--version'], text=True).strip(),
                    rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                    executable_sha256={s: hashlib.sha256(Path(e).read_bytes()).hexdigest()
                                       for s, e in executables.items()},
                    method='Four-side balanced orders; current_repeat uses the identical current binary; main mode rotation changes each four-block cycle; all samples retained.')
    (output/'environment.json').write_text(json.dumps(metadata, indent=2)+'\n')
    started = time.monotonic()
    with (output/'summaries.csv').open('w', newline='') as summaries, (output/'samples.csv').open('w', newline='') as samples, (output/'runs.csv').open('w', newline='') as runs:
        writers = {}
        for phase in phases:
            for block in range(phase['blocks']):
                for case in phase['cases']:
                    order = [names[i] for i in ORDERS[block % 4]]
                    for position, side in enumerate(order):
                        env = dict(os.environ, LIVEGRAPH_BENCH_CASE=case,
                                   LIVEGRAPH_BENCH_BATCH_MS=str(phase['batch_ms']),
                                   LIVEGRAPH_BENCH_SAMPLES=str(phase['samples']),
                                   LIVEGRAPH_BENCH_MODE_ROTATION=str(block//4 % 3+1))
                        prefix = dict(phase=phase['name'], block=block, case=case,
                                      side=side, position=position, mode_rotation=block//4 % 3+1)
                        run_start = time.monotonic()
                        info = dict(prefix, started_at=time.strftime('%Y-%m-%dT%H:%M:%S%z'),
                                    load1_before=os.getloadavg()[0])
                        result = subprocess.run([executables[side], 'updates'], cwd=roots[side], env=env,
                                                capture_output=True, text=True, check=True)
                        info.update(elapsed_seconds=time.monotonic()-run_start, load1_after=os.getloadavg()[0])
                        summary = list(csv.DictReader(io.StringIO(result.stdout)))
                        raw = list(csv.DictReader(io.StringIO(result.stderr)))
                        if len(summary) != 3 or len(raw) != 3*phase['samples']:
                            raise RuntimeError('unexpected benchmark output counts')
                        for label, stream, records in [
                            ('summaries', summaries, [dict(prefix, **r) for r in summary]),
                            ('samples', samples, [dict(prefix, **r) for r in raw]),
                            ('runs', runs, [info])]:
                            if label not in writers:
                                writers[label] = csv.DictWriter(stream, fieldnames=list(records[0]))
                                writers[label].writeheader()
                            writers[label].writerows(records)
                            stream.flush()
                    print(f"{phase['name']} block {block+1}/{phase['blocks']} case {case} ({time.monotonic()-started:.1f}s)", flush=True)
    for side, root in roots.items():
        original = hashes['current' if side == 'current_repeat' else side]
        for file, h in original.items():
            if hashlib.sha256((root/file).read_bytes()).hexdigest() != h:
                raise RuntimeError(f'source changed during measurements: {side} {file}')
    metadata.update(completed_at=time.strftime('%Y-%m-%dT%H:%M:%S%z'), elapsed_seconds=time.monotonic()-started)
    (output/'environment.json').write_text(json.dumps(metadata, indent=2)+'\n')
    print(f'Completed: {output}', flush=True)


if __name__ == '__main__':
    main()
