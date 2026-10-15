//! Minimal HTML SERP rendering (server-side).
//!
//! Server-rendered and fully usable without JavaScript. Implements the requirements in
//! `docs/api/search.md` § Accessibility: an `<ol>` of results, an accessible name from
//! the title, `aria-live` on the result count and notices, keyboard-usable controls, and
//! `prefers-color-scheme` / `prefers-contrast` theming.

use crate::models::SearchResponse;
use std::fmt::Write as _;

/// Render the SERP for `resp`, echoing the user's `q` back into the search box.
///
/// `offset` and `page_size` are the request's pagination inputs; they are echoed into the
/// prev/next links rather than recomputed from the result count, which would be wrong on
/// the last page. `has_more` says whether a next page exists, so a link is only shown when
/// one does.
#[must_use]
pub fn render_html(
    resp: &SearchResponse,
    q: &str,
    offset: usize,
    page_size: usize,
    has_more: bool,
) -> String {
    let mut h = String::with_capacity(4096);
    head(&mut h, q);
    body_start(&mut h, q);
    notices(&mut h, resp);
    results(&mut h, resp);
    pagination(&mut h, q, offset, page_size, resp.results.len(), has_more);
    operators_help(&mut h);
    h.push_str("</main></body></html>");
    h
}

fn head(h: &mut String, q: &str) {
    h.push_str(concat!(
        "<!DOCTYPE html>",
        "<html lang=\"en\"><head><meta charset=\"utf-8\">",
        "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">",
        "<meta name=\"referrer\" content=\"no-referrer\">",
        "<title>",
    ));
    h.push_str(&escape(q));
    h.push_str(" — LYNX</title>");
    h.push_str(STYLE);
    h.push_str("</head><body>");
    h.push_str("<a class=\"skip\" href=\"#results\">Skip to results</a>");
}

fn body_start(h: &mut String, q: &str) {
    h.push_str("<main id=\"main\">");
    h.push_str("<header><p class=\"brand\">LYNX</p>");
    h.push_str(
        "<form method=\"get\" action=\"/api/v1/search\" class=\"search\" role=\"search\" aria-label=\"Search LYNX\">",
    );
    h.push_str("<label class=\"visually-hidden\" for=\"q\">Search query</label>");
    h.push_str("<input id=\"q\" name=\"q\" value=\"");
    h.push_str(&escape(q));
    h.push_str("\" autocomplete=\"off\" autocapitalize=\"off\" spellcheck=\"false\">");
    h.push_str("<input type=\"hidden\" name=\"format\" value=\"html\">");
    h.push_str("<button type=\"submit\">Search</button>");
    h.push_str("</form></header>");
}

fn notices(h: &mut String, resp: &SearchResponse) {
    let truncated = resp.meta.truncated.unwrap_or(false);
    if let Some(notice) = &resp.meta.notice {
        h.push_str("<p class=\"notice\" role=\"status\">");
        h.push_str(&escape(notice));
        h.push_str("</p>");
    } else if truncated {
        h.push_str(
            "<p class=\"notice\" role=\"status\">The query was too long and was truncated at a term boundary.</p>",
        );
    }
    if resp.meta.degraded {
        h.push_str(
            "<p class=\"notice\" role=\"status\">Some shards were degraded; results may be incomplete.</p>",
        );
    }
}

fn results(h: &mut String, resp: &SearchResponse) {
    h.push_str("<p class=\"count\" aria-live=\"polite\">");
    if resp.results.is_empty() {
        h.push_str("No results. Try fewer or different terms.</p>");
    } else {
        let _ = write!(
            h,
            "{} result{} in {} ms</p>",
            resp.results.len(),
            if resp.results.len() == 1 { "" } else { "s" },
            resp.meta.took_ms
        );
    }
    h.push_str("<ol id=\"results\" class=\"results\" aria-busy=\"false\">");
    for r in &resp.results {
        h.push_str("<li class=\"result\">");
        h.push_str("<h2 class=\"title\"><a class=\"result-link\" href=\"");
        h.push_str(&escape(&r.url));
        h.push_str("\" rel=\"noreferrer noopener\">");
        h.push_str(&escape(&r.title));
        h.push_str("</a></h2>");
        h.push_str("<p class=\"url\">");
        h.push_str(&escape(&display_url(&r.url)));
        h.push_str("</p>");
        h.push_str("<p class=\"snippet\">");
        h.push_str(&escape(&r.snippet));
        h.push_str("</p>");
        h.push_str("</li>");
    }
    h.push_str("</ol>");
}

fn pagination(
    h: &mut String,
    q: &str,
    offset: usize,
    page_size: usize,
    returned: usize,
    has_more: bool,
) {
    if returned == 0 {
        return;
    }
    h.push_str("<nav class=\"pagination\" aria-label=\"Pagination\">");
    if offset > 0 {
        let prev = offset.saturating_sub(page_size.max(1));
        let _ = write!(
            h,
            "<a rel=\"prev\" href=\"{}&offset={prev}\">Previous</a>",
            query_string(q)
        );
    }
    if has_more {
        let next = offset + page_size.max(1);
        let _ = write!(
            h,
            "<a rel=\"next\" href=\"{}&offset={next}\">Next</a>",
            query_string(q)
        );
    }
    h.push_str("</nav>");
}

fn operators_help(h: &mut String) {
    h.push_str(concat!(
        "<details class=\"operators\"><summary>Search operators</summary>",
        "<dl>",
        "<dt><code>\"exact phrase\"</code></dt><dd>Match words in order.</dd>",
        "<dt><code>-term</code></dt><dd>Penalise a term (not an exclusion).</dd>",
        "<dt><code>OR</code></dt><dd>Match either side.</dd>",
        "<dt><code>site:example.com</code></dt><dd>Restrict to a site.</dd>",
        "<dt><code>inurl:/path</code></dt><dd>Match within the URL.</dd>",
        "<dt><code>intitle:word</code></dt><dd>Boost a word in the title.</dd>",
        "<dt><code>ext:pdf</code> / <code>filetype:pdf</code></dt><dd>Restrict by type.</dd>",
        "<dt><code>after:2025-01</code> / <code>before:2026</code></dt><dd>Date filter (excludes undated pages).</dd>",
        "<dt><code>freshness:week</code></dt><dd>Recent pages only.</dd>",
        "<dt><code>lang:de</code></dt><dd>Restrict by language.</dd>",
        "<dt><code>+term</code></dt><dd>Force-include a term.</dd>",
        "</dl></details>",
    ));
}

fn query_string(q: &str) -> String {
    format!("/api/v1/search?format=html&q={}", urlencode(q))
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn display_url(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .trim_end_matches('/')
        .to_string()
}

#[must_use]
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

const STYLE: &str = concat!(
    "<style>",
    ":root{--bg:#0b1026;--fg:#d8e1ff;--muted:#9fb7ff;--link:#7cb3ff;--line:#25305c;--card:#121a3a}",
    "@media (prefers-color-scheme:light){:root{--bg:#fbfbfd;--fg:#1a1a2e;--muted:#5a6478;--link:#1b4ed8;--line:#e2e5ee;--card:#fff}}",
    "@media (prefers-contrast:more){:root{--muted:#8fa0c8}a.result-link{text-decoration:underline}}",
    "*{box-sizing:border-box}",
    "body{font-family:system-ui,-apple-system,'Segoe UI',sans-serif;margin:0;padding:1.5rem max(1rem,5vw) 4rem;line-height:1.6;background:var(--bg);color:var(--fg)}",
    "main{max-width:52rem;margin:0 auto}",
    "a{color:var(--link)}",
    ".brand{font-weight:700;letter-spacing:.08em;margin:0 0 .5rem}",
    ".search{display:flex;gap:.5rem;margin-bottom:1.5rem}",
    ".search input[type=text],.search input:not([type]){flex:1;padding:.6rem .75rem;font-size:1rem;border:1px solid var(--line);border-radius:.5rem;background:var(--card);color:var(--fg)}",
    ".search button{padding:.6rem 1.1rem;font-size:1rem;border:1px solid var(--line);border-radius:.5rem;background:var(--card);color:var(--fg);cursor:pointer}",
    ".count{color:var(--muted);font-size:.9rem;margin:.25rem 0 1rem}",
    ".notice{border:1px solid var(--line);border-radius:.5rem;padding:.5rem .75rem;background:var(--card);color:var(--muted)}",
    ".results{list-style:none;padding:0;margin:0}",
    ".result{padding:1rem 0;border-bottom:1px solid var(--line)}",
    ".title{font-size:1.15rem;margin:0 0 .15rem;font-weight:600}",
    ".url{color:var(--muted);font-size:.85rem;margin:.1rem 0;overflow-wrap:anywhere}",
    ".snippet{margin:.25rem 0 0}",
    ".pagination{display:flex;gap:1rem;margin:1.5rem 0}",
    ".operators{margin-top:2rem;color:var(--muted)}",
    ".operators dt{margin-top:.5rem;color:var(--fg)}.operators dd{margin:.1rem 0 0 1rem}",
    ".visually-hidden{position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0 0 0 0)}",
    ".skip{position:absolute;left:-999px}.skip:focus{left:1rem;top:1rem;background:var(--card);padding:.5rem}",
    "@media (prefers-reduced-motion:no-preference){a{transition:color .15s ease}}",
    "</style>",
);
