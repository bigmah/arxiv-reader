//! Runtime configuration, all of it from the environment (or a `.env` file).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// Hard ceiling on papers per page. Every paper on a listing costs one LLM call,
/// so this stays small on purpose.
pub const MAX_PAGE_SIZE: usize = 5;

/// Which model to use for one job, and how hard to let it think.
///
/// The three jobs differ enough to deserve separate settings: listing blurbs are
/// throwaway one-liners from an abstract, while summarizing a whole paper is
/// worth a stronger model and real reasoning budget.
#[derive(Debug, Clone)]
pub struct ModelChoice {
    pub model: String,
    /// Sent as `reasoning_effort`. `None` omits the field, for models that
    /// reject it.
    pub effort: Option<String>,
    /// `max_completion_tokens`. Reasoning tokens are billed against this too, so
    /// a high effort needs a budget with room for the answer underneath it.
    pub max_tokens: u32,
}

impl ModelChoice {
    /// Identifies this setup in cache keys, so changing the model or the effort
    /// regenerates rather than serving text produced by a different one.
    pub fn cache_tag(&self) -> String {
        match &self.effort {
            Some(effort) => format!("{}@{effort}", self.model),
            None => self.model.clone(),
        }
    }

    fn from_env(prefix: &str, model: &str, effort: Option<&str>, max_tokens: u32) -> Self {
        let effort = env_opt(&format!("{prefix}_REASONING_EFFORT"))
            .or_else(|| effort.map(str::to_string))
            // An explicit "none"/"off" turns the parameter off entirely.
            .filter(|e| !matches!(e.to_ascii_lowercase().as_str(), "none" | "off"));

        Self {
            model: env_or(&format!("{prefix}_MODEL"), model),
            effort,
            max_tokens: env_opt(&format!("{prefix}_MAX_TOKENS"))
                .and_then(|v| v.parse().ok())
                .unwrap_or(max_tokens),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// Absent means the app runs fine, just without any AI features.
    pub openai_api_key: Option<String>,
    pub openai_base_url: String,
    /// One-paragraph blurbs for papers in a listing.
    pub listing_model: ModelChoice,
    /// The full read-the-paper summary on a paper page.
    pub summary_model: ModelChoice,
    /// Chat turns about a paper.
    pub chat_model: ModelChoice,
    pub page_size: usize,
    pub cache_dir: PathBuf,
    /// The Atom API endpoint. Configurable so it can be pointed at a mirror —
    /// or at a stub, which is how the request-count tests work.
    pub arxiv_api_url: String,
    /// Base URL for PDF downloads; `/{id}` is appended.
    pub arxiv_pdf_base: String,
    /// Minimum gap between arXiv requests. Their API terms ask for three
    /// seconds; raise it if you still see 429s.
    pub arxiv_min_interval: Duration,
    /// Extra attempts after arXiv answers 429 or 5xx.
    pub arxiv_max_retries: u32,
    /// Per-request timeout for arXiv metadata queries.
    pub arxiv_timeout: Duration,
    /// How long to stop asking after a 429, when arXiv gives no `Retry-After`.
    pub arxiv_cooldown: Duration,
    /// How long a fetched listing page stays fresh, so reloads and paging back
    /// don't re-query arXiv.
    pub listing_ttl: Duration,
    /// How much extracted PDF text to hand the model when summarizing.
    pub summary_context_chars: usize,
    /// How much to hand it per chat turn (re-sent every message, so it is smaller).
    pub chat_context_chars: usize,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind = env_or("BIND_ADDR", "127.0.0.1:3000").parse()?;

        let page_size = env_or("PAGE_SIZE", "5").parse::<usize>().unwrap_or(MAX_PAGE_SIZE);
        if page_size > MAX_PAGE_SIZE {
            tracing::warn!(
                "PAGE_SIZE={page_size} exceeds the {MAX_PAGE_SIZE}-paper cap; clamping to \
                 {MAX_PAGE_SIZE} to keep API usage in check"
            );
        }

        Ok(Self {
            bind,
            openai_api_key: std::env::var("OPENAI_API_KEY").ok().filter(|k| !k.trim().is_empty()),
            openai_base_url: env_or("OPENAI_BASE_URL", "https://api.openai.com/v1")
                .trim_end_matches('/')
                .to_string(),
            listing_model: ModelChoice::from_env("OPENAI_LISTING", "gpt-5.6-terra", None, 600),
            summary_model: ModelChoice::from_env(
                "OPENAI_SUMMARY",
                "gpt-5.6-sol",
                Some("xhigh"),
                16_000,
            ),
            chat_model: ModelChoice::from_env("OPENAI_CHAT", "gpt-5.6-sol", Some("high"), 8_000),
            page_size: page_size.clamp(1, MAX_PAGE_SIZE),
            cache_dir: PathBuf::from(env_or("CACHE_DIR", ".cache")),
            arxiv_api_url: env_or("ARXIV_API_URL", "https://export.arxiv.org/api/query")
                .to_string(),
            arxiv_pdf_base: env_or("ARXIV_PDF_BASE", "https://arxiv.org/pdf")
                .trim_end_matches('/')
                .to_string(),
            arxiv_min_interval: Duration::from_millis(
                env_or("ARXIV_MIN_INTERVAL_MS", "3000").parse().unwrap_or(3_000),
            ),
            arxiv_max_retries: env_or("ARXIV_MAX_RETRIES", "1").parse().unwrap_or(1),
            arxiv_timeout: Duration::from_secs(
                env_or("ARXIV_TIMEOUT_SECS", "20").parse().unwrap_or(20),
            ),
            arxiv_cooldown: Duration::from_secs(
                env_or("ARXIV_COOLDOWN_SECS", "60").parse().unwrap_or(60),
            ),
            listing_ttl: Duration::from_secs(
                env_or("LISTING_TTL_SECS", "300").parse().unwrap_or(300),
            ),
            summary_context_chars: env_or("SUMMARY_CONTEXT_CHARS", "60000").parse().unwrap_or(60_000),
            chat_context_chars: env_or("CHAT_CONTEXT_CHARS", "24000").parse().unwrap_or(24_000),
        })
    }
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn env_or(key: &str, default: &str) -> String {
    env_opt(key).unwrap_or_else(|| default.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_tag_distinguishes_model_and_effort() {
        let base = ModelChoice { model: "m".into(), effort: None, max_tokens: 100 };
        let thinking = ModelChoice { effort: Some("xhigh".into()), ..base.clone() };
        let harder = ModelChoice { effort: Some("high".into()), ..base.clone() };

        assert_eq!(base.cache_tag(), "m");
        assert_eq!(thinking.cache_tag(), "m@xhigh");
        assert_ne!(thinking.cache_tag(), harder.cache_tag());
    }
}
