mod app;
mod ui;

use anyhow::Result;
use app::{App, AppAction, AppState};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode};
use log::{error, info, warn};
use ratatui::widgets::ListState;
use lazy_zip_core::{http_reader::RemoteHttpReader, zip_explorer::ZipExplorer};
use std::time::Duration;
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

fn validate_url(url: &str) -> Result<(), lazy_zip_core::error::Error> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(lazy_zip_core::error::Error::InvalidUrl(
            "URL must start with http:// or https://".to_string(),
        ));
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    
    let args = Args::parse();
    info!("Starting remote-zip-explorer");

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
        filter_input: String::new(),
        explorer: None,
        root_nodes: Vec::new(),
        current_path: Vec::new(),
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
            match event::poll(tick_rate) {
                Ok(true) => {
                    match event::read() {
                        Ok(event) => {
                            if tx_input.send(AppAction::Input(event)).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            // Terminal closed or read error - exit gracefully
                            let _ = tx_input.send(AppAction::InputError(e.to_string())).await;
                            break;
                        }
                    }
                }
                Ok(false) => {
                    if tx_input.send(AppAction::Tick).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    // Poll error - exit gracefully
                    let _ = tx_input.send(AppAction::InputError(e.to_string())).await;
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
                                            if let Err(e) = validate_url(&app.url_input) {
                                                app.state = AppState::Error(e.to_string());
                                                continue;
                                            }
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
                            } else if matches!(app.state, AppState::Filtering) {
                                match key.code {
                                    KeyCode::Char(c) => {
                                        app.filter_input.push(c);
                                        update_display_list(&mut app);
                                        app.list_state.select(Some(0));
                                    }
                                    KeyCode::Backspace => {
                                        app.filter_input.pop();
                                        update_display_list(&mut app);
                                        app.list_state.select(Some(0));
                                    }
                                    KeyCode::Enter | KeyCode::Esc => {
                                        app.state = AppState::Exploring;
                                    }
                                    _ => {}
                                }
                            } else if matches!(app.state, AppState::Exploring) {
                                match key.code {
                                    KeyCode::Char('/') => {
                                        app.state = AppState::Filtering;
                                        app.filter_input.clear();
                                        update_display_list(&mut app);
                                    }
                                    KeyCode::Char('q') => break,
                                    KeyCode::Char('d') => {
                                        // Trigger download for selected file
                                         if let Some(i) = app.list_state.selected() {
                                            if let Some(item) = app.display_items.get(i) {
                                                if !item.is_dir && item.name != ".." {
                                                    let name = item.name.clone();
                                                    let size = item.size;
                                                    start_download(&mut app, &tx, name, size);
                                                } else {
                                                    app.message = Some(("Select a file to download".to_string(), std::time::Instant::now()));
                                                }
                                            }
                                         }
                                    }
                                    KeyCode::Down => {
                                        let i = match app.list_state.selected() {
                                            Some(i) => {
                                                if app.display_items.is_empty() {
                                                    0
                                                } else if i >= app.display_items.len() - 1 {
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
                                                if app.display_items.is_empty() {
                                                    0
                                                } else if i == 0 {
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
                                                if item.name == ".." {
                                                    app.current_path.pop();
                                                    app.filter_input.clear(); // Clear filter on nav
                                                    update_display_list(&mut app);
                                                    app.list_state.select(Some(0));
                                                } else if item.is_dir {
                                                    app.current_path.push(item.name.clone());
                                                    app.filter_input.clear(); // Clear filter on nav
                                                    update_display_list(&mut app);
                                                    app.list_state.select(Some(0));
                                                } else {
                                                    let name = item.name.clone();
                                                    let size = item.size;
                                                    start_download(&mut app, &tx, name, size);
                                                }
                                            }
                                        }
                                    }
                                    KeyCode::Esc => {
                                        if !app.filter_input.is_empty() {
                                            app.filter_input.clear();
                                            update_display_list(&mut app);
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
                AppAction::InputError(msg) => {
                    app.message = Some((msg, std::time::Instant::now()));
                }
                AppAction::Loaded(res) => {
                    match res {
                        Ok((explorer, nodes)) => {
                            app.explorer = Some(explorer);
                            app.root_nodes = nodes;
                            app.state = AppState::Exploring;
                            app.current_path.clear();
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

fn start_download(app: &mut App, tx: &mpsc::Sender<AppAction>, name: String, size: u64) {
    app.state = AppState::Downloading(name.clone(), 0, size);
    let tx_dl = tx.clone();
    
    let url = app.url_input.clone();
    
    // Construct full path for download
    let mut path_parts = app.current_path.clone();
    path_parts.push(name.clone());
    let path = path_parts.join("/");
    
    let filename = name; // Save as just the filename in current local dir
    
    tokio::spawn(async move {
        download_file(url, path, filename, tx_dl).await;
    });
}

async fn load_zip(url: String, tx: mpsc::Sender<AppAction>) {
    info!("Loading ZIP from: {}", url);
    let res = async {
        let mut reader = RemoteHttpReader::new(&url).await?;
        let mut explorer = ZipExplorer::new(reader);
        let scan = explorer.list_files().await?;
        Ok((explorer, scan.files))
    }
    .await;

    match &res {
        Ok((_, files)) => info!("Successfully loaded {} files", files.len()),
        Err(e) => error!("Failed to load ZIP: {}", e),
    }

    let _ = tx.send(AppAction::Loaded(res)).await;
}

async fn download_file(url: String, path: String, filename: String, tx: mpsc::Sender<AppAction>) {
    info!("Starting download: {} from {}", filename, path);
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
            let _ = tx.send(AppAction::DownloadProgress(chunk.len() as u64)).await;
        }
        
        Ok(())
    }.await;
    
    match &res {
        Ok(()) => info!("Download complete: {}", filename),
        Err(e) => warn!("Download failed: {}", e),
    }

    let _ = tx.send(AppAction::DownloadComplete(res)).await;
}
