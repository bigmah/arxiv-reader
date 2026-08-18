//! Askama template bindings. Templates live in `templates/` and are compiled in.

use askama::Template;
use axum::http::StatusCode;

use crate::arxiv::Paper;
use crate::papers::{ListedPaper, Pagination};
use crate::taxonomy::Group;

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub groups: &'static [Group],
    pub category_count: usize,
    pub ai_enabled: bool,
}

#[derive(Template)]
#[template(path = "category.html")]
pub struct CategoryTemplate {
    pub group_name: &'static str,
    pub category_id: &'static str,
    pub category_name: &'static str,
    pub papers: Vec<ListedPaper>,
    pub pagination: Pagination,
    pub ai_enabled: bool,
}

#[derive(Template)]
#[template(path = "paper.html")]
pub struct PaperTemplate {
    pub paper: Paper,
    /// The category we navigated in from, for the back link.
    pub from_category: Option<String>,
    pub from_page: usize,
    pub ai_enabled: bool,
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorTemplate<'a> {
    code: u16,
    reason: &'a str,
    message: &'a str,
}

/// Render the error page, degrading to plain text if even that fails.
pub fn render_error(status: StatusCode, message: &str) -> String {
    let template = ErrorTemplate {
        code: status.as_u16(),
        reason: status.canonical_reason().unwrap_or("Error"),
        message,
    };

    template.render().unwrap_or_else(|err| {
        tracing::error!("error template failed to render: {err}");
        format!("{} {}\n\n{}", status.as_u16(), status.canonical_reason().unwrap_or(""), message)
    })
}
