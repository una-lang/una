//! Effective ceilings (ADR 0122 rule 51): what each package of a program can ever exercise.
//!
//! The ceiling at a call into a package is the one the calling package writes for it with
//! `may`. A package's effective ceiling is the least fixpoint of
//!
//! ```text
//! eff(root) = everything the root's grants allow
//! eff(p)    = ⋃ over the packages d that need p of (ceiling_d(p) ∩ eff(d))
//! ```
//!
//! computed by a worklist, cycles included. Each set only grows, and has at most C elements for
//! C capability operations, so a package is revisited at most C times along each of its needs:
//! O((P + R) · C) for P packages and R needs.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// A capability operation, as `(capability, operation)`: `("Http", "get")`.
pub type Operation = (String, String);

/// A set of capability operations.
pub type Authority = BTreeSet<Operation>;

/// One need of the program: the package that needs, the package needed, and the ceiling the
/// first writes for the second.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    /// The dependent package, by index.
    pub from: usize,
    /// The dependency, by index.
    pub to: usize,
    /// The ceiling `from` writes for `to`.
    pub ceiling: Authority,
}

/// The effective ceiling of every package, by index, for `packages` packages of which `root`
/// is the root, whose grants allow `granted`. A package no need reaches from the root has the
/// empty set.
pub fn effective(packages: usize, root: usize, granted: &Authority, edges: &[Edge]) -> Vec<Authority> {
    let mut eff: Vec<Authority> = vec![Authority::new(); packages];
    let mut out: BTreeMap<usize, Vec<&Edge>> = BTreeMap::new();
    for edge in edges {
        out.entry(edge.from).or_default().push(edge);
    }
    if let Some(slot) = eff.get_mut(root) {
        slot.clone_from(granted);
    }
    let mut work: VecDeque<usize> = VecDeque::from([root]);
    let mut queued = vec![false; packages];
    if let Some(flag) = queued.get_mut(root) {
        *flag = true;
    }
    while let Some(from) = work.pop_front() {
        if let Some(flag) = queued.get_mut(from) {
            *flag = false;
        }
        let Some(edges) = out.get(&from) else {
            continue;
        };
        let source = eff.get(from).cloned().unwrap_or_default();
        for edge in edges {
            let flowing: Vec<Operation> = edge.ceiling.intersection(&source).cloned().collect();
            let Some(target) = eff.get_mut(edge.to) else {
                continue;
            };
            let before = target.len();
            target.extend(flowing);
            if target.len() > before
                && let Some(flag) = queued.get_mut(edge.to)
                && !*flag
            {
                *flag = true;
                work.push_back(edge.to);
            }
        }
    }
    eff
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn ops(list: &[(&str, &str)]) -> Authority {
        list.iter()
            .map(|(c, o)| ((*c).to_owned(), (*o).to_owned()))
            .collect()
    }

    #[test]
    fn a_ceiling_bounds_what_flows_and_a_cycle_converges() {
        // root -> a (Http[get], Files[read]); a -> b (Http[get, post]); b -> a (Files[read]).
        let granted = ops(&[("Http", "get"), ("Files", "read"), ("Console", "print")]);
        let edges = [
            Edge {
                from: 0,
                to: 1,
                ceiling: ops(&[("Http", "get"), ("Files", "read")]),
            },
            Edge {
                from: 1,
                to: 2,
                ceiling: ops(&[("Http", "get"), ("Http", "post")]),
            },
            Edge {
                from: 2,
                to: 1,
                ceiling: ops(&[("Files", "read")]),
            },
        ];
        let eff = effective(4, 0, &granted, &edges);
        assert_eq!(eff[1], ops(&[("Http", "get"), ("Files", "read")]));
        // `post` is in a's ceiling for b but not in a's own effective ceiling.
        assert_eq!(eff[2], ops(&[("Http", "get")]));
        // A package no need reaches can exercise nothing.
        assert!(eff[3].is_empty());
    }

    #[test]
    fn a_dependency_needed_twice_takes_the_union() {
        let granted = ops(&[("Http", "get"), ("Files", "read")]);
        let edges = [
            Edge { from: 0, to: 1, ceiling: ops(&[("Http", "get")]) },
            Edge { from: 0, to: 2, ceiling: ops(&[("Files", "read")]) },
            Edge { from: 1, to: 3, ceiling: ops(&[("Http", "get")]) },
            Edge { from: 2, to: 3, ceiling: ops(&[("Files", "read"), ("Http", "get")]) },
        ];
        let eff = effective(4, 0, &granted, &edges);
        assert_eq!(eff[3], ops(&[("Http", "get"), ("Files", "read")]));
    }
}
