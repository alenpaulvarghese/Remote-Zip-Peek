use crate::downloader::EntryCheck;
use crate::error::{Error, Result};
use crate::http_reader::RemoteHttpReader;
use crate::safe_path::{display_name, path_components};
use async_zip::base::read::seek::ZipFileReader;
use serde::Serialize;
use std::collections::HashMap;
use tokio::io::BufReader;
use tokio_util::compat::TokioAsyncReadCompatExt;

/// One row of the archive's tree. A parent always precedes its children in [`ScanResult::nodes`].
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TreeNode {
    /// Index of the parent in [`ScanResult::nodes`].
    pub parent: Option<u32>,
    pub name: String,
    /// The entry's name in the archive for files, or the normalized path for directories.
    pub path: String,
    pub size: u64,
    pub compressed_size: u64,
    pub is_dir: bool,
    /// Number of files at or under this node.
    pub file_count: u64,
    /// Uncompressed size of all files at or under this node.
    pub total_size: u64,
}

/// A nested view of the tree, for consumers that want one.
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

/// An entry left out of the tree because it can't be written safely.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RejectedEntry {
    pub name: String,
    pub reason: String,
}

#[derive(Serialize, Debug)]
pub struct ScanResult {
    pub nodes: Vec<TreeNode>,
    /// The first [`MAX_LISTED_REJECTIONS`] rejected entries.
    pub rejected: Vec<RejectedEntry>,
    pub rejected_count: u64,
    pub total_size: u64,
    pub fetched_size: u64,
}

impl ScanResult {
    pub fn tree(&self) -> Vec<FileNode> {
        let mut children: Vec<Vec<FileNode>> = vec![Vec::new(); self.nodes.len()];
        let mut roots = Vec::new();

        // Children have higher indexes than their parents, so walking backwards finishes each
        // node's children before the node itself. Depth is capped, so nesting stays shallow.
        for (index, node) in self.nodes.iter().enumerate().rev() {
            let mut own_children = std::mem::take(&mut children[index]);
            own_children.reverse();
            let file_node = FileNode {
                name: node.name.clone(),
                path: node.path.clone(),
                size: node.size,
                compressed_size: node.compressed_size,
                is_dir: node.is_dir,
                children: own_children,
                parent_path: node.parent.map(|p| self.nodes[p as usize].path.clone()),
            };
            match node.parent {
                Some(parent) => children[parent as usize].push(file_node),
                None => roots.push(file_node),
            }
        }
        roots.reverse();
        roots
    }
}

const MAX_LISTED_REJECTIONS: usize = 1000;
const ROOT_KEY: u32 = u32::MAX;

struct RawEntry {
    name: String,
    size: u64,
    compressed_size: u64,
    is_dir: bool,
    invalid_utf8: bool,
}

#[derive(Default)]
struct TreeBuilder {
    nodes: Vec<TreeNode>,
    rejected: Vec<RejectedEntry>,
    rejected_count: u64,
}

impl TreeBuilder {
    fn reject(&mut self, name: &str, reason: String) {
        self.rejected_count += 1;
        if self.rejected.len() < MAX_LISTED_REJECTIONS {
            self.rejected.push(RejectedEntry {
                name: display_name(name),
                reason,
            });
        }
    }
}

/// Builds the tree in one pass, with a hash lookup per path segment.
fn build_tree(mut entries: Vec<RawEntry>) -> TreeBuilder {
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let mut builder = TreeBuilder::default();
    let mut lookup: HashMap<(u32, &str), u32> = HashMap::with_capacity(entries.len());

    'entries: for entry in &entries {
        if entry.invalid_utf8 {
            builder.reject(&entry.name, "name is not valid UTF-8".to_string());
            continue;
        }
        let components = match path_components(&entry.name) {
            Ok(components) => components,
            Err(error) => {
                builder.reject(&entry.name, error.to_string());
                continue;
            }
        };

        let mut parent = None;
        for (position, component) in components.iter().enumerate() {
            let is_last = position + 1 == components.len();
            let key = (parent.unwrap_or(ROOT_KEY), *component);

            if let Some(&existing) = lookup.get(&key) {
                let node = &mut builder.nodes[existing as usize];
                if !is_last && node.is_dir {
                    parent = Some(existing);
                } else if is_last && node.is_dir && entry.is_dir {
                    // An explicit entry for a directory that already exists.
                } else if is_last && !node.is_dir && !entry.is_dir {
                    // The last duplicate wins, matching what the downloader extracts.
                    node.path = entry.name.clone();
                    node.size = entry.size;
                    node.compressed_size = entry.compressed_size;
                } else {
                    builder.reject(
                        &entry.name,
                        "a file and a folder have the same name".to_string(),
                    );
                    continue 'entries;
                }
                continue;
            }

            let index = builder.nodes.len() as u32;
            let is_file = is_last && !entry.is_dir;
            let path = if is_file {
                entry.name.clone()
            } else {
                match parent {
                    Some(p) => format!("{}/{}", builder.nodes[p as usize].path, component),
                    None => component.to_string(),
                }
            };
            builder.nodes.push(TreeNode {
                parent,
                name: component.to_string(),
                path,
                size: if is_file { entry.size } else { 0 },
                compressed_size: if is_file { entry.compressed_size } else { 0 },
                is_dir: !is_file,
                file_count: 0,
                total_size: 0,
            });
            lookup.insert(key, index);
            parent = Some(index);
        }
    }

    // Parents precede children, so a reverse pass rolls totals up without recursion.
    for index in (0..builder.nodes.len()).rev() {
        let node = &mut builder.nodes[index];
        if !node.is_dir {
            node.file_count = 1;
            node.total_size = node.size;
        }
        let (parent, file_count, total_size) = (node.parent, node.file_count, node.total_size);
        if let Some(parent) = parent {
            let parent = &mut builder.nodes[parent as usize];
            parent.file_count += file_count;
            parent.total_size += total_size;
        }
    }

    builder
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

        let entries = reader
            .file()
            .entries()
            .iter()
            .map(|entry| {
                let filename = entry.filename();
                let name = String::from_utf8_lossy(filename.as_bytes()).into_owned();
                let is_dir = entry.dir().unwrap_or(false) || name.ends_with('/');
                RawEntry {
                    invalid_utf8: filename.as_str().is_err(),
                    size: entry.uncompressed_size(),
                    compressed_size: entry.compressed_size(),
                    is_dir,
                    name,
                }
            })
            .collect();
        drop(reader);

        let tree = build_tree(entries);

        Ok(ScanResult {
            nodes: tree.nodes,
            rejected: tree.rejected,
            rejected_count: tree.rejected_count,
            total_size: self.reader.get_content_length(),
            fetched_size: self.reader.get_network_bytes(),
        })
    }

    pub async fn get_file_stream(
        self,
        path: &str,
    ) -> Result<impl futures::Stream<Item = Result<bytes::Bytes>>> {
        let path = path.to_string();
        let fetch_limit = self.reader.fetch_limit();
        let mut reader = ZipFileReader::new(BufReader::new(self.reader).compat()).await?;
        fetch_limit.lift();

        let index = reader
            .file()
            .entries()
            .iter()
            .rposition(|e| e.filename().as_str().unwrap_or("") == path)
            .ok_or_else(|| Error::FileNotFound(path.clone()))?;
        let entry = EntryCheck::new(&reader.file().entries()[index], &path)?;

        // Use async-stream to create a stream that owns the reader
        let stream = async_stream::try_stream! {
            let entry_reader = reader.reader_without_entry(index).await?;
            use tokio::io::AsyncReadExt;
            use tokio_util::compat::FuturesAsyncReadCompatExt;
            let mut compat_reader = entry_reader.compat();
            let mut limited = (&mut compat_reader).take(entry.read_limit());

            let mut buffer = vec![0u8; 262144]; // 256KB buffer
            let mut total = 0;
            loop {
                let n = limited.read(&mut buffer).await.map_err(Error::from)?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                entry.check_size_so_far(total, &path)?;
                yield bytes::Bytes::copy_from_slice(&buffer[..n]);
            }
            entry.check_finished(total, compat_reader.get_mut().compute_hash(), &path)?;
        };

        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> RawEntry {
        RawEntry {
            name: name.to_string(),
            size,
            compressed_size: size / 2,
            is_dir: false,
            invalid_utf8: false,
        }
    }

    fn dir(name: &str) -> RawEntry {
        RawEntry {
            is_dir: true,
            ..file(name, 0)
        }
    }

    fn scan(entries: Vec<RawEntry>) -> ScanResult {
        let tree = build_tree(entries);
        ScanResult {
            nodes: tree.nodes,
            rejected: tree.rejected,
            rejected_count: tree.rejected_count,
            total_size: 0,
            fetched_size: 0,
        }
    }

    fn node<'a>(result: &'a ScanResult, path: &str) -> &'a TreeNode {
        result
            .nodes
            .iter()
            .find(|n| n.path == path)
            .unwrap_or_else(|| panic!("{path} not in tree"))
    }

    #[test]
    fn builds_tree_with_rolled_up_totals() {
        let result = scan(vec![
            file("a/b/one.txt", 10),
            file("a/two.txt", 20),
            file("three.txt", 30),
        ]);
        let a = node(&result, "a");
        assert!(a.is_dir);
        assert_eq!((a.file_count, a.total_size), (2, 30));
        let b = node(&result, "a/b");
        assert_eq!((b.file_count, b.total_size), (1, 10));
        let three = node(&result, "three.txt");
        assert_eq!(three.parent, None);
        assert_eq!((three.file_count, three.total_size), (1, 30));
    }

    #[test]
    fn parents_always_precede_children() {
        let result = scan(vec![file("z/y/x.txt", 1), dir("z/"), file("a/b.txt", 1)]);
        for (index, node) in result.nodes.iter().enumerate() {
            if let Some(parent) = node.parent {
                assert!((parent as usize) < index, "{}", node.path);
            }
        }
    }

    #[test]
    fn explicit_parent_does_not_create_double_slash_paths() {
        let result = scan(vec![dir("a/"), file("a/b/c.txt", 1)]);
        assert_eq!(node(&result, "a/b").name, "b");
        assert!(result.nodes.iter().all(|n| !n.path.contains("//")));
    }

    #[test]
    fn unnormalized_files_keep_their_raw_name_for_lookup() {
        let result = scan(vec![file("a//b.txt", 1)]);
        let b = node(&result, "a//b.txt");
        assert_eq!(b.name, "b.txt");
        assert_eq!(result.nodes[b.parent.unwrap() as usize].path, "a");
    }

    #[test]
    fn traversal_entries_are_rejected_not_listed() {
        let result = scan(vec![
            file("a/b/../../../evil", 1),
            file("/etc/passwd", 1),
            file("C:\\evil", 1),
            file("ok.txt", 1),
        ]);
        assert_eq!(result.rejected_count, 3);
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].name, "ok.txt");
    }

    #[test]
    fn extremely_deep_paths_are_refused_instead_of_aborting() {
        // Matches the 128 KB archive that overflowed the stack: 32,000 levels in one name.
        let result = scan(vec![file(&"a/".repeat(32_000), 1)]);
        assert_eq!(result.rejected_count, 1);
        assert!(result.nodes.is_empty());

        let empty_parts = format!("a{}b", "/".repeat(4000));
        let result = scan(vec![file(&empty_parts, 1)]);
        assert_eq!(result.nodes.len(), 2);
    }

    #[test]
    fn deepest_allowed_tree_converts_and_drops_safely() {
        let name = format!("{}file", "d/".repeat(crate::safe_path::MAX_DEPTH - 1));
        let result = scan(vec![file(&name, 1)]);
        assert_eq!(result.nodes.len(), crate::safe_path::MAX_DEPTH);
        let tree = result.tree();
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn last_duplicate_wins() {
        let result = scan(vec![file("dup.txt", 16), file("dup.txt", 4)]);
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].size, 4);
    }

    #[test]
    fn file_and_folder_with_the_same_name_is_rejected() {
        let result = scan(vec![file("x/y", 1), file("x/y/z", 1), dir("x/y/")]);
        assert_eq!(result.rejected_count, 2);
        assert!(!node(&result, "x/y").is_dir);
    }

    #[test]
    fn invalid_utf8_names_are_rejected_instead_of_collapsing() {
        let mut one = file("caf\u{fffd}.txt", 1);
        one.invalid_utf8 = true;
        let mut two = file("na\u{fffd}ve.txt", 1);
        two.invalid_utf8 = true;
        let result = scan(vec![one, two, file("ok.txt", 1)]);
        assert_eq!(result.rejected_count, 2);
        assert_eq!(result.nodes.len(), 1);
    }

    #[test]
    fn rejection_list_is_capped_but_counted() {
        let entries = (0..MAX_LISTED_REJECTIONS + 5)
            .map(|i| file(&format!("../{i}"), 1))
            .collect();
        let result = scan(entries);
        assert_eq!(result.rejected_count, MAX_LISTED_REJECTIONS as u64 + 5);
        assert_eq!(result.rejected.len(), MAX_LISTED_REJECTIONS);
    }

    #[test]
    fn nested_view_matches_the_flat_nodes() {
        let result = scan(vec![file("a/b/one.txt", 10), file("a/two.txt", 20)]);
        let tree = result.tree();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].path, "a");
        assert_eq!(tree[0].parent_path, None);
        let names: Vec<_> = tree[0].children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["b", "two.txt"]);
        assert_eq!(tree[0].children[0].parent_path.as_deref(), Some("a"));
        assert_eq!(tree[0].children[0].children[0].path, "a/b/one.txt");
    }

    #[test]
    fn flat_directory_with_many_entries_builds_completely() {
        let entries = (0..300_000).map(|i| file(&format!("d/{i}"), 1)).collect();
        let result = scan(entries);
        assert_eq!(result.nodes.len(), 300_001);
        assert_eq!(result.nodes[0].file_count, 300_000);
    }
}
