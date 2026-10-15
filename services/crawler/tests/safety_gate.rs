//! The adversarial suite for the fetch safety gate.
//!
//! Every denied range, every numeric encoding, IPv6 unwrapping, mixed answers, and a
//! DNS-rebinding simulation. Plus property tests asserting the invariant that matters:
//! a validated URL pins a publicly routable address.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use lynx_crawler::safety::{
    self, validate_url_str, FetchPolicy, HostResolver, RedirectChain, ResolveError, SafetyError,
    StaticResolver,
};
use lynx_fixture_origin::bind_ephemeral;
use proptest::prelude::*;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

/// A resolver that records how many times it is asked. Pinning means "once".
#[derive(Debug)]
struct CountingResolver {
    calls: Arc<AtomicUsize>,
    addresses: Vec<IpAddr>,
}

impl HostResolver for CountingResolver {
    fn resolve<'a>(
        &'a self,
        _host: &'a str,
        _port: u16,
    ) -> safety::BoxFuture<'a, Result<Vec<IpAddr>, ResolveError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let addresses = self.addresses.clone();
        Box::pin(async move { Ok(addresses) })
    }
}

/// A resolver that answers public on the first lookup and private on the second — the
/// shape of a DNS-rebinding attack.
#[derive(Debug)]
struct RebindingResolver {
    calls: Arc<AtomicUsize>,
}

impl HostResolver for RebindingResolver {
    fn resolve<'a>(
        &'a self,
        _host: &'a str,
        _port: u16,
    ) -> safety::BoxFuture<'a, Result<Vec<IpAddr>, ResolveError>> {
        let lookup = self.calls.fetch_add(1, Ordering::SeqCst);
        let address = if lookup == 0 {
            ip("93.184.216.34")
        } else {
            ip("127.0.0.1")
        };
        Box::pin(async move { Ok(vec![address]) })
    }
}

#[tokio::test]
async fn resolves_once_and_pins_the_validated_address() {
    let calls = Arc::new(AtomicUsize::new(0));
    let resolver = CountingResolver {
        calls: Arc::clone(&calls),
        addresses: vec![ip("93.184.216.34")],
    };
    let validated = validate_url_str("https://example.com/", &FetchPolicy::default(), &resolver)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(validated.is_pinned_to(ip("93.184.216.34")));
}

#[tokio::test]
async fn rebinding_after_validation_cannot_change_the_pinned_address() {
    let calls = Arc::new(AtomicUsize::new(0));
    let resolver = RebindingResolver {
        calls: Arc::clone(&calls),
    };
    // First validation sees the public answer and pins it.
    let validated = validate_url_str(
        "https://rebind.example/",
        &FetchPolicy::default(),
        &resolver,
    )
    .await
    .unwrap();
    assert_eq!(validated.pinned(), ip("93.184.216.34"));
    // A second, independent resolution now sees loopback and is denied — this is why the
    // downloader must connect to the pinned address and never re-resolve.
    let second = validate_url_str(
        "https://rebind.example/",
        &FetchPolicy::default(),
        &resolver,
    )
    .await
    .unwrap_err();
    assert!(matches!(second, SafetyError::PrivateAddress { .. }));
}

#[tokio::test]
async fn a_mixed_answer_is_denied_entirely() {
    let calls = Arc::new(AtomicUsize::new(0));
    let resolver = CountingResolver {
        calls,
        addresses: vec![ip("93.184.216.34"), ip("10.0.0.1")],
    };
    let error = validate_url_str("https://mixed.example/", &FetchPolicy::default(), &resolver)
        .await
        .unwrap_err();
    assert!(matches!(error, SafetyError::PrivateAddress { .. }));
}

#[tokio::test]
async fn denied_ipv4_matrix() {
    let denied = [
        "0.0.0.0",
        "0.1.2.3",
        "10.0.0.1",
        "100.64.0.1",
        "100.127.255.255",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "172.31.255.254",
        "192.0.0.1",
        "192.0.2.1",
        "192.88.99.1",
        "192.168.1.1",
        "198.18.0.1",
        "198.19.255.255",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "239.255.255.255",
        "240.0.0.1",
        "255.255.255.255",
    ];
    for host in denied {
        let error = validate_url_str(
            &format!("http://{host}/"),
            &FetchPolicy::default(),
            &StaticResolver::new(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, SafetyError::PrivateAddress { .. }),
            "{host} gave {error:?}"
        );
    }
}

#[tokio::test]
async fn denied_ipv6_matrix() {
    let denied = [
        "[::]",
        "[::1]",
        "[::ffff:127.0.0.1]",
        "[::ffff:169.254.169.254]",
        "[::ffff:10.0.0.1]",
        "[64:ff9b::7f00:1]",
        "[64:ff9b::a00:1]",
        "[2002:7f00:1::]",
        "[2002:a00:1::]",
        "[::127.0.0.1]",
        "[fc00::1]",
        "[fdff::1]",
        "[fe80::1]",
        "[febf::1]",
        "[ff00::1]",
    ];
    for host in denied {
        let error = validate_url_str(
            &format!("http://{host}/"),
            &FetchPolicy::default(),
            &StaticResolver::new(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, SafetyError::PrivateAddress { .. }),
            "{host} gave {error:?}"
        );
    }
}

#[tokio::test]
async fn encoded_literal_matrix_is_denied() {
    let denied = [
        "2130706433", // 127.0.0.1 as an integer
        "0x7f.1",     // hex + short
        "0x7f000001", // hex integer
        "0177.0.0.1", // octal
        "127.1",      // short
        "127.0.1",    // short
        "0x7f.0x0.0x0.0x1",
    ];
    for host in denied {
        let error = validate_url_str(
            &format!("http://{host}/"),
            &FetchPolicy::default(),
            &StaticResolver::new(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, SafetyError::PrivateAddress { .. }),
            "{host} gave {error:?}"
        );
    }
}

#[tokio::test]
async fn trailing_dot_and_case_do_not_evade_the_deny_list() {
    for host in ["localhost.", "LOCALHOST", "Metadata.Goog.", "foo.LOCAL"] {
        let error = validate_url_str(
            &format!("http://{host}/"),
            &FetchPolicy::default(),
            &StaticResolver::new(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, SafetyError::DeniedHostname { .. }),
            "{host} gave {error:?}"
        );
    }
}

#[tokio::test]
async fn redirect_location_from_a_real_server_is_re_validated() {
    let (addr, _handle) = bind_ephemeral().await.unwrap();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(format!("http://{addr}/redirect/private"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 302);
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Start a chain from a benign public URL, then follow the server's Location.
    let policy = FetchPolicy::default();
    let resolver = StaticResolver::new().with("example.com", [ip("93.184.216.34")]);
    let start = validate_url_str("https://example.com/", &policy, &resolver)
        .await
        .unwrap();
    let mut chain = RedirectChain::start(&start, policy.max_redirects());
    let error = chain
        .follow(&location, &policy, &resolver)
        .await
        .unwrap_err();
    assert!(matches!(error, SafetyError::PrivateAddress { .. }));
    assert_eq!(chain.len(), 1, "the denied hop must not be recorded");
}

// Flat or-patterns read as the range list from the spec; nesting them would obscure it.
#[allow(clippy::unnested_or_patterns)]
fn reference_denied_v4(a: u8, b: u8, c: u8, d: u8) -> bool {
    matches!(
        (a, b, c, d),
        (0, _, _, _)
            | (10, _, _, _)
            | (100, 64..=127, _, _)
            | (127, _, _, _)
            | (169, 254, _, _)
            | (172, 16..=31, _, _)
            | (192, 0, 0, _)
            | (192, 0, 2, _)
            | (192, 88, 99, _)
            | (192, 168, _, _)
            | (198, 18..=19, _, _)
            | (198, 51, 100, _)
            | (203, 0, 113, _)
            | (224..=239, _, _, _)
            | (240..=255, _, _, _)
    )
}

proptest! {
    #[test]
    fn classify_matches_an_independent_reference(a in any::<u8>(), b in any::<u8>(), c in any::<u8>(), d in any::<u8>()) {
        let address = IpAddr::V4(Ipv4Addr::new(a, b, c, d));
        let is_public = safety::is_publicly_routable(address);
        prop_assert_eq!(is_public, !reference_denied_v4(a, b, c, d), "address {:?}", address);
    }

    #[test]
    fn decimal_literals_round_trip(n in any::<u32>()) {
        let expected = Ipv4Addr::from(n);
        prop_assert_eq!(
            lynx_crawler::safety::host::decode_ipv4_literal(&n.to_string()),
            Some(expected)
        );
    }

    #[test]
    fn hex_literals_round_trip(n in any::<u32>()) {
        let expected = Ipv4Addr::from(n);
        prop_assert_eq!(
            lynx_crawler::safety::host::decode_ipv4_literal(&format!("0x{n:x}")),
            Some(expected)
        );
    }
}
