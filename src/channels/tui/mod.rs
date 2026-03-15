//! Ratatui-based TUI with split-pane chat and neural canvas visualization.

pub mod app;
pub mod render;
pub mod runner;

pub use app::{BrainState, NeuronEdge, NeuronNode, TuiApp};
pub use runner::run_tui_demo;
