mod app;
mod ui;

use anyhow::Result;
use app::{App, AppAction, AppState};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode};
use ratatui::widgets::ListState;
use remote_zip_core::{http_reader::RemoteHttpReader, zip_explorer::ZipExplorer};
use std::{collections::HashSet, time::Duration};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use ui::{init_terminal, restore_terminal, ui, update_display_list};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// URL of the ZIP file to explore
    #[arg(index = 1)]
    url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Setup terminal
    let mut terminal = init_terminal()?;

    // Create app
    let mut app = App {
        state: if let Some(_url) = &args.url {
            AppState::Loading
        } else {
            AppState::InputUrl
        },
        url_input: String::new(),
        explorer: None,
        root_nodes: Vec::new(),
        expanded_paths: HashSet::new(),
        list_state: ListState::default(),
        display_items: Vec::new(),
        message: None,
    };

    let (tx, mut rx) = mpsc::channel(100);
    
    let args_url = args.url.clone();
    if let Some(url) = args_url {
        app.url_input = url.clone();
        let tx_load = tx.clone();
        tokio::spawn(async move {
            load_zip(url, tx_load).await;
        });
    }

    // Input loop
    let tx_input = tx.clone();
    tokio::spawn(async move {
        let tick_rate = Duration::from_millis(250);
        loop {
            if event::poll(tick_rate).unwrap() {
                let event = event::read().unwrap();
                if tx_input.send(AppAction::Input(event)).await.is_err() {
                    break;
                }
            } else {
                if tx_input.send(AppAction::Tick).await.is_err() {
                    break;
                }
            }
        }
    });

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if let Some(action) = rx.recv().await {
            match action {
                AppAction::Tick => {
                    // Check message expiration
                    if let Some((_, time)) = &app.message {
                        if time.elapsed() > Duration::from_secs(3) {
                            app.message = None;
                        }
                    }
                }
                AppAction::Input(event) => {
                    match event {
                        Event::Key(key) => {
                            if matches!(app.state, AppState::InputUrl) {
                                match key.code {
                                    KeyCode::Enter => {
                                        if !app.url_input.is_empty() {
                                            app.state = AppState::Loading;
                                            let tx_load = tx.clone();
                                            let url = app.url_input.clone();
                                            tokio::spawn(async move {
                                                load_zip(url, tx_load).await;
                                            });
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        app.url_input.push(c);
                                    }
                                    KeyCode::Backspace => {
                                        app.url_input.pop();
                                    }
                                    KeyCode::Esc => {
                                        break; // Quit
                                    }
                                    _ => {}
                                }
                            } else if matches!(app.state, AppState::Exploring) {
                                match key.code {
                                    KeyCode::Char('q') => break,
                                    KeyCode::Down => {
                                        let i = match app.list_state.selected() {
                                            Some(i) => {
                                                if i >= app.display_items.len() - 1 {
                                                    0
                                                } else {
                                                    i + 1
                                                }
                                            }
                                            None => 0,
                                        };
                                        app.list_state.select(Some(i));
                                    }
                                    KeyCode::Up => {
                                        let i = match app.list_state.selected() {
                                            Some(i) => {
                                                if i == 0 {
                                                    app.display_items.len() - 1
                                                } else {
                                                    i - 1
                                                }
                                            }
                                            None => 0,
                                        };
                                        app.list_state.select(Some(i));
                                    }
                                    KeyCode::Enter => {
                                        if let Some(i) = app.list_state.selected() {
                                            if let Some(item) = app.display_items.get(i) {
                                                if item.is_dir {
                                                    if app.expanded_paths.contains(&item.path) {
                                                        app.expanded_paths.remove(&item.path);
                                                    } else {
                                                        app.expanded_paths.insert(item.path.clone());
                                                    }
                                                    update_display_list(&mut app);
                                                } else {
                                                    // Start download process
                                                    app.state = AppState::Downloading(item.name.clone(), 0, item.size);
                                                    let tx_dl = tx.clone();
                                                    
                                                    let url = app.url_input.clone();
                                                    let path = item.path.clone();
                                                    let name = item.name.clone();
                                                    
                                                    tokio::spawn(async move {
                                                        download_file(url, path, name, tx_dl).await;
                                                    });
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            } else if matches!(app.state, AppState::Error(_)) {
                                if key.code == KeyCode::Esc || key.code == KeyCode::Enter {
                                    app.state = AppState::InputUrl; // Reset
                                    app.url_input.clear();
                                }
                            }
                        }
                        _ => {}
                    }
                }
                AppAction::LoadUrl(_url) => {
                     // Handled via Input event manually
                }
                AppAction::Loaded(res) => {
                    match res {
                        Ok((explorer, nodes)) => {
                            app.explorer = Some(explorer);
                            app.root_nodes = nodes;
                            app.state = AppState::Exploring;
                            app.expanded_paths.clear();
                            update_display_list(&mut app);
                            if !app.display_items.is_empty() {
                                app.list_state.select(Some(0));
                            }
                        }
                        Err(e) => {
                            app.state = AppState::Error(e.to_string());
                        }
                    }
                }
                AppAction::DownloadFile(_) => {
                    // Marker for start
                }
                AppAction::DownloadProgress(bytes) => {
                    if let AppState::Downloading(_, current, _) = &mut app.state {
                        *current += bytes;
                    }
                }
                AppAction::DownloadComplete(res) => {
                    match res {
                        Ok(_) => {
                            app.message = Some(("Download completed successfully!".to_string(), std::time::Instant::now()));
                        }
                        Err(e) => {
                            app.message = Some((format!("Download failed: {}", e), std::time::Instant::now()));
                        }
                    }
                    app.state = AppState::Exploring; // Return to explorer
                }
            }
        }
    }

    // Restore terminal
    restore_terminal(&mut terminal)?;

    Ok(())
}

async fn load_zip(url: String, tx: mpsc::Sender<AppAction>) {
    let res = async {
        let mut reader = RemoteHttpReader::new(&url).await?;
        let mut explorer = ZipExplorer::new(reader);
        let scan = explorer.list_files().await?;
        Ok((explorer, scan.files))
    }
    .await;

    tx.send(AppAction::Loaded(res)).await.unwrap();
}

async fn download_file(url: String, path: String, filename: String, tx: mpsc::Sender<AppAction>) {
    let res = async {
        let reader = RemoteHttpReader::new(&url).await?;
        let explorer = ZipExplorer::new(reader);
        
        let stream = explorer.get_file_stream(&path).await?;
        tokio::pin!(stream);

        use tokio_stream::StreamExt;
        
        let mut file = tokio::fs::File::create(&filename).await?;
        
        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res?;
            file.write_all(&chunk).await?;
            tx.send(AppAction::DownloadProgress(chunk.len() as u64)).await.unwrap();
        }
        
        Ok(())
    }.await;
    
    tx.send(AppAction::DownloadComplete(res)).await.unwrap();
}
