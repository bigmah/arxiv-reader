//! Client for the arXiv Atom API (<https://info.arxiv.org/help/api/>).

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::config::Config;

const USER_AGENT: &str = "arxiv-reader/0.1 (https://github.com/; local reading tool)";

/// Longest we'll honour a `Retry-After`, so a wide window can't hang a request.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(20);

/// Never retry instantly: arXiv has answered 503 with `Retry-After: 0`, and
/// taking that literally just hammers a service that is already struggling.
const MIN_RETRY_WAIT: Duration = Duration::from_secs(1);

/// Overall budget for one logical fetch, retries included. A browser tab waiting
/// on us is better served by a quick, clear failure than a slow one.
const REQUEST_BUDGET: Duration = Duration::from_secs(25);

/// PDFs are megabytes and deserve longer than a metadata query.
const PDF_TIMEOUT: Duration = Duration::from_secs(120);

/// Pause after arXiv goes quiet on us. Shorter than the rate-limit cooldown,
/// since a timeout may be a one-off rather than a policy.
const UNRESPONSIVE_COOLDOWN: Duration = Duration::from_secs(15);

/// Why arXiv isn't serving us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailability {
    /// Answered 429: we asked too often.
    RateLimited,
    /// Timed out, refused the connection, or returned a 5xx. In practice arXiv
    /// stops answering under load rather than saying no.
    Unresponsive,
}

/// arXiv is not serving us right now. Carried as its own type so routes can say
/// that plainly instead of rendering a generic 500 full of HTTP context.
#[derive(Debug)]
pub struct Unavailable {
    pub kind: Unavailability,
    /// How long until it's worth trying again, when we can tell.
    pub retry_after: Option<Duration>,
}

impl Unavailable {
    fn unresponsive(retry_after: Option<Duration>) -> Self {
        Self { kind: Unavailability::Unresponsive, retry_after }
    }
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            Unavailability::RateLimited => "arXiv is rate-limiting us",
            Unavailability::Unresponsive => "arXiv is not responding",
        };
        match self.retry_after {
            Some(wait) => write!(f, "{what}; retry in {}s", wait.as_secs()),
            None => write!(f, "{what}"),
        }
    }
}

impl std::error::Error for Unavailable {}

/// A single paper, flattened out of an Atom `<entry>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paper {
    /// Versioned id as arXiv returns it, e.g. `2608.14539v1`.
    pub id: String,
    pub title: String,
    pub summary: String,
    pub authors: Vec<String>,
    /// `YYYY-MM-DD`.
    pub published: String,
    pub updated: String,
    pub primary_category: String,
    pub categories: Vec<String>,
    pub comment: Option<String>,
    pub journal_ref: Option<String>,
    pub doi: Option<String>,
    pub abs_url: String,
    pub pdf_url: String,
}

impl Paper {
    /// Whether `tag` is the paper's primary category, for highlighting it.
    pub fn is_primary(&self, tag: &str) -> bool {
        self.primary_category == tag
    }

    /// Authors joined for display, abbreviated past four names.
    pub fn author_line(&self) -> String {
        match self.authors.len() {
            0 => "Unknown authors".to_string(),
            n if n <= 4 => self.authors.join(", "),
            _ => format!("{}, et al.", self.authors[..3].join(", ")),
        }
    }
}

/// One page of listing results.
#[derive(Debug, Clone)]
pub struct SearchPage {
    pub papers: Vec<Paper>,
    pub total: usize,
}

/// Identifies one listing query in the short-lived results cache.
type ListingKey = (String, usize, usize);

pub struct ArxivClient {
    http: reqwest::Client,
    last_request: Mutex<Option<Instant>>,
    min_interval: Duration,
    max_retries: u32,
    timeout: Duration,
    /// Set when arXiv turns us away. Until it passes, requests fail immediately
    /// instead of queueing behind the throttle to be refused one at a time. The
    /// kind is kept so the error still says which of the two happened.
    limited_until: Mutex<Option<(Instant, Unavailability)>>,
    cooldown: Duration,
    /// Recently-fetched listing pages. "Newest submissions" barely moves minute
    /// to minute, so a reload or a step back through the pager costs nothing.
    listings: Mutex<HashMap<ListingKey, (Instant, SearchPage)>>,
    listing_ttl: Duration,
    api_url: String,
    pdf_base: String,
}

impl ArxivClient {
    pub fn new(cfg: &Config) -> Result<Self> {
        // No client-wide timeout: it is set per request, since a PDF download
        // and a metadata query have very different expectations.
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .context("building arXiv HTTP client")?;

        Ok(Self {
            http,
            last_request: Mutex::new(None),
            min_interval: cfg.arxiv_min_interval,
            max_retries: cfg.arxiv_max_retries,
            timeout: cfg.arxiv_timeout,
            limited_until: Mutex::new(None),
            cooldown: cfg.arxiv_cooldown,
            listings: Mutex::new(HashMap::new()),
            listing_ttl: cfg.listing_ttl,
            api_url: cfg.arxiv_api_url.clone(),
            pdf_base: cfg.arxiv_pdf_base.clone(),
        })
    }

    /// The active cooldown, if any: how long is left and what caused it.
    async fn cooling_off(&self) -> Option<(Duration, Unavailability)> {
        let (until, kind) = (*self.limited_until.lock().await)?;
        let remaining = until.saturating_duration_since(Instant::now());
        (!remaining.is_zero()).then_some((remaining, kind))
    }

    /// Just the remaining time, for filling in a `retry_after`.
    async fn cooldown_remaining(&self) -> Option<Duration> {
        self.cooling_off().await.map(|(remaining, _)| remaining)
    }

    /// Stop asking for a while. A silent arXiv gets a shorter pause than an
    /// explicit refusal — it may just have been one slow request.
    async fn start_cooldown(&self, kind: Unavailability, retry_after: Option<Duration>) {
        let wait = retry_after.unwrap_or(match kind {
            Unavailability::RateLimited => self.cooldown,
            Unavailability::Unresponsive => self.cooldown.min(UNRESPONSIVE_COOLDOWN),
        });
        tracing::warn!("{kind:?}: pausing arXiv requests for {}s", wait.as_secs());
        *self.limited_until.lock().await = Some((Instant::now() + wait, kind));
    }

    async fn clear_cooldown(&self) {
        let mut limited = self.limited_until.lock().await;
        if limited.is_some() {
            tracing::info!("arXiv is answering again");
            *limited = None;
        }
    }

    /// Sleep as needed so we never exceed arXiv's requested request rate. The
    /// lock is deliberately held across the sleep: arXiv asks for one connection
    /// at a time, so requests queue rather than burst.
    async fn throttle(&self) {
        let mut last = self.last_request.lock().await;
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < self.min_interval {
                tokio::time::sleep(self.min_interval - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }

    /// GET with throttling, a backoff retry when arXiv pushes back, and a hard
    /// budget on the whole thing.
    async fn get(&self, url: &Url, timeout: Duration) -> Result<reqwest::Response> {
        let deadline = Instant::now() + REQUEST_BUDGET;

        for attempt in 0..=self.max_retries {
            // Refuse locally while cooling off, rather than spending the
            // request (and the wait) on a refusal we can already predict.
            if let Some((remaining, kind)) = self.cooling_off().await {
                return Err(Unavailable { kind, retry_after: Some(remaining) }.into());
            }

            self.throttle().await;

            // The budget bounds the request itself, not just the waits between
            // attempts — a hung arXiv shouldn't cost the full timeout twice.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            tracing::debug!(%url, attempt, "requesting from arXiv");

            let response = self.http.get(url.clone()).timeout(timeout.min(remaining)).send().await;
            let response = match response {
                Ok(response) => response,
                Err(err) if err.is_timeout() || err.is_connect() => {
                    tracing::warn!(%url, "arXiv request failed: {err}");
                    self.start_cooldown(Unavailability::Unresponsive, None).await;
                    if attempt < self.max_retries && sleep_before_retry(backoff(attempt), deadline).await {
                        continue;
                    }
                    return Err(Unavailable::unresponsive(self.cooldown_remaining().await).into());
                }
                Err(err) => return Err(err).with_context(|| format!("GET {url}")),
            };

            let status = response.status();
            if status.is_success() {
                self.clear_cooldown().await;
                return Ok(response);
            }

            let retry_after = retry_after(&response);
            if is_transient(status) {
                let kind = match status {
                    StatusCode::TOO_MANY_REQUESTS => Unavailability::RateLimited,
                    _ => Unavailability::Unresponsive,
                };
                self.start_cooldown(kind, retry_after).await;

                let wait = retry_after.unwrap_or_else(|| backoff(attempt)).max(MIN_RETRY_WAIT);
                if attempt < self.max_retries {
                    tracing::warn!(%url, %status, ?wait, "arXiv pushed back, backing off");
                    if sleep_before_retry(wait, deadline).await {
                        continue;
                    }
                }
                return Err(Unavailable { kind, retry_after: self.cooldown_remaining().await }.into());
            }

            return Err(response.error_for_status().unwrap_err())
                .with_context(|| format!("GET {url}"));
        }

        Err(Unavailable::unresponsive(self.cooldown_remaining().await).into())
    }

    async fn query(&self, params: &[(&str, &str)]) -> Result<Feed> {
        let url = Url::parse_with_params(&self.api_url, params)?;
        let body = self.get(&url, self.timeout).await?.text().await?;
        parse_feed(&body)
    }

    /// Most recent submissions in a category, newest first.
    pub async fn list_category(&self, category: &str, start: usize, count: usize) -> Result<SearchPage> {
        let key = (category.to_string(), start, count);

        if let Some(hit) = self.cached_listing(&key).await {
            tracing::debug!(%category, start, "serving listing from cache");
            return Ok(hit);
        }

        let feed = self
            .query(&[
                ("search_query", &format!("cat:{category}")),
                ("sortBy", "submittedDate"),
                ("sortOrder", "descending"),
                ("start", &start.to_string()),
                ("max_results", &count.to_string()),
            ])
            .await?;

        let page = SearchPage {
            papers: feed.entries.iter().map(Paper::from).collect(),
            total: feed.total_results(),
        };

        let mut listings = self.listings.lock().await;
        listings.retain(|_, (fetched, _)| fetched.elapsed() < self.listing_ttl);
        listings.insert(key, (Instant::now(), page.clone()));

        Ok(page)
    }

    async fn cached_listing(&self, key: &ListingKey) -> Option<SearchPage> {
        let listings = self.listings.lock().await;
        listings
            .get(key)
            .filter(|(fetched, _)| fetched.elapsed() < self.listing_ttl)
            .map(|(_, page)| page.clone())
    }

    /// Fetch a single paper's metadata by id (with or without a version suffix).
    pub async fn get_paper(&self, id: &str) -> Result<Paper> {
        let feed = self.query(&[("id_list", id), ("max_results", "1")]).await?;
        match feed.entries.first() {
            Some(entry) => Ok(Paper::from(entry)),
            None => bail!("arXiv has no paper with id {id}"),
        }
    }

    /// Download the PDF bytes for a paper.
    pub async fn fetch_pdf(&self, id: &str) -> Result<Vec<u8>> {
        let url = Url::parse(&format!("{}/{id}", self.pdf_base))?;
        Ok(self.get(&url, PDF_TIMEOUT).await?.bytes().await?.to_vec())
    }
}

/// Wait before another attempt, unless that would blow the budget. Returns
/// false when the caller should give up now.
async fn sleep_before_retry(wait: Duration, deadline: Instant) -> bool {
    if Instant::now() + wait >= deadline {
        tracing::warn!("giving up on arXiv rather than waiting past the request budget");
        return false;
    }
    tokio::time::sleep(wait).await;
    true
}

/// Statuses worth another attempt: rate limiting, and arXiv's own hiccups.
fn is_transient(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// `Retry-After` in delta-seconds form, capped so one absurd value can't stall
/// a request for minutes. The HTTP-date form is ignored; arXiv doesn't send it.
fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let seconds: u64 = response.headers().get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(seconds).min(MAX_RETRY_WAIT))
}

/// 2s, 4s, 8s…
fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(attempt + 1)).min(MAX_RETRY_WAIT)
}

fn parse_feed(body: &str) -> Result<Feed> {
    quick_xml::de::from_str(body).context("parsing arXiv Atom feed")
}

impl From<&Entry> for Paper {
    fn from(e: &Entry) -> Self {
        // <id> is a URL like http://arxiv.org/abs/2608.14539v1
        let id = e.id.rsplit("/abs/").next().unwrap_or(&e.id).to_string();

        // Built from the id rather than read from the feed's <link> elements —
        // see the note on `Entry` for why those are deliberately not parsed.
        let abs_url = format!("https://arxiv.org/abs/{id}");
        let pdf_url = format!("https://arxiv.org/pdf/{id}");

        Paper {
            id,
            title: squash(&e.title),
            summary: squash(&e.summary),
            authors: e.authors.iter().map(|a| squash(&a.name)).collect(),
            published: date_only(&e.published),
            updated: date_only(&e.updated),
            primary_category: e
                .primary_category
                .as_ref()
                .map(|c| c.term.clone())
                .or_else(|| e.categories.first().map(|c| c.term.clone()))
                .unwrap_or_default(),
            categories: e.categories.iter().map(|c| c.term.clone()).collect(),
            comment: e.comment.as_deref().map(squash).filter(|s| !s.is_empty()),
            journal_ref: e.journal_ref.as_deref().map(squash).filter(|s| !s.is_empty()),
            doi: e.doi.as_deref().map(squash).filter(|s| !s.is_empty()),
            abs_url,
            pdf_url,
        }
    }
}

/// arXiv hard-wraps titles and abstracts; collapse that back to single spaces.
fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `2026-08-14T17:51:30Z` -> `2026-08-14`.
fn date_only(s: &str) -> String {
    s.split('T').next().unwrap_or(s).to_string()
}

// --- Atom deserialization -------------------------------------------------
//
// quick-xml strips namespace prefixes from element names, so arXiv's
// `<opensearch:totalResults>` and `<arxiv:comment>` arrive as `totalResults` and
// `comment`. Attributes keep their prefix and gain a leading `@`.

#[derive(Debug, Deserialize)]
struct Feed {
    #[serde(rename = "totalResults", default)]
    total_results: Option<String>,
    #[serde(rename = "entry", default)]
    entries: Vec<Entry>,
}

impl Feed {
    fn total_results(&self) -> usize {
        self.total_results.as_deref().and_then(|t| t.trim().parse().ok()).unwrap_or(0)
    }
}

/// Note the absence of a `link` field. quick-xml's serde layer can only fold
/// repeated elements into a `Vec` when they are adjacent, and arXiv emits a
/// third `<link>` for the DOI *after* the authors — which would fail the whole
/// feed with "duplicate field `link`". Unknown elements are skipped one at a
/// time with no such restriction, and every URL we need is derivable from the
/// id anyway, so the links are left unparsed on purpose.
#[derive(Debug, Deserialize)]
struct Entry {
    id: String,
    title: String,
    summary: String,
    published: String,
    updated: String,
    #[serde(rename = "author", default)]
    authors: Vec<Author>,
    #[serde(rename = "category", default)]
    categories: Vec<CategoryRef>,
    #[serde(default)]
    primary_category: Option<CategoryRef>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    journal_ref: Option<String>,
    #[serde(default)]
    doi: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Author {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CategoryRef {
    #[serde(rename = "@term")]
    term: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/fixtures/feed.xml");
    /// A live feed containing an entry whose DOI `<link>` trails the author list.
    const WITH_DOI: &str = include_str!("../tests/fixtures/feed-with-doi.xml");

    #[test]
    fn parses_a_real_feed() {
        let feed = parse_feed(SAMPLE).expect("feed should parse");
        assert_eq!(feed.total_results(), 194769);
        assert_eq!(feed.entries.len(), 2);

        let papers: Vec<Paper> = feed.entries.iter().map(Paper::from).collect();
        let first = &papers[0];
        assert_eq!(first.id, "2608.14539v1");
        assert!(first.title.starts_with("Decoding the Past"));
        assert!(!first.title.contains('\n'));
        assert_eq!(first.authors.len(), 4);
        assert_eq!(first.authors[0], "Karel Becerra");
        assert_eq!(first.published, "2026-08-14");
        assert_eq!(first.primary_category, "cs.CV");
        assert_eq!(first.categories, ["cs.CV", "cs.AI", "cs.LG"]);
        assert_eq!(first.pdf_url, "https://arxiv.org/pdf/2608.14539v1");
        assert_eq!(first.abs_url, "https://arxiv.org/abs/2608.14539v1");
        assert!(first.comment.is_none());

        assert_eq!(papers[1].comment.as_deref(), Some("Project page: https://alayalab.github.io/Marionette/"));
    }

    /// Regression: entries with non-adjacent `<link>` elements used to fail the
    /// whole feed with "duplicate field `link`".
    #[test]
    fn parses_entries_whose_links_are_not_adjacent() {
        let feed = parse_feed(WITH_DOI).expect("feed with a trailing DOI link should parse");
        assert_eq!(feed.entries.len(), 5);

        let papers: Vec<Paper> = feed.entries.iter().map(Paper::from).collect();
        let with_doi = papers.iter().find(|p| p.doi.is_some()).expect("one entry has a DOI");
        assert_eq!(with_doi.doi.as_deref(), Some("10.1145/3799682.3839874"));
        assert_eq!(with_doi.pdf_url, format!("https://arxiv.org/pdf/{}", with_doi.id));
        assert!(papers.iter().all(|p| !p.title.is_empty() && !p.authors.is_empty()));
    }

    #[test]
    fn retries_rate_limits_and_server_errors_only() {
        assert!(is_transient(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_transient(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_transient(StatusCode::INTERNAL_SERVER_ERROR));
        // A bad id or a missing paper will never succeed on retry.
        assert!(!is_transient(StatusCode::NOT_FOUND));
        assert!(!is_transient(StatusCode::BAD_REQUEST));
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(backoff(0), Duration::from_secs(2));
        assert_eq!(backoff(1), Duration::from_secs(4));
        assert_eq!(backoff(2), Duration::from_secs(8));
        assert_eq!(backoff(30), MAX_RETRY_WAIT, "must not overflow or stall for hours");
    }

    #[test]
    fn honours_retry_after_within_the_cap() {
        let response = |value: &str| {
            let raw = http::Response::builder()
                .status(StatusCode::TOO_MANY_REQUESTS)
                .header(reqwest::header::RETRY_AFTER, value)
                .body("")
                .unwrap();
            reqwest::Response::from(raw)
        };

        assert_eq!(retry_after(&response("7")), Some(Duration::from_secs(7)));
        assert_eq!(retry_after(&response(" 4 ")), Some(Duration::from_secs(4)));
        assert_eq!(retry_after(&response("99999")), Some(MAX_RETRY_WAIT), "capped");
        // The HTTP-date form isn't parsed; callers fall back to backoff.
        assert_eq!(retry_after(&response("Wed, 21 Oct 2026 07:28:00 GMT")), None);
    }

    fn test_client() -> ArxivClient {
        let mut cfg = Config::from_env().unwrap();
        cfg.arxiv_cooldown = Duration::from_secs(30);
        ArxivClient::new(&cfg).unwrap()
    }

    #[tokio::test]
    async fn cooldown_blocks_then_expires() {
        let client = test_client();
        assert!(client.cooling_off().await.is_none(), "starts closed");

        client.start_cooldown(Unavailability::RateLimited, None).await;
        let (remaining, kind) = client.cooling_off().await.expect("should be cooling off");
        assert!(remaining <= Duration::from_secs(30) && remaining > Duration::from_secs(25));
        assert_eq!(kind, Unavailability::RateLimited, "the cause must survive the pause");

        // A timeout records itself as such, and gets the shorter pause.
        client.start_cooldown(Unavailability::Unresponsive, None).await;
        let (remaining, kind) = client.cooling_off().await.unwrap();
        assert_eq!(kind, Unavailability::Unresponsive);
        assert!(remaining <= UNRESPONSIVE_COOLDOWN, "unresponsive pause should be the shorter one");

        // A cooldown in the past is over, not perpetual.
        *client.limited_until.lock().await =
            Some((Instant::now() - Duration::from_secs(1), Unavailability::RateLimited));
        assert!(client.cooling_off().await.is_none(), "expired cooldown should clear");
    }

    #[tokio::test]
    async fn a_success_reopens_the_circuit() {
        let client = test_client();
        client.start_cooldown(Unavailability::RateLimited, Some(Duration::from_secs(10))).await;
        assert!(client.cooling_off().await.is_some());

        client.clear_cooldown().await;
        assert!(client.cooling_off().await.is_none());
    }

    #[tokio::test]
    async fn retry_sleep_respects_the_budget() {
        let now = Instant::now();
        assert!(sleep_before_retry(Duration::from_millis(1), now + Duration::from_secs(5)).await);
        // A wait that would run past the deadline means give up instead.
        assert!(!sleep_before_retry(Duration::from_secs(10), now + Duration::from_secs(5)).await);
    }

    #[test]
    fn abbreviates_long_author_lists() {
        let mut paper = Paper::from(&parse_feed(SAMPLE).unwrap().entries[0]);
        assert_eq!(paper.author_line(), "Karel Becerra, Boris Mederos, Dean Snow, Ramón A. Mollineda");

        paper.authors.push("Extra Person".into());
        assert_eq!(paper.author_line(), "Karel Becerra, Boris Mederos, Dean Snow, et al.");
    }
}
