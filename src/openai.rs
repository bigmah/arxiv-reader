//! Thin wrapper over the OpenAI chat completions API.
//!
//! The whole client is optional: with no `OPENAI_API_KEY` the app still browses
//! arXiv, it just shows raw abstracts instead of summaries.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::config::{Config, ModelChoice};

/// Bump when a prompt changes, so stale cached summaries are regenerated.
pub const PROMPT_VERSION: &str = "v2";

/// Appended to the prompts whose output is rendered as markdown. The renderer
/// turns these delimiters into MathML; other forms are translated where they can
/// be, so asking for one form up front keeps the output predictable.
const MATH_RULES: &str = "\
Write any mathematics as LaTeX: $…$ for inline math and $$…$$ on its own line for \
displayed equations. Use real notation rather than ASCII approximations — $O(n^2)$, \
not O(n^2). Do not wrap equations in \\begin{equation} environments or code fences, \
and do not use a bare $ for anything other than math.";

const BRIEF_SYSTEM: &str = "\
You help a researcher triage brand-new arXiv papers. Given a title and abstract, \
write 2-3 sentences (60 words maximum) in plain language covering what the authors \
did and why it matters. Lead with the contribution. Do not restate the title, do not \
open with filler like 'This paper', and do not use markdown, bullet points, or \
mathematical notation — describe quantities in words.";

const DETAILED_SYSTEM: &str = "\
You are a careful research assistant summarizing an academic paper for someone \
deciding whether to read it in full. Use GitHub-flavored markdown with these level-3 \
headings, in order: '### The problem', '### Approach', '### Key results', \
'### Limitations', '### Why it matters'. Keep each section to 2-4 sentences or a short \
bullet list. Quote concrete numbers from the paper wherever they exist. If the provided \
text is truncated or unreadable in places, say so under Limitations rather than guessing. \
Reproduce the paper's key equations where they carry the argument.";

const CHAT_SYSTEM: &str = "\
You are discussing one specific academic paper with a reader. Answer from the paper \
text provided below. When the text does not settle a question, say so explicitly \
instead of speculating, and make clear when you are drawing on general background \
knowledge rather than this paper. Be concise and concrete; cite section names or \
numbers from the paper when relevant. Short markdown is fine.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }
}

pub struct OpenAiClient {
    http: reqwest::Client,
    api_key: Option<String>,
    base_url: String,
    /// Cheap pass over an abstract, once per paper in a listing.
    pub listing: ModelChoice,
    /// The expensive read of the whole paper.
    pub summary: ModelChoice,
    /// One call per question asked.
    pub chat: ModelChoice,
}

impl OpenAiClient {
    pub fn new(cfg: &Config) -> Result<Self> {
        // Long enough for a high-effort reasoning pass over a full paper.
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .context("building OpenAI HTTP client")?;

        Ok(Self {
            http,
            api_key: cfg.openai_api_key.clone(),
            base_url: cfg.openai_base_url.clone(),
            listing: cfg.listing_model.clone(),
            summary: cfg.summary_model.clone(),
            chat: cfg.chat_model.clone(),
        })
    }

    /// False when no API key is configured; callers fall back to showing raw text.
    pub fn enabled(&self) -> bool {
        self.api_key.is_some()
    }

    async fn complete(&self, choice: &ModelChoice, messages: Vec<Message>) -> Result<String> {
        let Some(key) = self.api_key.as_deref() else {
            bail!("OPENAI_API_KEY is not set");
        };

        let url = format!("{}/chat/completions", self.base_url);
        let request = CompletionRequest {
            model: &choice.model,
            messages: &messages,
            // `max_completion_tokens` rather than the deprecated `max_tokens`, and no
            // `temperature`, so this works across both the gpt-4 and gpt-5 families.
            max_completion_tokens: choice.max_tokens,
            reasoning_effort: choice.effort.as_deref(),
        };

        let response = self
            .http
            .post(&url)
            .bearer_auth(key)
            .json(&request)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;

        let status = response.status();
        let body = response.text().await.context("reading OpenAI response body")?;

        if !status.is_success() {
            let detail = serde_json::from_str::<ErrorEnvelope>(&body)
                .ok()
                .map(|e| e.error.message)
                .unwrap_or_else(|| body.chars().take(500).collect());
            bail!("OpenAI returned {status}: {detail}");
        }

        let parsed: CompletionResponse =
            serde_json::from_str(&body).context("parsing OpenAI response")?;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("OpenAI returned no choices"))?;

        let content = choice.message.content.unwrap_or_default().trim().to_string();
        if !content.is_empty() {
            return Ok(content);
        }

        // A reasoning model bills its thinking against `max_completion_tokens`,
        // so too small a budget yields a truncated response with no answer in it.
        if choice.finish_reason.as_deref() == Some("length") {
            bail!(
                "{} hit its {}-token limit before writing an answer{}. Raise the limit for this \
                 request, or lower the reasoning effort.",
                request.model,
                request.max_completion_tokens,
                match request.reasoning_effort {
                    Some(effort) => format!(" (reasoning_effort={effort})"),
                    None => String::new(),
                }
            );
        }

        bail!("OpenAI returned an empty completion")
    }

    /// The short blurb shown for each paper in a listing.
    pub async fn brief_summary(&self, title: &str, abstract_text: &str) -> Result<String> {
        self.complete(
            &self.listing,
            vec![
                Message::system(BRIEF_SYSTEM),
                Message::user(format!("Title: {title}\n\nAbstract: {abstract_text}")),
            ],
        )
        .await
    }

    /// The structured walk-through shown on a paper's own page.
    pub async fn detailed_summary(&self, title: &str, authors: &str, body: &str) -> Result<String> {
        self.complete(
            &self.summary,
            vec![
                Message::system(format!("{DETAILED_SYSTEM}\n\n{MATH_RULES}")),
                Message::user(format!(
                    "Title: {title}\nAuthors: {authors}\n\nPaper text follows.\n\n{body}"
                )),
            ],
        )
        .await
    }

    /// One turn of the chat-with-the-paper conversation.
    pub async fn chat(&self, paper_context: &str, history: &[Message]) -> Result<String> {
        let mut messages =
            vec![Message::system(format!("{CHAT_SYSTEM}\n\n{MATH_RULES}\n\n{paper_context}"))];
        // Only the tail of the conversation is resent, to bound per-turn cost.
        let tail = history.len().saturating_sub(12);
        messages.extend(history[tail..].iter().cloned());
        self.complete(&self.chat, messages).await
    }
}

#[derive(Serialize)]
struct CompletionRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    max_completion_tokens: u32,
    /// Omitted entirely for models that don't accept it.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
}

#[derive(Deserialize)]
struct CompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}
