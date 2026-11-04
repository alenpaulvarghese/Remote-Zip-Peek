# lazy-zip for Python

Explore and download files from remote ZIP archives without downloading the entire archive.

**This is experimental and isn't on PyPI yet.** The API will change, and work on it is paused while the desktop app comes first.

## Installation

From this directory:

```bash
pip install maturin
maturin develop
```

## Usage

```python
import lazy_zip

with lazy_zip.RemoteZip("https://example.com/files.zip") as z:
    # The top level of the file tree. Folders have `children`.
    for entry in z.files:
        print(entry.name, entry.size, entry.is_dir)

    data = z.read("path/in/zip/file.txt")
    z.read_to_file("path/in/zip/file.txt", "/tmp/output.txt")
```

## API

| Method | Returns | Description |
|--------|---------|-------------|
| `RemoteZip(url)` | `RemoteZip` | Create an instance |
| `load()` | `None` | Load the archive's listing |
| `files` | `list[FileEntry]` | The top level of the file tree, loaded on first use |
| `read(path)` | `bytes` | Read a file into memory |
| `read_to_file(path, filepath)` | `None` | Save a file to disk |
| `close()` | `None` | Release resources |

## Known limitations

- `read_to_file` holds the whole file in memory before writing it.
- Calls hold the Python GIL, so they block other threads, and Ctrl-C waits for them to finish.
