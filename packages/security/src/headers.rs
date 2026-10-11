//! The canonical security header set, applied by every response path.
//!
//! Defined once so the API, admin surface, and any future static host cannot drift.
//! `Strict-Transport-Security` is only emitted when serving over TLS.

/// A response header as `(name, value)`.
pub type Header = (&'static str, String);

/// Build the security headers for a response.
///
/// `hsts` should be true only when the connection is TLS-terminated and the operator
/// has confirmed HTTPS-only serving — emitting HSTS over plain HTTP is a no-op at best
/// and a lockout risk at worst.
#[must_use]
pub fn security_headers(hsts: bool, hsts_max_age_seconds: u64) -> Vec<Header> {
    let mut headers: Vec<Header> = vec![
        (
            "Content-Security-Policy",
            "default-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"
                .to_string(),
        ),
        ("X-Content-Type-Options", "nosniff".to_string()),
        ("Referrer-Policy", "no-referrer".to_string()),
        ("X-Frame-Options", "DENY".to_string()),
        (
            "Permissions-Policy",
            "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), \
             microphone=(), payment=(), usb=()"
                .to_string(),
        ),
        ("Cross-Origin-Opener-Policy", "same-origin".to_string()),
        ("Cross-Origin-Resource-Policy", "same-origin".to_string()),
        ("Cross-Origin-Embedder-Policy", "require-corp".to_string()),
        (
            "Cache-Control",
            "no-store, no-cache, must-revalidate".to_string(),
        ),
        ("Pragma", "no-cache".to_string()),
    ];
    if hsts {
        headers.push((
            "Strict-Transport-Security",
            format!("max-age={hsts_max_age_seconds}; includeSubDomains"),
        ));
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_includes_baseline_headers() {
        let h = security_headers(false, 0);
        let names: Vec<_> = h.iter().map(|(n, _)| *n).collect();
        for expected in [
            "Content-Security-Policy",
            "X-Content-Type-Options",
            "Referrer-Policy",
            "X-Frame-Options",
            "Permissions-Policy",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }

    #[test]
    fn hsts_only_when_requested() {
        assert!(!security_headers(false, 0)
            .iter()
            .any(|(n, _)| *n == "Strict-Transport-Security"));
        let h = security_headers(true, 31_536_000);
        let hsts = h.iter().find(|(n, _)| *n == "Strict-Transport-Security");
        assert_eq!(
            hsts.map(|(_, v)| v.as_str()),
            Some("max-age=31536000; includeSubDomains")
        );
    }

    #[test]
    fn csp_denies_by_default() {
        let h = security_headers(false, 0);
        let csp = h
            .iter()
            .find(|(n, _)| *n == "Content-Security-Policy")
            .unwrap();
        assert!(csp.1.starts_with("default-src 'none'"));
    }
}
