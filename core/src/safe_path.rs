//! Validation of ZIP entry names, which come from an untrusted archive.

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_DEPTH: usize = 256;
const MAX_COMPONENT_BYTES: usize = 255;

fn unsafe_entry(entry: &str, reason: &'static str) -> Error {
    Error::UnsafeEntry {
        entry: display_name(entry),
        reason,
    }
}

/// Shortens a name for display, since an entry name can be up to 64 KB.
pub fn display_name(entry: &str) -> String {
    const MAX_CHARS: usize = 256;
    match entry.char_indices().nth(MAX_CHARS) {
        Some((end, _)) => format!("{}…", &entry[..end]),
        None => entry.to_string(),
    }
}

/// Splits an entry name into its normalized path components, dropping empty and `.` segments.
///
/// The same rules apply on every OS, so an archive behaves identically everywhere.
pub fn path_components(entry: &str) -> Result<Vec<&str>> {
    if entry.len() > MAX_PATH_BYTES {
        return Err(unsafe_entry(entry, "path is too long"));
    }
    if entry.starts_with('/') {
        return Err(unsafe_entry(entry, "absolute path"));
    }
    if entry.contains('\\') {
        return Err(unsafe_entry(entry, "contains a backslash"));
    }

    let mut components = Vec::new();
    for component in entry.split('/') {
        match component {
            "" | "." => continue,
            ".." => return Err(unsafe_entry(entry, "contains a parent directory reference")),
            _ => {
                if let Some(reason) = component_problem(component) {
                    return Err(unsafe_entry(entry, reason));
                }
                components.push(component);
            }
        }
    }

    if components.is_empty() {
        return Err(unsafe_entry(entry, "empty path"));
    }
    if components.len() > MAX_DEPTH {
        return Err(unsafe_entry(entry, "path is nested too deeply"));
    }
    Ok(components)
}

fn component_problem(component: &str) -> Option<&'static str> {
    if component.len() > MAX_COMPONENT_BYTES {
        return Some("file name is too long");
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return Some("name ends with a dot or space");
    }
    let forbidden = |c: char| {
        c.is_control()
            || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')
            || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
    };
    if component.chars().any(forbidden) {
        return Some("name contains a forbidden character");
    }

    let stem = component.split('.').next().unwrap_or("").trim_end();
    let stem = stem.to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (stem.chars().count() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem
            .chars()
            .nth(3)
            .is_some_and(|d| "123456789¹²³".contains(d)));
    reserved.then_some("name is reserved on Windows")
}

/// Computes where `entry` is written: under `root`, relative to the `base` directory in the archive.
pub fn destination(root: &Path, base: &str, entry: &str) -> Result<PathBuf> {
    let components = path_components(entry)?;
    let base_components = if base.is_empty() {
        Vec::new()
    } else {
        path_components(base)?
    };
    let relative = components
        .strip_prefix(base_components.as_slice())
        .filter(|rest| !rest.is_empty())
        .ok_or_else(|| unsafe_entry(entry, "not inside the selected folder"))?;

    let mut dest = root.to_path_buf();
    dest.extend(relative);
    Ok(dest)
}

/// Checks that an existing directory resolves inside `root`, so a symlink on disk can't redirect a write.
pub fn ensure_within(root: &Path, directory: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let directory = directory.canonicalize()?;
    if directory.starts_with(&root) {
        Ok(())
    } else {
        Err(unsafe_entry(
            &directory.to_string_lossy(),
            "resolves outside the destination folder",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_entries() {
        let cases = [
            ("../evil", "parent directory"),
            ("a/b/../../../evil", "parent directory"),
            ("a/..", "parent directory"),
            ("/etc/passwd", "absolute"),
            ("//host/share/file", "absolute"),
            ("C:\\evil", "backslash"),
            ("x/y/..\\..\\evil", "backslash"),
            ("\\\\host\\share\\f", "backslash"),
            ("C:foo", "forbidden character"),
            ("a/stream:name", "forbidden character"),
            ("nul\0byte", "forbidden character"),
            ("safe\u{202E}gnp.exe", "forbidden character"),
            ("what?.txt", "forbidden character"),
            ("CON", "reserved"),
            ("dir/nul.txt", "reserved"),
            ("com1", "reserved"),
            ("LPT9.log", "reserved"),
            ("trailing./file", "dot or space"),
            ("trailing /file", "dot or space"),
            ("", "empty"),
            ("./", "empty"),
        ];
        for (entry, expected) in cases {
            let error = path_components(entry).expect_err(entry).to_string();
            assert!(error.contains(expected), "{entry:?}: {error}");
        }
    }

    #[test]
    fn rejects_oversized_entries() {
        let too_deep = "a/".repeat(MAX_DEPTH + 1);
        assert!(path_components(&too_deep).is_err());
        assert!(path_components(&"a/".repeat(MAX_DEPTH)).is_ok());
        assert!(path_components(&"a".repeat(MAX_COMPONENT_BYTES + 1)).is_err());
        assert!(path_components(&"a/".repeat(MAX_PATH_BYTES)).is_err());
    }

    #[test]
    fn accepts_and_normalizes_ordinary_entries() {
        let cases: [(&str, &[&str]); 7] = [
            ("file.txt", &["file.txt"]),
            ("a/b/c.txt", &["a", "b", "c.txt"]),
            ("dir/", &["dir"]),
            ("a//b", &["a", "b"]),
            ("./a/./b", &["a", "b"]),
            ("console.log", &["console.log"]),
            ("日本語/ファイル.txt", &["日本語", "ファイル.txt"]),
        ];
        for (entry, expected) in cases {
            assert_eq!(path_components(entry).unwrap(), expected, "{entry:?}");
        }
    }

    #[test]
    fn destination_stays_under_root() {
        let root = Path::new("/out");
        assert_eq!(
            destination(root, "", "a/b.txt").unwrap(),
            root.join("a").join("b.txt")
        );
        assert_eq!(
            destination(root, "data/2024", "data/2024/q3/x.csv").unwrap(),
            root.join("q3").join("x.csv")
        );
        assert_eq!(
            destination(root, "a", "a//b.txt").unwrap(),
            root.join("b.txt")
        );
        assert!(destination(root, "", "a/b/../../../evil").is_err());
        assert!(destination(root, "data", "database/x.csv").is_err());
        assert!(destination(root, "data", "data").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn ensure_within_detects_symlink_escape() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(root.join("ok")).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();

        assert!(ensure_within(&root, &root.join("ok")).is_ok());
        assert!(ensure_within(&root, &root.join("link")).is_err());
    }

    #[test]
    fn display_name_truncates_on_a_char_boundary() {
        assert_eq!(display_name("short"), "short");
        assert_eq!(display_name(&"é".repeat(300)).chars().count(), 257);
    }
}
