# cinegraph

`cinegraph` is a local Rust workspace for downloading, idempotently caching, and importing film datasets into an embedded SQLite warehouse. The current implementation is IMDb-first and includes a Tantivy-based full-text search layer on top of the SQLite warehouse.

Progress is tracked in [docs/roadmap.md](/win/linux/Code/rust/cinegraph/docs/roadmap.md). Milestones 1 and 2 are implemented; later milestones remain planned.

## Current Architecture

The workspace is split by responsibility:

- `cinegraph`: CLI entry point and command dispatch
- `cinegraph-core`: shared config, paths, errors, and logging
- `cinegraph-fetch`: HTTP fetch, conditional requests, and content-addressed caching
- `cinegraph-db`: SQLite schema and query helpers
- `cinegraph-imdb`: IMDb gzip TSV parsing and import logic
- `cinegraph-search`: Tantivy index build and SQLite-backed search hydration
- `cinegraph-graph`, `cinegraph-export`: reserved for later milestones

The canonical local store is SQLite. Downloaded dataset artifacts are stored on disk by SHA-256 hash, import runs are tracked to avoid re-importing the same artifact with the same importer version, and the search index is rebuilt from SQLite instead of the raw TSV artifacts.

## Quickstart

Use the default config:

```bash
cargo run -- init
cargo run -- fetch imdb
cargo run -- import imdb
cargo run -- index search
cargo run -- stats
cargo run -- lookup title "Carmencita"
cargo run -- lookup person "Fred Astaire"
cargo run -- search title "samurai kurosawa"
cargo run -- search person "kurosawa ikiru"
```

The default config file is [config/cinegraph.example.toml](/win/linux/Code/rust/cinegraph/config/cinegraph.example.toml). Runtime data is written under `.data/` unless you point `--config` or `CINEGRAPH_CONFIG` at another config file. Tantivy indexes are written under `.data/index/tantivy/`.

## Repo Layout

```text
cinegraph/
├── config/
├── crates/
├── docs/
├── tmp/
└── Cargo.toml
```

- `config/`: example runtime configuration
- `crates/`: workspace crates
- `docs/`: roadmap and future project documentation
- `tmp/`: background notes and planning material

## Docs

- [Roadmap](/win/linux/Code/rust/cinegraph/docs/roadmap.md)
