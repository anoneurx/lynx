//! The fetch safety gate: the SSRF and resource-exhaustion control.
//!
//! > A URL is fetchable only if its **resolved IP address** is publicly routable, at
//! > every hop, at the moment the socket is opened.
//!
//! Hostnames are attacker-controlled; resolved addresses are what the kernel connects
//! to. The IP check is therefore authoritative and hostname rules are defence-in-depth
//! only.
//!
//! The gate resolves the host exactly once and returns the addresses it validated, so
//! the downloader can *pin* the socket to one of them. A second lookup by the HTTP
//! client would re-open the DNS-rebinding window between the check and the connect.
//!
//! See `docs/crawler/safety.md`.

pub mod host;
pub mod ip;
pub mod redirect;

use std::collections::HashMap;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;

use thiserror::Error;
use url::{Host, Url};

pub use host::{decode_ipv4_literal, is_denied_hostname, looks_numeric, HostKind};
pub use ip::{classify, is_publicly_routable, AddressClass};
pub use redirect::{RedirectChain, RedirectHop};

use host::{hostname_is_well_formed, normalise_hostname};

/// A boxed, `Send` future. Keeps [`HostResolver`] `dyn`-compatible without a proc-macro.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The schemes the crawler will ever fetch.
pub const ALLOWED_SCHEMES: [&str; 2] = ["http", "https"];

/// Hard ceiling on `max_response_bytes` (32 MiB).
pub const HARD_MAX_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;
/// Hard ceiling on the decompression ratio.
pub const HARD_MAX_DECOMPRESSION_RATIO: u32 = 1_000;
/// Hard ceiling on any single timeout (60 s).
pub const HARD_MAX_TIMEOUT_MS: u64 = 60_000;
/// Hard ceiling on the redirect budget.
pub const HARD_MAX_REDIRECTS: u8 = 10;

/// Why a URL was refused. Every variant is a permanent denial for that URL.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SafetyError {
    /// The scheme is not `http` or `https`.
    #[error("unsupported scheme {0:?}")]
    UnsupportedScheme(String),
    /// The URL carries a username or password.
    #[error("userinfo is not allowed in a fetch URL")]
    UserinfoNotAllowed,
    /// The explicit port is not in the allowlist.
    #[error("port {0} is not allowed")]
    PortNotAllowed(u16),
    /// The host is present but malformed.
    #[error("malformed host")]
    MalformedHost,
    /// The URL has no host.
    #[error("missing host")]
    EmptyHost,
    /// A numeric-looking host used an encoding that did not decode.
    #[error("host {0:?} uses a disallowed numeric encoding")]
    EncodedLiteral(String),
    /// A hostname matched the deny list.
    #[error("host {host:?} is denied by rule {rule:?}")]
    DeniedHostname {
        /// The offending hostname.
        host: String,
        /// The deny-list rule that matched.
        rule: &'static str,
    },
    /// DNS resolution failed or returned no addresses.
    #[error("dns resolution failed for {0:?}")]
    DnsFailure(String),
    /// At least one resolved address is not publicly routable.
    #[error("address {addr} is not publicly routable ({class:?})")]
    PrivateAddress {
        /// The offending address.
        addr: IpAddr,
        /// Its classification.
        class: AddressClass,
    },
    /// More than the permitted number of redirects.
    #[error("redirect budget of {0} exceeded")]
    RedirectBudgetExceeded(u8),
    /// A URL repeated within the redirect chain.
    #[error("redirect loop detected at {0}")]
    RedirectLoop(String),
    /// A `Location` header could not be resolved against the base URL.
    #[error("invalid redirect location {0:?}")]
    InvalidRedirect(String),
}

/// Why a [`FetchPolicy`] is invalid.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolicyError {
    /// `max_response_bytes` is zero or above the hard ceiling.
    #[error("max_response_bytes {0} is outside the permitted range")]
    ResponseBytes(u64),
    /// `max_decompression_ratio` is zero or above the hard ceiling.
    #[error("max_decompression_ratio {0} is outside the permitted range")]
    DecompressionRatio(u32),
    /// A timeout is zero or above the hard ceiling.
    #[error("timeout {0} ms is outside the permitted range")]
    Timeout(u64),
    /// `max_redirects` is above the hard ceiling.
    #[error("max_redirects {0} is outside the permitted range")]
    MaxRedirects(u8),
    /// The port allowlist is empty.
    #[error("the port allowlist must not be empty")]
    EmptyPorts,
}

/// A DNS resolution failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("dns resolution failed: {0}")]
pub struct ResolveError(pub String);

/// Resolves a hostname to addresses.
///
/// The production implementation wraps the async resolver; [`StaticResolver`] is used
/// in tests so no network is touched.
pub trait HostResolver: std::fmt::Debug + Send + Sync {
    /// Resolve `host` to one or more addresses. An empty result is a failure.
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<Vec<IpAddr>, ResolveError>>;
}

/// A resolver backed by a fixed table. Tests only.
#[derive(Debug, Default)]
pub struct StaticResolver {
    map: HashMap<String, Vec<IpAddr>>,
}

impl StaticResolver {
    /// An empty resolver.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entry, builder style.
    #[must_use]
    pub fn with(mut self, host: &str, addresses: impl IntoIterator<Item = IpAddr>) -> Self {
        self.map
            .insert(normalise_hostname(host), addresses.into_iter().collect());
        self
    }

    /// Add or replace an entry.
    pub fn insert(&mut self, host: &str, addresses: impl IntoIterator<Item = IpAddr>) {
        self.map
            .insert(normalise_hostname(host), addresses.into_iter().collect());
    }
}

impl HostResolver for StaticResolver {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        _port: u16,
    ) -> BoxFuture<'a, Result<Vec<IpAddr>, ResolveError>> {
        let result = self
            .map
            .get(&normalise_hostname(host))
            .cloned()
            .ok_or_else(|| ResolveError(format!("no entry for {host:?}")));
        Box::pin(async move { result })
    }
}

/// Resource limits enforced while fetching. Bounded by the hard ceilings so a config
/// mistake cannot remove the protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Maximum body bytes, enforced while streaming.
    pub max_response_bytes: u64,
    /// Maximum decompressed/compressed ratio.
    pub max_decompression_ratio: u32,
    /// Connection-establishment timeout.
    pub connect_timeout_ms: u64,
    /// Time to first response header.
    pub header_timeout_ms: u64,
    /// Idle timeout between body reads.
    pub read_idle_timeout_ms: u64,
    /// Absolute deadline for the whole fetch.
    pub total_timeout_ms: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_response_bytes: 5 * 1024 * 1024,
            max_decompression_ratio: 100,
            connect_timeout_ms: 5_000,
            header_timeout_ms: 5_000,
            read_idle_timeout_ms: 5_000,
            total_timeout_ms: 15_000,
        }
    }
}

impl ResourceLimits {
    /// Check every limit against its hard ceiling.
    ///
    /// # Errors
    /// Returns the first limit found outside the permitted range.
    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.max_response_bytes == 0 || self.max_response_bytes > HARD_MAX_RESPONSE_BYTES {
            return Err(PolicyError::ResponseBytes(self.max_response_bytes));
        }
        if self.max_decompression_ratio == 0
            || self.max_decompression_ratio > HARD_MAX_DECOMPRESSION_RATIO
        {
            return Err(PolicyError::DecompressionRatio(
                self.max_decompression_ratio,
            ));
        }
        for timeout in [
            self.connect_timeout_ms,
            self.header_timeout_ms,
            self.read_idle_timeout_ms,
            self.total_timeout_ms,
        ] {
            if timeout == 0 || timeout > HARD_MAX_TIMEOUT_MS {
                return Err(PolicyError::Timeout(timeout));
            }
        }
        Ok(())
    }
}

/// The complete fetch policy: what the gate will allow, and the resource bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPolicy {
    allowed_ports: Vec<u16>,
    max_redirects: u8,
    limits: ResourceLimits,
    allow_private_addresses: bool,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        Self {
            allowed_ports: vec![80, 443],
            max_redirects: 5,
            limits: ResourceLimits::default(),
            allow_private_addresses: false,
        }
    }
}

impl FetchPolicy {
    /// Build and validate a policy.
    ///
    /// # Errors
    /// Returns a [`PolicyError`] if the ports, redirect budget, or limits are invalid.
    pub fn new(
        allowed_ports: Vec<u16>,
        max_redirects: u8,
        limits: ResourceLimits,
    ) -> Result<Self, PolicyError> {
        if allowed_ports.is_empty() {
            return Err(PolicyError::EmptyPorts);
        }
        if max_redirects > HARD_MAX_REDIRECTS {
            return Err(PolicyError::MaxRedirects(max_redirects));
        }
        limits.validate()?;
        Ok(Self {
            allowed_ports,
            max_redirects,
            limits,
            allow_private_addresses: false,
        })
    }

    /// The permitted ports.
    #[must_use]
    pub fn allowed_ports(&self) -> &[u16] {
        &self.allowed_ports
    }

    /// The redirect budget.
    #[must_use]
    pub fn max_redirects(&self) -> u8 {
        self.max_redirects
    }

    /// The resource bounds.
    #[must_use]
    pub fn limits(&self) -> &ResourceLimits {
        &self.limits
    }

    /// Whether a scheme may be fetched.
    #[must_use]
    pub fn allows_scheme(&self, scheme: &str) -> bool {
        ALLOWED_SCHEMES.contains(&scheme)
    }

    /// Whether a port may be connected to.
    #[must_use]
    pub fn allows_port(&self, port: u16) -> bool {
        self.allowed_ports.contains(&port)
    }

    /// Whether non-public addresses are permitted.
    #[must_use]
    pub fn allows_private_addresses(&self) -> bool {
        self.allow_private_addresses
    }

    /// **Testing only.** Permit connections to non-public addresses.
    ///
    /// This exists solely so hermetic tests can point the downloader at the loopback
    /// fixture server. Production code must never call it: it defeats the entire SSRF
    /// control. The name is deliberately alarming.
    #[must_use]
    pub fn dangerously_allow_private_addresses(mut self) -> Self {
        self.allow_private_addresses = true;
        self
    }
}

/// A URL that passed the gate, with the addresses it resolved to.
#[derive(Debug, Clone)]
pub struct ValidatedUrl {
    url: Url,
    host: HostKind,
    port: u16,
    addresses: Vec<IpAddr>,
    pinned: IpAddr,
}

impl ValidatedUrl {
    /// The original URL.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The parsed host.
    #[must_use]
    pub fn host(&self) -> &HostKind {
        &self.host
    }

    /// The effective port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Every address the host resolved to; all are publicly routable.
    #[must_use]
    pub fn addresses(&self) -> &[IpAddr] {
        &self.addresses
    }

    /// The address the socket must be pinned to.
    #[must_use]
    pub fn pinned(&self) -> IpAddr {
        self.pinned
    }

    /// Whether `addr` is one of the validated addresses. Used to assert pinning.
    #[must_use]
    pub fn is_pinned_to(&self, addr: IpAddr) -> bool {
        self.pinned == addr && self.addresses.contains(&addr)
    }
}

/// Run the validation pipeline against an already-parsed URL.
///
/// Order: scheme, userinfo, port, host, literal decode, DNS, address classification,
/// pin. Every resolved address must be publicly routable.
///
/// # Errors
/// Returns the first [`SafetyError`] encountered.
pub async fn validate_url(
    url: &Url,
    policy: &FetchPolicy,
    resolver: &dyn HostResolver,
) -> Result<ValidatedUrl, SafetyError> {
    if !policy.allows_scheme(url.scheme()) {
        return Err(SafetyError::UnsupportedScheme(url.scheme().to_string()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SafetyError::UserinfoNotAllowed);
    }

    let port = url
        .port_or_known_default()
        .ok_or(SafetyError::MalformedHost)?;
    if !policy.allows_port(port) {
        return Err(SafetyError::PortNotAllowed(port));
    }

    let host = match url.host() {
        Some(Host::Ipv4(v4)) => HostKind::Ip(IpAddr::V4(v4)),
        Some(Host::Ipv6(v6)) => HostKind::Ip(IpAddr::V6(v6)),
        Some(Host::Domain(domain)) => classify_domain(domain)?,
        None => return Err(SafetyError::EmptyHost),
    };

    let addresses = match &host {
        HostKind::Ip(address) => vec![*address],
        HostKind::Name(name) => {
            let dns_addresses = resolver
                .resolve(name, port)
                .await
                .map_err(|error| SafetyError::DnsFailure(error.to_string()))?;
            if dns_addresses.is_empty() {
                return Err(SafetyError::DnsFailure(name.clone()));
            }
            dns_addresses
        }
    };

    if !policy.allows_private_addresses() {
        for address in &addresses {
            let class = classify(*address);
            if !class.is_public() {
                return Err(SafetyError::PrivateAddress {
                    addr: *address,
                    class,
                });
            }
        }
    }

    let pinned = addresses[0];
    Ok(ValidatedUrl {
        url: url.clone(),
        host,
        port,
        addresses,
        pinned,
    })
}

/// Parse and validate a URL string.
///
/// # Errors
/// Returns [`SafetyError::MalformedHost`] if the string is not a URL, or any
/// [`validate_url`] error.
pub async fn validate_url_str(
    raw: &str,
    policy: &FetchPolicy,
    resolver: &dyn HostResolver,
) -> Result<ValidatedUrl, SafetyError> {
    let url = Url::parse(raw).map_err(|_| SafetyError::MalformedHost)?;
    validate_url(&url, policy, resolver).await
}

fn classify_domain(raw: &str) -> Result<HostKind, SafetyError> {
    if !hostname_is_well_formed(raw) {
        return Err(SafetyError::MalformedHost);
    }
    let name = normalise_hostname(raw);
    if name.is_empty() {
        return Err(SafetyError::EmptyHost);
    }
    // The URL parser normalises standard dotted-decimal to `Host::Ipv4`, but defensively
    // handle a domain that still looks like an address.
    if let Ok(v4) = name.parse::<std::net::Ipv4Addr>() {
        return Ok(HostKind::Ip(IpAddr::V4(v4)));
    }
    if let Some(v4) = decode_ipv4_literal(&name) {
        return Ok(HostKind::Ip(IpAddr::V4(v4)));
    }
    if looks_numeric(&name) {
        return Err(SafetyError::EncodedLiteral(name));
    }
    if let Some(rule) = is_denied_hostname(&name) {
        return Err(SafetyError::DeniedHostname { host: name, rule });
    }
    Ok(HostKind::Name(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn public_resolver() -> StaticResolver {
        StaticResolver::new().with("example.com", [ip("93.184.216.34")])
    }

    #[tokio::test]
    async fn accepts_a_public_url() {
        let validated = validate_url_str(
            "https://example.com/path?q=1",
            &FetchPolicy::default(),
            &public_resolver(),
        )
        .await
        .unwrap();
        assert_eq!(validated.port(), 443);
        assert_eq!(validated.pinned(), ip("93.184.216.34"));
        assert!(validated.is_pinned_to(ip("93.184.216.34")));
    }

    #[tokio::test]
    async fn rejects_unsupported_scheme() {
        let error = validate_url_str(
            "ftp://example.com/",
            &FetchPolicy::default(),
            &public_resolver(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, SafetyError::UnsupportedScheme(_)));
    }

    #[tokio::test]
    async fn rejects_userinfo() {
        let error = validate_url_str(
            "https://user:pass@example.com/",
            &FetchPolicy::default(),
            &public_resolver(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, SafetyError::UserinfoNotAllowed);
    }

    #[tokio::test]
    async fn rejects_disallowed_port() {
        let error = validate_url_str(
            "https://example.com:8443/",
            &FetchPolicy::default(),
            &public_resolver(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, SafetyError::PortNotAllowed(8443));
    }

    #[tokio::test]
    async fn rejects_denied_hostname() {
        let error = validate_url_str(
            "http://localhost/",
            &FetchPolicy::default(),
            &public_resolver(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, SafetyError::DeniedHostname { .. }));
    }

    #[tokio::test]
    async fn rejects_ip_literal_by_classification() {
        let error = validate_url_str(
            "http://169.254.169.254/latest/meta-data/",
            &FetchPolicy::default(),
            &public_resolver(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, SafetyError::PrivateAddress { .. }));
    }

    #[tokio::test]
    async fn rejects_all_denied_ranges_as_literals() {
        let denied = [
            "http://0.0.0.0/",
            "http://10.0.0.1/",
            "http://100.64.0.1/",
            "http://127.0.0.1/",
            "http://169.254.169.254/",
            "http://172.16.5.4/",
            "http://192.0.0.1/",
            "http://192.0.2.1/",
            "http://192.88.99.1/",
            "http://192.168.0.1/",
            "http://198.18.0.1/",
            "http://198.51.100.1/",
            "http://203.0.113.1/",
            "http://224.0.0.1/",
            "http://240.0.0.1/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[64:ff9b::7f00:1]/",
            "http://[2002:7f00:1::]/",
            "http://[fc00::1]/",
            "http://[fe80::1]/",
            "http://[ff00::1]/",
        ];
        for url in denied {
            let error = validate_url_str(url, &FetchPolicy::default(), &public_resolver())
                .await
                .unwrap_err();
            assert!(
                matches!(error, SafetyError::PrivateAddress { .. }),
                "{url} gave {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn rejects_encoded_loopback_literals() {
        for url in [
            "http://2130706433/",
            "http://0x7f.1/",
            "http://0177.0.0.1/",
            "http://127.1/",
        ] {
            let error = validate_url_str(url, &FetchPolicy::default(), &public_resolver())
                .await
                .unwrap_err();
            assert!(
                matches!(error, SafetyError::PrivateAddress { .. }),
                "{url} gave {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn rejects_when_any_resolved_address_is_private() {
        // A host that resolves to both a public and a private address must be denied;
        // DNS rebinding often returns a mixed answer.
        let resolver =
            StaticResolver::new().with("evil.example", [ip("93.184.216.34"), ip("127.0.0.1")]);
        let error = validate_url_str("http://evil.example/", &FetchPolicy::default(), &resolver)
            .await
            .unwrap_err();
        assert!(matches!(error, SafetyError::PrivateAddress { .. }));
    }

    #[tokio::test]
    async fn unknown_host_is_a_dns_failure() {
        let error = validate_url_str(
            "http://nope.example/",
            &FetchPolicy::default(),
            &StaticResolver::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, SafetyError::DnsFailure(_)));
    }

    #[test]
    fn limits_above_the_ceiling_are_rejected() {
        let limits = ResourceLimits {
            max_response_bytes: HARD_MAX_RESPONSE_BYTES + 1,
            ..ResourceLimits::default()
        };
        assert_eq!(
            limits.validate(),
            Err(PolicyError::ResponseBytes(HARD_MAX_RESPONSE_BYTES + 1))
        );

        let limits = ResourceLimits {
            total_timeout_ms: 0,
            ..ResourceLimits::default()
        };
        assert_eq!(limits.validate(), Err(PolicyError::Timeout(0)));

        assert!(
            FetchPolicy::new(vec![80], HARD_MAX_REDIRECTS + 1, ResourceLimits::default()).is_err()
        );
        assert!(FetchPolicy::new(vec![], 5, ResourceLimits::default()).is_err());
    }
}
