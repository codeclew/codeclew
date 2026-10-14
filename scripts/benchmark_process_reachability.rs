//! Compile with rustc --edition 2024 -O; benchmark the actual production kernel.
//! Synthetic retained call graphs, not end-to-end capture or runtime evidence.
#[path = "../crates/clew/src/documentation/process_reachability.rs"]
mod reachability;
use std::{
    collections::{BTreeMap, BTreeSet},
    hint::black_box,
    time::Instant,
};

fn full_scan<'a>(
    edges: &[(&'a str, &'a str)],
    seeds: &[Option<&'a str>],
) -> BTreeMap<&'a str, u32> {
    let mut result = BTreeMap::new();
    for (bit, seed) in seeds.iter().enumerate() {
        let mut seen = BTreeSet::new();
        if let Some(seed) = seed {
            seen.insert(*seed);
        }
        loop {
            let before = seen.len();
            for &(from, to) in edges {
                if seen.contains(to) {
                    seen.insert(from);
                }
            }
            if seen.len() == before {
                break;
            }
        }
        for id in seen {
            *result.entry(id).or_insert(0) |= 1_u32 << bit;
        }
    }
    result
}

fn median(mut run: impl FnMut(), iterations: u32) -> u128 {
    let mut times = Vec::new();
    for _ in 0..11 {
        let start = Instant::now();
        for _ in 0..iterations {
            run();
        }
        times.push(start.elapsed().as_nanos() / u128::from(iterations));
    }
    times.sort_unstable();
    times[times.len() / 2]
}

fn main() {
    println!("shape,methods,edges,outcomes,full_scan_ns,index_worklist_ns,speedup");
    for size in [16, 32, 64] {
        let names: Vec<_> = (0..size).map(|i| format!("m{i:03}")).collect();
        let methods: Vec<_> = names.iter().map(String::as_str).collect();
        for shape in ["chain", "reverse-chain", "star", "dense"] {
            let mut pairs: Vec<_> = match shape {
                "chain" | "reverse-chain" => (0..size - 1).map(|i| (i, i + 1)).collect(),
                "star" => (1..size).map(|i| (0, i)).collect(),
                _ => (0..size)
                    .flat_map(|i| (i + 1..size).map(move |j| (i, j)))
                    .collect(),
            };
            if shape == "reverse-chain" { pairs.reverse(); }
            let edges: Vec<_> = pairs
                .iter()
                .map(|&(i, j)| (methods[i], methods[j]))
                .collect();
            for count in [1, 32] {
                let seeds: Vec<_> = (0..count)
                    .map(|i| Some(methods[size - 1 - i % size]))
                    .collect();
                assert_eq!(
                    reachability::outcome_masks(&methods, &edges, &seeds),
                    full_scan(&edges, &seeds)
                );
                let old = median(
                    || {
                        black_box(full_scan(black_box(&edges), black_box(&seeds)));
                    },
                    100,
                );
                let new = median(
                    || {
                        black_box(reachability::outcome_masks(
                            black_box(&methods),
                            black_box(&edges),
                            black_box(&seeds),
                        ));
                    },
                    100,
                );
                println!(
                    "{shape},{size},{},{count},{old},{new},{:.3}",
                    edges.len(),
                    old as f64 / new as f64
                );
            }
        }
    }
}
