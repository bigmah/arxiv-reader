//! The service layer: arXiv metadata + PDF text + OpenAI summaries, all cached.

use anyhow::{Context, Result};
use futures::future;

use crate::arxiv::{Paper, SearchPage};
use crate::openai::PROMPT_VERSION;
use crate::state::AppState;

/// arXiv's API gets slow at deep offsets, and nobody is paging that far by hand.
pub const MAX_LISTED_PAPERS: usize = 10_000;

/// A paper as shown in a listing, with whatever blurb we could produce for it.
#[derive(Debug, Clone)]
pub struct ListedPaper {
    pub paper: Paper,
    pub blurb: String,
    /// False when the blurb is just a trimmed abstract (no API key, or the call failed).
    pub ai_generated: bool,
}

/// Everything a listing page needs to draw its pager.
#[derive(Debug, Clone)]
pub struct Pagination {
    pub page: usize,
    pub total: usize,
    pub total_pages: usize,
    pub prev_page: Option<usize>,
    pub next_page: Option<usize>,
    /// 1-based index of the first result on this page.
    pub first_result: usize,
    pub last_result: usize,
}

/// Fetch one page of a category and summarize every paper on it concurrently.
pub async fn list_category(
    state: &AppState,
    category: &str,
    page: usize,
) -> Result<(Vec<ListedPaper>, Pagination)> {
    let per_page = state.config.page_size;
    let page = page.max(1);
    let start = (page - 1) * per_page;

    // A hand-typed page number past the paging limit becomes an empty page
    // rather than a huge offset for arXiv to reject.
    let results = if start >= MAX_LISTED_PAPERS {
        SearchPage { papers: Vec::new(), total: 0 }
    } else {
        state.arxiv.list_category(category, start, per_page).await?
    };

    // One concurrent summary per paper. `page_size` is capped at 5, so this is
    // at most five in-flight OpenAI calls.
    let listed = future::join_all(results.papers.iter().map(|paper| blurb(state, paper))).await;

    let total = results.total.min(MAX_LISTED_PAPERS);
    let total_pages = total.div_ceil(per_page).max(1);

    let pagination = Pagination {
        page,
        total: results.total,
        total_pages,
        prev_page: (page > 1).then(|| page - 1),
        next_page: (page < total_pages && !listed.is_empty()).then(|| page + 1),
        first_result: if listed.is_empty() { 0 } else { start + 1 },
        last_result: start + listed.len(),
    };

    Ok((listed, pagination))
}

/// The short summary for one paper in a listing, falling back to the abstract.
async fn blurb(state: &AppState, paper: &Paper) -> ListedPaper {
    if !state.openai.enabled() {
        return ListedPaper {
            blurb: truncate(&paper.summary, 320),
            paper: paper.clone(),
            ai_generated: false,
        };
    }

    let key = format!("{}-{}-{}", paper.id, state.openai.listing.cache_tag(), PROMPT_VERSION);
    let built = state
        .cache
        .text_or_build("brief", &key, || async {
            state.openai.brief_summary(&paper.title, &paper.summary).await
        })
        .await;

    match built {
        Ok(text) => ListedPaper { paper: paper.clone(), blurb: text, ai_generated: true },
        Err(err) => {
            tracing::warn!(id = %paper.id, "brief summary failed, showing abstract: {err:#}");
            ListedPaper {
                blurb: truncate(&paper.summary, 320),
                paper: paper.clone(),
                ai_generated: false,
            }
        }
    }
}

/// Paper metadata, cached so repeat views don't re-hit the throttled arXiv API.
///
/// Loading a paper page asks for this three times over (the page itself, then
/// the summary and each chat turn behind it), and every request is serialized
/// behind the rate limiter — so this cache is what keeps a paper page quick and
/// keeps us well under arXiv's limits.
///
/// A versioned id (`2608.14539v1`) names immutable content. A bare id resolves
/// to the latest version, so a revision published after we cached it won't be
/// noticed until the cache is cleared.
pub async fn metadata(state: &AppState, id: &str) -> Result<Paper> {
    // The suffix is a schema version: changing `Paper`'s shape invalidates the
    // cached JSON instead of failing to parse it.
    let key = format!("{id}-v1");

    let json = state
        .cache
        .text_or_build("meta", &key, || async {
            let paper = state.arxiv.get_paper(id).await?;
            Ok(serde_json::to_string(&paper)?)
        })
        .await?;

    serde_json::from_str(&json).with_context(|| format!("reading cached metadata for {id}"))
}

/// The raw PDF, cached on disk after the first download.
pub async fn pdf(state: &AppState, id: &str) -> Result<Vec<u8>> {
    state
        .cache
        .bytes_or_build("pdf", id, "pdf", || async { state.arxiv.fetch_pdf(id).await })
        .await
}

/// Text extracted from the PDF, cached. `None` when the PDF can't be parsed
/// (scanned images, encryption, malformed files) — callers fall back to the abstract.
pub async fn full_text(state: &AppState, id: &str) -> Option<String> {
    let result = state
        .cache
        .text_or_build("text", id, || async {
            let bytes = pdf(state, id).await?;
            extract_text(bytes).await
        })
        .await;

    match result {
        Ok(text) if text.trim().len() > 500 => Some(text),
        Ok(_) => {
            tracing::warn!(%id, "PDF produced too little text to be useful");
            None
        }
        Err(err) => {
            tracing::warn!(%id, "PDF text extraction failed: {err:#}");
            None
        }
    }
}

/// PDF parsing is CPU-bound and can panic on malformed files, so it runs on the
/// blocking pool where a panic surfaces as a `JoinError` instead of killing us.
async fn extract_text(bytes: Vec<u8>) -> Result<String> {
    let text = tokio::task::spawn_blocking(move || pdf_extract::extract_text_from_mem(&bytes))
        .await
        .context("PDF extraction panicked")?
        .context("PDF extraction failed")?;

    Ok(tidy(&text))
}

/// The detailed, section-by-section summary shown on a paper page (markdown).
pub async fn detailed_summary(state: &AppState, paper: &Paper) -> Result<String> {
    let key = format!("{}-{}-{}", paper.id, state.openai.summary.cache_tag(), PROMPT_VERSION);
    state
        .cache
        .text_or_build("detailed", &key, || async {
            let body = match full_text(state, &paper.id).await {
                Some(text) => truncate(&text, state.config.summary_context_chars),
                None => format!(
                    "(Full text unavailable — only the abstract could be retrieved.)\n\nAbstract: {}",
                    paper.summary
                ),
            };
            state
                .openai
                .detailed_summary(&paper.title, &paper.author_line(), &body)
                .await
        })
        .await
}

/// The system-prompt context handed to the model on every chat turn.
pub async fn chat_context(state: &AppState, paper: &Paper) -> String {
    let body = match full_text(state, &paper.id).await {
        Some(text) => truncate(&text, state.config.chat_context_chars),
        None => "(Full text unavailable; only the abstract is provided.)".to_string(),
    };

    format!(
        "PAPER\nTitle: {}\nAuthors: {}\narXiv id: {}\nCategories: {}\nSubmitted: {}\n\n\
         ABSTRACT\n{}\n\nPAPER TEXT (may be truncated)\n{}",
        paper.title,
        paper.author_line(),
        paper.id,
        paper.categories.join(", "),
        paper.published,
        paper.summary,
        body,
    )
}

/// Cut to `limit` characters on a char boundary, at a word break where possible.
fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }

    let mut cut: String = text.chars().take(limit).collect();
    if let Some(space) = cut.rfind(char::is_whitespace)
        && space > limit.saturating_sub(40)
    {
        cut.truncate(space);
    }
    cut.push('…');
    cut
}

/// Collapse the ragged whitespace `pdf-extract` leaves behind, so we aren't
/// paying for tokens made entirely of blank space.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;

    for line in text.lines() {
        let line = line.replace('\u{c}', " ");
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(&line);
        out.push('\n');
    }

    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_short_text_intact() {
        assert_eq!(truncate("short", 100), "short");
    }

    #[test]
    fn truncate_breaks_on_a_word_boundary() {
        let out = truncate("the quick brown fox jumps over the lazy dog", 20);
        assert!(out.ends_with('…'));
        assert!(out.starts_with("the quick brown fox"));
        assert!(!out.contains("jump"));
    }

    #[test]
    fn truncate_handles_multibyte_text() {
        let out = truncate("naïve café — résumé données", 10);
        assert!(out.chars().count() <= 11);
    }

    #[test]
    fn tidy_collapses_blank_runs_and_form_feeds() {
        let out = tidy("one   two\n\n\n\nthree\u{c}four\n");
        assert_eq!(out, "one two\n\nthree four");
    }
}
