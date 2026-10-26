from pathlib import Path
import csv, json, math, statistics as st, random, hashlib, tarfile
ROOT=Path('outputs/livegraph');OUT=ROOT/'benchmark-results/broadcast-long'
rows=list(csv.DictReader((OUT/'summaries.csv').open()))
raw=list(csv.DictReader((OUT/'samples.csv').open()))
runs=list(csv.DictReader((OUT/'runs.csv').open()))
meta=json.loads((OUT/'environment.json').read_text())
assert 'completed_at' in meta
assert (len(rows),len(raw),len(runs))==(288,2880,96)
assert meta['executable_sha256']['current']==meta['executable_sha256']['current_repeat']
for file,h in meta['source_sha256']['current'].items():assert hashlib.sha256((ROOT/file).read_bytes()).hexdigest()==h
old=json.loads((ROOT/'benchmark-results/access-scheduler-triple/environment.json').read_text())
assert old['source_sha256']['after']['src/lib.rs']==meta['source_sha256']['current']['src/lib.rs']
assert len({h['Cargo.lock'] for h in meta['source_sha256'].values()})==1
keys=['phase','case','mode'];groups={}
for r in rows:groups.setdefault(tuple(r[k] for k in keys),{}).setdefault(int(r['block']),{})[r['side']]=r

def change(logratio):return 100*math.expm1(logratio)
def paired(blocks,a,b):
 values=[math.log(float(d[a]['median_ns'])/float(d[b]['median_ns'])) for _,d in sorted(blocks.items())]
 rng=random.Random(20261026)
 boot=sorted(st.median(rng.choices(values,k=len(values))) for _ in range(20000))
 return dict(median_percent=change(st.median(values)),ci_low=change(boot[500]),ci_high=change(boot[19499]),
             min_percent=change(min(values)),max_percent=change(max(values)),faster=sum(v<0 for v in values),n=len(values),
             ratios_percent=[change(v) for v in values])
analysis={}
for k,blocks in groups.items():
 analysis['/'.join(k)]={pair:paired(blocks,a,b) for pair,a,b in [('vs_reference','current','reference'),('vs_previous','current','previous'),('aa','current_repeat','current')]}
(OUT/'analysis.json').write_text(json.dumps(analysis,indent=2)+'\n')
lines=['# Longer broadcast measurements: regression or noise?', '', '## Experiment held constant', '', 'The library implementation is unchanged. Its source SHA-256 matches the previous delivered revision. This investigation changes only benchmark controls and documentation. The same typed graph, update operations, snapshot publication, and correctness checks run in all trees.', '', 'Three frozen versions are measured: pre-adaptive (`reference`), the earlier adaptive build (`previous`), and the delivered access/scheduler fix (`current`). `current_repeat` invokes the **identical current executable** a second time, providing an A/A execution-noise control. The primary comparison always uses the preassigned `current` label; it is not selected or averaged with the faster duplicate after seeing the results.', '', '## Protocol', '', '- Main case: **4,000 nodes, broadcast graph, 1% changed roots, zero synthetic compute rounds**. Twelve fresh-process blocks per label; each mode uses eleven batches calibrated to **150 ms**, versus the former nine ~20 ms batches and three passes.', '- Every invocation measures incremental update, full recomputation, and fresh reconstruction. Full recomputation stages all resources; reconstruction includes graph construction and destruction. Calibration itself warms the workload but is not included in the retained batch means.', '- Four-side balanced orders repeat three times: reference / previous / duplicate / current; previous / current / reference / duplicate; current / duplicate / previous / reference; duplicate / reference / current / previous. Each label occupies each position three times, and adjacent ordering is balanced. The starting mode rotates each four-block cycle, so all three modes occur first for every label/position combination.', '- Neighbor controls: broadcast 10%/light, broadcast 1%/64 rounds, and partitioned 1%/light, all with 4,000 nodes. Each has four balanced blocks, nine batches targeted at 100 ms, and the same A/A duplicate. These controls are descriptive and have less repetition than the main case.', '- No outliers, blocks, or processes are removed. Load averages and process durations are recorded. The host is not CPU-pinned or isolated, and other processes are not stopped. A/A measures execution variability, not every possible environmental bias.', '', f"Measured host: {meta['platform']}, {meta['logical_cpus']} logical CPUs, {meta['rustc']}. Elapsed measurement time: **{meta['elapsed_seconds']:.1f} seconds**, excluding compilation. All three benchmark and lockfile hashes match; only the intended core revisions differ.", '', '## Main result', '', 'Times below are medians of the twelve process medians, in µs/update. The duplicate is shown independently. Detailed paired differences follow; ratios of aggregate medians can differ from medians of within-block ratios.', '', '| Mode | Pre-adaptive | Earlier adaptive | Current | Same-binary duplicate |', '| --- | ---: | ---: | ---: | ---: |']
maincase='1000:true:1:0'
for mode in ['incremental','full_recompute','rebuild']:
 b=groups[('main',maincase,mode)];v=[st.median(float(d[s]['median_ns'])/1000 for d in b.values()) for s in ['reference','previous','current','current_repeat']]
 lines.append('| '+mode+' | '+' | '.join(f'{x:.2f}' for x in v)+' |')
lines+=['','Paired percentage changes use log(current/baseline) within each complete block and then the median. Negative means faster. Bootstrap intervals resample **whole blocks**, with a fixed seed and 20,000 resamples; batch samples are not treated as independent experiments. Intervals are descriptive sensitivity estimates under an exchangeable-block assumption. Serial drift and dependence can invalidate formal coverage; twelve blocks cannot establish exact equality or universality. A practical 2% difference is the investigation threshold inherited from the earlier local diagnostic, not a product guarantee.','','| Mode | Comparison | Median change | 95% block-bootstrap interval | Block range | Faster blocks |','| --- | --- | ---: | ---: | ---: | ---: |']
for mode in ['incremental','full_recompute','rebuild']:
 for pair in ['vs_reference','vs_previous','aa']:
  a=analysis[f'main/{maincase}/{mode}'][pair]
  lines.append(f"| {mode} | {pair} | {a['median_percent']:+.2f}% | {a['ci_low']:+.2f}% … {a['ci_high']:+.2f}% | {a['min_percent']:+.2f}% … {a['max_percent']:+.2f}% | {a['faster']}/{a['n']} |")
lines+=['','## Every main block','','Incremental-update process medians, µs/update. The final column compares the two labels that run the same binary; it must not be interpreted as a code change.','','| Block | Run order | Pre-adaptive | Earlier adaptive | Current | Duplicate | Current vs pre-adaptive | Duplicate vs current |','| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |']
b=groups[('main',maincase,'incremental')]
for block,d in sorted(b.items()):
 vals={s:float(r['median_ns'])/1000 for s,r in d.items()};order=' → '.join(s for s,r in sorted(d.items(),key=lambda x:int(x[1]['position'])))
 lines.append(f"| {block+1} | {order} | {vals['reference']:.2f} | {vals['previous']:.2f} | {vals['current']:.2f} | {vals['current_repeat']:.2f} | {(vals['current']/vals['reference']-1)*100:+.2f}% | {(vals['current_repeat']/vals['current']-1)*100:+.2f}% |")
lines+=['','## Order and time sensitivity','','The groups below each contain only four or six paired observations; they are checks for large ordering effects, not independently powered experiments.','','| Main incremental grouping | Blocks | Median current vs pre-adaptive | Median duplicate vs current |','| --- | ---: | ---: | ---: |']
for label,pred in [(f'mode rotation {rot}',lambda d,rot=rot:int(d['current']['mode_rotation'])==rot) for rot in [1,2,3]]+[
 ('current runs before reference',lambda d:int(d['current']['position'])<int(d['reference']['position'])),
 ('current runs after reference',lambda d:int(d['current']['position'])>int(d['reference']['position']))]:
 subset={i:d for i,d in b.items() if pred(d)};p=paired(subset,'current','reference');aa=paired(subset,'current_repeat','current')
 lines.append(f"| {label} | {len(subset)} | {p['median_percent']:+.2f}% | {aa['median_percent']:+.2f}% |")
loads=[float(r[x]) for r in runs for x in ['load1_before','load1_after']]
lines+=['',f"Recorded one-minute load average ranged from {min(loads):.2f} to {max(loads):.2f}; it is context, not a CPU-temperature or per-process interference measurement. The experiment cannot determine which background activity caused a specific slow sample.",'','## Neighbor and dense controls','','Four blocks per label, process medians in µs/update. All three modes are included. Changes are paired medians, and the A/A range shows all four duplicate/current block ratios.','','| Case | Mode | Pre-adaptive | Earlier adaptive | Current | vs pre-adaptive | vs earlier adaptive | A/A range |','| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |']
for case in ['1000:true:10:0','1000:true:1:64','1000:false:1:0']:
 for mode in ['incremental','full_recompute','rebuild']:
  b=groups[('controls',case,mode)];v=[st.median(float(d[s]['median_ns'])/1000 for d in b.values()) for s in ['reference','previous','current']];a=analysis[f'controls/{case}/{mode}']
  lines.append(f"| {case} | {mode} | {v[0]:.2f} | {v[1]:.2f} | {v[2]:.2f} | {a['vs_reference']['median_percent']:+.2f}% | {a['vs_previous']['median_percent']:+.2f}% | {a['aa']['min_percent']:+.2f}% … {a['aa']['max_percent']:+.2f}% |")
lines+=['','## Evidence and reproduction','','- [Environment, protocol and source/executable hashes](benchmark-results/broadcast-long/environment.json). Current and duplicate executable hashes are identical.','- [All 288 process/mode summaries](benchmark-results/broadcast-long/summaries.csv), [all 2,880 batch means](benchmark-results/broadcast-long/samples.csv), [all 96 process records](benchmark-results/broadcast-long/runs.csv), [paired analysis](benchmark-results/broadcast-long/analysis.json).','- [Pre-adaptive source archive](benchmark-results/broadcast-long/reference-source.tar.gz), [earlier adaptive source archive](benchmark-results/broadcast-long/previous-source.tar.gz). Both contain the same extended benchmark harness as the current package.','','```sh','python3 benches/broadcast_long.py --baseline /path/to/earlier-adaptive --reference /path/to/pre-adaptive --output benchmark-results/broadcast-repeat','```','','Use a new output directory. Defaults in the original benchmark remain nine batches targeted at 20 ms and the same mode order. The new runner explicitly sets `LIVEGRAPH_BENCH_BATCH_MS`, `LIVEGRAPH_BENCH_SAMPLES`, and `LIVEGRAPH_BENCH_MODE_ROTATION` before launching each process. All settings are parsed outside timed operations. The library and workload body are unchanged. This is a focused follow-up; it does not rerun the entire 400/4,000/40,000-node matrix or the reader/retention families. Historical measurements remain in [the previous investigation](ACCESS_SCHEDULER_BENCHMARKS.md).']
(ROOT/'BROADCAST_STABILITY_REPORT.md').write_text('\n'.join(lines)+'\n')
for name in ['reference','previous']:
 with tarfile.open(OUT/f'{name}-source.tar.gz','w:gz') as tar:
  for f in ['Cargo.toml','Cargo.lock','src/lib.rs','benches/performance.rs']:tar.add(Path('work/broadcast-long')/name/f,arcname=f)
print(json.dumps(analysis[f'main/{maincase}/incremental'],indent=2))
