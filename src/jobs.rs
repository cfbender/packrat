//! Background work: the first-boot catalog import, periodic price refreshes,
//! and refreshes requested over the API. One worker runs jobs one at a time
//! and swaps in a fresh in-memory catalog after each.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::api::SharedState;
use crate::catalog::Catalog;
use crate::{db, mtgjson, prices};

#[derive(Debug, Clone, Copy)]
pub enum Request {
    /// Import the catalog (when forced, even if the published version is
    /// already imported), then refresh prices.
    Catalog {
        force: bool,
    },
    Prices,
}

pub fn spawn(state: SharedState, mut requests: mpsc::Receiver<Request>) {
    tokio::spawn(async move {
        // Boot: import when the catalog is empty or MTGJSON published a new
        // build, then refresh prices when they are older than the interval.
        match boot_needs(&state).await {
            Ok((catalog, prices)) => {
                if catalog {
                    run(&state, Request::Catalog { force: false }).await;
                } else if prices {
                    run(&state, Request::Prices).await;
                }
            }
            Err(error) => tracing::error!(%error, "boot check failed"),
        }

        let interval = Duration::from_secs(state.config.price_refresh_hours.max(1) * 3600);
        loop {
            tokio::select! {
                request = requests.recv() => match request {
                    Some(request) => run(&state, request).await,
                    None => break,
                },
                _ = tokio::time::sleep(interval) => run(&state, Request::Catalog { force: false }).await,
            }
        }
    });
}

async fn boot_needs(state: &SharedState) -> anyhow::Result<(bool, bool)> {
    let catalog_empty = mtgjson::is_catalog_empty(&state.pool).await?;
    let updated = db::meta_get(&state.pool, "prices_updated_at")
        .await?
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let now = db::now().parse::<u64>().unwrap_or(0);
    let stale = now.saturating_sub(updated) >= state.config.price_refresh_hours.max(1) * 3600;
    Ok((catalog_empty || stale, stale))
}

async fn run(state: &SharedState, request: Request) {
    let progress: mtgjson::Progress = {
        let state = Arc::clone(state);
        Arc::new(move |message: String| {
            tracing::info!("{message}");
            state.set_status(|s| s.message = message.clone());
        })
    };
    state.set_status(|s| {
        s.busy = true;
        s.error = None;
    });
    let result = match request {
        Request::Catalog { force } => {
            let imported = mtgjson::import(
                &state.pool,
                &state.client,
                &state.config.mtgjson_base_url,
                &state.config.data_dir,
                force,
                progress.clone(),
            )
            .await;
            match imported {
                Ok(changed) => {
                    if changed {
                        reload(state).await;
                    }
                    prices::refresh_all(
                        &state.pool,
                        &state.client,
                        &state.config.mtgjson_base_url,
                        progress,
                    )
                    .await
                }
                Err(error) => Err(error),
            }
        }
        Request::Prices => {
            prices::refresh_all(
                &state.pool,
                &state.client,
                &state.config.mtgjson_base_url,
                progress,
            )
            .await
        }
    };
    reload(state).await;
    state.set_status(|s| {
        s.busy = false;
        if let Err(error) = &result {
            tracing::error!(%error, "job failed");
            s.error = Some(format!("{error:#}"));
            s.message = "Refresh failed".to_owned();
        } else {
            s.message = "Up to date".to_owned();
        }
    });
}

async fn reload(state: &SharedState) {
    match Catalog::load(&state.pool).await {
        Ok(catalog) => {
            tracing::info!(
                boosters = catalog.boosters.len(),
                products = catalog.products.len(),
                "catalog loaded"
            );
            state.set_catalog(catalog);
        }
        Err(error) => tracing::error!(%error, "catalog reload failed"),
    }
}
