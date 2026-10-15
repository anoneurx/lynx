//! Host classification: numeric literal decoding and the hostname deny list.
//!
//! A hostname is attacker-controlled. `http://2130706433/`, `http://0x7f.1/`,
//! `http://0177.0.0.1/`, and `http://127.1/` all denote loopback, and a filter that
//! only recognises dotted-decimal misses three of them. We decode the legacy
//! `inet_aton` forms so every spelling of an address ends up classified as an address,
//! then the address check (not the spelling) makes the decision.
//!
//! The hostname deny list is defence-in-depth only and deliberately small: a deny rule
//! that is wrong in the permissive direction does not matter, because the resolved-IP
//! check still refuses the connection. Being wrong in the restrictive direction costs
//! coverage, so we do not guess.

use std::net::{IpAddr, Ipv4Addr};

/// A parsed host, before resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKind {
    /// A DNS name (already punycode/lowercased by the URL parser).
    Name(String),
    /// A literal address, however it was spelled.
    Ip(IpAddr),
}

impl HostKind {
    /// The host as a string suitable for logs and DNS.
    #[must_use]
    pub fn as_str(&self) -> String {
        match self {
            Self::Name(name) => name.clone(),
            Self::Ip(ip) => ip.to_string(),
        }
    }
}

/// Decode a legacy `inet_aton` IPv4 literal: decimal, octal (`0` prefix), hex
/// (`0x`/`0X` prefix), and the 1–4 part short forms.
///
/// Returns `None` for anything that is not a well-formed numeric literal.
#[must_use]
pub fn decode_ipv4_literal(host: &str) -> Option<Ipv4Addr> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }

    let mut values = [0u32; 4];
    for (index, part) in parts.iter().enumerate() {
        values[index] = parse_numeric_part(part)?;
    }

    let (value, ok) = match parts.len() {
        1 => (values[0], true),
        2 => (
            (values[0] << 24) | (values[1] & 0x00ff_ffff),
            values[0] <= 0xff && values[1] <= 0x00ff_ffff,
        ),
        3 => (
            (values[0] << 24) | (values[1] << 16) | (values[2] & 0xffff),
            values[0] <= 0xff && values[1] <= 0xff && values[2] <= 0xffff,
        ),
        4 => (
            (values[0] << 24) | (values[1] << 16) | (values[2] << 8) | values[3],
            values.iter().all(|value| *value <= 0xff),
        ),
        _ => return None,
    };

    ok.then(|| Ipv4Addr::from(value))
}

fn parse_numeric_part(part: &str) -> Option<u32> {
    if part.is_empty() {
        return None;
    }
    let (radix, digits) =
        if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
            (16u32, hex)
        } else if part.len() > 1 && part.starts_with('0') {
            (8u32, &part[1..])
        } else {
            (10u32, part)
        };

    if digits.is_empty() {
        // "0x" with no digits is invalid; a leading "0"/"00" is octal zero.
        return part.chars().all(|c| c == '0').then_some(0);
    }
    // Reject signs, whitespace, and anything else from_str_radix might tolerate.
    if !digits.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    u32::from_str_radix(digits, radix).ok()
}

/// Whether a host is shaped like a numeric literal (and so should never reach DNS).
///
/// Used to deny an undecodable numeric spelling rather than hand it to the resolver.
#[must_use]
pub fn looks_numeric(host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    let lower = host.to_ascii_lowercase();
    if lower.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return true;
    }
    lower.split('.').any(|part| part.starts_with("0x"))
}

/// The hostname deny list. Returns the matched rule name, or `None`.
#[allow(clippy::case_sensitive_file_extension_comparisons)] // `.local` is a hostname suffix, not a file extension; the input is already lowercased
#[must_use]
pub fn is_denied_hostname(host: &str) -> Option<&'static str> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return Some("localhost");
    }
    if host == "local" || host.ends_with(".local") {
        return Some("local");
    }
    if host.ends_with(".internal") {
        return Some("internal");
    }
    if host == "home.arpa" || host.ends_with(".home.arpa") {
        return Some("home.arpa");
    }
    match host.as_str() {
        "instance-data" => Some("instance-data"),
        "metadata.goog" | "metadata.google.internal" | "metadata.azure.com" => {
            Some("cloud-metadata")
        }
        _ => None,
    }
}

/// Normalise a hostname: lowercase and strip a single trailing dot (the root label).
#[must_use]
pub fn normalise_hostname(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// A non-empty hostname with no NUL or control characters.
#[must_use]
pub fn hostname_is_well_formed(host: &str) -> bool {
    !host.is_empty() && !host.chars().any(|c| c.is_control() || c == '\0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn decodes_dotted_decimal() {
        assert_eq!(decode_ipv4_literal("127.0.0.1"), Some(Ipv4Addr::LOCALHOST));
        assert_eq!(
            decode_ipv4_literal("8.8.8.8"),
            Some(Ipv4Addr::new(8, 8, 8, 8))
        );
    }

    #[test]
    fn decodes_single_integer_form() {
        assert_eq!(decode_ipv4_literal("2130706433"), Some(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn decodes_hex_form() {
        assert_eq!(decode_ipv4_literal("0x7f.1"), Some(Ipv4Addr::LOCALHOST));
        assert_eq!(decode_ipv4_literal("0x7f000001"), Some(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn decodes_octal_form() {
        assert_eq!(decode_ipv4_literal("0177.0.0.1"), Some(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn decodes_short_forms() {
        assert_eq!(decode_ipv4_literal("127.1"), Some(Ipv4Addr::LOCALHOST));
        assert_eq!(decode_ipv4_literal("127.0.1"), Some(Ipv4Addr::LOCALHOST));
        assert_eq!(
            decode_ipv4_literal("10.1"),
            Some(Ipv4Addr::new(10, 0, 0, 1))
        );
    }

    #[test]
    fn rejects_non_numeric_and_out_of_range() {
        assert_eq!(decode_ipv4_literal("example.com"), None);
        assert_eq!(decode_ipv4_literal("256.0.0.1"), None);
        assert_eq!(decode_ipv4_literal("1.2.3.4.5"), None);
        assert_eq!(decode_ipv4_literal("0x7f."), None);
        assert_eq!(decode_ipv4_literal("1..2"), None);
        assert_eq!(decode_ipv4_literal("99999999999"), None);
        // signs are rejected even though from_str_radix might accept them
        assert_eq!(decode_ipv4_literal("+127.0.0.1"), None);
        // a decimal part > 255 in a 4-part literal
        assert_eq!(decode_ipv4_literal("127.0.0.999"), None);
    }

    #[test]
    fn octal_digits_8_and_9_are_rejected() {
        assert_eq!(decode_ipv4_literal("09"), None);
        assert_eq!(decode_ipv4_literal("0x"), None);
        assert_eq!(decode_ipv4_literal("00"), Some(Ipv4Addr::UNSPECIFIED));
    }

    #[test]
    fn detects_numeric_shapes() {
        assert!(looks_numeric("2130706433"));
        assert!(looks_numeric("0x7f.1"));
        assert!(looks_numeric("1.2.3.4"));
        assert!(!looks_numeric("example.com"));
        assert!(!looks_numeric("127.0.0.1.example"));
    }

    #[test]
    fn deny_list_matches_suffixes() {
        for denied in [
            "localhost",
            "foo.localhost",
            "printer.local",
            "service.internal",
            "host.home.arpa",
            "instance-data",
            "metadata.google.internal",
            "metadata.goog",
            "metadata.azure.com",
        ] {
            assert!(is_denied_hostname(denied).is_some(), "{denied}");
        }
        for allowed in ["example.com", "localhost.example.com", "internal.org"] {
            assert!(is_denied_hostname(allowed).is_none(), "{allowed}");
        }
    }

    #[test]
    fn normalises_and_validates() {
        assert_eq!(normalise_hostname("Example.COM."), "example.com");
        assert!(hostname_is_well_formed("example.com"));
        assert!(!hostname_is_well_formed(""));
        assert!(!hostname_is_well_formed("bad\u{0}host"));
        assert!(!hostname_is_well_formed("bad\nhost"));
    }
}
