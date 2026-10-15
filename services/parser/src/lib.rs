//! `lynx-parser` — HTML extraction and sanitisation.
//!
//! Produces a bounded [`ParsedDocument`]: title, headings, cleaned body, image alt text,
//! description, language, dates, outbound links with their anchor text, a content hash,
//! and a near-duplicate `SimHash`. No script, no markup, and no unbounded text reaches the
//! index.
//!
//! The extractor streams html5ever tokens through an explicit open-element stack. It
//! never materialises a DOM, so hostile nesting cannot exhaust memory beyond the hard
//! limits in [`ParseLimits`] (`docs/crawler/parser.md`). Every limit truncates and sets a
//! flag; parse never panics on malformed markup.
//!
//! ```no_run
//! let base: url::Url = "https://example.com/a".parse().unwrap();
//! let doc = lynx_parser::parse("<title>Hi</title><p>Hello</p>", &base);
//! assert_eq!(doc.title, "Hi");
//! ```

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

mod extract;
mod limits;
mod simhash;

use url::Url;

pub use limits::{ParseFlags, ParseLimits};

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "parser";

/// Parser version. Bump on any change to extraction that alters output, so an index can
/// be attributed to the parser that produced it.
pub const PARSER_VERSION: &str = "lynx-parser-1";

/// Parse an HTML document under the v0.1 limits.
///
/// `base` is the URL the document was fetched from; relative links resolve against it and
/// it decides same-host canonical hints. Parsing is infallible: malformed markup,
/// adversarial nesting, and absurd documents truncate rather than error.
#[must_use]
pub fn parse(input: &str, base: &Url) -> lynx_models::domain::ParsedDocument {
    parse_with_limits(input, base, &ParseLimits::default()).0
}

/// Parse an HTML document under caller-supplied limits, returning the document and the
/// flags recording which limits were hit.
#[must_use]
pub fn parse_with_limits(
    input: &str,
    base: &Url,
    limits: &ParseLimits,
) -> (lynx_models::domain::ParsedDocument, ParseFlags) {
    extract::run(input, base, limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use time::macros::datetime;

    fn base(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn limits() -> ParseLimits {
        ParseLimits::default()
    }

    #[test]
    fn service_name_and_version_are_stable() {
        assert_eq!(SERVICE_NAME, "parser");
        assert_eq!(PARSER_VERSION, "lynx-parser-1");
    }

    #[test]
    fn extracts_title_headings_and_body() {
        let html = "<html lang=\"en\"><head><title>Fixture Page</title>\
            <meta name=\"description\" content=\"A short description.\"></head>\
            <body><h1>Big Heading</h1><h2>Sub</h2>\
            <p>The body sentence.</p></body></html>";
        let doc = parse(html, &base("https://e.com/page"));
        assert_eq!(doc.title, "Fixture Page");
        assert_eq!(doc.headings, vec!["Big Heading", "Sub"]);
        assert!(doc.body.contains("The body sentence."));
        assert_eq!(doc.description.as_deref(), Some("A short description."));
        assert_eq!(doc.lang.as_deref(), Some("en"));
        assert_eq!(doc.canonical, "https://e.com/page");
        assert!(!doc.truncated);
    }

    #[test]
    fn keeps_word_boundaries_between_adjacent_block_elements() {
        let html = "<body><h1>Rust Async</h1><p>Concurrency without threads</p>\
            <p>Second paragraph</p></body></html>";
        let doc = parse(html, &base("https://e.com/page"));
        assert!(doc.body.contains("Rust Async Concurrency without threads"));
        assert!(doc.body.contains("threads Second paragraph"));
        assert!(!doc.body.contains("AsyncConcurrency"));
        assert!(!doc.body.contains("threadsSecond"));
    }

    #[test]
    fn collects_links_with_anchor_text_and_canonicalises_them() {
        let html = "<a href=\"/b?utm_source=x&amp;z=1\">Anchor Words</a>\
            <a href=\"/b?z=1\">duplicate target</a>\
            <a href=\"mailto:x@e.com\">mail</a>\
            <a href=\"#frag\">self</a>\
            <a href=\"https://other.test/x\">other</a>";
        let doc = parse(html, &base("https://e.com/a"));
        assert_eq!(
            doc.links,
            vec!["https://e.com/b?z=1", "https://other.test/x"]
        );
        assert!(doc.anchors.iter().any(|a| a == "Anchor Words"));
    }

    #[test]
    fn excludes_script_style_and_hidden_text() {
        let html = "<body><p>visible</p><script>var x = 'secret';</script>\
            <style>.x{color:red}</style>\
            <div hidden>hidden words</div>\
            <p style=\"display:none\">stylish secret</p>\
            <span aria-hidden=\"true\">aria secret</span></body>";
        let doc = parse(html, &base("https://e.com/"));
        assert!(doc.body.contains("visible"));
        assert!(!doc.body.contains("secret"));
        assert!(!doc.body.contains("hidden words"));
        assert!(!doc.body.contains("stylish"));
        assert!(!doc.body.contains("aria"));
    }

    #[test]
    fn ignores_comments_and_markup() {
        let html = "<!-- ignore me --><p>real <b>text</b></p>";
        let doc = parse(html, &base("https://e.com/"));
        assert!(doc.body.contains("real"));
        assert!(doc.body.contains("text"));
        assert!(!doc.body.contains("ignore me"));
        assert!(!doc.body.contains('<'));
    }

    #[test]
    fn same_host_canonical_hint_is_honoured_cross_host_is_not() {
        let same = "<link rel=\"canonical\" href=\"/real\"><p>x</p>";
        let doc = parse(same, &base("https://e.com/discovered"));
        assert_eq!(doc.canonical, "https://e.com/real");

        let cross = "<link rel=\"canonical\" href=\"https://evil.test/steal\"><p>x</p>";
        let doc = parse(cross, &base("https://e.com/discovered"));
        assert_eq!(doc.canonical, "https://e.com/discovered");
    }

    #[test]
    fn honours_noindex() {
        let html = "<meta name=\"robots\" content=\"noindex, nofollow\"><p>x</p>";
        let doc = parse(html, &base("https://e.com/"));
        assert!(doc.noindex);
    }

    #[test]
    fn extracts_dates_by_priority() {
        let html = "<meta property=\"article:published_time\" content=\"2026-02-18T10:00:00Z\">\
            <time datetime=\"2020-01-01\">old</time><p>x</p>";
        let doc = parse(html, &base("https://e.com/"));
        assert_eq!(doc.published_at, Some(datetime!(2026-02-18 10:00 UTC)));
    }

    #[test]
    fn jsonld_date_beats_meta_date() {
        let html = "<script type=\"application/ld+json\">{\"datePublished\":\"2026-03-01T00:00:00Z\"}</script>\
            <meta property=\"article:published_time\" content=\"2026-02-18T10:00:00Z\"><p>x</p>";
        let doc = parse(html, &base("https://e.com/"));
        assert_eq!(doc.published_at, Some(datetime!(2026-03-01 00:00 UTC)));
    }

    #[test]
    fn url_path_date_is_a_last_resort() {
        let doc = parse("<p>x</p>", &base("https://e.com/2026/02/18/story"));
        assert_eq!(doc.published_at, Some(datetime!(2026-02-18 00:00 UTC)));
    }

    #[test]
    fn no_date_is_never_guessed() {
        let doc = parse("<p>no date anywhere</p>", &base("https://e.com/story/one"));
        assert_eq!(doc.published_at, None);
    }

    #[test]
    fn collects_image_alt_text() {
        let html = "<img src=\"/a.png\" alt=\"A red fox\"><p>x</p>";
        let doc = parse(html, &base("https://e.com/"));
        assert!(doc.alt.contains("A red fox"));
        assert!(doc.links.contains(&"https://e.com/a.png".to_string()));
    }

    #[test]
    fn prefers_landmark_text_as_the_body() {
        let filler = "nav nav nav ".repeat(40);
        let article = "the actual article content ".repeat(20);
        let html = format!("<body><nav>{filler}</nav><article><p>{article}</p></article></body>");
        let doc = parse(&html, &base("https://e.com/"));
        assert!(doc.body.contains("the actual article content"));
    }

    #[test]
    fn content_hash_changes_with_content() {
        let a = parse("<p>one</p>", &base("https://e.com/"));
        let b = parse("<p>two</p>", &base("https://e.com/"));
        assert_ne!(a.content_hash, b.content_hash);
        assert_eq!(a.content_hash.len(), 64);
    }

    #[test]
    fn simhash_is_populated_and_stable() {
        let a = parse("<p>the quick brown fox</p>", &base("https://e.com/"));
        let b = parse("<p>the quick brown fox</p>", &base("https://e.com/"));
        assert_ne!(a.simhash, 0);
        assert_eq!(a.simhash, b.simhash);
    }

    #[test]
    fn sanitises_invisible_characters() {
        let html = "<p>safe\u{202E}evil\u{200B}text</p>";
        let doc = parse(html, &base("https://e.com/"));
        assert_eq!(doc.body, "safeeviltext");
    }

    #[test]
    fn text_cap_truncates_and_flags() {
        let big = format!("<p>{}</p>", "word ".repeat(1000));
        let small = ParseLimits {
            max_text_bytes: 64,
            ..limits()
        };
        let (doc, flags) = parse_with_limits(&big, &base("https://e.com/"), &small);
        assert!(flags.text_truncated);
        assert!(doc.truncated);
        assert!(doc.body.len() <= 64);
    }

    #[test]
    fn link_cap_truncates_and_flags() {
        use std::fmt::Write as _;
        let mut html = String::new();
        for i in 0..50 {
            let _ = write!(html, "<a href=\"/p/{i}\">l</a>");
        }
        let small = ParseLimits {
            max_links: 10,
            ..limits()
        };
        let (doc, flags) = parse_with_limits(&html, &base("https://e.com/"), &small);
        assert!(flags.links_truncated);
        assert_eq!(doc.links.len(), 10);
    }

    #[test]
    fn depth_cap_is_flagged_not_fatal() {
        let deep = format!("{}x{}", "<div>".repeat(40), "</div>".repeat(40));
        let small = ParseLimits {
            max_depth: 8,
            ..limits()
        };
        let (doc, flags) = parse_with_limits(&deep, &base("https://e.com/"), &small);
        assert!(flags.depth_limit_hit);
        assert!(doc.truncated);
        assert!(doc.body.contains('x'));
    }

    #[test]
    fn input_cap_truncates_and_flags() {
        let big = "<p>".to_string() + &"a".repeat(10_000) + "</p>";
        let small = ParseLimits {
            max_input_bytes: 128,
            ..limits()
        };
        let (doc, flags) = parse_with_limits(&big, &base("https://e.com/"), &small);
        assert!(flags.input_truncated);
        assert!(doc.truncated);
    }

    #[test]
    fn malformed_markup_does_not_panic() {
        let html = "<div><p>unclosed<span><a href=\"/x\">link<b>bold</b>\
            </div></p></span></a><<<>><title>ok";
        let doc = parse(html, &base("https://e.com/"));
        assert!(doc.body.contains("unclosed"));
        assert!(doc.body.contains("ok"));
    }

    #[test]
    fn empty_input_is_an_empty_document() {
        let doc = parse("", &base("https://e.com/"));
        assert!(doc.body.is_empty());
        assert!(doc.title.is_empty());
        assert!(doc.links.is_empty());
        assert!(!doc.truncated);
    }

    #[test]
    fn heading_text_in_inline_elements_is_kept() {
        let doc = parse("<h1>A <em>fancy</em> heading</h1>", &base("https://e.com/"));
        assert_eq!(doc.headings, vec!["A fancy heading"]);
    }

    #[test]
    fn immediately_closed_paragraphs_settle_cleanly() {
        let html = "<p>first<p>second<p>third";
        let doc = parse(html, &base("https://e.com/"));
        assert!(doc.body.contains("first"));
        assert!(doc.body.contains("second"));
        assert!(doc.body.contains("third"));
    }

    #[test]
    fn nested_same_skip_element_does_not_leak_content() {
        let html = "<template>outer<template>inner</template>still hidden</template><p>visible</p>";
        let doc = parse(html, &base("https://e.com/"));
        assert!(doc.body.contains("visible"));
        assert!(!doc.body.contains("inner"));
        assert!(!doc.body.contains("hidden"));
    }

    #[test]
    fn jsonld_headline_is_a_title_fallback() {
        let html = "<script type=\"application/ld+json\">\
            {\"headline\":\"JSON-LD Headline\"}</script><p>body</p>";
        let doc = parse(html, &base("https://e.com/"));
        assert_eq!(doc.title, "JSON-LD Headline");
    }
}
