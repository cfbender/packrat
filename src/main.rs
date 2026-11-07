//! packrat: a self-hosted Magic: The Gathering booster pack and sealed
//! product simulator with expected-value statistics.

mod api;
mod catalog;
mod config;
mod db;
mod http;
mod jobs;
mod mtgjson;
mod prices;
mod sim;
mod stats;

use std::sync::{Arc, Mutex, RwLock};

use axum::Router;
use clap::{Parser, Subcommand};
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};

// The release image is a static musl binary; musl's allocator is several
// times slower than glibc's on the simulation's allocation-heavy hot paths.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use crate::api::{AppState, JobStatus};
use crate::catalog::Catalog;
use crate::config::Config;

#[derive(Parser)]
#[command(name = "packrat", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the web server (default).
    Serve,
    /// Import the MTGJSON catalog and refresh prices, then exit.
    Import {
        /// Re-import even when the published MTGJSON build is already imported.
        #[arg(long)]
        force: bool,
    },
    /// Refresh prices only, then exit.
    Prices,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "packrat=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let config = Config::from_env();
    let pool = db::open(&config.database_path()).await?;
    let client = http::client();

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve(config, pool, client).await,
        Command::Import { force } => {
            let progress: mtgjson::Progress = Arc::new(|m: String| tracing::info!("{m}"));
            mtgjson::import(
                &pool,
                &client,
                &config.mtgjson_base_url,
                &config.data_dir,
                force,
                progress.clone(),
            )
            .await?;
            prices::refresh_all(&pool, &client, &config.mtgjson_base_url, progress).await
        }
        Command::Prices => {
            let progress: mtgjson::Progress = Arc::new(|m: String| tracing::info!("{m}"));
            prices::refresh_all(&pool, &client, &config.mtgjson_base_url, progress).await
        }
    }
}

async fn serve(
    config: Config,
    pool: sqlx::SqlitePool,
    client: reqwest::Client,
) -> anyhow::Result<()> {
    let catalog = Catalog::load(&pool).await?;
    tracing::info!(
        boosters = catalog.boosters.len(),
        products = catalog.products.len(),
        "catalog loaded"
    );
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let state = Arc::new(AppState {
        pool,
        catalog: RwLock::new(Arc::new(catalog)),
        status: Mutex::new(JobStatus {
            busy: false,
            message: "Idle".to_owned(),
            error: None,
        }),
        config: config.clone(),
        client,
        refresh: tx,
    });
    jobs::spawn(Arc::clone(&state), rx);

    let index = config.static_dir.join("index.html");
    let app = Router::new()
        .nest("/api", api::router())
        .fallback_service(ServeDir::new(&config.static_dir).fallback(ServeFile::new(index)))
        .layer(CompressionLayer::new())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
    tracing::info!("listening on http://0.0.0.0:{}", config.port);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
