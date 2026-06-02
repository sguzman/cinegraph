# Wikidata Import

`cinegraph` imports Wikidata from a local dump file that you provide. It does not fetch or mirror the dump itself.

## Config

Set the Wikidata source in your runtime config:

```toml
[sources.wikidata]
enabled = true
dump_path = "/absolute/path/to/latest-all.json.gz"
language = "en"
```

`dump_path` may point to a plain JSON dump or a `.gz`-compressed dump. The importer reads the file in place, hashes it for idempotent tracking, and records that hash in SQLite metadata.

## Command

Run:

```bash
cargo run -- import wikidata
```

The importer will:

- hash and register the local dump as a metadata artifact without copying or downloading it
- rebuild Wikidata entity, link, and claim tables from that dump
- create title links when a Wikidata item exposes an IMDb `tt...` ID that matches a local title
- create person links when a Wikidata item exposes an IMDb `nm...` ID that matches a local person
- import selected facts for linked titles and people into SQLite for graph projection

## Imported Facts

The current import keeps a focused subset of Wikidata properties for graph enrichment:

- `P31` instance of
- `P57` director
- `P161` cast member
- `P136` genre
- `P364` original language of work
- `P495` country of origin
- `P577` publication or release date
- `P569` date of birth
- `P570` date of death

These facts are projected into the Oxigraph store when you run:

```bash
cargo run -- index graph
```

After that, `graph query` continues to read from the Oxigraph store directly. For interactive CLI use, `graph neighbors` and `graph collaborations` now use SQLite-backed fast paths that still surface linked Wikidata entities, while `graph neighbors-heavy` and `graph collaborations-heavy` keep the live Oxigraph-backed path. If you want a live full-store aggregate refresh instead of the cached summary, run `graph stats-heavy`.
