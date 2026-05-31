# cinegraph

`cinegraph` is a local Rust workspace for downloading, idempotently caching, and importing film datasets into an embedded SQLite warehouse. The current implementation is IMDb-first and focuses on reproducible fetch/import behavior, structured logging, and a CLI that can grow into search and graph workflows.

Progress is tracked in [docs/roadmap.md](/win/linux/Code/rust/cinegraph/docs/roadmap.md). Milestone 1 is implemented; later milestones remain planned.

## Current Architecture

The workspace is split by responsibility:

- `cinegraph`: CLI entry point and command dispatch
- `cinegraph-core`: shared config, paths, errors, and logging
- `cinegraph-fetch`: HTTP fetch, conditional requests, and content-addressed caching
- `cinegraph-db`: SQLite schema and query helpers
- `cinegraph-imdb`: IMDb gzip TSV parsing and import logic
- `cinegraph-search`, `cinegraph-graph`, `cinegraph-export`: reserved for later milestones

The canonical local store is SQLite. Downloaded dataset artifacts are stored on disk by SHA-256 hash, and import runs are tracked to avoid re-importing the same artifact with the same importer version.

## Quickstart

Use the default config:

```bash
cargo run -- init
cargo run -- fetch imdb
cargo run -- import imdb
cargo run -- stats
cargo run -- lookup title "Carmencita"
cargo run -- lookup person "Fred Astaire"
```

The default config file is [config/cinegraph.example.toml](/win/linux/Code/rust/cinegraph/config/cinegraph.example.toml). Runtime data is written under `.data/` unless you point `--config` or `CINEGRAPH_CONFIG` at another config file.

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
