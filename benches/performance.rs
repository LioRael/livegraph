//! Standalone reproducible benchmarks; emits CSV and uses no extra dependencies.
use livegraph::{GraphBuilder, LiveGraph, Node, Snapshot};
use std::{
    collections::VecDeque,
    hint::black_box,
    process::Command,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn setting(name: &str, default: usize, max: usize) -> usize {
    match std::env::var(name) {
        Ok(value) => {
            let value: usize = value.parse().expect("benchmark setting must be an integer");
            assert!((1..=max).contains(&value), "benchmark setting out of range");
            value
        }
        Err(std::env::VarError::NotPresent) => default,
        Err(error) => panic!("invalid benchmark setting: {error}"),
    }
}

fn work(mut value: u64, rounds: usize) -> u64 {
    for _ in 0..rounds {
        value = value
            .wrapping_mul(6364136223846793005)
            .rotate_left(13)
            .wrapping_add(1);
    }
    value
}

struct Fixture {
    graph: LiveGraph,
    inputs: Vec<Node<u64>>,
    leaves: Vec<Node<u64>>,
}

fn fixture(values: &[u64], broadcast: bool, rounds: usize) -> Fixture {
    let mut b = GraphBuilder::new();
    let inputs: Vec<_> = values
        .iter()
        .enumerate()
        .map(|(i, v)| b.resource(format!("input-{i}"), *v))
        .collect();
    let mut leaves = Vec::new();
    for (i, &input) in inputs.iter().enumerate() {
        let root = if broadcast { inputs[0] } else { input };
        let mut previous = b
            .derived(
                format!("branch-{i}-0"),
                [input.dep(), root.dep()],
                move |cx| Ok(work(cx.get(input)?.wrapping_add(*cx.get(root)?), rounds)),
            )
            .unwrap();
        for depth in 1..3 {
            let dependency = previous;
            previous = b
                .derived(
                    format!("branch-{i}-{depth}"),
                    [dependency.dep()],
                    move |cx| Ok(work(*cx.get(dependency)?, rounds)),
                )
                .unwrap();
        }
        leaves.push(previous);
    }
    Fixture {
        graph: b.build().unwrap(),
        inputs,
        leaves,
    }
}

fn edit(values: &mut [u64], changed: usize) {
    for i in 0..changed {
        let index = i * values.len() / changed;
        values[index] = values[index].wrapping_add(1);
    }
}

fn commit(f: &Fixture, values: &[u64], changed: usize, full: bool) {
    let mut changes = f.graph.changes();
    let count = if full { values.len() } else { changed };
    for i in 0..count {
        let index = i * values.len() / count;
        changes.set(f.inputs[index], values[index]).unwrap();
    }
    black_box(f.graph.commit(changes).unwrap());
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * fraction).round() as usize]
}

fn measure(
    target: Duration,
    sample_count: usize,
    mut operation: impl FnMut(),
) -> (usize, Vec<f64>) {
    let mut iterations = 1;
    loop {
        let start = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        if start.elapsed() >= target || iterations >= 1 << 20 {
            break;
        }
        iterations *= 2;
    }
    let mut samples = Vec::new();
    for _ in 0..sample_count {
        let start = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        samples.push(start.elapsed().as_secs_f64() * 1e9 / iterations as f64);
    }
    (iterations, samples)
}

fn updates() {
    // Configuration is read outside all measured operations. Defaults preserve
    // the original matrix; mode rotation is optional for repeated experiments.
    let sample_count = setting("LIVEGRAPH_BENCH_SAMPLES", 9, 10_000);
    let target = Duration::from_millis(setting("LIVEGRAPH_BENCH_BATCH_MS", 20, 60_000) as u64);
    let mode_rotation = setting("LIVEGRAPH_BENCH_MODE_ROTATION", 1, 3) - 1;
    eprintln!("topology,resources,changed_percent,rounds,mode,sample,ns_per_op");
    println!(
        "topology,resources,nodes,changed_percent,affected_derived,rounds,mode,iterations_per_sample,samples,median_ns,p10_ns,p90_ns"
    );
    for n in [100, 1000, 10000] {
        for broadcast in [false, true] {
            for percent in [1, 10, 100] {
                for rounds in [0, 64] {
                    // Optional untimed selector for interleaving before/after cases.
                    if let Ok(case) = std::env::var("LIVEGRAPH_BENCH_CASE")
                        && case != format!("{n}:{broadcast}:{percent}:{rounds}")
                    {
                        continue;
                    }
                    let changed = n * percent / 100;
                    let initial: Vec<_> = (0..n as u64).collect();
                    // Check all leaves against a fresh build before timing each scenario.
                    let incremental = fixture(&initial, broadcast, rounds);
                    let full = fixture(&initial, broadcast, rounds);
                    let mut next = initial.clone();
                    edit(&mut next, changed);
                    commit(&incremental, &next, changed, false);
                    commit(&full, &next, changed, true);
                    let rebuilt = fixture(&next, broadcast, rounds);
                    let a = incremental.graph.snapshot();
                    let b = full.graph.snapshot();
                    let c = rebuilt.graph.snapshot();
                    for i in 0..n {
                        assert_eq!(
                            a.get(incremental.leaves[i]).unwrap(),
                            b.get(full.leaves[i]).unwrap()
                        );
                        assert_eq!(
                            a.get(incremental.leaves[i]).unwrap(),
                            c.get(rebuilt.leaves[i]).unwrap()
                        );
                    }
                    drop((a, b, c, incremental, full, rebuilt));
                    // Rotate mode order by scenario to reduce systematic ordering bias.
                    let modes = ["incremental", "full_recompute", "rebuild"];
                    let offset = (percent + rounds + usize::from(broadcast) + mode_rotation) % 3;
                    for j in 0..3 {
                        let mode = modes[(j + offset) % 3];
                        let f = fixture(&initial, broadcast, rounds);
                        let mut values = initial.clone();
                        let (iterations, samples) = measure(target, sample_count, || {
                            edit(&mut values, changed);
                            if mode == "rebuild" {
                                black_box(fixture(black_box(&values), broadcast, rounds));
                            } else {
                                commit(&f, black_box(&values), changed, mode == "full_recompute");
                            }
                        });
                        for (sample, ns) in samples.iter().enumerate() {
                            eprintln!(
                                "{},{n},{percent},{rounds},{mode},{sample},{ns:.3}",
                                if broadcast {
                                    "broadcast"
                                } else {
                                    "partitioned"
                                }
                            );
                        }
                        println!(
                            "{},{n},{},{percent},{},{rounds},{mode},{iterations},{sample_count},{:.1},{:.1},{:.1}",
                            if broadcast {
                                "broadcast"
                            } else {
                                "partitioned"
                            },
                            4 * n,
                            if broadcast { 3 * n } else { 3 * changed },
                            percentile(&samples, 0.5),
                            percentile(&samples, 0.1),
                            percentile(&samples, 0.9)
                        );
                    }
                }
            }
        }
    }
}

fn reading_sample<const BORROWED: bool>(readers: usize, pinned: bool, writing: bool) -> (f64, f64) {
    let mut b = GraphBuilder::new();
    let value = b.resource("value", 0_u64);
    let double = b
        .derived("double", [value.dep()], move |cx| {
            Ok(cx.get(value)?.wrapping_mul(2))
        })
        .unwrap();
    let graph = b.build().unwrap();
    let barrier = Barrier::new(readers + 1 + usize::from(writing));
    let stop = AtomicBool::new(false);
    thread::scope(|scope| {
        let writer = writing.then(|| {
            scope.spawn(|| {
                barrier.wait();
                let mut count = 0_u64;
                while !stop.load(Ordering::Relaxed) {
                    count += 1;
                    let mut changes = graph.changes();
                    changes.set(value, count).unwrap();
                    black_box(graph.commit(changes).unwrap());
                }
                count
            })
        });
        let handles: Vec<_> = (0..readers)
            .map(|_| {
                scope.spawn(|| {
                    let snapshot = graph.snapshot();
                    barrier.wait();
                    let start = Instant::now();
                    let mut count = 0_u64;
                    while start.elapsed() < Duration::from_millis(200) {
                        for _ in 0..256 {
                            if BORROWED {
                                graph.with_snapshot(|current| {
                                    let a = *black_box(current).get(value).unwrap();
                                    let b = *current.get(double).unwrap();
                                    assert_eq!(b, a.wrapping_mul(2));
                                    black_box((a, b));
                                });
                            } else {
                                let acquired;
                                let current = if pinned {
                                    &snapshot
                                } else {
                                    acquired = graph.snapshot();
                                    &acquired
                                };
                                let a = *black_box(current).get(value).unwrap();
                                let b = *current.get(double).unwrap();
                                assert_eq!(b, a.wrapping_mul(2));
                                black_box((a, b));
                            }
                            count += 1;
                        }
                    }
                    count
                })
            })
            .collect();
        barrier.wait();
        let start = Instant::now();
        let total: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        stop.store(true, Ordering::Relaxed);
        let elapsed = start.elapsed().as_secs_f64();
        let written = writer.map(|h| h.join().unwrap()).unwrap_or(0);
        (total as f64 / elapsed, written as f64 / elapsed)
    })
}

fn readers() {
    eprintln!("mode,writer,readers,sample,reads_per_sec,writes_per_sec");
    println!(
        "mode,writer,readers,samples,median_reads_per_sec,p10_reads_per_sec,p90_reads_per_sec,median_writes_per_sec"
    );
    for writing in [false, true] {
        for count in [1, 2, 4, 8] {
            for mode in ["acquire", "pinned", "borrowed"] {
                let sample = || {
                    if mode == "borrowed" {
                        reading_sample::<true>(count, false, writing)
                    } else {
                        reading_sample::<false>(count, mode == "pinned", writing)
                    }
                };
                black_box(sample());
                let (reads, writes): (Vec<_>, Vec<_>) = (0..5).map(|_| sample()).unzip();
                for (sample, (read, write)) in reads.iter().zip(&writes).enumerate() {
                    eprintln!("{mode},{writing},{count},{sample},{read:.3},{write:.3}");
                }
                println!(
                    "{},{writing},{count},5,{:.0},{:.0},{:.0},{:.0}",
                    mode,
                    percentile(&reads, 0.5),
                    percentile(&reads, 0.1),
                    percentile(&reads, 0.9),
                    percentile(&writes, 0.5)
                );
            }
        }
    }
}

struct Payload {
    bytes: Vec<u8>,
    live: Arc<AtomicUsize>,
    seed: u64,
}
impl Payload {
    fn new(seed: u64, live: &Arc<AtomicUsize>) -> Self {
        let bytes = vec![seed as u8; 4096];
        live.fetch_add(bytes.capacity(), Ordering::Relaxed);
        Self {
            bytes,
            live: live.clone(),
            seed,
        }
    }
}
impl Drop for Payload {
    fn drop(&mut self) {
        self.live
            .fetch_sub(self.bytes.capacity(), Ordering::Relaxed);
    }
}

fn rss_kib() -> u64 {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn retention(keep: usize, percent: usize) {
    let baseline_rss = rss_kib();
    let live = Arc::new(AtomicUsize::new(0));
    let mut b = GraphBuilder::new();
    let inputs: Vec<_> = (0..200)
        .map(|i| b.resource(format!("input-{i}"), Payload::new(i, &live)))
        .collect();
    for (i, &input) in inputs.iter().enumerate() {
        let mut previous = input;
        for depth in 0..3 {
            let dependency = previous;
            let tracker = live.clone();
            previous = b
                .derived(
                    format!("branch-{i}-{depth}"),
                    [dependency.dep()],
                    move |cx| Ok(Payload::new(cx.get(dependency)?.seed, &tracker)),
                )
                .unwrap();
        }
    }
    let graph = b.build().unwrap();
    let graph_rss = rss_kib();
    let mut held: VecDeque<Arc<Snapshot>> = VecDeque::with_capacity(keep + 1);
    let mut samples = Vec::new();
    let changed = (200 * percent / 100).max(1);
    for generation in 1..=120 {
        let start = Instant::now();
        if keep > 0 {
            held.push_back(graph.snapshot());
        }
        let mut changes = graph.changes();
        for i in 0..changed {
            changes
                .set(inputs[i * 200 / changed], Payload::new(generation, &live))
                .unwrap();
        }
        black_box(graph.commit(changes).unwrap());
        if held.len() > keep {
            held.pop_front();
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6);
    }
    let live_bytes = live.load(Ordering::Relaxed);
    let retained_rss = rss_kib();
    let start = Instant::now();
    drop(held);
    let release_us = start.elapsed().as_secs_f64() * 1e6;
    let released_bytes = live.load(Ordering::Relaxed);
    assert_eq!(released_bytes, 800 * 4096);
    let released_rss = rss_kib();
    // Final 20 commits are in steady state even for a 100-snapshot window.
    println!(
        "{keep},{percent},{changed},800,4096,{baseline_rss},{graph_rss},{retained_rss},{released_rss},{live_bytes},{released_bytes},{:.2},{:.2},{:.2},{release_us:.2}",
        percentile(&samples[100..], 0.5),
        percentile(&samples[100..], 0.1),
        percentile(&samples[100..], 0.9)
    );
    drop(graph);
    assert_eq!(live.load(Ordering::Relaxed), 0);
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        None | Some("--test") => {
            let f = fixture(&[1, 2], false, 64);
            commit(&f, &[2, 2], 1, false);
            assert_eq!(
                *f.graph.snapshot().get(f.leaves[0]).unwrap(),
                work(work(work(4, 64), 64), 64)
            );
        }
        Some("--bench") | Some("updates") => updates(),
        Some("readers") => readers(),
        Some("retention") => retention(args[2].parse().unwrap(), args[3].parse().unwrap()),
        _ => panic!("usage: performance updates | readers | retention KEEP PERCENT"),
    }
}
