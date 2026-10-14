//! Structural caller reachability only; not data/effect dependence or valid paths.
use std::collections::{BTreeMap, VecDeque};

/// Each bit identifies one requested outcome (at most 32, admitted by Query).
/// Only edges with both endpoints in the retained method envelope participate.
pub(super) fn outcome_masks<'a>(
    methods: &[&'a str],
    edges: &[(&'a str, &'a str)],
    seeds: &[Option<&'a str>],
) -> BTreeMap<&'a str, u32> {
    assert!(seeds.len() <= 32, "outcomes must pass query admission");
    if seeds.iter().all(Option::is_none) {
        return BTreeMap::new();
    }
    // A shallow single-outcome query often converges in two scans. Avoid index
    // construction there, but cap speculative scans before indexed propagation.
    // Slice membership is linear; the warm-up can cost O(VE), with V <= 64.
    let mut warm = BTreeMap::new();
    if let [Some(seed)] = seeds {
        if !methods.contains(seed) {
            return warm;
        }
        warm.insert(*seed, 1_u32);
        for _ in 0..2 {
            let before = warm.len();
            for &(from, to) in edges {
                if warm.contains_key(to) && !warm.contains_key(from) && methods.contains(&from) {
                    warm.insert(from, 1);
                }
            }
            if warm.len() == before || warm.len() == methods.len() {
                return warm;
            }
        }
    }
    let ids: BTreeMap<_, _> = methods.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut predecessors = vec![Vec::new(); methods.len()];
    for &(from, to) in edges {
        if let (Some(&from), Some(&to)) = (ids.get(from), ids.get(to)) {
            predecessors[to].push(from);
        }
    }
    let mut masks = vec![0_u32; methods.len()];
    let mut pending = vec![false; methods.len()];
    let mut queue = VecDeque::new();
    for (bit, seed) in seeds.iter().enumerate() {
        if let Some(&node) = seed.and_then(|id| ids.get(id)) {
            masks[node] |= 1_u32 << bit;
            if !pending[node] {
                pending[node] = true;
                queue.push_back(node);
            }
        }
    }
    for (id, mask) in warm {
        let node = ids[id];
        masks[node] |= mask;
        if !pending[node] {
            pending[node] = true;
            queue.push_back(node);
        }
    }
    while let Some(node) = queue.pop_front() {
        pending[node] = false;
        let incoming = masks[node];
        for &caller in &predecessors[node] {
            let updated = masks[caller] | incoming;
            if updated != masks[caller] {
                masks[caller] = updated;
                if !pending[caller] {
                    pending[caller] = true;
                    queue.push_back(caller);
                }
            }
        }
    }
    ids.into_iter()
        .filter_map(|(id, i)| (masks[i] != 0).then_some((id, masks[i])))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independent per-outcome full-scan reference, intentionally not a worklist.
    fn reference<'a>(
        methods: &[&'a str],
        edges: &[(&'a str, &'a str)],
        seeds: &[Option<&'a str>],
    ) -> BTreeMap<&'a str, u32> {
        let mut result = BTreeMap::new();
        for (bit, seed) in seeds.iter().enumerate() {
            let mut seen = std::collections::BTreeSet::new();
            if let Some(id) = seed.filter(|id| methods.contains(id)) {
                seen.insert(id);
            }
            loop {
                let before = seen.len();
                for &(from, to) in edges {
                    if methods.contains(&from) && methods.contains(&to) && seen.contains(to) {
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

    #[test]
    fn cycles_diamonds_duplicates_and_unretained_endpoints_keep_outcomes_separate() {
        let methods = ["root", "left", "right", "cycle", "unrelated"];
        let edges = [
            ("root", "left"),
            ("root", "right"),
            ("left", "cycle"),
            ("cycle", "left"),
            ("root", "left"),
            ("left", "missing"),
            ("missing", "right"),
        ];
        let seeds = [Some("left"), Some("right"), None, Some("missing")];
        let actual = outcome_masks(&methods, &edges, &seeds);
        assert_eq!(actual, reference(&methods, &edges, &seeds));
        assert_eq!(actual["root"], 3);
        assert_eq!(actual["cycle"], 1);
        assert_eq!(actual["right"], 2);
        assert!(!actual.contains_key("unrelated"));
    }

    #[test]
    fn exhaustive_three_node_graphs_match_independent_reference_for_every_seed_pair() {
        let methods = ["a", "b", "c"];
        for graph in 0..512_u32 {
            let edges: Vec<_> = (0..9)
                .filter(|bit| graph & (1 << bit) != 0)
                .map(|bit| (methods[bit / 3], methods[bit % 3]))
                .collect();
            for first in 0..4 {
                let seed = [methods.get(first).copied()];
                assert_eq!(
                    outcome_masks(&methods, &edges, &seed),
                    reference(&methods, &edges, &seed)
                );
                for second in 0..4 {
                    let seeds = [methods.get(first).copied(), methods.get(second).copied()];
                    assert_eq!(
                        outcome_masks(&methods, &edges, &seeds),
                        reference(&methods, &edges, &seeds),
                        "graph={graph} seeds={seeds:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn reversed_chains_repeated_outcomes_and_edge_order_match_reference() {
        let names: Vec<_> = (0..64).map(|i| format!("m{i:02}")).collect();
        let methods: Vec<_> = names.iter().map(String::as_str).collect();
        let mut edges: Vec<_> = methods.windows(2).map(|pair| (pair[0], pair[1])).collect();
        edges.extend([
            (methods[1], methods[2]),
            (methods[0], "missing"),
            ("missing", methods[63]),
        ]);
        for seeds in [
            vec![],
            vec![None],
            vec![Some(methods[63])],
            vec![
                Some(methods[31]),
                Some(methods[63]),
                Some(methods[31]),
                None,
                Some("missing"),
            ],
        ] {
            let expected = reference(&methods, &edges, &seeds);
            assert_eq!(outcome_masks(&methods, &edges, &seeds), expected);
            edges.reverse();
            assert_eq!(outcome_masks(&methods, &edges, &seeds), expected);
        }
    }

    #[test]
    fn all_32_bits_and_empty_queries_are_supported() {
        let seeds = [Some("target"); 32];
        assert_eq!(
            outcome_masks(&["root", "target"], &[("root", "target")], &seeds)["root"],
            u32::MAX
        );
        assert!(outcome_masks(&["root"], &[], &[]).is_empty());
        assert!(outcome_masks(&[], &[], &[Some("missing")]).is_empty());
    }
}
