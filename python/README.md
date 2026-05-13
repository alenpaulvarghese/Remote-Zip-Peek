# lazy-zip

Python library for exploring and downloading files from remote ZIP archives without downloading the entire archive.

## Installation

```bash
# Using maturin (recommended)
pip install maturin
maturin develop

# Or build manually
cargo build --release -p lazy-zip
cp target/release/deps/lazy_zip.so lazy_zip.so
```

## Usage

```python
import lazy_zip

# Context manager (recommended - ensures cleanup)
with lazy_zip.RemoteZip("https://example.com/files.zip") as z:
    # List files (lazy loaded on first access)
    for entry in z.files:
        print(entry.name, entry.size, entry.is_dir)
    
    # Read file into memory
    data = z.read("path/in/zip/file.txt")
    
    # Save directly to file (better for large files)
    z.read_to_file("path/in/zip/file.txt", "/tmp/output.txt")

# Manual control
z = lazy_zip.RemoteZip("https://example.com/files.zip")
z.load()  # Explicit load
files = z.files
data = z.read("file.txt")
z.close()
```

## API

| Method | Returns | Description |
|--------|---------|-------------|
| `RemoteZip(url)` | RemoteZip | Create instance |
| `load()` | None | Load ZIP metadata |
| `files` | List[FileEntry] | Get file list |
| `read(path)` | bytes | Download file to memory |
| `read_to_file(path, filepath)` | None | Save to disk |
| `close()` | None | Close connection |
