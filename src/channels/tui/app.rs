//! TUI application state: neurons, edges, brain state, and demo data.

/// A single node in the neural canvas.
#[derive(Debug, Clone)]
pub struct NeuronNode {
    pub label: String,
    pub x: f64,
    pub y: f64,
    pub activation: f64,
    pub layer: String,
}

/// A directed edge between two neurons.
#[derive(Debug, Clone)]
pub struct NeuronEdge {
    pub from_idx: usize,
    pub to_idx: usize,
    pub weight: f64,
    pub active: bool,
}

/// Aggregate brain statistics displayed alongside the neural canvas.
#[derive(Debug, Clone)]
pub struct BrainState {
    pub memory_count: u64,
    pub kg_entity_count: u64,
    pub queue_depth: u32,
    pub last_memory: String,
    pub active_nodes: Vec<NeuronNode>,
    pub edges: Vec<NeuronEdge>,
    pub activation_wave: f64,
}

impl Default for BrainState {
    fn default() -> Self {
        Self {
            memory_count: 0,
            kg_entity_count: 0,
            queue_depth: 0,
            last_memory: String::new(),
            active_nodes: Vec::new(),
            edges: Vec::new(),
            activation_wave: 0.0,
        }
    }
}

/// Top-level TUI application state.
pub struct TuiApp {
    pub agent_id: String,
    pub messages: Vec<(String, String)>,
    pub input: String,
    pub brain_state: BrainState,
    pub should_quit: bool,
}

impl TuiApp {
    /// Create a new empty TUI app.
    pub fn new(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            messages: Vec::new(),
            input: String::new(),
            brain_state: BrainState::default(),
            should_quit: false,
        }
    }

    /// Create a demo app with sample neurons and edges for visual testing.
    pub fn demo() -> Self {
        let nodes = vec![
            NeuronNode {
                label: "options".into(),
                x: 50.0,
                y: 30.0,
                activation: 0.0,
                layer: "concept".into(),
            },
            NeuronNode {
                label: "gamma".into(),
                x: 70.0,
                y: 40.0,
                activation: 0.0,
                layer: "concept".into(),
            },
            NeuronNode {
                label: "memory_spy".into(),
                x: 30.0,
                y: 60.0,
                activation: 0.0,
                layer: "memory".into(),
            },
            NeuronNode {
                label: "axiom_patience".into(),
                x: 60.0,
                y: 70.0,
                activation: 0.0,
                layer: "axiom".into(),
            },
            NeuronNode {
                label: "gap_vanna".into(),
                x: 80.0,
                y: 20.0,
                activation: 0.0,
                layer: "gap".into(),
            },
            NeuronNode {
                label: "gex_flow".into(),
                x: 40.0,
                y: 45.0,
                activation: 0.0,
                layer: "concept".into(),
            },
        ];
        let edges = vec![
            NeuronEdge {
                from_idx: 0,
                to_idx: 1,
                weight: 0.8,
                active: false,
            },
            NeuronEdge {
                from_idx: 0,
                to_idx: 5,
                weight: 0.6,
                active: false,
            },
            NeuronEdge {
                from_idx: 1,
                to_idx: 3,
                weight: 0.5,
                active: false,
            },
            NeuronEdge {
                from_idx: 2,
                to_idx: 5,
                weight: 0.7,
                active: false,
            },
            NeuronEdge {
                from_idx: 4,
                to_idx: 0,
                weight: 0.4,
                active: false,
            },
        ];

        let mut app = Self {
            agent_id: "demo".into(),
            messages: vec![(
                "gyre".into(),
                "Neural canvas demo — press Enter to fire nodes, q/Esc to quit.".into(),
            )],
            input: String::new(),
            brain_state: BrainState {
                memory_count: 42,
                kg_entity_count: 128,
                queue_depth: 3,
                last_memory: "Explored options pricing models".into(),
                active_nodes: nodes,
                edges,
                activation_wave: 0.0,
            },
            should_quit: false,
        };

        // Fire initial demo nodes.
        app.fire_nodes(&["options", "gamma"]);
        app
    }

    /// Append a chat message.
    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push((role.to_string(), content.to_string()));
    }

    /// Set activation to 1.0 for nodes matching any of `labels` and mark
    /// adjacent edges as active.
    pub fn fire_nodes(&mut self, labels: &[&str]) {
        let fired_indices: Vec<usize> = self
            .brain_state
            .active_nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| labels.contains(&n.label.as_str()))
            .map(|(i, _)| i)
            .collect();

        for &idx in &fired_indices {
            self.brain_state.active_nodes[idx].activation = 1.0;
        }

        for edge in &mut self.brain_state.edges {
            if fired_indices.contains(&edge.from_idx) || fired_indices.contains(&edge.to_idx) {
                edge.active = true;
            }
        }
    }

    /// Advance the animation clock: bump `activation_wave`, decay activations.
    pub fn tick(&mut self) {
        self.brain_state.activation_wave = (self.brain_state.activation_wave + 0.05) % 1.0;

        for node in &mut self.brain_state.active_nodes {
            node.activation = (node.activation - 0.1).max(0.0);
        }

        // Deactivate edges whose source has fully decayed.
        for edge in &mut self.brain_state.edges {
            let src = &self.brain_state.active_nodes[edge.from_idx];
            if src.activation <= 0.0 {
                edge.active = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_demo_creates_nodes() {
        let app = TuiApp::demo();
        assert_eq!(app.brain_state.active_nodes.len(), 6);
    }

    #[test]
    fn test_fire_nodes_sets_activation() {
        let mut app = TuiApp::demo();
        // Demo fires options+gamma already; re-fire a single node to check.
        app.fire_nodes(&["memory_spy"]);
        let node = app
            .brain_state
            .active_nodes
            .iter()
            .find(|n| n.label == "memory_spy")
            .unwrap();
        assert!((node.activation - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_tick_advances_wave() {
        let mut app = TuiApp::demo();
        app.brain_state.activation_wave = 0.0;
        app.tick();
        assert!(app.brain_state.activation_wave > 0.0);
    }

    #[test]
    fn test_decay() {
        let mut app = TuiApp::demo();
        app.fire_nodes(&["options"]);
        for _ in 0..15 {
            app.tick();
        }
        let node = app
            .brain_state
            .active_nodes
            .iter()
            .find(|n| n.label == "options")
            .unwrap();
        assert!(node.activation < 0.5);
    }

    #[test]
    fn test_add_message() {
        let mut app = TuiApp::new("test");
        app.add_message("user", "hello");
        assert!(app.messages.contains(&("user".into(), "hello".into())));
    }
}
