# Bovista guide

Source for the Bovista user guide, built with [mdBook](https://rust-lang.github.io/mdBook/).
Chapters live in [`src/`](./src); the table of contents is [`src/SUMMARY.md`](./src/SUMMARY.md).

## Build

```bash
cargo install mdbook   # once

mdbook serve           # live-reload at http://localhost:3000
mdbook build           # or ./build.sh — writes to book/ (gitignored)
```
