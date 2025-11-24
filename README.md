# Remote ZIP Peek

**List ZIP files without fully downloading the archive.**

`remote-zip-peek` is a Rust CLI tool that allows you to inspect the contents of a remote ZIP file using HTTP Range requests. It fetches only the Central Directory (metadata) located at the end of the file, saving bandwidth and time.

Perfect for checking large archives when you only need to know what's inside or verify file sizes.

## Features

-   **Bandwidth Efficient**: Fetches only the necessary metadata bytes (often < 1% of the file size).
-   **Fast**: Lists contents of gigabyte-sized archives in seconds.
-   **Secure**: Does not execute or extract files, only lists them.

## Installation

Ensure you have [Rust installed](https://www.rust-lang.org/tools/install).

```bash
git clone https://github.com/yourusername/remote-zip-peek.git
cd remote-zip-peek
cargo build --release
```

## Usage

Run the tool by providing the URL of the ZIP file you want to inspect.

```bash
# Basic usage
cargo run -- <URL>

# With human-readable sizes
cargo run -- -H <URL>
```

### Examples

**List files in a remote ZIP:**

```bash
cargo run -- https://example.com/large-dataset.zip
```

**Output:**

```text
Fetching ZIP from: https://example.com/large-dataset.zip
Total file size: 511.01 MB

Found 5 files:
- data.csv (Size: 120.50 MB, Compressed: 45.00 MB)
- images/logo.png (Size: 20.00 KB, Compressed: 19.53 KB)
...

Stats:
Total file size: 511.01 MB
Data fetched: 128.71 KB
Efficiency: 0.02% fetched
```

## How It Works

ZIP files store their directory structure (the Central Directory) at the *end* of the file. `remote-zip-peek` uses **HTTP Range requests** to:
1.  Fetch the last few bytes of the file to locate the Central Directory.
2.  Fetch only the Central Directory range.
3.  Parse the directory structure locally.

This approach avoids downloading the actual compressed file content, making it possible to list the contents of a 10GB ZIP file by downloading only a few kilobytes.

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).

## Keywords & Tags

`rust`, `zip`, `cli`, `http-range`, `remote-file`, `partial-download`, `bandwidth-saver`, `list-zip-contents`, `cloud-zip-viewer`
