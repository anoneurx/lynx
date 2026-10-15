//! The tokenizer sink that turns hostile markup into bounded fields.
//!
//! html5ever's [`Tokenizer`] is fed the whole document and emits tags, text, comments,
//! and doctypes in order. We never build a DOM: a token stream with an explicit open
//! element stack is enough for text, headings, links, and metadata, and it cannot be
//! blown up by deep or repeated nesting beyond the limits we enforce
//! (`docs/crawler/parser.md` § Text extraction).
//!
//! The sink's `process_token` takes `&self`, so all mutable state lives behind a
//! [`RefCell`]. That is the price of the trait, and it is contained to this module.

use std::cell::RefCell;
use std::time::Instant;

use html5ever::buffer_queue::BufferQueue;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use html5ever::{LocalName, TokenizerResult};
use lynx_common::hash::ContentHash;
use lynx_common::text;
use lynx_models::domain::ParsedDocument;
use time::format_description::well_known::{Iso8601, Rfc3339};
use time::Month;
use time::OffsetDateTime;
use url::Url;

use crate::limits::{ParseFlags, ParseLimits};
use crate::simhash;

/// Elements that never have children and take no end tag.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Elements whose text is never indexed. The content is dropped, not sanitised.
const SKIP: &[&str] = &["script", "style", "noscript", "template", "svg", "iframe"];

/// Elements whose text is a heading.
const HEADINGS: &[&str] = &["h1", "h2", "h3", "h4", "h5", "h6"];

/// Landmarks whose text is preferred as the main content.
const MAINLAND: &[&str] = &["main", "article"];

/// Block-level elements that implicitly close an open paragraph and other block starts.
const BLOCK: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "ul",
];

/// Maximum graphemes kept for the title.
pub const MAX_TITLE_CHARS: usize = 512;
/// Maximum graphemes kept for a description.
pub const MAX_DESCRIPTION_CHARS: usize = 1024;
/// Maximum graphemes kept for one heading.
pub const MAX_HEADING_CHARS: usize = 512;
/// Maximum graphemes kept for one anchor text.
pub const MAX_ANCHOR_CHARS: usize = 512;
/// Maximum headings retained.
pub const MAX_HEADINGS: usize = 200;
/// Minimum main-content length before it is preferred over the full text.
const MIN_MAIN_CHARS: usize = 100;

/// A frame in the excluded-subtree stack.
struct SkipFrame {
    name: LocalName,
    capture_jsonld: bool,
}

/// One settled anchor start, accumulating its visible text.
#[derive(Default)]
struct AnchorFrame {
    text: String,
}

/// Everything the extractor accumulates, plus the limits it runs under.
struct State {
    base: Url,
    limits: ParseLimits,
    started: Instant,
    flags: ParseFlags,
    aborted: bool,

    stack: Vec<LocalName>,
    skip: Vec<SkipFrame>,
    nodes: u64,

    title_depth: usize,
    title_buf: String,
    heading_depth: usize,
    heading_buf: String,
    headings: Vec<String>,
    main_depth: usize,
    current_anchor: Option<AnchorFrame>,

    full_text: String,
    main_text: String,
    alt_buf: String,
    jsonld_buf: String,

    links: Vec<String>,
    seen_links: std::collections::HashSet<String>,
    anchor_texts: Vec<String>,

    description: Option<String>,
    og_description: Option<String>,
    twitter_description: Option<String>,
    jsonld_description: Option<String>,
    og_title: Option<String>,
    twitter_title: Option<String>,
    jsonld_title: Option<String>,
    noindex: bool,
    lang: Option<String>,
    canonical_hint: Option<String>,

    jsonld_published: Option<OffsetDateTime>,
    jsonld_modified: Option<OffsetDateTime>,
    meta_published: Option<OffsetDateTime>,
    meta_modified: Option<OffsetDateTime>,
    time_dt: Option<OffsetDateTime>,
}

/// The tokenizer sink.
pub(crate) struct Extractor {
    state: RefCell<State>,
}

impl TokenSink for Extractor {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
        let mut state = self.state.borrow_mut();
        state.tick();
        if state.aborted {
            return TokenSinkResult::Continue;
        }
        match token {
            Token::TagToken(tag) => state.on_tag(&tag),
            Token::CharacterTokens(chars) => state.on_text(&chars),
            // Comments, doctypes, parse errors, and nulls carry no indexable content.
            _ => {}
        }
        TokenSinkResult::Continue
    }

    fn end(&self) {
        // Buffers are flushed in `finish`; nothing to do here, but the hook is kept so
        // the intent is explicit.
    }
}

/// Parse `input`, resolving relative URLs against `base`, under `limits`.
pub(crate) fn run(input: &str, base: &Url, limits: &ParseLimits) -> (ParsedDocument, ParseFlags) {
    let (input, input_truncated) = clamp_input(input, limits.max_input_bytes);
    let mut state = State::new(base, limits);
    if input_truncated {
        state.flags.input_truncated = true;
    }

    let tokenizer = Tokenizer::new(
        Extractor {
            state: RefCell::new(state),
        },
        TokenizerOpts::default(),
    );
    let buffer = BufferQueue::default();
    buffer.push_back(StrTendril::from(input));
    loop {
        if matches!(tokenizer.feed(&buffer), TokenizerResult::Done) {
            break;
        }
    }
    tokenizer.end();

    let extractor = tokenizer.sink;
    let state = extractor.state.into_inner();
    state.finish(input)
}

/// Truncate `input` to at most `max` bytes on a char boundary.
fn clamp_input(input: &str, max: usize) -> (&str, bool) {
    if input.len() <= max {
        return (input, false);
    }
    let mut end = max;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    (&input[..end], true)
}

impl State {
    fn new(base: &Url, limits: &ParseLimits) -> Self {
        Self {
            base: base.clone(),
            limits: *limits,
            started: Instant::now(),
            flags: ParseFlags::default(),
            aborted: false,
            stack: Vec::new(),
            skip: Vec::new(),
            nodes: 0,
            title_depth: 0,
            title_buf: String::new(),
            heading_depth: 0,
            heading_buf: String::new(),
            headings: Vec::new(),
            main_depth: 0,
            current_anchor: None,
            full_text: String::new(),
            main_text: String::new(),
            alt_buf: String::new(),
            jsonld_buf: String::new(),
            links: Vec::new(),
            seen_links: std::collections::HashSet::new(),
            anchor_texts: Vec::new(),
            description: None,
            og_description: None,
            twitter_description: None,
            jsonld_description: None,
            og_title: None,
            twitter_title: None,
            jsonld_title: None,
            noindex: false,
            lang: None,
            canonical_hint: None,
            jsonld_published: None,
            jsonld_modified: None,
            meta_published: None,
            meta_modified: None,
            time_dt: None,
        }
    }

    /// Cooperatively stop once the wall-clock budget is spent.
    fn tick(&mut self) {
        if !self.aborted && self.started.elapsed() > self.limits.max_duration {
            self.aborted = true;
            self.flags.duration_limit_hit = true;
        }
    }

    fn on_tag(&mut self, tag: &Tag) {
        if let Some(frame) = self.skip.last() {
            let top = frame.name.clone();
            if tag.name == top {
                match tag.kind {
                    TagKind::StartTag => {
                        if !is_void(&tag.name) {
                            self.skip.push(SkipFrame {
                                name: tag.name.clone(),
                                capture_jsonld: false,
                            });
                        }
                    }
                    TagKind::EndTag => {
                        if frame.capture_jsonld {
                            let raw = std::mem::take(&mut self.jsonld_buf);
                            self.parse_jsonld(&raw);
                        }
                        self.skip.pop();
                    }
                }
            }
            return;
        }

        match tag.kind {
            TagKind::StartTag => self.on_start(tag),
            TagKind::EndTag => self.on_end(tag),
        }
    }

    fn on_start(&mut self, tag: &Tag) {
        let name = tag.name.as_ref();
        self.nodes += 1;
        if self.nodes > self.limits.max_nodes {
            self.flags.node_limit_hit = true;
        }

        // Metadata-only elements are read regardless of structure.
        match name {
            "html" => {
                if let Some(lang) = attr(tag, "lang") {
                    self.set_lang(lang);
                }
            }
            "meta" => self.on_meta(tag),
            "link" => self.on_link(tag),
            "time" => {
                if self.time_dt.is_none() {
                    if let Some(dt) = attr(tag, "datetime").and_then(parse_datetime) {
                        self.time_dt = Some(dt);
                    }
                }
            }
            "img" => {
                if let Some(alt) = attr(tag, "alt") {
                    let cap = self.limits.max_metadata_bytes;
                    push_capped(
                        &mut self.alt_buf,
                        alt,
                        cap,
                        &mut self.flags.metadata_truncated,
                    );
                    push_capped(
                        &mut self.alt_buf,
                        " ",
                        cap,
                        &mut self.flags.metadata_truncated,
                    );
                }
                if let Some(src) = attr(tag, "src") {
                    if let Some(canonical) = self.link_from(src) {
                        self.add_link(canonical);
                    }
                }
            }
            _ => {}
        }

        let hidden = has_hidden(tag);
        if SKIP.contains(&name) || hidden {
            if let Some(src) = attr(tag, "src") {
                if let Some(canonical) = self.link_from(src) {
                    self.add_link(canonical);
                }
            }
            if is_void(&tag.name) {
                return;
            }
            let capture_jsonld = name == "script"
                && attr(tag, "type").is_some_and(|t| t.eq_ignore_ascii_case("application/ld+json"));
            self.skip.push(SkipFrame {
                name: tag.name.clone(),
                capture_jsonld,
            });
            return;
        }

        self.implicit_close_for(name);

        if is_block_edge(name) {
            self.separate_blocks();
        }

        if name == "title" {
            if self.title_depth == 0 {
                self.title_buf.clear();
            }
            self.title_depth += 1;
        }
        if HEADINGS.contains(&name) {
            if self.heading_depth == 0 {
                self.heading_buf.clear();
            }
            self.heading_depth += 1;
        }
        if MAINLAND.contains(&name) {
            self.main_depth += 1;
        }
        if name == "a" || name == "area" {
            if let Some(href) = attr(tag, "href") {
                if let Some(canonical) = self.link_from(href) {
                    self.add_link(canonical);
                    if name == "a" {
                        self.current_anchor = Some(AnchorFrame::default());
                    }
                }
            }
        }

        if is_void(&tag.name) {
            return;
        }
        if self.stack.len() >= self.limits.max_depth {
            self.flags.depth_limit_hit = true;
        } else {
            self.stack.push(tag.name.clone());
        }
    }

    fn on_end(&mut self, tag: &Tag) {
        if is_void(&tag.name) {
            return;
        }
        if let Some(pos) = self
            .stack
            .iter()
            .rposition(|open| open.as_ref() == tag.name.as_ref())
        {
            while self.stack.len() > pos {
                self.pop_top();
            }
        }
    }

    /// Pop elements this start tag implicitly ends, then close an open paragraph or anchor.
    fn implicit_close_for(&mut self, name: &str) {
        let closes: &[&str] = match name {
            "li" => &["li"],
            "dt" | "dd" => &["dt", "dd"],
            "p" => &["p"],
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => &["h1", "h2", "h3", "h4", "h5", "h6"],
            "tr" => &["tr", "td", "th"],
            "td" | "th" => &["td", "th"],
            "option" => &["option"],
            _ => &[],
        };
        loop {
            let close = self
                .stack
                .last()
                .is_some_and(|top| closes.contains(&top.as_ref()));
            if close {
                self.pop_top();
            } else {
                break;
            }
        }
        let closes_paragraph = BLOCK.contains(&name);
        loop {
            let close = self
                .stack
                .last()
                .is_some_and(|top| top.as_ref() == "p" && closes_paragraph);
            if close {
                self.pop_top();
            } else {
                break;
            }
        }
        if (name == "a" || name == "area")
            && self.stack.last().is_some_and(|top| top.as_ref() == "a")
        {
            self.pop_top();
        }
    }

    fn pop_top(&mut self) {
        if let Some(name) = self.stack.pop() {
            self.leave(name.as_ref());
        }
    }

    fn leave(&mut self, name: &str) {
        if is_block_edge(name) {
            self.separate_blocks();
        }
        if name == "title" {
            self.title_depth = self.title_depth.saturating_sub(1);
        }
        if HEADINGS.contains(&name) {
            self.heading_depth = self.heading_depth.saturating_sub(1);
            if self.heading_depth == 0 {
                self.flush_heading();
            }
        }
        if MAINLAND.contains(&name) {
            self.main_depth = self.main_depth.saturating_sub(1);
        }
        if name == "a" || name == "area" {
            self.finish_anchor();
        }
    }

    fn flush_heading(&mut self) {
        let text = sanitize(&self.heading_buf);
        self.heading_buf.clear();
        if !text.is_empty() && self.headings.len() < MAX_HEADINGS {
            self.headings
                .push(text::truncate_graphemes(&text, MAX_HEADING_CHARS));
        }
    }

    fn finish_anchor(&mut self) {
        if let Some(frame) = self.current_anchor.take() {
            let text = sanitize(&frame.text);
            if !text.is_empty() && self.anchor_texts.len() < self.limits.max_links {
                self.anchor_texts
                    .push(text::truncate_graphemes(&text, MAX_ANCHOR_CHARS));
            }
        }
    }

    /// Keep a token boundary at a block edge, so minified markup (`</h1><p>`) cannot glue
    /// two words together in the extracted text.
    fn separate_blocks(&mut self) {
        for buf in [&mut self.full_text, &mut self.main_text] {
            if !buf.is_empty() && !buf.ends_with(char::is_whitespace) {
                buf.push(' ');
            }
        }
    }

    fn on_text(&mut self, chars: &str) {
        if let Some(frame) = self.skip.last_mut() {
            if frame.capture_jsonld {
                let cap = self.limits.max_metadata_bytes;
                push_capped(
                    &mut self.jsonld_buf,
                    chars,
                    cap,
                    &mut self.flags.metadata_truncated,
                );
            }
            return;
        }

        let meta = self.limits.max_metadata_bytes;
        let text_cap = self.limits.max_text_bytes;

        if self.title_depth > 0 {
            push_capped(
                &mut self.title_buf,
                chars,
                meta,
                &mut self.flags.metadata_truncated,
            );
        }
        if self.heading_depth > 0 {
            push_capped(
                &mut self.heading_buf,
                chars,
                meta,
                &mut self.flags.metadata_truncated,
            );
        }
        if let Some(frame) = self.current_anchor.as_mut() {
            push_capped(
                &mut frame.text,
                chars,
                meta,
                &mut self.flags.metadata_truncated,
            );
        }
        if self.main_depth > 0 {
            push_capped(
                &mut self.main_text,
                chars,
                text_cap,
                &mut self.flags.text_truncated,
            );
        }
        push_capped(
            &mut self.full_text,
            chars,
            text_cap,
            &mut self.flags.text_truncated,
        );
    }

    fn on_meta(&mut self, tag: &Tag) {
        let content = attr(tag, "content").unwrap_or("");
        let Some(key) = attr(tag, "name")
            .or_else(|| attr(tag, "property"))
            .or_else(|| attr(tag, "itemprop"))
        else {
            if attr(tag, "http-equiv").is_some_and(|v| v.eq_ignore_ascii_case("content-language")) {
                self.set_lang(content);
            }
            return;
        };
        match key.to_ascii_lowercase().as_str() {
            "description" if self.description.is_none() => self.description = non_empty(content),
            "robots" => {
                let lower = content.to_ascii_lowercase();
                if lower.split([',', ' ']).any(|t| t.trim() == "noindex") {
                    self.noindex = true;
                }
            }
            "og:title" if self.og_title.is_none() => self.og_title = non_empty(content),
            "og:description" if self.og_description.is_none() => {
                self.og_description = non_empty(content);
            }
            "og:url" if self.canonical_hint.is_none() => self.canonical_hint = non_empty(content),
            "twitter:title" if self.twitter_title.is_none() => {
                self.twitter_title = non_empty(content);
            }
            "twitter:description" if self.twitter_description.is_none() => {
                self.twitter_description = non_empty(content);
            }
            "article:published_time" if self.meta_published.is_none() => {
                self.meta_published = parse_datetime(content);
            }
            "article:modified_time" if self.meta_modified.is_none() => {
                self.meta_modified = parse_datetime(content);
            }
            "content-language" => self.set_lang(content),
            // keywords are deliberately ignored for ranking.
            _ => {}
        }
    }

    fn on_link(&mut self, tag: &Tag) {
        let Some(href) = attr(tag, "href") else {
            return;
        };
        let rel = attr(tag, "rel").unwrap_or("").to_ascii_lowercase();
        let rels: Vec<&str> = rel.split_whitespace().collect();
        if rels.contains(&"canonical") {
            if self.canonical_hint.is_none() {
                self.canonical_hint = non_empty(href);
            }
            return;
        }
        if rels.contains(&"alternate") {
            if let Some(hreflang) = attr(tag, "hreflang") {
                self.set_lang(hreflang);
            }
        }
        if let Some(canonical) = self.link_from(href) {
            self.add_link(canonical);
        }
    }

    /// Resolve and canonicalise an href/src, or `None` when it is not a usable web URL.
    fn link_from(&self, raw: &str) -> Option<String> {
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with('#') {
            return None;
        }
        let joined = self.base.join(raw).ok()?;
        if !matches!(joined.scheme(), "http" | "https") {
            return None;
        }
        lynx_common::urlnorm::normalise(joined.as_str()).ok()
    }

    fn add_link(&mut self, canonical: String) {
        if !self.seen_links.insert(canonical.clone()) {
            return;
        }
        if self.links.len() >= self.limits.max_links {
            self.flags.links_truncated = true;
            return;
        }
        self.links.push(canonical);
    }

    fn set_lang(&mut self, raw: &str) {
        if self.lang.is_some() {
            return;
        }
        let value = raw
            .split([';', ','])
            .next()
            .unwrap_or(raw)
            .trim()
            .to_ascii_lowercase();
        let valid = !value.is_empty()
            && value.len() <= 35
            && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if valid {
            self.lang = Some(value);
        }
    }

    /// Walk JSON-LD with a node budget, taking the first value seen for each field.
    fn parse_jsonld(&mut self, raw: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
            return;
        };
        let mut budget = self.limits.max_jsonld_nodes.max(1);
        self.walk_jsonld(&value, &mut budget);
    }

    fn walk_jsonld(&mut self, value: &serde_json::Value, budget: &mut u64) {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        match value {
            serde_json::Value::Object(map) => {
                for (key, val) in map {
                    match key.as_str() {
                        "datePublished" if self.jsonld_published.is_none() => {
                            self.jsonld_published = val.as_str().and_then(parse_datetime);
                        }
                        "dateModified" if self.jsonld_modified.is_none() => {
                            self.jsonld_modified = val.as_str().and_then(parse_datetime);
                        }
                        "headline" if self.jsonld_title.is_none() => {
                            self.jsonld_title = val.as_str().and_then(non_empty);
                        }
                        "description" if self.jsonld_description.is_none() => {
                            self.jsonld_description = val.as_str().and_then(non_empty);
                        }
                        _ => {}
                    }
                    self.walk_jsonld(val, budget);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    self.walk_jsonld(item, budget);
                }
            }
            _ => {}
        }
    }

    fn canonical(&self) -> String {
        let discovered = lynx_common::urlnorm::normalise(self.base.as_str())
            .unwrap_or_else(|_| self.base.to_string());
        if let Some(hint) = &self.canonical_hint {
            if let Ok(hint_url) = self.base.join(hint) {
                if hint_url.host_str() == self.base.host_str() {
                    if let Ok(canonical) = lynx_common::urlnorm::normalise(hint_url.as_str()) {
                        return canonical;
                    }
                }
            }
        }
        discovered
    }

    fn pick_title(&self) -> String {
        let raw = first_non_empty([
            self.title_buf.as_str(),
            self.og_title.as_deref().unwrap_or(""),
            self.twitter_title.as_deref().unwrap_or(""),
            self.jsonld_title.as_deref().unwrap_or(""),
        ]);
        text::truncate_graphemes(&sanitize(raw), MAX_TITLE_CHARS)
    }

    fn pick_description(&self) -> Option<String> {
        let raw = first_non_empty([
            self.description.as_deref().unwrap_or(""),
            self.og_description.as_deref().unwrap_or(""),
            self.twitter_description.as_deref().unwrap_or(""),
            self.jsonld_description.as_deref().unwrap_or(""),
        ]);
        let clean = sanitize(raw);
        if clean.is_empty() {
            None
        } else {
            Some(text::truncate_graphemes(&clean, MAX_DESCRIPTION_CHARS))
        }
    }

    fn published(&self) -> Option<OffsetDateTime> {
        self.jsonld_published
            .or(self.meta_published)
            .or(self.time_dt)
            .or_else(|| url_path_date(&self.base))
    }

    fn modified(&self) -> Option<OffsetDateTime> {
        self.jsonld_modified.or(self.meta_modified)
    }

    fn finish(mut self, input: &str) -> (ParsedDocument, ParseFlags) {
        // Flush anything still open at EOF.
        if self.heading_depth > 0 {
            self.heading_depth = 0;
            self.flush_heading();
        }
        self.finish_anchor();
        if !self.jsonld_buf.is_empty() {
            let raw = std::mem::take(&mut self.jsonld_buf);
            self.parse_jsonld(&raw);
        }

        let full = sanitize(&self.full_text);
        let main = sanitize(&self.main_text);
        let body = choose_body(main, &full);
        let content_hash = ContentHash::of(input.as_bytes()).as_str().to_string();
        let simhash = simhash::of_text(&body);

        // Compute display fields before moving any owned field out of `self`.
        let canonical = self.canonical();
        let title = self.pick_title();
        let description = self.pick_description();
        let alt = sanitize(&self.alt_buf);
        let published_at = self.published();
        let modified_at = self.modified();

        let doc = ParsedDocument {
            canonical,
            title,
            headings: self.headings,
            body,
            anchors: self.anchor_texts,
            links: self.links,
            lang: self.lang.clone(),
            content_hash,
            description,
            alt,
            published_at,
            modified_at,
            noindex: self.noindex,
            truncated: self.flags.any(),
            simhash,
        };
        (doc, self.flags)
    }
}

/// Clean and NFC-normalise a display field.
fn sanitize(input: &str) -> String {
    text::nfc(&text::clean(input))
}

fn first_non_empty<'a>(items: impl IntoIterator<Item = &'a str>) -> &'a str {
    items
        .into_iter()
        .find(|s| !s.trim().is_empty())
        .unwrap_or("")
}

fn non_empty(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Prefer the landmark text when it is substantial enough to be the page's content.
fn choose_body(main: String, full: &str) -> String {
    if main.chars().count() >= MIN_MAIN_CHARS && main.len() * 3 >= full.len() {
        main
    } else {
        full.to_string()
    }
}

/// Append `s` up to `cap` bytes, marking truncation when something is dropped.
fn push_capped(dst: &mut String, s: &str, cap: usize, truncated: &mut bool) {
    if dst.len() >= cap {
        if !s.is_empty() {
            *truncated = true;
        }
        return;
    }
    let room = cap - dst.len();
    if s.len() <= room {
        dst.push_str(s);
        return;
    }
    let mut end = room;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    dst.push_str(&s[..end]);
    *truncated = true;
}

fn is_void(name: &LocalName) -> bool {
    VOID.contains(&name.as_ref())
}

/// True for element edges that should keep a token boundary in the extracted text: block
/// elements, line breaks, and links.
fn is_block_edge(name: &str) -> bool {
    BLOCK.contains(&name) || matches!(name, "br" | "a" | "area")
}

/// True when the element is hidden from users and should not contribute text.
fn has_hidden(tag: &Tag) -> bool {
    for a in &tag.attrs {
        match a.name.local.as_ref() {
            "hidden" => return true,
            "aria-hidden" => {
                if a.value.eq_ignore_ascii_case("true") {
                    return true;
                }
            }
            "style" => {
                let style = a.value.to_ascii_lowercase().replace(' ', "");
                if style.contains("display:none") || style.contains("visibility:hidden") {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Get an attribute value by (lowercase, unnamespaced) name.
fn attr<'a>(tag: &'a Tag, name: &str) -> Option<&'a str> {
    tag.attrs
        .iter()
        .find(|a| a.name.local.as_ref() == name)
        .map(|a| &*a.value)
}

/// Parse an ISO-8601 / RFC-3339 timestamp or a bare `YYYY-MM-DD` date.
fn parse_datetime(raw: &str) -> Option<OffsetDateTime> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = OffsetDateTime::parse(s, &Rfc3339) {
        return Some(dt);
    }
    if let Ok(dt) = OffsetDateTime::parse(s, &Iso8601::DEFAULT) {
        return Some(dt);
    }
    if let Ok(date) = time::Date::parse(s, &Iso8601::DEFAULT) {
        return Some(date.midnight().assume_utc());
    }
    None
}

/// Extract a date from `/YYYY/MM/DD/` or `/YYYY-MM-DD/` path segments, a last resort
/// that is still a real date and never a filename guess (`docs/crawler/parser.md` § Dates).
fn url_path_date(url: &Url) -> Option<OffsetDateTime> {
    let segments: Vec<&str> = url.path().split('/').filter(|s| !s.is_empty()).collect();

    for window in segments.windows(3) {
        if let (Some(y), Some(m), Some(d)) = (
            parse_number(window[0], 4),
            parse_number(window[1], 2),
            parse_number(window[2], 2),
        ) {
            if let Some(date) = calendar_date(y, m, d) {
                return Some(date);
            }
        }
    }

    for segment in segments {
        let bytes = segment.as_bytes();
        if bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' {
            if let (Some(y), Some(m), Some(d)) = (
                parse_number(&segment[0..4], 4),
                parse_number(&segment[5..7], 2),
                parse_number(&segment[8..10], 2),
            ) {
                if let Some(date) = calendar_date(y, m, d) {
                    return Some(date);
                }
            }
        }
    }
    None
}

fn parse_number(s: &str, digits: usize) -> Option<u32> {
    if s.len() != digits || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn calendar_date(year: u32, month: u32, day: u32) -> Option<OffsetDateTime> {
    let year = i32::try_from(year).ok()?;
    let month = u8::try_from(month).ok()?;
    let day = u8::try_from(day).ok()?;
    let month = Month::try_from(month).ok()?;
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    Some(date.midnight().assume_utc())
}
