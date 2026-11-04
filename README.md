# Lazy ZIP

**Browse a ZIP archive by URL and download only the files you pick.**

Lazy ZIP reads an archive's table of contents using HTTP Range requests, so you can see what's inside a huge remote ZIP for a small fraction of its size. It works on any server that supports Range requests, including Zenodo, Kaggle, GitHub release assets and most S3-style hosts.

![Lazy ZIP browsing an archive](docs/screenshot.png)

## Features

- **Open huge archives:** paste a URL and browse the file tree. Hundreds of thousands of entries stay smooth.
- **Find files:** search by name, or use a pattern like `*.json`.
- **Pick what you want:** select individual files, ranges or whole folders, and see the total size before downloading.
- **Download manager:** live progress in bytes, cancel, retry only what failed, and show the result in its folder.
- **Keyboard shortcuts:** the bar along the bottom shows what works right now. Press `?` for the full list.
- **Safe with untrusted archives:** unsafe file names are refused, every file's size and checksum are verified, and a failed download never damages an existing file.

## Download

Get the latest version from the [Releases](https://github.com/alenpaulvarghese/lazy-zip/releases) page.

| Platform | File |
|----------|------|
| Windows | `.msi` or `.exe` |
| macOS (Apple Silicon) | `aarch64.dmg` |
| macOS (Intel) | `x64.dmg` |
| Linux | `.deb` |

### First launch

These builds aren't signed yet, so your system will warn you the first time.

- **macOS:** open the app, then go to **System Settings > Privacy & Security** and choose **Open Anyway**.
- **Windows:** on the SmartScreen warning, choose **More info**, then **Run anyway**.
- **Linux:** run `sudo apt install ./<the file you downloaded>.deb`.

### Verifying a download

Each release includes `SHA256SUMS.txt`, and a provenance attestation showing the file was built by this repository's release workflow:

```bash
shasum -a 256 --ignore-missing -c SHA256SUMS.txt
gh attestation verify <file> --repo alenpaulvarghese/lazy-zip
```

## How much data does it use?

ZIP files keep their table of contents at the end. Lazy ZIP fetches the end of the file first, then only the rest of the table of contents if it needs more. These were measured on real archives:

| Archive | Size | Entries | Data used to list it |
|---------|------|---------|----------------------|
| [COCO val2017](http://images.cocodataset.org/zips/val2017.zip) | 778 MB | 5,000 | 459 KB |
| [Node.js for Windows](https://nodejs.org/dist/v22.11.0/node-v22.11.0-win-x64.zip) | 33 MB | 3,149 | 500 KB |

Listing costs at least 66 KB, or the whole file if it's smaller, plus roughly 50–100 bytes per entry.

Downloading costs more than the files themselves, because each file currently re-reads the table of contents and fetches at least 1 MiB. One 158 KB image from COCO val2017 used 1.4 MB. **Selecting many small files can use more data than downloading the whole archive.** The download manager shows the real figure for each job, and fixing this is the next major improvement.

## Limitations

- **The server must support Range requests.** Without them you get a clear error rather than a large download. GitHub's automatic source archives are an example.
- **Compression:** Stored, Deflate, Deflate64, BZip2, Zstandard and XZ can be extracted. LZMA is listed but can't be extracted.
- **One entry with any other method, such as PPMd or AES encryption, stops the whole archive from opening.**
- **Encrypted files can't be extracted.** They fail their checksum instead of saving garbage.
- **Refused names:** entries with `..`, absolute paths, backslashes, Windows reserved names, or characters like `?` and `:` are listed as unsafe and can't be downloaded, on every platform. Names that aren't valid UTF-8 are refused too.
- **Size limits:** archives with more than 5 million entries, or a table of contents over 512 MB, are refused.
- **No custom headers or logins yet,** so private files need a URL that works on its own, such as a presigned one.
- **No resuming after you quit the app.** Dropped connections are retried, but an interrupted file starts again.

## Troubleshooting

Check whether a server supports Range requests:

```bash
curl -sI -H 'Range: bytes=0-99' '<url>' | head -n 1
```

`206` means it does. `200` means it doesn't, and the archive can't be browsed remotely.

| Message | What it means |
|---------|---------------|
| The server doesn't support partial downloads | The server ignored the Range request, so use a browser to download the whole file. |
| The server returned HTTP 403 or 404 | The link is wrong, expired, or needs a login. |
| This file isn't a ZIP archive | The URL returned something else, often a web page. Use the direct link to the file. |
| The archive changed on the server | The file was replaced after you scanned it. Scan it again. |
| ... is corrupted | The data didn't match its checksum or size. Try again, and if it persists, the archive itself is damaged. |
| compression not supported | An entry uses a method listed under Limitations, so the archive can't be opened. |

## Build from source

You need Rust, Node.js 22 and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform.

```bash
git clone https://github.com/alenpaulvarghese/lazy-zip.git
cd lazy-zip/desktop
npm ci
npm run tauri build
```

## Project structure

| Directory | Description |
|-----------|-------------|
| `core/` | `lazy-zip-core`: HTTP reader, ZIP handling and downloader |
| `desktop/` | Tauri and SvelteKit desktop app |
| `python/` | Experimental Python bindings, not yet on PyPI. See [its README](python/README.md). |

See [CONTRIBUTING.md](CONTRIBUTING.md) to work on the project, and [SECURITY.md](SECURITY.md) to report a vulnerability.

## Releasing

Run `just bump 0.2.0`, commit, and merge to `master`. GitHub Actions runs CI, builds every installer above, and publishes the release once everything has passed. Other commits don't build a release.

## License

[MIT](LICENSE)
