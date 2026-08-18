//! Shared application state.

use std::sync::Arc;

use anyhow::Result;

use crate::arxiv::ArxivClient;
use crate::cache::Cache;
use crate::config::Config;
use crate::openai::OpenAiClient;

pub struct AppState {
    pub config: Config,
    pub arxiv: ArxivClient,
    pub openai: OpenAiClient,
    pub cache: Cache,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        let arxiv = ArxivClient::new(&config)?;
        let openai = OpenAiClient::new(&config)?;
        let cache = Cache::new(&config.cache_dir).await?;
        Ok(Self { config, arxiv, openai, cache })
    }
}

pub type SharedState = Arc<AppState>;
