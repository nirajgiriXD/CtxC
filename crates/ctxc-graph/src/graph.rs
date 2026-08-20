//! The dependency graph.
//!
//! An in-memory view over edges the store hands back, answering the questions
//! retrieval will need: what does this file depend on, what depends on it, and
//! which files does everything depend on. Building it is cheap enough to do per
//! query, which keeps it honest — there is no second copy of the truth to go
//! stale.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::model::RelationshipKind;

/// One edge between two files.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: RelationshipKind,
}

/// File-level relationships, indexed in both directions.
#[derive(Debug, Default)]
pub struct DependencyGraph {
    outgoing: BTreeMap<String, BTreeSet<String>>,
    incoming: BTreeMap<String, BTreeSet<String>>,
    edges: Vec<Edge>,
}

impl DependencyGraph {
    /// Build a graph from resolved edges.
    pub fn from_edges(edges: impl IntoIterator<Item = Edge>) -> Self {
        let mut graph = DependencyGraph::default();
        for edge in edges {
            graph
                .outgoing
                .entry(edge.from.clone())
                .or_default()
                .insert(edge.to.clone());
            graph
                .incoming
                .entry(edge.to.clone())
                .or_default()
                .insert(edge.from.clone());
            // Files with no edges at all are not nodes: the graph describes
            // relationships, and the index already knows every file.
            graph.edges.push(edge);
        }
        graph
    }

    /// Files this file depends on.
    pub fn dependencies_of(&self, path: &str) -> Vec<&str> {
        self.outgoing
            .get(path)
            .map(|set| set.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// Files that depend on this file.
    pub fn dependents_of(&self, path: &str) -> Vec<&str> {
        self.incoming
            .get(path)
            .map(|set| set.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// Files with the most dependents, most depended on first.
    ///
    /// This is the cheapest useful importance signal a repository offers, and
    /// ranking will lean on it.
    pub fn most_depended_on(&self, limit: usize) -> Vec<(&str, usize)> {
        let mut ranked: Vec<(&str, usize)> = self
            .incoming
            .iter()
            .map(|(path, dependents)| (path.as_str(), dependents.len()))
            .collect();

        // Count descending, then path ascending, so output is deterministic.
        ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
        ranked.truncate(limit);
        ranked
    }

    /// Every file reachable from `path` by following dependencies.
    ///
    /// Breadth first and cycle safe: repositories contain import cycles, and a
    /// traversal that cannot survive one is not usable on real code.
    pub fn reachable_from(&self, path: &str, max_depth: usize) -> Vec<&str> {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::new();
        queue.push_back((path, 0usize));
        seen.insert(path);

        let mut reached = Vec::new();
        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            for next in self.dependencies_of(current) {
                if seen.insert(next) {
                    reached.push(next);
                    queue.push_back((next, depth + 1));
                }
            }
        }
        reached
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Files that appear at either end of an edge.
    pub fn node_count(&self) -> usize {
        self.outgoing
            .keys()
            .chain(self.incoming.keys())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(from: &str, to: &str) -> Edge {
        Edge {
            from: from.into(),
            to: to.into(),
            kind: RelationshipKind::Imports,
        }
    }

    fn sample() -> DependencyGraph {
        DependencyGraph::from_edges([
            edge("src/api.ts", "src/auth.ts"),
            edge("src/auth.ts", "src/database.ts"),
            edge("src/auth.ts", "src/session.ts"),
            edge("src/session.ts", "src/database.ts"),
            edge("src/worker.ts", "src/database.ts"),
        ])
    }

    #[test]
    fn edges_are_navigable_in_both_directions() {
        let graph = sample();

        assert_eq!(
            graph.dependencies_of("src/auth.ts"),
            vec!["src/database.ts", "src/session.ts"]
        );
        assert_eq!(graph.dependents_of("src/auth.ts"), vec!["src/api.ts"]);
        assert!(graph.dependencies_of("src/unknown.ts").is_empty());
    }

    #[test]
    fn importance_is_measured_by_dependents() {
        let graph = sample();
        let ranked = graph.most_depended_on(2);

        assert_eq!(ranked[0], ("src/database.ts", 3));
        assert_eq!(ranked[1].1, 1);
    }

    #[test]
    fn ranking_is_deterministic_for_ties() {
        let graph = sample();
        let first = graph.most_depended_on(10);
        let second = graph.most_depended_on(10);
        assert_eq!(first, second);
    }

    #[test]
    fn traversal_follows_dependencies_to_a_depth() {
        let graph = sample();

        let one_hop = graph.reachable_from("src/api.ts", 1);
        assert_eq!(one_hop, vec!["src/auth.ts"]);

        let mut everything = graph.reachable_from("src/api.ts", 10);
        everything.sort();
        assert_eq!(
            everything,
            vec!["src/auth.ts", "src/database.ts", "src/session.ts"]
        );
    }

    #[test]
    fn traversal_survives_cycles() {
        let graph = DependencyGraph::from_edges([
            edge("a.rs", "b.rs"),
            edge("b.rs", "c.rs"),
            edge("c.rs", "a.rs"),
        ]);

        let mut reached = graph.reachable_from("a.rs", 10);
        reached.sort();
        assert_eq!(reached, vec!["b.rs", "c.rs"]);
    }

    #[test]
    fn duplicate_edges_do_not_inflate_neighbours() {
        let graph = DependencyGraph::from_edges([edge("a.rs", "b.rs"), edge("a.rs", "b.rs")]);

        assert_eq!(graph.dependencies_of("a.rs"), vec!["b.rs"]);
        assert_eq!(graph.edge_count(), 2, "the raw edges are still all there");
        assert_eq!(graph.node_count(), 2);
    }

    #[test]
    fn an_empty_graph_answers_everything_with_nothing() {
        let graph = DependencyGraph::default();
        assert!(graph.dependencies_of("anything").is_empty());
        assert!(graph.most_depended_on(5).is_empty());
        assert_eq!(graph.node_count(), 0);
    }
}
