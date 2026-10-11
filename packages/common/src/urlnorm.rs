//! URL canonicalisation, shared by the crawler and the API so both agree on what
//! "the same URL" means.
//!
//! The transform is deterministic and idempotent: `normalise(normalise(u)) ==
//! normalise(u)`. It lowercases scheme and host, removes the fragment, drops default
//! ports, strips known tracking parameters, sorts the remaining query pairs, and
//! resolves dot segments in the path. It does **not** decide crawlability, and it
//! never touches the network.

use std::collections::BTreeSet;

use url::Url;

/// Query parameters removed during canonicalisation because they identify a campaign
/// or referrer rather than the resource.
const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "utm_name",
    "gclid",
    "fbclid",
    "msclkid",
    "mc_eid",
    "mc_cid",
    "igshid",
    "yclid",
    "dclid",
    "ref",
    "ref_src",
    "si",
];

/// Errors from canonicalisation. These describe the input, not a system fault.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    /// The input could not be parsed as a URL.
    #[error("invalid URL: {0}")]
    Parse(String),
    /// The scheme is not http/https.
    #[error("unsupported scheme: {0}")]
    UnsupportedScheme(String),
    /// The URL has no host.
    #[error("URL has no host")]
    MissingHost,
}

/// Canonicalise a URL for dedup and identity.
///
/// # Errors
/// Returns a [`UrlError`] if the input is not a valid absolute http(s) URL.
pub fn normalise(input: &str) -> Result<String, UrlError> {
    let trimmed = input.trim();
    let mut url = Url::parse(trimmed).map_err(|e| UrlError::Parse(e.to_string()))?;

    match url.scheme() {
        "http" | "https" => {}
        other => return Err(UrlError::UnsupportedScheme(other.to_string())),
    }

    if url.host_str().is_none() {
        return Err(UrlError::MissingHost);
    }

    // Remove fragment: it is client-side state, not part of the resource.
    url.set_fragment(None);

    // Authoritative canonical host string (url already lowercases + IDNA-encodes).
    if let Some(host) = url.host_str() {
        let canonical_host = host.to_string();
        url.set_host(Some(&canonical_host))
            .map_err(|e| UrlError::Parse(e.to_string()))?;
    }

    // Drop the default port explicitly (url keeps it only when non-default, but be
    // explicit so behaviour is obvious).
    match url.scheme() {
        "http" if url.port() == Some(80) => {
            let _ = url.set_port(None);
        }
        "https" if url.port() == Some(443) => {
            let _ = url.set_port(None);
        }
        _ => {}
    }

    // Canonicalise the path: collapse empty segments produced by `//`, keep a single
    // trailing slash for non-root paths only if the original had one.
    let path = canonical_path(url.path());
    url.set_path(&path);

    // Canonicalise the query: strip tracking params, then sort.
    let canonical_query = canonical_query(url.query());
    url.set_query(canonical_query.as_deref());

    Ok(url.to_string())
}

/// Resolve dot segments and collapse duplicate slashes. `percent-encoded` bytes are
/// left untouched; we only touch literal `.`/`..`/empty segments.
fn canonical_path(path: &str) -> String {
    let had_trailing = path.ends_with('/') && path.len() > 1;
    let mut segments: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    let mut out = String::with_capacity(path.len());
    out.push('/');
    out.push_str(&segments.join("/"));
    if had_trailing && !out.ends_with('/') {
        out.push('/');
    }
    out
}

/// Remove tracking parameters and sort the rest by `(key, value)`.
///
/// Returns `None` when nothing remains.
fn canonical_query(query: Option<&str>) -> Option<String> {
    let raw = query?;
    let tracking: BTreeSet<&str> = TRACKING_PARAMS.iter().copied().collect();
    let mut pairs: Vec<(String, String)> = url::form_urlencoded::parse(raw.as_bytes())
        .filter(|(k, _)| !tracking.contains(k.as_ref()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if pairs.is_empty() {
        return None;
    }
    pairs.sort();
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in &pairs {
        serializer.append_pair(k, v);
    }
    Some(serializer.finish())
}

/// True when two URLs canonicalise to the same string.
#[must_use]
pub fn same(a: &str, b: &str) -> bool {
    match (normalise(a), normalise(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn lowercases_scheme_and_host() {
        assert_eq!(
            normalise("HTTP://Example.COM/Path").unwrap(),
            "http://example.com/Path"
        );
    }

    #[test]
    fn removes_fragment() {
        assert_eq!(
            normalise("https://example.com/a#section").unwrap(),
            "https://example.com/a"
        );
    }

    #[test]
    fn removes_default_ports() {
        assert_eq!(
            normalise("http://example.com:80/").unwrap(),
            "http://example.com/"
        );
        assert_eq!(
            normalise("https://example.com:443/").unwrap(),
            "https://example.com/"
        );
    }

    #[test]
    fn keeps_non_default_ports() {
        assert_eq!(
            normalise("http://example.com:8080/").unwrap(),
            "http://example.com:8080/"
        );
    }

    #[test]
    fn strips_tracking_params_and_sorts() {
        let a = normalise("https://e.com/p?b=2&a=1&utm_source=x").unwrap();
        let b = normalise("https://e.com/p?a=1&b=2").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, "https://e.com/p?a=1&b=2");
    }

    #[test]
    fn resolves_dot_segments() {
        assert_eq!(
            normalise("https://e.com/a/b/../c/./d").unwrap(),
            "https://e.com/a/c/d"
        );
    }

    #[test]
    fn collapses_duplicate_slashes() {
        assert_eq!(
            normalise("https://e.com/a//b").unwrap(),
            "https://e.com/a/b"
        );
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert_eq!(
            normalise("ftp://example.com/file"),
            Err(UrlError::UnsupportedScheme("ftp".into()))
        );
    }

    #[test]
    fn is_idempotent() {
        for input in [
            "https://Example.com:443/a/./b/../c?z=1&a=2&utm_medium=e#frag",
            "http://e.com/p?a=1&a=2",
            "https://e.com/",
        ] {
            let once = normalise(input).unwrap();
            let twice = normalise(&once).unwrap();
            assert_eq!(once, twice, "not idempotent for {input:?}");
        }
    }

    #[test]
    fn same_urls_match() {
        assert!(same("https://e.com/p?utm_source=x", "https://e.com/p"));
        assert!(!same("https://e.com/a", "https://e.com/b"));
    }
}
