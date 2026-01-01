# Remote ZIP Explorer

**Explore and download files from remote ZIP archives without downloading the entire archive.**

`remote-zip-explorer` is a Rust-based TUI (Terminal User Interface) tool that allows you to inspect the contents of a remote ZIP file using HTTP Range requests. It fetches only the necessary metadata to show you the file tree and allows you to download specific files on demand.

## Features

-   **Interactive TUI**: Navigate large ZIP archives easily with a file-tree explorer.
-   **On-demand Downloads**: Download only the files you need from the archive.
-   **Bandwidth Efficient**: Lists contents of gigabyte-sized archives by fetching only a few kilobytes of metadata.
-   **High Performance**: Uses an optimized 1MB buffer for fast remote streaming and downloads.
-   **Workspace Based**: Cleanly separated core logic (`core`) and TUI implementation (`cli`).

## Installation

Ensure you have [Rust installed](https://www.rust-lang.org/tools/install).

```bash
git clone https://github.com/yourusername/remote-zip-explorer.git
cd remote-zip-explorer
cargo build --release
```

## Usage

Run the tool by providing the URL of the ZIP file you want to explore, or enter it manually in the TUI.

```bash
# Start with a URL
cargo run -p remote-zip-cli -- <URL>

# Start and enter URL in the interface
cargo run -p remote-zip-cli
```

### Controls

-   **Arrow Up/Down**: Navigate the file list.
-   **Enter**:
    -   On a folder: Expand or collapse the directory.
    -   On a file: Start downloading the file to your current directory.
-   **'q'**: Quit the application.
-   **Esc**: Return to the URL input screen or exit error states.

## How It Works

ZIP files store their directory structure (the Central Directory) at the *end* of the file. `remote-zip-explorer` uses **HTTP Range requests** to:
1.  Fetch the last few bytes of the file to locate the Central Directory.
2.  Fetch only the Central Directory range to build the local file tree.
3.  When a download is requested, it streams only the specific range of the file where the compressed data is stored.

This approach makes it possible to explore a 10GB ZIP file by downloading only a few kilobytes, and then selectively download a 5MB file from it without ever touching the other 9.995GB.

## Project Structure

-   `core/`: Library containing the `RemoteHttpReader` and `ZipExplorer` logic.
-   `cli/`: TUI application built with `ratatui` and `crossterm`.

## License

This project is licensed under the [MIT License](LICENSE).

## Keywords & Tags

`rust`, `zip`, `cli`, `tui`, `ratatui`, `http-range`, `remote-file`, `partial-download`, `bandwidth-saver`, `list-zip-contents`, `cloud-zip-viewer`