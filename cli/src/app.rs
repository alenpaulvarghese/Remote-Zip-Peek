use anyhow::Result;
use crossterm::event::Event;
use lazy_zip_core::zip_explorer::{FileNode, ZipExplorer};
use ratatui::widgets::ListState;
use std::time::Instant;

pub enum AppState {
    InputUrl,
    Loading,
    Exploring,
    Filtering, // New state for typing filter
    Downloading(String, u64, u64),
    Error(String),
}

pub struct App {
    pub state: AppState,
    pub url_input: String,
    pub filter_input: String, // New field for filter text
    pub explorer: Option<ZipExplorer>,
    pub root_nodes: Vec<FileNode>,
    pub current_path: Vec<String>, // Stack of folder names
    pub list_state: ListState,
    pub display_items: Vec<DisplayItem>, // Items in current folder
    pub message: Option<(String, Instant)>,
}

pub struct DisplayItem {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

pub enum AppAction {
    Tick,
    Input(Event),
    InputError(String),
    Loaded(Result<(ZipExplorer, Vec<FileNode>)>),
    DownloadFile(String),
    DownloadProgress(u64),
    DownloadComplete(Result<()>),
}
