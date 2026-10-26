from pathlib import Path
import csv, json, statistics as st, math, hashlib, shutil
from collections import Counter
root=Path('outputs/livegraph');out=root/'benchmark-results/broadcast-long';cpu=root/'benchmark-results/broadcast-cpu-probe'
rows=list(csv.DictReader((out/'runs.csv').open()))
for side in ['reference','previous','current','current_repeat']:
 r=[x for x in rows if x['phase']=='main' and x['side']==side]
 assert Counter(x['position'] for x in r)==dict.fromkeys(['0','1','2','3'],3)
 assert len({(x['position'],x['mode_rotation']) for x in r})==12
m=json.loads((cpu/'environment.json').read_text());assert 'completed_at' in m
r=list(csv.DictReader((cpu/'runs.csv').open()));s=list(csv.DictReader((cpu/'summaries.csv').open()));samples=list(csv.DictReader((cpu/'samples.csv').open()))
assert (len(r),len(s),len(samples))==(16,48,528)
assert m['executable_sha256']['current']==m['executable_sha256']['current_repeat']
first=json.loads((out/'environment.json').read_text());assert m['source_sha256']==first['source_sha256'];assert m['executable_sha256']==first['executable_sha256']
for x in r:
 x['cpu_seconds']=float(x['child_user_seconds'])+float(x['child_system_seconds'])
 x['cpu_wall_fraction']=x['cpu_seconds']/float(x['elapsed_seconds'])
summary={side:dict(median_cpu_wall=st.median(x['cpu_wall_fraction'] for x in r if x['side']==side),min_cpu_wall=min(x['cpu_wall_fraction'] for x in r if x['side']==side),max_cpu_wall=max(x['cpu_wall_fraction'] for x in r if x['side']==side)) for side in first['names']}
(cpu/'cpu-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
p=root/'BROADCAST_STABILITY_REPORT.md';text=p.read_text()
intro='''## Decision

**Keep the delivered library implementation unchanged.** The target incremental case did not reproduce a stable +4% regression against the preceding adaptive build. Its median paired difference is **−0.13%**, with a descriptive 95% whole-block bootstrap interval of **−2.54% to +3.83%**. Against pre-adaptive it is **+0.41%** (−5.34% to +2.79%).

The identical-binary A/A control has a median difference of +0.21%, an interval of −4.12% to +5.12%, and individual block differences from **−10.84% to +16.65%**. This directly demonstrates execution variability greater than the previously reported +4% point estimate. Both code comparisons include zero and overlap the A/A variability. These observations do not support tuning the implementation to that earlier point estimate.

This is **not proof that all of the earlier difference was noise**, nor an equivalence result within ±2%. A small real overhead remains unresolved on this shared host. A longer run did not make environmental disturbance disappear. No favorable rerun or outlier exclusion is substituted for these results.

The controls also prevent a blanket “everything is noise” conclusion: main-case full recomputation is **+3.79% versus the preceding adaptive build** (descriptive interval +3.15% to +6.34%). Its point estimate remains −4.59% versus pre-adaptive, but that interval crosses zero. This is a separate full-update tradeoff, consistent with why the earlier report retained small dense-case differences; it is not evidence of an incremental broadcast regression. Fresh reconstruction also varies. All modes are reported below.

The existing sparse/dense implementation, API, snapshot representation, atomic publication and rollback behavior are preserved byte-for-byte. No new optimization is justified by the target-case evidence from this experiment.

'''
text=text.replace('## Experiment held constant',intro+'## Experiment held constant',1)
section=['## CPU/wall-time follow-up','','After the long run finished, a separate four-block probe ran the same four labels and identical executables, with eleven 150 ms-targeted batches per mode. Parent-side `getrusage(RUSAGE_CHILDREN)` differences record child user/system CPU time and context switches. No CPU probe overlapped the long experiment. These are **whole-process totals**, including setup, calibration and all three modes. They cannot be used to subtract scheduling time from an individual incremental batch or normalize its latency.','','| Label | Median CPU/wall | Range |','| --- | ---: | ---: |']
for side,x in summary.items():section.append(f"| {side} | {x['median_cpu_wall']*100:.1f}% | {x['min_cpu_wall']*100:.1f}%–{x['max_cpu_wall']*100:.1f}% |")
section+=['','Every process is retained below. A low CPU/wall fraction establishes time not accounted for as child CPU execution, but does not identify the responsible process or separate all scheduling, launch, page-fault and I/O effects. Frequency/cache effects may also change CPU time itself. The supplemental probe has only four blocks and cannot certify equivalence.','','| Block | Position | Label | Wall seconds | Child CPU seconds | CPU/wall |','| ---: | ---: | --- | ---: | ---: | ---: |']
for x in r:section.append(f"| {int(x['block'])+1} | {int(x['position'])+1} | {x['side']} | {float(x['elapsed_seconds']):.3f} | {x['cpu_seconds']:.3f} | {x['cpu_wall_fraction']*100:.1f}% |")
section+=['',f"Probe duration excluding compilation: {m['elapsed_seconds']:.1f} seconds. [Probe metadata](benchmark-results/broadcast-cpu-probe/environment.json), [all 16 CPU/wall records](benchmark-results/broadcast-cpu-probe/runs.csv), [48 summaries](benchmark-results/broadcast-cpu-probe/summaries.csv), [528 batches](benchmark-results/broadcast-cpu-probe/samples.csv), [CPU summary](benchmark-results/broadcast-cpu-probe/cpu-summary.json).",'','The original long run predates CPU accounting; its exact runner is archived as [runner-used.py](benchmark-results/broadcast-long/runner-used.py). The delivered runner adds CPU accounting to future long runs and supports `--cpu-probe` for the shorter follow-up. Measurement operations, graph inputs and compiled executables were identical across both experiments.','']
text=text.replace('## Evidence and reproduction','\n'.join(section)+'\n## Evidence and reproduction')
p.write_text(text)
shutil.copy(root/'benches/broadcast_long.py',cpu/'runner-used.py')
print(json.dumps(summary,indent=2))
