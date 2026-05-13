use crate::app::{App, AppState, DisplayItem};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use lazy_zip_core::zip_explorer::FileNode;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::io;

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
            let text = Paragraph::new("Loading... Please wait.")
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(text, chunks[0]);
        }
        AppState::Exploring | AppState::Filtering | AppState::Downloading(..) => {
            let current_path_str = if app.current_path.is_empty() {
                "/".to_string()
            } else {
                format!("/{}", app.current_path.join("/"))
            };

            let title = if matches!(app.state, AppState::Filtering) {
                format!(
                    "Files - {} (Filter: {})",
                    current_path_str, app.filter_input
                )
            } else if !app.filter_input.is_empty() {
                format!(
                    "Files - {} (Filter: {})",
                    current_path_str, app.filter_input
                )
            } else {
                format!("Files - {}", current_path_str)
            };

            let items: Vec<ListItem> = app
                .display_items
                .iter()
                .map(|i| {
                    let icon = if i.name == ".." {
                        "⬆️ "
                    } else if i.is_dir {
                        "📂 "
                    } else {
                        "📄 "
                    };

                    let size_str = if i.is_dir || i.name == ".." {
                        "".to_string()
                    } else {
                        format!(" ({})", format_size(i.size))
                    };

                    let content = format!("{}{}{}", icon, i.name, size_str);
                    ListItem::new(content)
                })
                .collect();

            let border_style = if matches!(app.state, AppState::Filtering) {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };

            let list = List::new(items)
                .block(
                    Block::default()
                        .title(title)
                        .borders(Borders::ALL)
                        .border_style(border_style),
                )
                .highlight_style(
                    Style::default()
                        .add_modifier(Modifier::BOLD)
                        .fg(Color::Yellow),
                )
                .highlight_symbol("> ");

            f.render_stateful_widget(list, chunks[0], &mut app.list_state);

            // Status bar
            let status_text = match &app.state {
                AppState::Downloading(name, curr, total) => {
                    let pct = if *total > 0 {
                        (*curr as f64 / *total as f64) * 100.0
                    } else {
                        0.0
                    };
                    format!(
                        "Downloading: {} - {:.1}% ({}/{})",
                        name,
                        pct,
                        format_size(*curr),
                        format_size(*total)
                    )
                }
                AppState::Filtering => "Type to filter, Enter to apply, Esc to cancel".to_string(),
                _ => {
                    if let Some((msg, _)) = &app.message {
                        msg.clone()
                    } else {
                        "Enter: Open/Download | '..': Back | '/': Filter | 'd': Download | 'q': Quit".to_string()
                    }
                }
            };

            let status = Paragraph::new(status_text).block(Block::default().borders(Borders::ALL));
            f.render_widget(status, chunks[1]);

            // Overlay for downloading
            if let AppState::Downloading(name, curr, total) = &app.state {
                let area = centered_rect(60, 20, f.size());
                let block = Block::default().title("Downloading").borders(Borders::ALL);

                let pct = if *total > 0 {
                    (*curr as f64 / *total as f64) * 100.0
                } else {
                    0.0
                };

                let text = Paragraph::new(format!("Downloading {}\n{:.1}%", name, pct))
                    .style(Style::default().fg(Color::Green))
                    .block(block);
                f.render_widget(Clear, area); // Clear background
                f.render_widget(text, area);
            }
        }
        AppState::Error(msg) => {
            let text = Paragraph::new(format!("Error: {}\nPress Esc to retry.", msg)).block(
                Block::default()
                    .title("Error")
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Red)),
            );
            f.render_widget(text, chunks[0]);
        }
    }
}

pub fn update_display_list(app: &mut App) {
    app.display_items.clear();

    // Add ".." if not root
    if !app.current_path.is_empty() {
        app.display_items.push(DisplayItem {
            name: "..".to_string(),
            is_dir: true,
            size: 0,
        });
    }

    // Find children of current path
    let mut current_nodes = &app.root_nodes;
    for segment in &app.current_path {
        if let Some(node) = current_nodes
            .iter()
            .find(|n| n.name == *segment && n.is_dir)
        {
            current_nodes = &node.children;
        } else {
            // Should not happen if path is valid
            return;
        }
    }

    for node in current_nodes {
        if !app.filter_input.is_empty()
            && !node
                .name
                .to_lowercase()
                .contains(&app.filter_input.to_lowercase())
        {
            continue;
        }

        app.display_items.push(DisplayItem {
            name: node.name.clone(),
            is_dir: node.is_dir,
            size: node.size,
        });
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
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
