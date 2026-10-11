//! The parsed and compiled query, and its component types.
//!
//! [`QueryPlan`] is the boundary object between the query processor and the index. It
//! is serialisable so `/explain` can show exactly what was understood, and it is
//! deliberately the *only* shape derived from a query — there is no field that holds the
//! raw string beyond its length.

use serde::{Deserialize, Serialize};
use time::Date;

/// A single query term. Most parsing produces [`Term::Exact`]; the other variants exist
/// for the correction and expansion stages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Term {
    /// A literal term.
    Exact(String),
    /// A prefix term. Only used by suggestions, never by result retrieval.
    Prefix(String),
    /// A fuzzy term, edit-distance-gated.
    Fuzzy(String),
    /// A term replaced by a dictionary expansion, with weight.
    Expanded {
        /// The term the user typed.
        original: String,
        /// The term actually searched for.
        expansion: String,
        /// Weight applied to the expansion (< 1.0).
        weight: f32,
    },
    /// A term corrected for spelling, with weight.
    Corrected {
        /// The term the user typed.
        original: String,
        /// The corrected term.
        corrected: String,
        /// Weight applied to the correction.
        weight: f32,
    },
}

impl Term {
    /// The string this term searches for.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Exact(s) | Self::Prefix(s) | Self::Fuzzy(s) => s,
            Self::Expanded { expansion, .. } => expansion,
            Self::Corrected { corrected, .. } => corrected,
        }
    }
}

/// An exact phrase, optionally with slop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phrase {
    /// The phrase text, whitespace-normalised.
    pub text: String,
    /// Allowed term gap; 0 for an exact adjacency match.
    pub slop: u32,
}

/// A recency window for `freshness:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessWindow {
    /// Last hour.
    Hour,
    /// Last day.
    Day,
    /// Last week.
    Week,
    /// Last month.
    Month,
    /// Last year.
    Year,
}

impl FreshnessWindow {
    /// Parse a window name, case-insensitively.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "hour" | "h" | "1h" => Some(Self::Hour),
            "day" | "d" | "1d" => Some(Self::Day),
            "week" | "w" | "1w" => Some(Self::Week),
            "month" | "m" | "1m" => Some(Self::Month),
            "year" | "y" | "1y" => Some(Self::Year),
            _ => None,
        }
    }

    /// The window's approximate length in seconds, for computing a cutoff.
    #[must_use]
    pub const fn approx_seconds(self) -> i64 {
        match self {
            Self::Hour => 3_600,
            Self::Day => 86_400,
            Self::Week => 604_800,
            Self::Month => 2_592_000,
            Self::Year => 31_536_000,
        }
    }
}

/// Hard filters. A document that fails any of these is excluded, not merely downranked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filters {
    /// Restrict to a host or registrable domain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// Substring match on the URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inurl: Option<String>,
    /// Terms that must appear in the title (a strong signal, not a filter, but retained
    /// here for the title-boost stage).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intitle: Vec<String>,
    /// Allowed file extensions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ext: Vec<String>,
    /// Allowed MIME types.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filetype: Vec<String>,
    /// Language filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Exclude documents newer than this date? No: exclude documents at or after it is
    /// `before`; see [`Filters::after`]. Stored as the *inclusive* lower bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Date>,
    /// Inclusive lower bound on document date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Date>,
    /// Recency window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness: Option<FreshnessWindow>,
}

impl Filters {
    /// Whether any hard filter is present.
    #[must_use]
    pub fn any(&self) -> bool {
        self.site.is_some()
            || self.inurl.is_some()
            || !self.intitle.is_empty()
            || !self.ext.is_empty()
            || !self.filetype.is_empty()
            || self.lang.is_some()
            || self.before.is_some()
            || self.after.is_some()
            || self.freshness.is_some()
    }
}

/// The typed list of operators the user employed. This is the checklist: every variant
/// must have a documented behaviour and a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
    /// `"…"`
    Phrase,
    /// `-term` or `NOT term`
    Exclude,
    /// `site:`
    Site,
    /// `inurl:`
    Inurl,
    /// `intitle:`
    Intitle,
    /// `ext:`
    Ext,
    /// `before:`
    Before,
    /// `after:`
    After,
    /// `freshness:`
    Freshness,
    /// `filetype:`
    Filetype,
    /// `lang:`
    Lang,
    /// `OR`
    Or,
    /// `+term`
    Force,
}

/// The compiled query: everything the index needs, and nothing about the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryPlan {
    /// Length in bytes of the raw query, for the privacy length check.
    pub raw_len: u16,
    /// Terms all of which must be present.
    pub must: Vec<Term>,
    /// Terms at least one of which must be present, when `disjunction`.
    pub should: Vec<Term>,
    /// Terms whose presence downranks the document (a penalty, not an exclusion).
    pub must_not: Vec<Term>,
    /// Exact phrases.
    pub phrases: Vec<Phrase>,
    /// Whether `should` is a real disjunction (an `OR` was present).
    pub disjunction: bool,
    /// Hard filters.
    pub filters: Filters,
    /// Every operator used, de-duplicated and ordered by first appearance.
    pub operators_used: Vec<Operator>,
    /// Whether any part of the query was truncated.
    pub truncated: bool,
    /// Human-readable, query-free notices about what was dropped or adjusted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<String>,
}

impl QueryPlan {
    /// Whether the plan would match anything at all (no positive terms and no filters).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.must.is_empty()
            && self.should.is_empty()
            && self.phrases.is_empty()
            && !self.filters.any()
    }

    /// Total number of positive terms (must + should + phrases).
    #[must_use]
    pub fn term_count(&self) -> usize {
        self.must.len() + self.should.len() + self.phrases.len()
    }
}
