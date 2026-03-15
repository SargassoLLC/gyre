//! TUI state-only tests (no terminal required).

use gyre::channels::tui::TuiApp;

#[test]
fn test_demo_creates_nodes() {
    let app = TuiApp::demo();
    assert_eq!(app.brain_state.active_nodes.len(), 6);
}

#[test]
fn test_fire_nodes_sets_activation() {
    let mut app = TuiApp::demo();
    // Reset all activations first.
    for node in &mut app.brain_state.active_nodes {
        node.activation = 0.0;
    }
    app.fire_nodes(&["options"]);
    let node = app
        .brain_state
        .active_nodes
        .iter()
        .find(|n| n.label == "options")
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
    // Reset, then fire one node.
    for node in &mut app.brain_state.active_nodes {
        node.activation = 0.0;
    }
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
