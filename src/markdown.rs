//! Markdown -> HTML for model-written text, including LaTeX math.
//!
//! Math is converted to MathML here on the server, which browsers render
//! natively — so a summary full of equations costs the page no JavaScript, no
//! web fonts, and no CDN.

use pulldown_cmark::{Event, Options, Parser, html};
use pulldown_latex::config::DisplayMode;
use pulldown_latex::{RenderConfig, Storage, push_mathml};

/// Render markdown, turning `$…$` and `$$…$$` into MathML and dropping any raw
/// HTML the model emitted, so generated text can never inject markup.
pub fn to_html(markdown: &str) -> String {
    let source = normalize_math_delimiters(markdown);

    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_MATH;

    let events = Parser::new_ext(&source, options).filter_map(|event| match event {
        // Model-written HTML never reaches the page...
        Event::Html(_) | Event::InlineHtml(_) => None,
        // ...but the MathML we generate ourselves is trusted markup.
        Event::InlineMath(latex) => {
            Some(Event::InlineHtml(render_math(&latex, DisplayMode::Inline).into()))
        }
        Event::DisplayMath(latex) => {
            Some(Event::InlineHtml(render_math(&latex, DisplayMode::Block).into()))
        }
        other => Some(other),
    });

    let mut out = String::with_capacity(source.len() + source.len() / 2);
    html::push_html(&mut out, events);
    out
}

/// One LaTeX fragment to MathML. Unparseable math degrades to its source text
/// rather than taking down the surrounding summary.
fn render_math(latex: &str, display_mode: DisplayMode) -> String {
    let storage = Storage::new();
    let parser = pulldown_latex::Parser::new(latex, &storage);
    let config = RenderConfig {
        display_mode,
        // Keeps the original source in the markup, so copying an equation out
        // of the page yields LaTeX rather than symbol soup.
        annotation: Some(latex),
        ..RenderConfig::default()
    };

    let mut out = String::new();
    match push_mathml(&mut out, parser, config) {
        Ok(()) => out,
        Err(err) => {
            tracing::debug!("could not render LaTeX {latex:?}: {err}");
            format!(r#"<code class="math-raw">{}</code>"#, html_escape::encode_text(latex))
        }
    }
}

/// Models reach for `\(…\)` and `\[…\]` about as often as the dollar forms, but
/// pulldown-cmark only recognizes the latter. Translate them outside code.
fn normalize_math_delimiters(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut in_fence = false;

    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }

        if in_fence || trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            out.push_str(line);
        } else {
            convert_delimiters(line, &mut out);
        }
        out.push('\n');
    }

    out
}

fn convert_delimiters(line: &str, out: &mut String) {
    let mut chars = line.chars().peekable();
    let mut in_code_span = false;

    while let Some(c) = chars.next() {
        match c {
            '`' => {
                in_code_span = !in_code_span;
                out.push('`');
            }
            '\\' if !in_code_span => match chars.peek() {
                // `\\` is an escaped backslash (and a line break inside math);
                // consume both so its second half can't pair with what follows.
                Some('\\') => {
                    chars.next();
                    out.push_str("\\\\");
                }
                Some('(' | ')') => {
                    chars.next();
                    out.push('$');
                }
                Some('[' | ']') => {
                    chars.next();
                    out.push_str("$$");
                }
                _ => out.push('\\'),
            },
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_headings_and_lists() {
        let html = to_html("### The problem\n\n- first\n- second\n");
        assert!(html.contains("<h3>The problem</h3>"));
        assert!(html.contains("<li>first</li>"));
    }

    #[test]
    fn strips_raw_html() {
        let html = to_html("hello <script>alert(1)</script> world\n\n<div>block</div>");
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<div>"));
        assert!(html.contains("hello"));
    }

    #[test]
    fn renders_inline_math_as_mathml() {
        let html = to_html(r"The bound is $O(n^2)$ overall.");
        assert!(html.contains("<math"), "expected MathML, got: {html}");
        assert!(!html.contains("$O(n^2)$"), "dollars should be consumed: {html}");
        // n and the exponent 2 become separate MathML elements.
        assert!(html.contains("<msup>"), "expected a superscript: {html}");
    }

    #[test]
    fn renders_display_math_as_block_mathml() {
        let html = to_html("Loss:\n\n$$\\sum_{i=1}^{n} x_i^2$$\n");
        assert!(html.contains(r#"display="block""#), "expected block display: {html}");
        assert!(html.contains("<munderover>") || html.contains("<msubsup>"), "expected limits: {html}");
    }

    #[test]
    fn accepts_backslash_delimiters() {
        let inline = to_html(r"Given \(x \in \mathbb{R}\), we proceed.");
        assert!(inline.contains("<math"), "expected MathML from \\(…\\): {inline}");
        assert!(!inline.contains(r"\("));

        let display = to_html("Then\n\n\\[ E = mc^2 \\]\n");
        assert!(display.contains(r#"display="block""#), "expected block display: {display}");
    }

    #[test]
    fn keeps_backslashes_inside_code_untouched() {
        let html = to_html("Use `\\(x\\)` literally.\n\n```\n\\[ y \\]\n```\n");
        assert!(html.contains(r"\(x\)"), "inline code was rewritten: {html}");
        assert!(html.contains(r"\[ y \]"), "fenced code was rewritten: {html}");
        assert!(!html.contains("<math"));
    }

    #[test]
    fn double_backslash_does_not_form_a_delimiter() {
        // `\\` then `[2pt]` must not be read as the `\[` display opener.
        let html = to_html(r"$$a \\[2pt] b$$");
        assert!(html.contains("<math"), "expected MathML: {html}");
        assert!(!html.contains("$$"), "delimiters should be consumed: {html}");
    }

    #[test]
    fn invalid_latex_falls_back_to_source_text() {
        let html = to_html(r"broken $\frac{$ math");
        assert!(!html.contains("panic"));
        assert!(html.contains("broken"));
    }
}
