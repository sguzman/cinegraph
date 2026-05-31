mod args;

use anyhow::Context;
use cinegraph_core::{AppConfig, AppPaths, logging::init_logging};
use cinegraph_db::{Database, queries};
use cinegraph_fetch::Fetcher;
use cinegraph_imdb::import::ImdbImporter;
use cinegraph_search::SearchService;
use clap::Parser;
use tracing::info;

use crate::args::{
    Cli, Commands, FetchCommands, ImportCommands, IndexCommands, LookupCommands, SearchCommands,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = AppConfig::load(&cli.config).or_else(|_| AppConfig::from_example())?;
    config.validate()?;
    let paths = AppPaths::from_config(&config);
    paths.ensure_dirs(&config)?;
    init_logging(&config.logging)?;

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
                    "stats": stats
                }))?
            );
        }
        Commands::Init => unreachable!(),
    }
    Ok(())
}
