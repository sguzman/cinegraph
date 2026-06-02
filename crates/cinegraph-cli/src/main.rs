mod args;

use anyhow::Context;
use cinegraph_core::{AppConfig, AppPaths, logging::init_logging};
use cinegraph_db::{Database, queries};
use cinegraph_fetch::Fetcher;
use cinegraph_graph::GraphService;
use cinegraph_imdb::import::ImdbImporter;
use cinegraph_search::SearchService;
use cinegraph_tmdb::TmdbImporter;
use cinegraph_wikidata::WikidataImporter;
use clap::Parser;
use tracing::info;

use crate::args::{
    Cli, Commands, FetchCommands, GraphCommands, ImportCommands, IndexCommands, LookupCommands,
    SearchCommands,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = AppConfig::load(&cli.config).or_else(|_| AppConfig::from_example())?;
    config.validate()?;
    let paths = AppPaths::from_config(&config);
    paths.ensure_dirs(&config)?;
    init_logging(&config.logging)?;
    info!(
        config_path = %cli.config.display(),
        command = %command_name(&cli.command),
        data_root = %paths.root.display(),
        "cinegraph command starting"
    );

    match cli.command {
        Commands::Init => {
            let db = Database::connect(&config, &paths).await?;
            db.migrate().await?;
            info!("cinegraph initialized");
            println!("initialized {}", paths.root.display());
        }
        command => {
            let db = Database::connect(&config, &paths).await?;
            db.migrate().await?;
            run_command(command, &config, &paths, &db).await?;
        }
    }

    Ok(())
}

async fn run_command(
    command: Commands,
    config: &AppConfig,
    paths: &AppPaths,
    db: &Database,
) -> anyhow::Result<()> {
    match command {
        Commands::Index { command } => match command {
            IndexCommands::Search => {
                let service = SearchService::open(paths)?;
                let stats = service.rebuild(db).await?;
                println!("{}", serde_json::to_string_pretty(&stats)?);
            }
            IndexCommands::Graph => {
                GraphService::reset_store(paths)?;
                let service = GraphService::open(config, paths)?;
                let stats = service.rebuild(db).await?;
                println!("{}", serde_json::to_string_pretty(&stats)?);
            }
        },
        Commands::Fetch { command } => match command {
            FetchCommands::Imdb => {
                let fetcher = Fetcher::new(config)?;
                for outcome in fetcher.fetch_imdb(db, config, paths).await? {
                    println!(
                        "{}\t{}\t{}",
                        outcome.dataset_name,
                        if outcome.changed {
                            "fetched"
                        } else {
                            "not-modified"
                        },
                        outcome.artifact.sha256
                    );
                }
            }
            FetchCommands::Tmdb => {
                let fetcher = Fetcher::new(config)?;
                for outcome in fetcher.fetch_tmdb(db, config, paths).await? {
                    println!(
                        "{}\t{}\t{}",
                        outcome.dataset_name,
                        if outcome.changed {
                            "fetched"
                        } else {
                            "not-modified"
                        },
                        outcome.artifact.sha256
                    );
                }
            }
        },
        Commands::Import { command } => match command {
            ImportCommands::Imdb => {
                let importer = ImdbImporter::new(db);
                for (dataset, stats) in importer.import_latest().await? {
                    println!(
                        "{}\tseen={}\tinserted={}\tskipped={}",
                        dataset, stats.rows_seen, stats.rows_inserted, stats.rows_skipped
                    );
                }
            }
            ImportCommands::Tmdb => {
                let importer = TmdbImporter::new(db, config)?;
                let stats = importer.import_latest(config).await?;
                println!("{}", serde_json::to_string_pretty(&stats)?);
            }
            ImportCommands::Wikidata => {
                let importer = WikidataImporter::new(db);
                let stats = importer.import_dump(config).await?;
                println!("{}", serde_json::to_string_pretty(&stats)?);
            }
        },
        Commands::Stats => {
            let stats = queries::stats(db.pool()).await?;
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        Commands::Search { command } => match command {
            SearchCommands::Title { query, limit } => {
                let service = SearchService::open(paths)?;
                let rows = service.search_titles(db, &query, limit).await?;
                println!("{}", serde_json::to_string_pretty(&rows)?);
            }
            SearchCommands::Person { query, limit } => {
                let service = SearchService::open(paths)?;
                let rows = service.search_people(db, &query, limit).await?;
                println!("{}", serde_json::to_string_pretty(&rows)?);
            }
        },
        Commands::Graph { command } => match command {
            GraphCommands::Query { sparql_file } => {
                let service = GraphService::open(config, paths)?;
                let output = service.query_file(&sparql_file)?;
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            GraphCommands::Neighbors { entity_id } => {
                let output = GraphService::neighbors_fast(db, &entity_id).await?;
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            GraphCommands::NeighborsHeavy { entity_id } => {
                let service = GraphService::open(config, paths)?;
                let output = service.neighbors_heavy(&entity_id)?;
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            GraphCommands::Collaborations { person_id } => {
                let output = GraphService::collaborations_fast(db, &person_id).await?;
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            GraphCommands::CollaborationsHeavy { person_id } => {
                let service = GraphService::open(config, paths)?;
                let output = service.collaborations_heavy(&person_id)?;
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            GraphCommands::Stats => {
                let service = GraphService::open(config, paths)?;
                let output = service.stats()?;
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            GraphCommands::StatsHeavy => {
                let service = GraphService::open(config, paths)?;
                let output = service.stats_heavy()?;
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
        },
        Commands::Lookup { command } => match command {
            LookupCommands::Title { query } => {
                let rows = queries::lookup_title(db.pool(), &query).await?;
                println!("{}", serde_json::to_string_pretty(&rows)?);
            }
            LookupCommands::Person { query } => {
                let rows = queries::lookup_person(db.pool(), &query).await?;
                println!("{}", serde_json::to_string_pretty(&rows)?);
            }
        },
        Commands::Doctor => {
            let stats = queries::stats(db.pool())
                .await
                .context("stats unavailable")?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "data_root": paths.root,
                    "sqlite_path": paths.sqlite_path(config),
                    "search_index_path": paths.search_index_dir(),
                    "graph_store_path": paths.graph_store_dir(),
                    "stats": stats
                }))?
            );
        }
        Commands::Init => unreachable!(),
    }
    Ok(())
}

fn command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Init => "init",
        Commands::Index { command } => match command {
            IndexCommands::Search => "index search",
            IndexCommands::Graph => "index graph",
        },
        Commands::Fetch { command } => match command {
            FetchCommands::Imdb => "fetch imdb",
            FetchCommands::Tmdb => "fetch tmdb",
        },
        Commands::Import { command } => match command {
            ImportCommands::Imdb => "import imdb",
            ImportCommands::Tmdb => "import tmdb",
            ImportCommands::Wikidata => "import wikidata",
        },
        Commands::Stats => "stats",
        Commands::Search { command } => match command {
            SearchCommands::Title { .. } => "search title",
            SearchCommands::Person { .. } => "search person",
        },
        Commands::Graph { command } => match command {
            GraphCommands::Query { .. } => "graph query",
            GraphCommands::Neighbors { .. } => "graph neighbors",
            GraphCommands::NeighborsHeavy { .. } => "graph neighbors-heavy",
            GraphCommands::Collaborations { .. } => "graph collaborations",
            GraphCommands::CollaborationsHeavy { .. } => "graph collaborations-heavy",
            GraphCommands::Stats => "graph stats",
            GraphCommands::StatsHeavy => "graph stats-heavy",
        },
        Commands::Lookup { command } => match command {
            LookupCommands::Title { .. } => "lookup title",
            LookupCommands::Person { .. } => "lookup person",
        },
        Commands::Doctor => "doctor",
    }
}
