//! Rendering logic for the split-pane TUI: chat + neural canvas + brain stats.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Paragraph, Wrap,
        canvas::{Canvas, Circle, Line as CanvasLine},
    },
};

use super::app::{BrainState, TuiApp};

/// Render the full TUI frame: chat pane (left) + brain pane (right).
pub fn render(frame: &mut Frame, app: &TuiApp) {
    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(frame.area());

    render_chat_pane(frame, outer[0], app);
    render_brain_pane(frame, outer[1], &app.brain_state);
}

// ── Chat pane ────────────────────────────────────────────────────────────

fn render_chat_pane(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    // Message history (last 20 messages).
    let visible: Vec<Line<'_>> = app
        .messages
        .iter()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|(role, text)| {
            let color = if role == "user" {
                Color::Cyan
            } else {
                Color::Green
            };
            Line::from(vec![
                Span::styled(format!("{role}: "), Style::default().fg(color)),
                Span::raw(text),
            ])
        })
        .collect();

    let chat = Paragraph::new(visible)
        .block(Block::default().borders(Borders::ALL).title("Chat"))
        .wrap(Wrap { trim: false });
    frame.render_widget(chat, chunks[0]);

    // Input line.
    let input = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title("Input"));
    frame.render_widget(input, chunks[1]);
}

// ── Brain pane ───────────────────────────────────────────────────────────

fn render_brain_pane(frame: &mut Frame, area: Rect, state: &BrainState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_neural_canvas(frame, chunks[0], state);
    render_brain_stats(frame, chunks[1], state);
}

/// Draw the neural-network canvas using Braille markers.
pub fn render_neural_canvas(frame: &mut Frame, area: Rect, state: &BrainState) {
    let wave_pct = (state.activation_wave * 100.0) as u16;
    let title = format!("Neural Canvas [{wave_pct}%]");

    let canvas = Canvas::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .marker(ratatui::symbols::Marker::Braille)
        .x_bounds([0.0, 100.0])
        .y_bounds([0.0, 100.0])
        .paint(|ctx| {
            // Draw edges first (below nodes).
            for edge in &state.edges {
                if edge.from_idx >= state.active_nodes.len()
                    || edge.to_idx >= state.active_nodes.len()
                {
                    continue;
                }
                let from = &state.active_nodes[edge.from_idx];
                let to = &state.active_nodes[edge.to_idx];
                let color = if edge.active {
                    Color::Cyan
                } else {
                    Color::DarkGray
                };
                ctx.draw(&CanvasLine::new(from.x, from.y, to.x, to.y, color));
            }

            // Draw nodes.
            for node in &state.active_nodes {
                let color = match node.layer.as_str() {
                    "concept" => Color::Blue,
                    "memory" => Color::Green,
                    "axiom" => Color::Yellow,
                    "gap" => Color::Red,
                    _ => Color::White,
                };
                ctx.draw(&Circle {
                    x: node.x,
                    y: node.y,
                    radius: 1.5 + 0.5 * node.activation,
                    color,
                });
            }
        });

    frame.render_widget(canvas, area);
}

/// Render brain statistics and an activation bar.
pub fn render_brain_stats(frame: &mut Frame, area: Rect, state: &BrainState) {
    let filled = (state.activation_wave * 8.0) as usize;
    let empty = 8usize.saturating_sub(filled);
    let bar = format!("[{}{}]", "#".repeat(filled), "░".repeat(empty));

    let last_mem_preview = if state.last_memory.len() > 40 {
        format!("{}…", &state.last_memory[..40])
    } else {
        state.last_memory.clone()
    };

    let text = vec![
        Line::from(format!("Memories:     {}", state.memory_count)),
        Line::from(format!("KG Entities:  {}", state.kg_entity_count)),
        Line::from(format!("Queue Depth:  {}", state.queue_depth)),
        Line::from(format!("Last Memory:  {}", last_mem_preview)),
        Line::from(""),
        Line::from(vec![
            Span::raw("Activation:   "),
            Span::styled(bar, Style::default().fg(Color::Cyan)),
        ]),
    ];

    let stats = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Brain"))
        .wrap(Wrap { trim: false });
    frame.render_widget(stats, area);
}
