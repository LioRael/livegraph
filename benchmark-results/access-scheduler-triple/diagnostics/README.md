# Diagnostic experiments

These local probes selected three existing benchmark cases outside timing; the timed operations were unchanged. Each used three alternating passes and nine batches per mode. A 2% sparse/broadcast threshold was a local diagnostic signal, not a portable CI timing assertion. JSON records core hashes and per-run summaries. Paths identify the source trees used at measurement time.

- `reproduce.json`: pre-adaptive vs previous adaptive, 4,000 nodes. Failed the threshold on broadcast (+2.8%).
- `direct-result.json`: previous adaptive vs direct candidate access, 4,000 nodes. Broadcast −3.2%.
- `scheduler-result.json`: previous adaptive vs restoring only old scheduling, 4,000 nodes. Broadcast +0.5%; no measurable scheduler-only benefit.
- `direct-vs-original-large.json`: pre-adaptive vs direct access, 40,000 nodes. Failed the threshold on sparse (+2.3%).
- `entry-choice-large.json`: pre-adaptive vs direct access plus entry-time scheduling choice, 40,000 nodes. Passed: sparse +1.4%, broadcast −1.4%, full −6.4%.

`exploratory-sources.tar.gz` contains the three experiment trees. They are diagnostic snapshots, not the delivered library. The final source also includes updated policy tests and comments. The parent directory contains baseline archives.

On the measured host, after extracting the desired source trees:

```sh
python3 probe.py /path/to/pre-adaptive /path/to/candidate /path/to/new-probe.json
PROBE_CASES='10000:false:1:0;10000:true:1:0;10000:false:100:0' python3 probe.py /path/to/pre-adaptive /path/to/candidate /path/to/large-probe.json
```

`probe.py` resolves the comparison helper in this package. It uses the measured host's native Cargo 1.99 path and clears compiler-wrapper variables. Output JSON is written before a regression exit status. Treat individual probe differences cautiously; the complete rotated three-tree matrix is the final evidence. No diagnostic timing instrumentation remains in the library.

Final verification: `final-recheck.json` reruns the original three 4,000-node cases against the delivered source. It passed the diagnostic threshold (sparse −7.2%, broadcast −10.7%, full −6.8% versus its contemporaneous pre-adaptive baseline). The changed baseline timings show that noise remains; these figures do not replace the complete three-tree matrix.
