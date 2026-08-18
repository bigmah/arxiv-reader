//! One error type for every handler, rendered as an HTML page or JSON depending
//! on what the caller asked for.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::arxiv::{Unavailability, Unavailable};

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub message: String,
    /// JSON endpoints get `{"error": ...}`; page routes get an HTML error page.
    pub as_json: bool,
}

impl AppError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self { status: StatusCode::NOT_FOUND, message: message.into(), as_json: false }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: message.into(), as_json: false }
    }

    /// Mark this error as coming from a JSON API route.
    pub fn json(mut self) -> Self {
        self.as_json = true;
        self
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        let err: anyhow::Error = err.into();

        // arXiv being unavailable is a normal, temporary condition — say so
        // rather than dumping a stack of HTTP context as an internal error.
        if let Some(unavailable) = err.downcast_ref::<Unavailable>() {
            tracing::warn!("upstream unavailable: {unavailable}");

            let cause = match unavailable.kind {
                Unavailability::RateLimited => "arXiv is rate-limiting requests right now",
                Unavailability::Unresponsive => "arXiv isn't responding right now",
            };
            let wait = unavailable
                .retry_after
                .map(|w| format!("about {} seconds", w.as_secs().max(1)))
                .unwrap_or_else(|| "a few seconds".to_string());

            return Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: format!(
                    "{cause}. Wait {wait} and reload — papers you've already opened are \
                     cached and still work."
                ),
                as_json: false,
            };
        }

        tracing::error!("request failed: {err:#}");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("{err:#}"),
            as_json: false,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if self.as_json {
            return (self.status, Json(serde_json::json!({ "error": self.message }))).into_response();
        }

        let body = crate::templates::render_error(self.status, &self.message);
        (self.status, Html(body)).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn rate_limits_become_a_retryable_503() {
        let err = AppError::from(anyhow::Error::from(Unavailable {
            kind: Unavailability::RateLimited,
            retry_after: Some(Duration::from_secs(12)),
        }));

        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(err.message.contains("rate-limiting"), "got: {}", err.message);
        assert!(err.message.contains("about 12 seconds"), "got: {}", err.message);
        assert!(!err.message.contains("429"), "raw HTTP detail leaked: {}", err.message);
        assert!(!err.message.contains("export.arxiv.org"), "raw URL leaked: {}", err.message);
    }

    #[test]
    fn an_unresponsive_arxiv_reads_differently_from_a_refusal() {
        let err = AppError::from(anyhow::Error::from(Unavailable {
            kind: Unavailability::Unresponsive,
            retry_after: None,
        }));

        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(err.message.contains("isn't responding"), "got: {}", err.message);
        assert!(err.message.contains("a few seconds"), "got: {}", err.message);
    }

    #[test]
    fn other_failures_stay_500() {
        let err = AppError::from(anyhow::anyhow!("disk caught fire"));
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.message.contains("disk caught fire"));
    }
}
