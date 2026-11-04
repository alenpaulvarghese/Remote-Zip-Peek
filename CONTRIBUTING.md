# Contributing

## Setup

You need Rust, Node.js 22 (see `.nvmrc`), the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform, and optionally [`just`](https://github.com/casey/just).

```bash
cd desktop && npm ci
```

## Checks

`just ci` runs everything below:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p lazy-zip-core
cd desktop && npm run check && npm test
```

`cargo test -p desktop` needs the frontend built first with `npm run build`.

## Tests

- `core/tests/support/` has a local HTTP server that misbehaves on purpose. Add a `Behavior` to it when fixing how the reader handles a server.
- Tests count requests and bytes instead of measuring time, so they stay reliable.
- `core/tests/data/make_methods_zip.py` regenerates the compression test archive.

## Dependencies

The macOS build must only link system libraries, or the app won't launch on other machines. CI checks this when building a release. If a new dependency wraps a C library, make sure it links statically, and check with `otool -L` on the built app.

## Releases

See [Releasing](README.md#releasing). The version lives only in the root `Cargo.toml`.
