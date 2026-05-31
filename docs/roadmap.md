# Cinegraph Roadmap

Last reviewed: 2026-05-30

## Status

- [x] Milestone 1: IMDb-only SQLite warehouse
- [x] Milestone 2: Full-text search
- [x] Milestone 3: Graph projection
- [ ] Milestone 4: TMDb enrichment
- [ ] Milestone 5: Wikidata enrichment

## Milestone 1: IMDb-only SQLite warehouse

- [x] Restructure the repo into a Rust workspace with dedicated crates for CLI, core, fetch, DB, and IMDb import
- [x] Expose the `cinegraph` CLI binary
- [x] Implement `cinegraph init`
- [x] Implement `cinegraph fetch imdb`
- [x] Implement `cinegraph import imdb`
- [x] Implement `cinegraph stats`
- [x] Implement `cinegraph lookup title <query>`
- [x] Implement `cinegraph lookup person <query>`
- [x] Implement `cinegraph doctor`
- [x] Create the `.data/` directory layout for raw files, blobs, DB, logs, and reserved index/graph directories
- [x] Use SQLite as the canonical embedded warehouse
- [x] Create metadata tables for `datasets`, `download_artifacts`, and `import_runs`
- [x] Create film tables for `titles`, `people`, `title_ratings`, `title_akas`, `title_crew`, `credits`, and `episode_edges`
- [x] Fetch IMDb datasets with conditional request support using stored `ETag` / `Last-Modified`
- [x] Store canonical downloaded artifacts by SHA-256 content hash
- [x] Avoid duplicate artifact records for the same dataset hash
- [x] Avoid re-importing the same artifact with the same importer version
- [x] Parse gzip-compressed IMDb TSV files in a streaming importer path
- [x] Normalize IMDb `\N` values consistently
- [x] Import `name.basics.tsv.gz`
- [x] Import `title.basics.tsv.gz`
- [x] Import `title.ratings.tsv.gz`
- [x] Import `title.akas.tsv.gz`
- [x] Import `title.crew.tsv.gz`
- [x] Import `title.principals.tsv.gz`
- [x] Import `title.episode.tsv.gz`
- [x] Expand `title.crew` directors and writers into `credits`
- [x] Import IMDb datasets in foreign-key-safe dependency order
- [x] Add tests for cache path derivation, directory bootstrap, idempotent fetch, importer idempotence, lookup hydration, and dataset import ordering

## Milestone 2: Full-text Search

- [x] Add the `cinegraph-search` implementation crate
- [x] Implement `cinegraph index search`
- [x] Implement `cinegraph search title <query>`
- [x] Implement `cinegraph search person <query>`
- [x] Build Tantivy indexes from SQLite instead of raw TSVs
- [x] Hydrate final search results from SQLite after ranking
- [x] Add search index tests and CLI coverage

## Milestone 3: Graph Projection

- [x] Add the `cinegraph-graph` implementation crate
- [x] Implement `cinegraph index graph`
- [x] Implement `cinegraph graph query <sparql-file>`
- [x] Implement `cinegraph graph neighbors <entity-id>`
- [x] Implement `cinegraph graph collaborations <person-id>`
- [x] Project RDF triples from SQLite instead of raw TSVs
- [x] Use Oxigraph as a reproducible derived store, not the source of truth
- [x] Add graph projection and SPARQL tests

## Milestone 4: TMDb Enrichment

- [ ] Add TMDb fetch and import support
- [ ] Model TMDb as API-hydrated enrichment rather than bulk truth
- [ ] Add rate-limited hydration of movie and credit data
- [ ] Add SQLite crosswalk/enrichment coverage and tests

## Milestone 5: Wikidata Enrichment

- [ ] Add Wikidata import support
- [ ] Add title/person crosswalk enrichment into SQLite
- [ ] Extend graph projection with Wikidata-linked entities and facts
- [ ] Add tests for imported links and projected graph data

## Progress Rule

- [x] Roadmap is maintained as Markdown task lists in this file
- [x] Checked items should only represent code that already exists in the repo
- [x] Update roadmap checkboxes in the same change that completes the underlying work
