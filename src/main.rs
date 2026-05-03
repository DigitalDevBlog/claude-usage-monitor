mod app;
mod config;
mod pricing;
mod quota;
mod ui;
mod usage;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::{App, Interval, Tab};

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    res
}

fn run<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>) -> Result<()> {
    let mut app = App::new();
    loop {
        app.tick();
        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(Duration::from_millis(500))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Edit mode hijacks input on the Config tab.
                if app.tab == Tab::Config && app.editing {
                    match key.code {
                        KeyCode::Esc => app.edit_cancel(),
                        KeyCode::Enter => app.edit_commit(),
                        KeyCode::Backspace => app.edit_backspace(),
                        KeyCode::Char(c) => app.edit_push(c),
                        _ => {}
                    }
                    continue;
                }

                if app.tab == Tab::Config {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Esc => break,
                        KeyCode::Tab => app.cycle_tab(),
                        KeyCode::Up | KeyCode::Char('k') => app.select_prev_field(),
                        KeyCode::Down | KeyCode::Char('j') => app.select_next_field(),
                        KeyCode::Enter | KeyCode::Char(' ') => app.activate_field(),
                        KeyCode::Char('x') | KeyCode::Delete => app.clear_field(),
                        _ => {}
                    }
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('r') => app.trigger_refresh(),
                    KeyCode::Char('1') => app.set_interval(Interval::One),
                    KeyCode::Char('5') => app.set_interval(Interval::Five),
                    KeyCode::Char('0') => app.set_interval(Interval::Ten),
                    KeyCode::Tab | KeyCode::Char('t') => app.cycle_tab(),
                    _ => {}
                }
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
