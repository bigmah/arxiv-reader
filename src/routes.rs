//! HTTP routes.

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;

use crate::error::{AppError, AppResult};
use crate::openai::Message;
use crate::state::SharedState;
use crate::templates::{CategoryTemplate, IndexTemplate, PaperTemplate};
use crate::{markdown, papers, taxonomy};

/// Longest chat message we'll relay, per message.
const MAX_CHAT_CHARS: usize = 4_000;

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/c/{category}", get(category))
        .route("/p/{*id}", get(paper))
        .route("/pdf/{*id}", get(pdf))
        .route("/api/summary/{*id}", get(api_summary))
        .route("/api/chat/{*id}", post(api_chat))
        .route("/static/style.css", get(style))
        .route("/static/app.js", get(script))
        .route("/healthz", get(|| async { "ok" }))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    #[serde(default)]
    page: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FromQuery {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    page: Option<usize>,
}

async fn index(State(state): State<SharedState>) -> AppResult<Html<String>> {
    let page = IndexTemplate {
        groups: taxonomy::GROUPS,
        category_count: taxonomy::category_count(),
        ai_enabled: state.openai.enabled(),
    };
    Ok(Html(page.render()?))
}

async fn category(
    State(state): State<SharedState>,
    Path(category): Path<String>,
    Query(query): Query<PageQuery>,
) -> AppResult<Html<String>> {
    let Some((group, cat)) = taxonomy::find(&category) else {
        return Err(AppError::not_found(format!("`{category}` is not an arXiv category.")));
    };

    let page = query.page.unwrap_or(1).max(1);
    let (papers, pagination) = papers::list_category(&state, cat.id, page).await?;

    let rendered = CategoryTemplate {
        group_name: group.name,
        category_id: cat.id,
        category_name: cat.name,
        papers,
        pagination,
        ai_enabled: state.openai.enabled(),
    };
    Ok(Html(rendered.render()?))
}

async fn paper(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(query): Query<FromQuery>,
) -> AppResult<Html<String>> {
    let id = validate_id(&id)?;
    let paper = papers::metadata(&state, id).await?;

    let rendered = PaperTemplate {
        paper,
        from_category: query.from.filter(|c| taxonomy::find(c).is_some()),
        from_page: query.page.unwrap_or(1).max(1),
        ai_enabled: state.openai.enabled(),
    };
    Ok(Html(rendered.render()?))
}

/// Proxy the PDF rather than framing arxiv.org directly, which keeps the viewer
/// working regardless of arXiv's framing headers and lets us cache the bytes.
async fn pdf(State(state): State<SharedState>, Path(id): Path<String>) -> AppResult<Response> {
    let id = validate_id(&id)?;
    let bytes = papers::pdf(&state, id).await?;

    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf".to_string()),
            (header::CONTENT_DISPOSITION, format!("inline; filename=\"{}.pdf\"", sanitize_filename(id))),
            (header::CACHE_CONTROL, "public, max-age=86400".to_string()),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Serialize)]
struct SummaryResponse {
    available: bool,
    html: String,
    model: String,
}

/// The detailed summary is fetched after the page paints — it can take a while,
/// since a cold paper means downloading a PDF, extracting text, and one LLM call.
async fn api_summary(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> AppResult<Json<SummaryResponse>> {
    let id = validate_id(&id).map_err(AppError::json)?;

    if !state.openai.enabled() {
        return Ok(Json(SummaryResponse {
            available: false,
            html: String::new(),
            model: String::new(),
        }));
    }

    let paper = papers::metadata(&state, id).await.map_err(|e| AppError::from(e).json())?;
    let summary = papers::detailed_summary(&state, &paper)
        .await
        .map_err(|e| AppError::from(e).json())?;

    Ok(Json(SummaryResponse {
        available: true,
        html: markdown::to_html(&summary),
        model: state.openai.summary.model.clone(),
    }))
}

#[derive(Debug, Deserialize)]
struct ChatRequest {
    messages: Vec<Message>,
}

#[derive(Serialize)]
struct ChatResponse {
    reply: String,
    html: String,
}

async fn api_chat(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(request): Json<ChatRequest>,
) -> AppResult<Json<ChatResponse>> {
    let id = validate_id(&id).map_err(AppError::json)?;

    if !state.openai.enabled() {
        return Err(AppError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "Chat needs an OPENAI_API_KEY to be configured.".into(),
            as_json: true,
        });
    }

    if request.messages.is_empty() {
        return Err(AppError::bad_request("No messages supplied.").json());
    }
    if let Some(long) = request.messages.iter().find(|m| m.content.chars().count() > MAX_CHAT_CHARS) {
        return Err(AppError::bad_request(format!(
            "Message is too long ({} characters, limit {MAX_CHAT_CHARS}).",
            long.content.chars().count()
        ))
        .json());
    }

    // Only user/assistant turns come from the browser; the system prompt is ours.
    let history: Vec<Message> = request
        .messages
        .into_iter()
        .filter(|m| matches!(m.role.as_str(), "user" | "assistant"))
        .collect();
    if history.is_empty() {
        return Err(AppError::bad_request("No usable messages supplied.").json());
    }

    let paper = papers::metadata(&state, id).await.map_err(|e| AppError::from(e).json())?;
    let context = papers::chat_context(&state, &paper).await;
    let reply = state
        .openai
        .chat(&context, &history)
        .await
        .map_err(|e| AppError::from(e).json())?;

    Ok(Json(ChatResponse { html: markdown::to_html(&reply), reply }))
}

async fn style() -> Response {
    asset(include_str!("../static/style.css"), "text/css; charset=utf-8")
}

async fn script() -> Response {
    asset(include_str!("../static/app.js"), "text/javascript; charset=utf-8")
}

fn asset(body: &'static str, content_type: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

/// arXiv ids look like `2608.14539v1` or, for older papers, `math/0309136v2`.
/// Anything else is rejected before it reaches a URL or a cache path.
fn validate_id(id: &str) -> AppResult<&str> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && !id.contains("..")
        && !id.starts_with('/')
        && !id.ends_with('/')
        && id.matches('/').count() <= 1
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/'));

    if ok {
        Ok(id)
    } else {
        Err(AppError::bad_request(format!("`{id}` is not a valid arXiv id.")))
    }
}

fn sanitize_filename(id: &str) -> String {
    id.replace('/', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_modern_and_legacy_ids() {
        assert!(validate_id("2608.14539v1").is_ok());
        assert!(validate_id("2608.14539").is_ok());
        assert!(validate_id("math/0309136v2").is_ok());
        assert!(validate_id("cond-mat/0102536").is_ok());
    }

    #[test]
    fn rejects_traversal_and_junk() {
        assert!(validate_id("../../etc/passwd").is_err());
        assert!(validate_id("").is_err());
        assert!(validate_id("/abs/1234").is_err());
        assert!(validate_id("1234;rm -rf").is_err());
        assert!(validate_id(&"1".repeat(65)).is_err());
    }
}
