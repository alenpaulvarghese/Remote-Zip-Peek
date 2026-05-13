# Lazy ZIP Explorer

**Explore and download files from remote ZIP archives — without downloading the entire archive.**

Lazy ZIP Explorer uses HTTP Range requests to read only the ZIP metadata, letting you browse and selectively download files from multi-gigabyte archives while fetching just a few kilobytes of data.

## Desktop App

A cross-platform desktop application built with [Tauri](https://tauri.app/) + [Svelte](https://svelte.dev/).

### Features

- **Browse remote ZIPs** — paste a URL and instantly see the file tree
- **Selective downloads** — download individual files or entire folders
- **Concurrent workers** — configurable parallel downloads (default: 10 workers)
- **Smart skip** — detects existing files and lets you skip or re-download
- **Bandwidth stats** — shows total ZIP size vs. data actually fetched
- **Dark mode** — premium dark UI with indigo accents

### Download

Grab the latest release for your platform from the [Releases](https://github.com/AlenPaulVarworkunity/lazy-zip/releases) page:

| Platform | Format |
|----------|--------|
| macOS (Apple Silicon) | `.dmg` |
| macOS (Intel) | `.dmg` |
| Windows | `.msi` / `.exe` |
| Linux | `.deb` / `.AppImage` |

### Build from source

```bash
# Prerequisites: Rust, Node.js 20+
git clone https://github.com/alenpaulvarghese/lazy-zip.git
cd lazy-zip/desktop
npm install
npm run tauri build
```

---

## CLI (TUI)

A terminal interface built with `ratatui`.

```bash
# Run with a URL
cargo run -p lazy-zip-cli -- <URL>

# Or start and enter URL interactively
cargo run -p lazy-zip-cli
```

### Controls

- **Arrow Up/Down** — navigate the file list
- **Enter** — expand/collapse folders, or download a file
- **q** — quit
- **Esc** — return to URL input

---

## Python Library

A synchronous Python library for scripting.

```bash
pip install maturin
cd lazy-zip
maturin develop
```

```python
import lazy_zip

with lazy_zip.RemoteZip("https://example.com/files.zip") as z:
    for entry in z.files:
        print(entry.name, entry.size, entry.is_dir)

    data = z.read("path/in/zip/file.txt")
    z.read_to_file("path/in/zip/file.txt", "/tmp/output.txt")
```

---

## How It Works

ZIP files store their Central Directory at the **end** of the file. Lazy ZIP uses HTTP Range requests to:

1. Fetch the last few bytes to locate the Central Directory
2. Fetch only the Central Directory to build the file tree
3. When downloading, stream only the specific byte range for that file

This means you can explore a **10 GB** ZIP by downloading only a few **kilobytes**, then selectively download a 5 MB file without touching the other 9.995 GB.

## Project Structure

| Crate | Description |
|-------|-------------|
| `core/` | `lazy-zip-core` — HTTP reader, ZIP explorer, downloader |
| `cli/` | `lazy-zip-cli` — Terminal UI with ratatui |
| `desktop/` | Tauri + SvelteKit desktop app |
| `python/` | PyO3 Python bindings |

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).

## Keywords & Tags

`rust`, `zip`, `tauri`, `svelte`, `cli`, `tui`, `http-range`, `remote-file`, `partial-download`, `bandwidth-saver`, `desktop-app`