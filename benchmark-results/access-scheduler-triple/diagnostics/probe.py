import sys, pathlib, importlib.util, os, csv, io, subprocess, statistics, json, hashlib
spec=importlib.util.spec_from_file_location('compare', pathlib.Path(__file__).resolve().parents[3] / 'benches/compare.py');mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
roots=[pathlib.Path(p).resolve() for p in sys.argv[1:3]]
out=pathlib.Path(sys.argv[3]);out.parent.mkdir(parents=True,exist_ok=True)
cargo=os.environ.get('CARGO', 'cargo')
os.environ['PATH']=str(pathlib.Path(cargo).parent)+':'+os.environ['PATH'];os.environ['RUSTC_WRAPPER']='';os.environ['RUSTC_WORKSPACE_WRAPPER']=''
exe=[mod.build(r,cargo) for r in roots];cases=os.environ.get('PROBE_CASES','1000:false:1:0;1000:true:1:0;1000:false:100:0').split(';');data=[]
for repeat in range(3):
 for j,case in enumerate(cases):
  for side in ([0,1] if (repeat+j)%2==0 else [1,0]):
   p=subprocess.run([exe[side],'updates'],env=dict(os.environ,LIVEGRAPH_BENCH_CASE=case),capture_output=True,text=True,check=True)
   r=next(r for r in csv.DictReader(io.StringIO(p.stdout)) if r['mode']=='incremental')
   data.append(dict(repeat=repeat,side=side,case=case,**r))
ratios=[]
for case in cases:
 a,b=[statistics.median(float(r['median_ns']) for r in data if r['case']==case and r['side']==side) for side in [0,1]]
 ratios.append(b/a);print(case,f'{a:.1f} -> {b:.1f} ns ({(b/a-1)*100:+.1f}%)')
out.write_text(json.dumps(dict(roots=list(map(str,roots)),hashes=[hashlib.sha256((r/'src/lib.rs').read_bytes()).hexdigest() for r in roots],rows=data,ratios=ratios),indent=2)+'\n')
# Diagnostic threshold, not a portable CI timing test.
if max(ratios[:2])>1.02:print('REGRESSION: sparse or broadcast exceeds 2%');sys.exit(1)
print('PASS: sparse and broadcast within 2%')
