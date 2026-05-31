# cinegraph

`cinegraph` is a local Rust workspace for downloading, idempotently caching, and importing film datasets into an embedded SQLite warehouse. The current implementation supports IMDb as the canonical bulk dataset, plus TMDb as API-hydrated enrichment layered onto the same SQLite warehouse, with Tantivy-based full-text search and an Oxigraph-based RDF/SPARQL graph projection built from SQLite.

Progress is tracked in [docs/roadmap.md](/win/linux/Code/rust/cinegraph/docs/roadmap.md). Milestones 1 through 4 are implemented; milestone 5 remains planned.

## Current Architecture

The workspace is split by responsibility:

- `cinegraph`: CLI entry point and command dispatch
- `cinegraph-core`: shared config, paths, errors, and logging
- `cinegraph-fetch`: HTTP fetch, conditional requests, and content-addressed caching
- `cinegraph-db`: SQLite schema and query helpers
- `cinegraph-imdb`: IMDb gzip TSV parsing and import logic
- `cinegraph-tmdb`: TMDb export ingestion, rate-limited API hydration, and SQLite enrichment writes
- `cinegraph-search`: Tantivy index build and SQLite-backed search hydration
- `cinegraph-graph`: Oxigraph store rebuild, SPARQL execution, neighbors, and collaboration queries
- `cinegraph-export`: reserved for later milestones

The canonical local store is SQLite. Downloaded dataset artifacts are stored on disk by SHA-256 hash, import runs are tracked to avoid re-importing the same artifact with the same importer version, TMDb movie IDs are fetched from daily exports and then hydrated through the TMDb API with request throttling, and both the Tantivy search index and the Oxigraph graph store are rebuilt from SQLite instead of raw source artifacts.

## Quickstart

Use the default config:

```bash
cargo run -- init
cargo run -- fetch imdb
cargo run -- import imdb
cargo run -- fetch tmdb
cargo run -- import tmdb
cargo run -- index search
cargo run -- index graph
cargo run -- stats
cargo run -- lookup title "Carmencita"
cargo run -- lookup person "Fred Astaire"
cargo run -- search title "samurai kurosawa"
cargo run -- search person "kurosawa ikiru"
cargo run -- graph neighbors nm0000001
cargo run -- graph collaborations nm0000001
cargo run -- graph query queries/director_filmography.rq
```

TMDb import requires a configured `sources.tmdb.api_read_access_token` in your runtime config. The default config file is [config/cinegraph.example.toml](/win/linux/Code/rust/cinegraph/config/cinegraph.example.toml). Runtime data is written under `.data/` unless you point `--config` or `CINEGRAPH_CONFIG` at another config file. Tantivy indexes are written under `.data/index/tantivy/`, and the Oxigraph store is written under `.data/graph/oxigraph/`.

## Repo Layout

```text
cinegraph/
├── config/
├── crates/
├── docs/
├── queries/
├── tmp/
└── Cargo.toml
```

- `config/`: example runtime configuration
- `crates/`: workspace crates
- `docs/`: roadmap and future project documentation
- `queries/`: sample SPARQL queries for the graph CLI
- `tmp/`: background notes and planning material

## Docs

- [Roadmap](/win/linux/Code/rust/cinegraph/docs/roadmap.md)
