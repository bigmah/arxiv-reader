//! arxiv-reader — browse arXiv with AI summaries bolted on.

mod arxiv;
mod cache;
mod config;
mod error;
mod markdown;
mod openai;
mod papers;
mod routes;
mod state;
mod taxonomy;
mod templates;

use std::sync::Arc;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("arxiv_reader=info,tower_http=warn,warn")),
        )
        .init();

    let config = Config::from_env().context("reading configuration")?;
    let bind = config.bind;

    if config.openai_api_key.is_none() {
        tracing::warn!(
            "OPENAI_API_KEY is not set — browsing works, but summaries and chat are disabled"
        );
    } else {
        tracing::info!(
            listings = %config.listing_model.cache_tag(),
            summaries = %config.summary_model.cache_tag(),
            chat = %config.chat_model.cache_tag(),
            "AI features enabled"
        );
    }
    tracing::info!(
        page_size = config.page_size,
        cache = %config.cache_dir.display(),
        "starting arxiv-reader"
    );

    let state = Arc::new(AppState::new(config).await?);
    let app = routes::router(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!("listening on http://{bind}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}
