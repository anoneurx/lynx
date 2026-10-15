//! Internal domain types shared by the query and crawl planes.
//!
//! These are not the public API. See [`crate::api`] for the outward-facing contract.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub use lynx_common::ids::{DocumentId, DomainId, HostId, JobId, UrlId};

/// A registrable domain (`example.com`), the unit of authority and budget decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Domain {
    /// Identity.
    pub id: DomainId,
    /// The registrable domain, lowercased ASCII (punycode if internationalised).
    pub name: String,
}

/// A specific host (`docs.example.com`), the unit of politeness and safety decisions.
///
/// Host and domain are separate on purpose: a parked domain must not inherit a
/// subdomain's crawl budget, and robots is per-host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    /// Identity.
    pub id: HostId,
    /// The owning registrable domain.
    pub domain_id: DomainId,
    /// The full host name, lowercased ASCII.
    pub name: String,
}

/// A canonical URL and its crawl lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Url {
    /// Identity.
    pub id: UrlId,
    /// The owning host.
    pub host_id: HostId,
    /// The canonicalised URL string.
    pub canonical: String,
    /// When the URL was first discovered.
    pub discovered_at: OffsetDateTime,
}

/// The current indexed state of a URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// Identity, derived from the canonical URL.
    pub id: DocumentId,
    /// The canonical URL this document represents.
    pub url_id: UrlId,
    /// Extracted title.
    pub title: String,
    /// Content hash of the latest version.
    pub content_hash: String,
    /// Language tag if detected.
    pub lang: Option<String>,
    /// The index generation that currently serves this document.
    pub indexed_generation: u64,
    /// When the document was last updated.
    pub updated_at: OffsetDateTime,
}

/// One crawl of one URL: content hash plus extracted, bounded content.
///
/// Only the newest version is indexed; the rest is retained for history and audits
/// according to the retention class in force.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentVersion {
    /// The document this version belongs to.
    pub document_id: DocumentId,
    /// Monotonic version counter per document.
    pub version: u32,
    /// Content hash of the fetched body.
    pub content_hash: String,
    /// When this version was fetched.
    pub fetched_at: OffsetDateTime,
    /// HTTP status of the fetch.
    pub status_code: u16,
}

/// A directed edge in the link graph, from one document to a target URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// Source document.
    pub from: DocumentId,
    /// Target canonical URL.
    pub to_canonical: String,
    /// Anchor text, cleaned and bounded.
    pub anchor: String,
}

/// The parser → indexer contract: a bounded, sanitised document ready to index.
///
/// Every field is owned and bounded: no raw markup, no script text, no blob survives
/// extraction, so the hostile input buffer can be dropped as soon as parsing returns.
/// Defaulted so tests and callers can populate only the fields they exercise; new
/// optional signals can be added without breaking existing literals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ParsedDocument {
    /// Canonical URL.
    pub canonical: String,
    /// Title text.
    pub title: String,
    /// Heading text, one entry per heading.
    pub headings: Vec<String>,
    /// Body text, cleaned and bounded.
    pub body: String,
    /// Anchor text of the links this page carries, used to seed inbound anchors.
    pub anchors: Vec<String>,
    /// Outbound links discovered on the page, canonicalised and deduplicated.
    pub links: Vec<String>,
    /// Detected language tag.
    pub lang: Option<String>,
    /// Content hash of the source body.
    pub content_hash: String,
    /// Meta description or Open Graph description, if any.
    pub description: Option<String>,
    /// Image alt text and captions.
    pub alt: String,
    /// Publication time, when a trusted source declared one.
    pub published_at: Option<OffsetDateTime>,
    /// Last modified time, when a trusted source declared one.
    pub modified_at: Option<OffsetDateTime>,
    /// The document asked not to be indexed (`<meta name=robots content=noindex>`).
    pub noindex: bool,
    /// A parser limit was hit and some text or link was dropped.
    pub truncated: bool,
    /// 64-bit shingled `SimHash` over the body, for near-duplicate candidate lookup.
    pub simhash: u64,
}

/// Identifies which shards, tokenizer, and ranking config produced an index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexManifest {
    /// Monotonic generation number.
    pub generation: u64,
    /// Number of shards.
    pub shards: u32,
    /// Tokenizer/schema version.
    pub schema_version: String,
    /// Ranking config version (bumps on any weight change).
    pub ranking_config_version: String,
    /// When the generation finished building.
    pub built_at: OffsetDateTime,
}

/// A frontier queue entry: a URL waiting to be crawled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrawlFrontierItem {
    /// Job identity.
    pub id: JobId,
    /// The target URL.
    pub url_id: UrlId,
    /// Higher priority is claimed first.
    pub priority: i32,
    /// Earliest time the URL may be fetched.
    pub scheduled_at: OffsetDateTime,
    /// Attempts made so far.
    pub attempts: u32,
}

/// Outcome of one fetch attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrawlAttempt {
    /// The URL attempted.
    pub url_id: UrlId,
    /// When the attempt started.
    pub started_at: OffsetDateTime,
    /// Outcome class (never a raw upstream message, which can contain the URL).
    pub outcome: AttemptOutcome,
    /// Bytes received, if any.
    pub bytes: u64,
    /// HTTP status, if a response was received.
    pub status_code: Option<u16>,
}

/// Closed set of attempt outcomes, safe to log and aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[non_exhaustive]
pub enum AttemptOutcome {
    /// Fetched and parsed successfully.
    Ok,
    /// robots.txt disallowed.
    RobotsDenied,
    /// Blocked by the fetch safety gate.
    SafetyBlocked,
    /// HTTP 4xx.
    ClientError,
    /// HTTP 5xx.
    ServerError,
    /// Timed out.
    Timeout,
    /// Exceeded a byte/decompression cap.
    CapExceeded,
    /// Network error.
    Network,
}

/// Parsed robots.txt policy for one host and one user-agent group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotsPolicy {
    /// The host this policy applies to.
    pub host_id: HostId,
    /// `Crawl-delay` in seconds, if declared.
    pub crawl_delay_seconds: Option<u64>,
    /// Disallowed path prefixes for our user-agent group.
    pub disallow: Vec<String>,
    /// When the policy was fetched.
    pub fetched_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn document_id_is_derived_from_canonical_url() {
        let a = Document {
            id: DocumentId::from_canonical_url("https://e.com/x"),
            url_id: UrlId::new(),
            title: "X".into(),
            content_hash: "h".into(),
            lang: Some("en".into()),
            indexed_generation: 1,
            updated_at: datetime!(2026-02-18 00:00 UTC),
        };
        let b = DocumentId::from_canonical_url("https://e.com/x");
        assert_eq!(a.id, b);
    }

    #[test]
    fn attempt_outcome_serialises_to_stable_strings() {
        let s = serde_json::to_string(&AttemptOutcome::CapExceeded).unwrap();
        assert_eq!(s, "\"CAP_EXCEEDED\"");
    }

    #[test]
    fn parsed_document_round_trips() {
        let doc = ParsedDocument {
            canonical: "https://e.com/".into(),
            title: "T".into(),
            headings: vec!["H".into()],
            body: "body".into(),
            anchors: vec![],
            links: vec!["https://e.com/a".into()],
            lang: None,
            content_hash: "deadbeef".into(),
            description: Some("d".into()),
            alt: "an image".into(),
            published_at: Some(datetime!(2026-02-18 00:00 UTC)),
            modified_at: None,
            noindex: false,
            truncated: true,
            simhash: 42,
        };
        let json = serde_json::to_string(&doc).unwrap();
        let back: ParsedDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(doc, back);
    }

    #[test]
    fn parsed_document_defaults_deserialise_from_a_minimal_object() {
        let doc: ParsedDocument =
            serde_json::from_str(r#"{"canonical":"https://e.com/"}"#).unwrap();
        assert_eq!(doc.canonical, "https://e.com/");
        assert!(!doc.noindex);
        assert_eq!(doc.simhash, 0);
    }
}
