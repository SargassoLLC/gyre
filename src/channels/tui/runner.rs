//! TUI demo runner — launches a full-screen Ratatui terminal.

use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use super::{TuiApp, render::render};

/// Run the interactive TUI demo (blocks until the user quits).
pub async fn run_tui_demo() -> std::io::Result<()> {
    // Set up terminal.
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = TuiApp::demo();

    loop {
        terminal.draw(|f| render(f, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key {
                    // Quit on Esc or Ctrl-C.
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    }
                    | KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    } => {
                        app.should_quit = true;
                    }
                    // Quit on 'q' only when input is empty (so you can type words with q).
                    KeyEvent {
                        code: KeyCode::Char('q'),
                        modifiers: KeyModifiers::NONE,
                        ..
                    } if app.input.is_empty() => {
                        app.should_quit = true;
                    }
                    // Submit input.
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => {
                        if !app.input.is_empty() {
                            let text = app.input.clone();
                            app.add_message("user", &text);
                            app.input.clear();
                            app.fire_nodes(&["options", "gamma"]);
                            app.add_message("gyre", "Processing...");
                        }
                    }
                    // Delete last char.
                    KeyEvent {
                        code: KeyCode::Backspace,
                        ..
                    } => {
                        app.input.pop();
                    }
                    // Regular character input.
                    KeyEvent {
                        code: KeyCode::Char(c),
                        ..
                    } => {
                        app.input.push(c);
                    }
                    _ => {}
                }
            }
        }

        app.tick();

        if app.should_quit {
            break;
        }
    }

    // Restore terminal.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
