use crate::app::{App, AppState, DisplayItem};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
    Terminal,
};
use remote_zip_core::zip_explorer::FileNode;
use std::{collections::HashSet, io};

pub fn init_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

pub fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
        .split(f.size());

    match &app.state {
        AppState::InputUrl => {
            let block = Block::default().title("Enter URL").borders(Borders::ALL);
            let text = Paragraph::new(app.url_input.clone()).block(block);
            f.render_widget(text, chunks[0]);
        }
        AppState::Loading => {
            let text = Paragraph::new("Loading... Please wait.").block(Block::default().borders(Borders::ALL));
            f.render_widget(text, chunks[0]);
        }
        AppState::Exploring | AppState::Downloading(..) => {
            let items: Vec<ListItem> = app
                .display_items
                .iter()
                .map(|i| {
                    let indent = "  ".repeat(i.depth);
                    let icon = if i.is_dir {
                        if app.expanded_paths.contains(&i.path) {
                            "📂 "
                        } else {
                            "📁 "
                        }
                    } else {
                        "📄 "
                    };
                    let content = format!("{}{}{}", indent, icon, i.name);
                    ListItem::new(content)
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().title("Files").borders(Borders::ALL))
                .highlight_style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow))
                .highlight_symbol("> ");

            f.render_stateful_widget(list, chunks[0], &mut app.list_state);

            // Status bar
            let status_text = match &app.state {
                AppState::Downloading(name, curr, total) => {
                    format!("Downloading: {} ({}/{})", name, curr, total)
                }
                _ => {
                    if let Some((msg, _)) = &app.message {
                        msg.clone()
                    } else {
                        "Use Arrow Keys to navigate, Enter to open/download, q to quit".to_string()
                    }
                }
            };
            
            let status = Paragraph::new(status_text).block(Block::default().borders(Borders::ALL));
            f.render_widget(status, chunks[1]);
            
            // Overlay for downloading
            if let AppState::Downloading(name, ..) = &app.state {
                 let area = centered_rect(60, 20, f.size());
                 let block = Block::default().title("Downloading").borders(Borders::ALL);
                 let text = Paragraph::new(format!("Downloading {}...", name)).block(block);
                 f.render_widget(Clear, area); // Clear background
                 f.render_widget(text, area);
            }
        }
        AppState::Error(msg) => {
            let text = Paragraph::new(format!("Error: {}\nPress Esc to retry.", msg))
                .block(Block::default().title("Error").borders(Borders::ALL).style(Style::default().fg(Color::Red)));
            f.render_widget(text, chunks[0]);
        }
    }
}

pub fn update_display_list(app: &mut App) {
    app.display_items = flatten_tree(&app.root_nodes, &app.expanded_paths, 0);
}

fn flatten_tree(nodes: &[FileNode], expanded: &HashSet<String>, depth: usize) -> Vec<DisplayItem> {
    let mut items = Vec::new();
    for node in nodes {
        items.push(DisplayItem {
            name: node.name.clone(),
            path: node.path.clone(),
            is_dir: node.is_dir,
            depth,
            size: node.size,
        });

        if node.is_dir && expanded.contains(&node.path) {
            items.extend(flatten_tree(&node.children, expanded, depth + 1));
        }
    }
    items
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
