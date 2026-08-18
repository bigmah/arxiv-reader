# arxiv-reader

A small Rust web service that browses arXiv like the real site — subject groups,
categories, recent papers — but replaces the wall of abstracts with AI summaries,
and lets you read a paper next to a chat window that has read it too.

```
/                  every subject group and its categories
/c/{category}       latest papers in a category, 5 per page, each with a short summary
/p/{arxiv_id}       the PDF, a section-by-section summary, and chat
```

## Running it

```sh
cp .env.example .env       # add your OPENAI_API_KEY
cargo run
```

Then open <http://127.0.0.1:3000>.

Without an `OPENAI_API_KEY` everything still works as a plain arXiv browser: the
listings show trimmed abstracts and the summary/chat sections explain that they
need a key.

## How it works

**Subjects.** arXiv publishes no taxonomy endpoint, so `src/taxonomy.rs` bakes in
the 8 groups / 20 archives / 155 categories from
[arxiv.org/category_taxonomy](https://arxiv.org/category_taxonomy).

**Listings.** `src/arxiv.rs` queries the [arXiv API](https://info.arxiv.org/help/api/)
(`cat:{category}`, sorted by submission date) and parses the Atom feed. Requests
are throttled to one every three seconds, as arXiv asks, and serialized behind a
single lock so they queue rather than burst.

**Staying on arXiv's good side.** arXiv rate-limits by IP, and the block outlasts
the burst that caused it, so the client is built to ask as little as possible:

- Listing pages are cached for `LISTING_TTL_SECS` (default 5 minutes) and paper
  metadata is cached on disk indefinitely. Opening a paper used to cost three
  identical metadata queries — the page, the summary, and each chat turn — and
  now costs one.
- When arXiv answers `429`, or stops answering at all, the client stops asking
  for a cooldown window. During it, requests fail immediately with a clear "try
  again in N seconds" page instead of each one queueing behind the throttle to
  be refused. Anything already cached keeps working.
- Every fetch has a hard time budget, so a struggling arXiv produces a fast
  failure rather than a browser tab hanging for two minutes.

If you do get blocked, it clears on its own; raising `ARXIV_MIN_INTERVAL_MS`
makes it less likely to recur.

**Summaries.** Each paper on a listing gets one short OpenAI call against its
abstract, all five issued concurrently. Opening a paper downloads the PDF,
extracts its text with `pdf-extract`, and asks for a structured summary
(problem / approach / results / limitations / why it matters). That request runs
after the page paints, so the PDF is readable immediately.

The three jobs use different models, since skimming an abstract and reading a
whole paper aren't the same work:

| job | model | reasoning effort | token budget |
| --- | --- | --- | --- |
| listing blurbs | `gpt-5.6-terra` | off | 600 |
| paper summary | `gpt-5.6-sol` | `xhigh` | 16000 |
| chat | `gpt-5.6-sol` | `high` | 8000 |

Reasoning tokens bill against `max_completion_tokens`, so the budgets leave room
for an answer underneath the thinking; if one is still too tight the error says
so explicitly rather than showing an empty summary. Every value is overridable —
see [`.env.example`](.env.example).

**Chat.** Each turn re-sends the paper context (title, abstract, and up to 24k
characters of extracted text) plus the last 12 messages. Answers are rendered
from markdown with raw HTML stripped.

**Math.** Summaries and chat replies render LaTeX. `$…$` and `$$…$$` (plus the
`\(…\)` / `\[…\]` forms models like to use) are converted to MathML on the
server by `pulldown-latex`, which browsers typeset natively — so equation-heavy
papers cost the page no JavaScript, no web fonts, and no CDN. Unparseable LaTeX
falls back to showing its source rather than breaking the surrounding summary.
Listing blurbs stay deliberately notation-free; they're plain-language triage
text.

**The PDF** is proxied through `/pdf/{id}` rather than framed from arxiv.org, so
the viewer doesn't depend on arXiv's framing headers and the bytes get cached.

## Keeping the bill down

- **5 papers per page, hard-capped.** `PAGE_SIZE` is clamped to 5 in
  `src/config.rs`; a listing can never cost more than five calls.
- **Everything is cached to disk** under `CACHE_DIR` (default `.cache`) — PDFs,
  extracted text, paper metadata, and both kinds of summary — so revisiting a
  paper or paging back costs nothing. Cache keys include the model *and* its reasoning effort, so
  changing either regenerates rather than serving stale text. Delete the
  directory to reset.
- Chat is the only unbounded cost: it's one call per question.

## Configuration

Every setting is an environment variable, read from the environment or `.env`.
See [`.env.example`](.env.example) — beyond the per-job model settings above, the
useful ones are `CACHE_DIR`, `BIND_ADDR`, and `RUST_LOG`.

## Layout

| file | what's in it |
| --- | --- |
| `src/arxiv.rs` | arXiv API client and Atom parsing |
| `src/taxonomy.rs` | the baked-in subject tree |
| `src/openai.rs` | chat-completions client and the three prompts |
| `src/papers.rs` | listings, PDF text extraction, summary orchestration |
| `src/cache.rs` | memory + disk cache |
| `src/routes.rs` | HTTP routes |
| `templates/` | Askama templates (compiled in — edits need a rebuild) |
| `static/` | CSS and the chat/summary JavaScript |

## Tests

```sh
cargo test
```

Covers Atom parsing against recorded feeds (including the awkward one whose
`<link>` elements aren't adjacent), id validation, cache behaviour, markdown
sanitizing, LaTeX rendering in all four delimiter forms, and text truncation.
Nothing in the test suite hits the network.

## Notes

- Summaries are machine-generated and can be confidently wrong. The original
  abstract is one click away on every listing, and the PDF is right there on the
  paper page.
- Papers and metadata come from arXiv under its
  [API terms of use](https://info.arxiv.org/help/api/tou.html). This is a personal
  reading tool, not a mirror.
