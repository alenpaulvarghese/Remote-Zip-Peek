use anyhow::Result;
use crossterm::event::Event;
use ratatui::widgets::ListState;
use remote_zip_core::zip_explorer::{FileNode, ZipExplorer};
use std::collections::HashSet;
use std::time::Instant;

pub enum AppState {
    InputUrl,
    Loading,
    Exploring,
    Downloading(String, u64, u64), // Filename, current bytes, total bytes
    Error(String),
}

pub struct App {
    pub state: AppState,
    pub url_input: String,
    pub explorer: Option<ZipExplorer>,
    pub root_nodes: Vec<FileNode>,
    pub expanded_paths: HashSet<String>,
    pub list_state: ListState,
    pub display_items: Vec<DisplayItem>,
    pub message: Option<(String, Instant)>,
}

pub struct DisplayItem {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub depth: usize,
    pub size: u64,
}

pub enum AppAction {
    Tick,
    Input(Event),
    LoadUrl(String),
    Loaded(Result<(ZipExplorer, Vec<FileNode>)>),
    DownloadFile(String),
    DownloadProgress(u64),
    DownloadComplete(Result<()>),
}
