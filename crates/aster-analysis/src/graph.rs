use std::collections::HashMap;
use std::path::{Path, PathBuf};

use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};

/// The require graph of a project: nodes are files (paths relative to the
/// project root), edges are resolved `require` calls.
pub struct ModuleGraph {
    graph: DiGraph<PathBuf, ()>,
    index: HashMap<PathBuf, NodeIndex>,
}

impl ModuleGraph {
    pub(crate) fn build(files: &[PathBuf], edges: &[(PathBuf, PathBuf)]) -> Self {
        let mut graph = DiGraph::new();
        let mut index = HashMap::new();
        for file in files {
            let node = graph.add_node(file.clone());
            index.insert(file.clone(), node);
        }
        for (from, to) in edges {
            if let (Some(&a), Some(&b)) = (index.get(from), index.get(to)) {
                graph.update_edge(a, b, ());
            }
        }
        Self { graph, index }
    }

    /// All modules in the graph, sorted by path.
    pub fn modules(&self) -> Vec<&Path> {
        let mut modules: Vec<&Path> = self.graph.node_weights().map(PathBuf::as_path).collect();
        modules.sort();
        modules
    }

    /// Direct dependencies of `module`, sorted. Empty if the module is unknown.
    pub fn dependencies(&self, module: &Path) -> Vec<&Path> {
        let Some(&node) = self.index.get(module) else {
            return Vec::new();
        };
        let mut deps: Vec<&Path> = self
            .graph
            .neighbors_directed(node, Direction::Outgoing)
            .map(|n| self.graph[n].as_path())
            .collect();
        deps.sort();
        deps
    }

    /// Modules with no incoming edges, sorted.
    pub fn entry_points(&self) -> Vec<&Path> {
        let mut entries: Vec<&Path> = self
            .graph
            .node_indices()
            .filter(|&n| {
                self.graph
                    .neighbors_directed(n, Direction::Incoming)
                    .next()
                    .is_none()
            })
            .map(|n| self.graph[n].as_path())
            .collect();
        entries.sort();
        entries
    }

    /// All dependency cycles, each as a sorted list of module paths.
    /// A single module requiring itself counts as a cycle.
    pub(crate) fn cycles(&self) -> Vec<Vec<PathBuf>> {
        let mut cycles = Vec::new();
        for scc in petgraph::algo::tarjan_scc(&self.graph) {
            let is_cycle =
                scc.len() > 1 || (scc.len() == 1 && self.graph.contains_edge(scc[0], scc[0]));
            if is_cycle {
                let mut cycle: Vec<PathBuf> = scc.iter().map(|&n| self.graph[n].clone()).collect();
                cycle.sort();
                cycles.push(cycle);
            }
        }
        cycles.sort();
        cycles
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn builds_nodes_and_edges() {
        let graph = ModuleGraph::build(
            &[p("main.lua"), p("a.lua"), p("b.lua")],
            &[(p("main.lua"), p("a.lua")), (p("main.lua"), p("b.lua"))],
        );
        assert_eq!(
            graph.modules(),
            vec![
                Path::new("a.lua"),
                Path::new("b.lua"),
                Path::new("main.lua")
            ]
        );
        assert_eq!(
            graph.dependencies(Path::new("main.lua")),
            vec![Path::new("a.lua"), Path::new("b.lua")]
        );
        assert!(graph.dependencies(Path::new("a.lua")).is_empty());
        assert!(graph.dependencies(Path::new("unknown.lua")).is_empty());
    }

    #[test]
    fn entry_points_have_no_incoming_edges() {
        let graph =
            ModuleGraph::build(&[p("main.lua"), p("a.lua")], &[(p("main.lua"), p("a.lua"))]);
        assert_eq!(graph.entry_points(), vec![Path::new("main.lua")]);
    }

    #[test]
    fn duplicate_edges_are_not_duplicated_in_dependencies() {
        let graph = ModuleGraph::build(
            &[p("main.lua"), p("a.lua")],
            &[(p("main.lua"), p("a.lua")), (p("main.lua"), p("a.lua"))],
        );
        assert_eq!(
            graph.dependencies(Path::new("main.lua")),
            vec![Path::new("a.lua")]
        );
    }

    #[test]
    fn detects_cycles() {
        let graph = ModuleGraph::build(
            &[p("a.lua"), p("b.lua")],
            &[(p("a.lua"), p("b.lua")), (p("b.lua"), p("a.lua"))],
        );
        assert_eq!(graph.cycles(), vec![vec![p("a.lua"), p("b.lua")]]);
    }

    #[test]
    fn detects_self_loop_cycle() {
        let graph = ModuleGraph::build(&[p("a.lua")], &[(p("a.lua"), p("a.lua"))]);
        assert_eq!(graph.cycles(), vec![vec![p("a.lua")]]);
    }

    #[test]
    fn acyclic_graph_has_no_cycles() {
        let graph =
            ModuleGraph::build(&[p("main.lua"), p("a.lua")], &[(p("main.lua"), p("a.lua"))]);
        assert!(graph.cycles().is_empty());
    }
}
