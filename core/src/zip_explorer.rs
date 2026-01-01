use crate::http_reader::RemoteHttpReader;
use anyhow::{anyhow, Result};
use async_zip::base::read::seek::ZipFileReader;
use serde::Serialize;
use tokio::io::BufReader;
use tokio_util::compat::TokioAsyncReadCompatExt;

#[derive(Serialize, Clone, Debug)]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub compressed_size: u64,
    pub is_dir: bool,
    pub children: Vec<FileNode>,
    pub parent_path: Option<String>,
}

pub struct ZipExplorer {
    pub reader: RemoteHttpReader,
}

impl ZipExplorer {
    pub fn new(reader: RemoteHttpReader) -> Self {
        Self { reader }
    }

    pub async fn list_files(&mut self) -> Result<ScanResult> {
        // Wrap the reader in a BufReader and compat for async_zip
        let reader = ZipFileReader::new(BufReader::new(&mut self.reader).compat()).await?;

        let mut root_nodes = Vec::new();
        let mut entries: Vec<(String, u64, u64, bool)> = Vec::new();

        for index in 0..reader.file().entries().len() {
            let entry = reader.file().entries().get(index).unwrap();
            let path = entry.filename().as_str().unwrap_or("unknown").to_string();
            let size = entry.uncompressed_size();
            let compressed_size = entry.compressed_size();
            let is_dir = entry.dir().unwrap_or(false) || path.ends_with('/');
            entries.push((path, size, compressed_size, is_dir));
        }

        // Sort entries by path length to ensure parents come before children (mostly)
        // Actually, sorting by path string is better to group them.
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Build tree
        // This is a simplified tree builder. For a robust one, we might need a map.
        // Given the constraints, let's use a recursive approach or a map-based one.
        // Since we need to return a Vec<FileNode> which represents the root level.

        // Let's use a helper to insert into the tree.
        for (path, size, compressed_size, is_dir) in entries {
            let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
            Self::insert_node(
                &mut root_nodes,
                &parts,
                path.clone(),
                size,
                compressed_size,
                is_dir,
                None,
            );
        }

        let total_size = self.reader.get_content_length();
        let fetched_size = self.reader.get_total_bytes_read();

        Ok(ScanResult {
            files: root_nodes,
            total_size,
            fetched_size,
        })
    }

    fn insert_node(
        nodes: &mut Vec<FileNode>,
        parts: &[&str],
        full_path: String,
        size: u64,
        compressed_size: u64,
        is_dir: bool,
        parent_path: Option<String>,
    ) {
        if parts.is_empty() {
            return;
        }

        let name = parts[0];
        let is_last = parts.len() == 1;

        // Check if node already exists
        if let Some(node) = nodes.iter_mut().find(|n| n.name == name) {
            if is_last {
                // Update existing node (e.g. if it was created as an implicit parent)
                node.size = size;
                node.compressed_size = compressed_size;
                node.is_dir = is_dir;
                node.path = full_path;
            } else {
                // Recurse
                let current_path = node.path.clone(); // Use current node's path as parent for children
                Self::insert_node(
                    &mut node.children,
                    &parts[1..],
                    full_path,
                    size,
                    compressed_size,
                    is_dir,
                    Some(current_path),
                );
            }
        } else {
            // Create new node
            let mut new_node = FileNode {
                name: name.to_string(),
                path: if is_last {
                    full_path.clone()
                } else {
                    // Reconstruct path for implicit parent
                    match &parent_path {
                        Some(p) => format!("{}/{}", p, name),
                        None => name.to_string(),
                    }
                },
                size: if is_last { size } else { 0 }, // Directory size 0 for now
                compressed_size: if is_last { compressed_size } else { 0 },
                is_dir: if is_last { is_dir } else { true }, // Implicit nodes are dirs
                children: Vec::new(),
                parent_path: parent_path.clone(),
            };

            if !is_last {
                let current_path = new_node.path.clone();
                Self::insert_node(
                    &mut new_node.children,
                    &parts[1..],
                    full_path,
                    size,
                    compressed_size,
                    is_dir,
                    Some(current_path),
                );
            }

            nodes.push(new_node);
        }
    }

    pub async fn get_file_stream(
        self,
        path: &str,
    ) -> Result<impl futures::Stream<Item = Result<bytes::Bytes>>> {
        let path = path.to_string();
        let mut reader = ZipFileReader::new(BufReader::new(self.reader).compat()).await?;

        let index = reader
            .file()
            .entries()
            .iter()
            .position(|e| e.filename().as_str().unwrap_or("") == path)
            .ok_or_else(|| anyhow!("File not found"))?;

        // Use async-stream to create a stream that owns the reader
        let stream = async_stream::try_stream! {
            let entry_reader = reader.reader_without_entry(index).await.map_err(|e| anyhow::Error::from(e))?;
            use tokio::io::AsyncReadExt;
            use tokio_util::compat::FuturesAsyncReadCompatExt;
            let mut compat_reader = entry_reader.compat();

            let mut buffer = [0u8; 65536]; // 64KB buffer
            loop {
                let n = compat_reader.read(&mut buffer).await.map_err(|e| anyhow::Error::from(e))?;
                if n == 0 {
                    break;
                }
                yield bytes::Bytes::copy_from_slice(&buffer[..n]);
            }
        };

        Ok(stream)
    }
}

#[derive(Serialize, Debug)]
pub struct ScanResult {
    pub files: Vec<FileNode>,
    pub total_size: u64,
    pub fetched_size: u64,
}
