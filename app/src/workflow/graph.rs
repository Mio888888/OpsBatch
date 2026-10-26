//! Editing operations on a workflow graph (nodes + connections), independent of any view.

use ops_workflow::executor::build_execution_levels;
use ops_workflow::model::{node_type_entry, CanvasNode, Connection};

use super::node_form::SwitchConfig;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<CanvasNode>,
    pub connections: Vec<Connection>,
}

/// `(handle, label)` pairs a node can branch on.
pub type BranchOption = (String, String);

impl Graph {
    pub fn new(nodes: Vec<CanvasNode>, connections: Vec<Connection>) -> Self {
        Self { nodes, connections }
    }

    pub fn node(&self, id: &str) -> Option<&CanvasNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    fn node_mut(&mut self, id: &str) -> Option<&mut CanvasNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Add a node of `kind` (named after its palette label, like the original) in a tidy grid slot.
    pub fn add_node(&mut self, kind: &str, id: String) -> &CanvasNode {
        let n = self.nodes.len();
        let name = node_type_entry(kind).map(|t| t.fallback_label).unwrap_or(kind).to_string();
        let mut node = CanvasNode::new(id, kind, &name, "");
        node.x = 80.0 + 220.0 * (n % 4) as f64;
        node.y = 60.0 + 100.0 * (n / 4) as f64;
        self.nodes.push(node);
        self.nodes.last().expect("just pushed")
    }

    pub fn remove_node(&mut self, id: &str) {
        self.nodes.retain(|n| n.id != id);
        self.connections.retain(|c| c.from_id != id && c.to_id != id);
    }

    pub fn set_position(&mut self, id: &str, x: f64, y: f64) {
        if let Some(n) = self.node_mut(id) {
            n.x = x;
            n.y = y;
        }
    }

    pub fn set_name(&mut self, id: &str, name: &str) {
        if let Some(n) = self.node_mut(id) {
            n.name = name.to_string();
        }
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if let Some(n) = self.node_mut(id) {
            n.enabled = enabled;
        }
    }

    pub fn set_config(&mut self, id: &str, config: &str) {
        if let Some(n) = self.node_mut(id) {
            n.config = config.to_string();
        }
    }

    /// Add `from → to` (optionally from a branch handle). Self loops and exact duplicates are ignored.
    pub fn connect(&mut self, from: &str, to: &str, handle: Option<&str>) -> bool {
        if from == to || self.node(from).is_none() || self.node(to).is_none() {
            return false;
        }
        if self.connections.iter().any(|c| c.from_id == from && c.to_id == to && c.source_handle.as_deref() == handle) {
            return false;
        }
        let id = format!("edge-{from}-{to}{}", handle.map(|h| format!("-{h}")).unwrap_or_default());
        self.connections.push(Connection { id, from_id: from.into(), to_id: to.into(), source_handle: handle.map(str::to_string) });
        true
    }

    pub fn disconnect(&mut self, edge_id: &str) {
        self.connections.retain(|c| c.id != edge_id);
    }

    pub fn outgoing(&self, id: &str) -> Vec<&Connection> {
        self.connections.iter().filter(|c| c.from_id == id).collect()
    }

    /// Node ids per execution level.
    pub fn levels(&self) -> Vec<Vec<String>> {
        build_execution_levels(&self.nodes, &self.connections).into_iter().map(|l| l.into_iter().map(|n| n.id.clone()).collect()).collect()
    }

    /// Move a node one place up/down inside its level. The order inside a level is the execution order, which comes
    /// from the node order (first level) or from the order of the parents' connections (later levels), so whichever
    /// of the two changes the level's order is swapped.
    pub fn move_in_level(&mut self, id: &str, delta: i32) -> bool {
        let before = self.levels();
        let Some(level) = before.iter().find(|l| l.iter().any(|n| n == id)) else { return false };
        let pos = level.iter().position(|n| n == id).expect("in level");
        let other = pos as i32 + delta;
        if other < 0 || other as usize >= level.len() {
            return false;
        }
        let other_id = level[other as usize].clone();
        let mut wanted = level.clone();
        wanted.swap(pos, other as usize);
        let level_of = |g: &Graph| g.levels().into_iter().find(|l| l.iter().any(|n| n == id)).unwrap_or_default();

        // 1. swap the nodes themselves
        let (a, b) = (self.nodes.iter().position(|n| n.id == id), self.nodes.iter().position(|n| n.id == other_id));
        if let (Some(a), Some(b)) = (a, b) {
            self.nodes.swap(a, b);
            if level_of(self) == wanted {
                return true;
            }
            self.nodes.swap(a, b);
        }
        // 2. swap their first incoming connections
        let first_into = |g: &Graph, node: &str| g.connections.iter().position(|c| c.to_id == node);
        if let (Some(a), Some(b)) = (first_into(self, id), first_into(self, &other_id)) {
            self.connections.swap(a, b);
            if level_of(self) == wanted {
                return true;
            }
            self.connections.swap(a, b);
        }
        false
    }

    /// Branch handles of a node: `true`/`false` for conditions, `case-N` for switches, none otherwise.
    pub fn branch_options(&self, id: &str, true_label: &str, false_label: &str) -> Vec<BranchOption> {
        let Some(node) = self.node(id) else { return Vec::new() };
        match node.kind.as_str() {
            "condition" => vec![("true".into(), true_label.into()), ("false".into(), false_label.into())],
            "switch" => SwitchConfig::parse(&node.config).cases.into_iter().enumerate().map(|(i, (label, _))| (format!("case-{i}"), if label.is_empty() { format!("case-{i}") } else { label })).collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Graph;
    use ops_workflow::model::CanvasNode;

    fn graph() -> Graph {
        let mut g = Graph::default();
        for (id, kind) in [("a", "start"), ("b", "command"), ("c", "command"), ("d", "end")] {
            g.add_node(kind, id.to_string());
        }
        g
    }

    #[test]
    fn nodes_get_palette_names_and_grid_positions() {
        let g = graph();
        assert_eq!(g.node("a").unwrap().name, "开始");
        assert_eq!(g.node("b").unwrap().name, "执行命令");
        assert_eq!((g.node("a").unwrap().x, g.node("a").unwrap().y), (80.0, 60.0));
        assert_eq!((g.node("d").unwrap().x, g.node("d").unwrap().y), (80.0 + 660.0, 60.0));
        let mut g = g;
        g.add_node("delay", "e".into());
        assert_eq!(g.node("e").unwrap().y, 160.0, "fifth node starts the second row");
    }

    #[test]
    fn connections_reject_loops_duplicates_and_unknown_nodes() {
        let mut g = graph();
        assert!(g.connect("a", "b", None));
        assert!(!g.connect("a", "b", None) && !g.connect("a", "a", None) && !g.connect("a", "zzz", None));
        assert!(g.connect("a", "b", Some("true")), "a different handle is a different edge");
        assert_eq!(g.connections[1].id, "edge-a-b-true");
        g.disconnect("edge-a-b");
        assert_eq!(g.connections.len(), 1);
        g.remove_node("b");
        assert!(g.connections.is_empty() && g.node("b").is_none());
    }

    #[test]
    fn levels_and_reordering_inside_a_level() {
        let mut g = graph();
        g.connect("a", "b", None);
        g.connect("a", "c", None);
        g.connect("b", "d", None);
        g.connect("c", "d", None);
        assert_eq!(g.levels(), vec![vec!["a"], vec!["b", "c"], vec!["d"]]);
        assert!(g.move_in_level("c", -1));
        assert_eq!(g.levels()[1], ["c", "b"]);
        assert!(!g.move_in_level("c", -1), "already first");
        assert!(!g.move_in_level("d", 1), "alone in its level");
        assert!(!g.move_in_level("nope", 1));
    }

    #[test]
    fn branches_come_from_condition_and_switch_nodes() {
        let mut g = Graph::default();
        g.add_node("condition", "c".into());
        g.add_node("switch", "s".into());
        g.add_node("command", "x".into());
        g.set_config("s", r#"{"expression":"","cases":[{"label":"成功","value":"0"},{"label":"","value":"*"}]}"#);
        assert_eq!(g.branch_options("c", "是", "否"), vec![("true".to_string(), "是".to_string()), ("false".to_string(), "否".to_string())]);
        assert_eq!(g.branch_options("s", "", ""), vec![("case-0".to_string(), "成功".to_string()), ("case-1".to_string(), "case-1".to_string())]);
        assert!(g.branch_options("x", "", "").is_empty());
    }

    #[test]
    fn property_setters() {
        let mut g = Graph::new(vec![CanvasNode::new("n", "command", "old", "")], vec![]);
        g.set_name("n", "new");
        g.set_enabled("n", false);
        g.set_config("n", "ls");
        let n = g.node("n").unwrap();
        assert_eq!((n.name.as_str(), n.enabled, n.config.as_str()), ("new", false, "ls"));
    }
}
