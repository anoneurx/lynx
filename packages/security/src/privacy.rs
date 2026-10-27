//! Global Privacy Control (GPC) support.
//!
//! LYNX sets no cookies and runs no tracking, so a `Sec-GPC: 1` request is honoured
//! by construction — there is nothing to turn off. The signal is still handled
//! explicitly, for two reasons:
//!
//! 1. **Enforcement, not just absence.** If a future change introduces a
//!    `Set-Cookie` anywhere, a GPC-signalling client must still not receive one.
//!    [`gpc_requested`] is the predicate the API middleware applies to the request
//!    headers to strip every `Set-Cookie` from the response — and a test asserts
//!    normal responses carry none in the first place, so the strip is defence in
//!    depth rather than the primary guarantee.
//! 2. **A documented, tested answer.** "Nothing to disable" has to be verifiable;
//!    the tests under `apps/api/tests` pin the response to a GPC-sending client:
//!    identical body, no cookies, no extra headers.
//!
//! Per the GPC spec the preference is expressed as `Sec-GPC: 1` exactly; other
//! values (including `0`) mean "no signal" and change nothing — there is no
//! opt-in state to record. This module stays free of HTTP-crate types so any
//! transport layer can use it.

/// The request header name for the Global Privacy Control signal.
pub const GPC_HEADER: &str = "Sec-GPC";

/// Whether a `Sec-GPC` header *value* expresses the GPC "do not sell or share"
/// preference.
///
/// Only the exact value `1` counts, per the GPC spec; any other value means no
/// signal. The function is deliberately total over any input string: a malformed
/// value is not an error and is not logged, because recording any per-visitor
/// detail is precisely what GPC asks us not to do.
#[must_use]
pub fn gpc_value_means_signal(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim() == "1")
}

/// Whether the request carries the GPC signal, given the `Sec-GPC` header values
/// already looked up by the caller.
///
/// Header lookup by constant name is O(1) in the HTTP frameworks LYNX uses, so the
/// caller extracts values and this applies [`gpc_value_means_signal`] to each; any
/// one signal-bearing value counts. An empty input is "no signal".
#[must_use]
pub fn gpc_requested<'a, I>(values: I) -> bool
where
    I: IntoIterator<Item = Option<&'a str>>,
{
    values.into_iter().any(gpc_value_means_signal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exact_signal_value_is_honoured() {
        assert!(gpc_value_means_signal(Some("1")));
    }

    #[test]
    fn surrounding_whitespace_is_harmless() {
        // Header libraries vary on trimming; accept both so the signal whose
        // *meaning* is unambiguous is never dropped on a technicality.
        assert!(gpc_value_means_signal(Some(" 1")));
        assert!(gpc_value_means_signal(Some("1 ")));
    }

    #[test]
    fn other_values_mean_no_signal() {
        for value in [
            None,
            Some("0"),
            Some("true"),
            Some("yes"),
            Some(""),
            Some("10"),
        ] {
            assert!(
                !gpc_value_means_signal(value),
                "value {value:?} does not count as GPC"
            );
        }
    }

    #[test]
    fn an_empty_header_list_is_no_signal() {
        assert!(!gpc_requested(std::iter::empty::<Option<&str>>()));
        assert!(!gpc_requested([None::<&str>]));
    }

    #[test]
    fn one_signal_bearing_value_among_many_counts() {
        assert!(gpc_requested([None, Some("0"), Some("1")]));
    }
}
