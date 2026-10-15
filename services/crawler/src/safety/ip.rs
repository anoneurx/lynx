//! Address classification for the SSRF gate.
//!
//! Policy is **deny-based on every resolved address**, not an allow-list of ranges we
//! happened to think of. The only public result is [`AddressClass::Public`]; everything
//! else is refused.
//!
//! IPv6 special forms are unwrapped before classification, because they reach an IPv4
//! destination just as surely as the plain form: `::ffff:127.0.0.1` (IPv4-mapped),
//! `64:ff9b::7f00:1` (NAT64), `2002:7f00:1::` (6to4), and the deprecated IPv4-compatible
//! `::127.0.0.1` all denote loopback. A deny list that misses these is decoration.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The classification of a single address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressClass {
    /// Publicly routable; the only class the crawler will connect to.
    Public,
    /// `0.0.0.0/8`, or the IPv6 unspecified address.
    Unspecified,
    /// `127.0.0.0/8` or `::1`.
    Loopback,
    /// `10/8`, `172.16/12`, `192.168/16`.
    Private,
    /// `100.64.0.0/10`, carrier-grade NAT.
    Shared,
    /// `169.254.0.0/16` or `fe80::/10`.
    LinkLocal,
    /// IETF protocol assignments: `192.0.0.0/24`, `192.88.99.0/24`.
    ProtocolAssignment,
    /// `192.0.2.0/24`, `198.51.100.0/24`, `203.0.113.0/24`.
    Documentation,
    /// `198.18.0.0/15`.
    Benchmarking,
    /// `224.0.0.0/4` or `ff00::/8`.
    Multicast,
    /// `240.0.0.0/4` (reserved; includes broadcast).
    Reserved,
    /// `255.255.255.255/32`.
    Broadcast,
    /// `fc00::/7`, unique local addresses.
    UniqueLocal,
}

impl AddressClass {
    /// Whether an address of this class may be connected to.
    #[must_use]
    pub fn is_public(self) -> bool {
        matches!(self, Self::Public)
    }

    /// A stable `snake_case` token, for logs, metrics, and error payloads.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Unspecified => "unspecified",
            Self::Loopback => "loopback",
            Self::Private => "private",
            Self::Shared => "shared",
            Self::LinkLocal => "link_local",
            Self::ProtocolAssignment => "protocol_assignment",
            Self::Documentation => "documentation",
            Self::Benchmarking => "benchmarking",
            Self::Multicast => "multicast",
            Self::Reserved => "reserved",
            Self::Broadcast => "broadcast",
            Self::UniqueLocal => "unique_local",
        }
    }
}

/// Classify an address, unwrapping IPv6 forms that embed an IPv4 destination.
#[must_use]
pub fn classify(addr: IpAddr) -> AddressClass {
    match addr {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

/// Whether an address may be connected to.
#[must_use]
pub fn is_publicly_routable(addr: IpAddr) -> bool {
    classify(addr).is_public()
}

fn classify_v4(ip: Ipv4Addr) -> AddressClass {
    let [a, b, c, d] = ip.octets();
    match [a, b, c, d] {
        [0, ..] => AddressClass::Unspecified,
        [100, 64..=127, ..] => AddressClass::Shared,
        [127, ..] => AddressClass::Loopback,
        [169, 254, ..] => AddressClass::LinkLocal,
        [10, ..] | [172, 16..=31, ..] | [192, 168, ..] => AddressClass::Private,
        [192, 0, 0, ..] | [192, 88, 99, ..] => AddressClass::ProtocolAssignment,
        [192, 0, 2, ..] | [198, 51, 100, ..] | [203, 0, 113, ..] => AddressClass::Documentation,
        [198, 18..=19, ..] => AddressClass::Benchmarking,
        [224..=239, ..] => AddressClass::Multicast,
        [255, 255, 255, 255] => AddressClass::Broadcast,
        [240..=255, ..] => AddressClass::Reserved,
        _ => AddressClass::Public,
    }
}

fn classify_v6(ip: Ipv6Addr) -> AddressClass {
    // Unspecified and loopback first, before any prefix unwrapping.
    if ip.is_unspecified() {
        return AddressClass::Unspecified;
    }
    if ip.is_loopback() {
        return AddressClass::Loopback;
    }

    let segments = ip.segments();
    let is_multicast = (segments[0] & 0xff00) == 0xff00;
    if is_multicast {
        return AddressClass::Multicast;
    }

    // fc00::/7 unique local.
    if (segments[0] & 0xfe00) == 0xfc00 {
        return AddressClass::UniqueLocal;
    }

    // fe80::/10 link local.
    if (segments[0] & 0xffc0) == 0xfe80 {
        return AddressClass::LinkLocal;
    }

    // IPv4-mapped ::ffff:0:0/96.
    if segments[0..5] == [0, 0, 0, 0, 0] && segments[5] == 0xffff {
        return classify_v4(embedded_v4(segments[6], segments[7]));
    }

    // IPv4-compatible ::/96 (deprecated), excluding :: and ::1 handled above.
    if segments[0..6] == [0, 0, 0, 0, 0, 0] {
        return classify_v4(embedded_v4(segments[6], segments[7]));
    }

    // NAT64 64:ff9b::/96.
    if segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2..6] == [0, 0, 0, 0] {
        return classify_v4(embedded_v4(segments[6], segments[7]));
    }

    // 6to4 2002::/16 embeds the IPv4 destination in the next 32 bits.
    if segments[0] == 0x2002 {
        return classify_v4(embedded_v4(segments[1], segments[2]));
    }

    AddressClass::Public
}

fn embedded_v4(hi: u16, lo: u16) -> Ipv4Addr {
    let n = (u32::from(hi) << 16) | u32::from(lo);
    Ipv4Addr::from(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> IpAddr {
        IpAddr::V4(s.parse().unwrap())
    }

    fn v6(s: &str) -> IpAddr {
        IpAddr::V6(s.parse().unwrap())
    }

    #[test]
    fn classifies_every_denied_ipv4_range() {
        let cases: &[(&str, AddressClass)] = &[
            ("0.0.0.0", AddressClass::Unspecified),
            ("0.255.255.255", AddressClass::Unspecified),
            ("10.0.0.0", AddressClass::Private),
            ("10.255.255.255", AddressClass::Private),
            ("100.64.0.0", AddressClass::Shared),
            ("100.127.255.255", AddressClass::Shared),
            ("127.0.0.1", AddressClass::Loopback),
            ("169.254.169.254", AddressClass::LinkLocal),
            ("172.16.0.0", AddressClass::Private),
            ("172.31.255.255", AddressClass::Private),
            ("192.0.0.1", AddressClass::ProtocolAssignment),
            ("192.0.2.1", AddressClass::Documentation),
            ("192.88.99.1", AddressClass::ProtocolAssignment),
            ("192.168.1.1", AddressClass::Private),
            ("198.18.0.1", AddressClass::Benchmarking),
            ("198.19.255.255", AddressClass::Benchmarking),
            ("198.51.100.1", AddressClass::Documentation),
            ("203.0.113.1", AddressClass::Documentation),
            ("224.0.0.1", AddressClass::Multicast),
            ("239.255.255.255", AddressClass::Multicast),
            ("240.0.0.1", AddressClass::Reserved),
            ("255.255.255.255", AddressClass::Broadcast),
        ];
        for (ip, expected) in cases {
            assert_eq!(classify(v4(ip)), *expected, "ip {ip}");
            assert!(
                !is_publicly_routable(v4(ip)),
                "ip {ip} should not be public"
            );
        }
    }

    #[test]
    fn boundary_just_outside_denied_ranges_is_public() {
        for ip in [
            "1.1.1.1",
            "9.255.255.255",
            "11.0.0.0",
            "100.63.255.255",
            "100.128.0.0",
            "126.255.255.255",
            "128.0.0.0",
            "169.253.255.255",
            "169.255.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "192.0.1.0",
            "192.0.3.0",
            "192.167.255.255",
            "192.169.0.0",
            "198.17.255.255",
            "198.20.0.0",
            "223.255.255.255",
        ] {
            assert_eq!(classify(v4(ip)), AddressClass::Public, "ip {ip}");
            assert!(is_publicly_routable(v4(ip)), "ip {ip}");
        }
    }

    #[test]
    fn classifies_denied_ipv6_ranges() {
        assert_eq!(classify(v6("::")), AddressClass::Unspecified);
        assert_eq!(classify(v6("::1")), AddressClass::Loopback);
        assert_eq!(classify(v6("fc00::1")), AddressClass::UniqueLocal);
        assert_eq!(classify(v6("fdff::1")), AddressClass::UniqueLocal);
        assert_eq!(classify(v6("fe80::1")), AddressClass::LinkLocal);
        assert_eq!(classify(v6("febf::1")), AddressClass::LinkLocal);
        assert_eq!(classify(v6("ff00::1")), AddressClass::Multicast);
    }

    #[test]
    fn unwraps_ipv4_mapped() {
        assert_eq!(classify(v6("::ffff:127.0.0.1")), AddressClass::Loopback);
        assert_eq!(
            classify(v6("::ffff:169.254.169.254")),
            AddressClass::LinkLocal
        );
        assert_eq!(classify(v6("::ffff:10.0.0.1")), AddressClass::Private);
        assert_eq!(classify(v6("::ffff:8.8.8.8")), AddressClass::Public);
    }

    #[test]
    fn unwraps_nat64() {
        assert_eq!(classify(v6("64:ff9b::7f00:1")), AddressClass::Loopback);
        assert_eq!(classify(v6("64:ff9b::a00:1")), AddressClass::Private);
        assert_eq!(classify(v6("64:ff9b::808:808")), AddressClass::Public);
    }

    #[test]
    fn unwraps_6to4() {
        // 2002:7f00:0001:: embeds 127.0.0.1
        assert_eq!(classify(v6("2002:7f00:1::")), AddressClass::Loopback);
        // 2002:0a00:0001:: embeds 10.0.0.1
        assert_eq!(classify(v6("2002:a00:1::")), AddressClass::Private);
    }

    #[test]
    fn unwraps_ipv4_compatible() {
        assert_eq!(classify(v6("::127.0.0.1")), AddressClass::Loopback);
        assert_eq!(classify(v6("::0a00:1")), AddressClass::Private);
    }

    #[test]
    fn public_ipv6_is_public() {
        assert_eq!(classify(v6("2606:4700:4700::1111")), AddressClass::Public);
        assert!(is_publicly_routable(v6("2001:4860:4860::8888")));
    }
}
