//! `ByteSize`: a parseable, formattable byte count used by every crawl budget.

#![allow(clippy::cast_precision_loss)] // display/parse use f64 for human-readable units

use std::fmt;
use std::ops::{Add, Sub};

/// A number of bytes, with human-readable parsing and formatting (binary units).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ByteSize(pub u64);

impl ByteSize {
    /// Zero bytes.
    pub const ZERO: Self = Self(0);
    /// One kibibyte.
    pub const KIB: Self = Self(1024);
    /// One mebibyte.
    pub const MIB: Self = Self(1024 * 1024);
    /// One gibibyte.
    pub const GIB: Self = Self(1024 * 1024 * 1024);

    /// The raw byte count.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Saturating addition, never overflowing a budget.
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// Parse a human string like `"5 MiB"`, `"512kb"`, `"1.5 GB"`.
    ///
    /// Accepts binary and (approximate) decimal suffixes, case-insensitively:
    /// `b`, `kib`/`kb`, `mib`/`mb`, `gib`/`gb`, `tib`/`tb`.
    ///
    /// # Errors
    /// Returns a human-readable message if the value cannot be parsed.
    pub fn parse(input: &str) -> Result<Self, String> {
        let s = input.trim();
        if s.is_empty() {
            return Err("empty byte size".to_string());
        }
        let split = s
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '_'))
            .unwrap_or(s.len());
        let (num, unit) = s.split_at(split);
        let num: f64 = num
            .replace('_', "")
            .parse()
            .map_err(|_| format!("invalid number in byte size: {input:?}"))?;
        if num < 0.0 {
            return Err(format!("negative byte size: {input:?}"));
        }
        let mult: u64 = match unit.trim().to_ascii_lowercase().as_str() {
            "" | "b" => 1,
            "k" | "kb" | "kib" => 1024,
            "m" | "mb" | "mib" => 1024 * 1024,
            "g" | "gb" | "gib" => 1024 * 1024 * 1024,
            "t" | "tb" | "tib" => 1024_u64.pow(4),
            other => return Err(format!("unknown byte-size unit: {other:?}")),
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bytes = (num * mult as f64) as u64;
        Ok(Self(bytes))
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.0;
        if n >= Self::GIB.0 {
            write!(f, "{:.2} GiB", n as f64 / Self::GIB.0 as f64)
        } else if n >= Self::MIB.0 {
            write!(f, "{:.2} MiB", n as f64 / Self::MIB.0 as f64)
        } else if n >= Self::KIB.0 {
            write!(f, "{:.2} KiB", n as f64 / Self::KIB.0 as f64)
        } else {
            write!(f, "{n} B")
        }
    }
}

impl Add for ByteSize {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for ByteSize {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0.saturating_sub(rhs.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binary_suffixes() {
        assert_eq!(ByteSize::parse("1024").unwrap(), ByteSize(1024));
        assert_eq!(ByteSize::parse("1 KiB").unwrap(), ByteSize(1024));
        assert_eq!(ByteSize::parse("1.5 MiB").unwrap(), ByteSize(1_572_864));
        assert_eq!(
            ByteSize::parse("5gb").unwrap(),
            ByteSize(5 * 1024 * 1024 * 1024)
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(ByteSize::parse("").is_err());
        assert!(ByteSize::parse("abc").is_err());
        assert!(ByteSize::parse("1 zib").is_err());
    }

    #[test]
    fn formats_readably() {
        assert_eq!(ByteSize(0).to_string(), "0 B");
        assert_eq!(ByteSize(1024).to_string(), "1.00 KiB");
        assert_eq!(ByteSize(5 * 1024 * 1024 * 1024).to_string(), "5.00 GiB");
    }

    #[test]
    fn subtraction_saturates() {
        assert_eq!(ByteSize(10) - ByteSize(100), ByteSize(0));
    }
}
