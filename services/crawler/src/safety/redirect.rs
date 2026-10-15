//! Per-hop redirect validation.
//!
//! Redirects are the classic SSRF bypass: the fetch looks safe at hop 0 and arrives
//! somewhere private at hop 3. Every hop is therefore resolved against the current URL
//! per RFC 3986 and pushed back through the *whole* validation pipeline — scheme,
//! userinfo, port, host, DNS, and address classification — before the next request is
//! made. `Location: //169.254.169.254/` is caught at the next hop exactly like any
//! other literal.
//!
//! No cookies, `Authorization`, or `Referer` are carried across a hop; the downloader
//! holds no cookie jar, so that is structural rather than conventional.

use std::net::IpAddr;

use url::Url;

use super::{validate_url, FetchPolicy, HostResolver, SafetyError, ValidatedUrl};

/// One hop in a redirect chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectHop {
    /// The URL fetched at this hop.
    pub url: Url,
    /// The validated addresses for this hop.
    pub addresses: Vec<IpAddr>,
}

/// A validated redirect chain, including the initial request as hop 0.
#[derive(Debug, Clone)]
pub struct RedirectChain {
    hops: Vec<RedirectHop>,
    max_redirects: u8,
}

impl RedirectChain {
    /// Start a chain from an already-validated initial URL.
    #[must_use]
    pub fn start(initial: &ValidatedUrl, max_redirects: u8) -> Self {
        Self {
            hops: vec![RedirectHop {
                url: initial.url().clone(),
                addresses: initial.addresses().to_vec(),
            }],
            max_redirects,
        }
    }

    /// All hops, oldest first.
    #[must_use]
    pub fn hops(&self) -> &[RedirectHop] {
        &self.hops
    }

    /// The number of hops including the initial request.
    #[must_use]
    pub fn len(&self) -> usize {
        self.hops.len()
    }

    /// Whether the chain has no hops. Never true for a started chain.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hops.is_empty()
    }

    /// The number of redirects followed so far (hops minus the initial request).
    #[must_use]
    pub fn redirects_followed(&self) -> usize {
        self.hops.len().saturating_sub(1)
    }

    /// The URL currently being fetched.
    #[must_use]
    pub fn current_url(&self) -> &Url {
        // A started chain always has at least one hop.
        &self.hops[self.hops.len() - 1].url
    }

    /// Whether `url` already appears in the chain.
    #[must_use]
    pub fn is_loop(&self, url: &Url) -> bool {
        self.hops.iter().any(|hop| &hop.url == url)
    }

    /// Resolve `location` against the current URL, validate the target, and append it.
    ///
    /// # Errors
    /// Returns [`SafetyError::RedirectBudgetExceeded`], [`SafetyError::RedirectLoop`],
    /// [`SafetyError::InvalidRedirect`], or any validation error for the target.
    pub async fn follow(
        &mut self,
        location: &str,
        policy: &FetchPolicy,
        resolver: &dyn HostResolver,
    ) -> Result<RedirectHop, SafetyError> {
        if self.redirects_followed() >= usize::from(self.max_redirects) {
            return Err(SafetyError::RedirectBudgetExceeded(self.max_redirects));
        }

        let next = self
            .current_url()
            .join(location)
            .map_err(|_| SafetyError::InvalidRedirect(location.to_string()))?;

        if self.is_loop(&next) {
            return Err(SafetyError::RedirectLoop(next.to_string()));
        }

        let validated = validate_url(&next, policy, resolver).await?;
        let hop = RedirectHop {
            url: validated.url().clone(),
            addresses: validated.addresses().to_vec(),
        };
        self.hops.push(hop.clone());
        Ok(hop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::{validate_url_str, StaticResolver};

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn resolver() -> StaticResolver {
        StaticResolver::new()
            .with("example.com", [ip("93.184.216.34")])
            .with("www.example.com", [ip("93.184.216.36")])
    }

    async fn start(url: &str) -> RedirectChain {
        let validated = validate_url_str(url, &FetchPolicy::default(), &resolver())
            .await
            .unwrap();
        RedirectChain::start(&validated, FetchPolicy::default().max_redirects())
    }

    #[tokio::test]
    async fn follows_a_relative_location() {
        let mut chain = start("https://example.com/a/b").await;
        let hop = chain
            .follow("/c", &FetchPolicy::default(), &resolver())
            .await
            .unwrap();
        assert_eq!(hop.url.as_str(), "https://example.com/c");
        assert_eq!(chain.redirects_followed(), 1);
    }

    #[tokio::test]
    async fn follows_a_scheme_relative_location() {
        let mut chain = start("https://example.com/a").await;
        let hop = chain
            .follow("//www.example.com/x", &FetchPolicy::default(), &resolver())
            .await
            .unwrap();
        assert_eq!(hop.url.as_str(), "https://www.example.com/x");
    }

    #[tokio::test]
    async fn denies_a_redirect_to_a_private_literal() {
        let mut chain = start("https://example.com/a").await;
        let error = chain
            .follow(
                "//169.254.169.254/latest",
                &FetchPolicy::default(),
                &resolver(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, SafetyError::PrivateAddress { .. }));
        // The denied hop is not recorded.
        assert_eq!(chain.len(), 1);
    }

    #[tokio::test]
    async fn denies_a_redirect_to_an_unsupported_scheme() {
        let mut chain = start("https://example.com/a").await;
        let error = chain
            .follow(
                "ftp://example.com/file",
                &FetchPolicy::default(),
                &resolver(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, SafetyError::UnsupportedScheme(_)));
    }

    #[tokio::test]
    async fn detects_a_loop() {
        let mut chain = start("https://example.com/a").await;
        chain
            .follow("/b", &FetchPolicy::default(), &resolver())
            .await
            .unwrap();
        let error = chain
            .follow("/a", &FetchPolicy::default(), &resolver())
            .await
            .unwrap_err();
        assert!(matches!(error, SafetyError::RedirectLoop(_)));
    }

    #[tokio::test]
    async fn enforces_the_redirect_budget() {
        let policy =
            FetchPolicy::new(vec![80, 443], 2, super::super::ResourceLimits::default()).unwrap();
        let validated = validate_url_str("https://example.com/0", &policy, &resolver())
            .await
            .unwrap();
        let mut chain = RedirectChain::start(&validated, policy.max_redirects());

        chain.follow("/1", &policy, &resolver()).await.unwrap();
        chain.follow("/2", &policy, &resolver()).await.unwrap();
        assert_eq!(chain.redirects_followed(), 2);

        let error = chain.follow("/3", &policy, &resolver()).await.unwrap_err();
        assert_eq!(error, SafetyError::RedirectBudgetExceeded(2));
    }

    #[tokio::test]
    async fn malformed_location_is_an_invalid_redirect() {
        let mut chain = start("https://example.com/a").await;
        // An unclosed IPv6 literal makes the location unparseable.
        let error = chain
            .follow("http://[::1", &FetchPolicy::default(), &resolver())
            .await
            .unwrap_err();
        assert!(matches!(error, SafetyError::InvalidRedirect(_)));
    }
}
