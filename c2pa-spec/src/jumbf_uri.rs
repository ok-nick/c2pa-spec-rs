//! Parser for JUMBF URI references (`self#jumbf=...`).
//!
//! The grammar is the `jumbf-uri-type` rule in the CDDL:
//!
//! ```text
//! jumbf-uri-type = tstr .regexp "self#jumbf=[\w\d/][\w\d./:-]+[\w\d]"
//! ```
//!
//! That is, `self#jumbf=` followed by a slash-separated path whose
//! characters come from `[A-Za-z0-9_/.:-]`. The path typically names a
//! specific box inside a JUMBF container; one of the segments is often a
//! [`urn:c2pa:` URN][Urn] identifying the active manifest.
//!
//! # Examples
//!
//! ```
//! use c2pa_spec::jumbf_uri::JumbfUri;
//!
//! let u: JumbfUri =
//!     "self#jumbf=/c2pa/urn:c2pa:12345678-1234-1234-1234-123456789abc/c2pa.assertions/c2pa.actions"
//!         .parse()
//!         .unwrap();
//!
//! assert_eq!(u.path(), "/c2pa/urn:c2pa:12345678-1234-1234-1234-123456789abc/c2pa.assertions/c2pa.actions");
//! let segments: Vec<_> = u.segments().collect();
//! assert_eq!(segments[0], "c2pa");
//! assert!(u.embedded_c2pa_urn().is_some());
//! ```

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::urn::Urn;

const PREFIX: &str = "self#jumbf=";

/// A parsed JUMBF URI reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JumbfUri {
    path: String,
}

impl JumbfUri {
    /// The path portion (everything after `self#jumbf=`).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Iterate over the `/`-separated, non-empty path segments.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.path.split('/').filter(|s| !s.is_empty())
    }

    /// Whether the path starts with `/`, meaning it's rooted at the
    /// enclosing JUMBF container (typically `/c2pa/<manifest-urn>/...`)
    /// rather than being resolved against the current manifest.
    pub fn is_absolute(&self) -> bool {
        self.path.starts_with('/')
    }

    /// Whether the path is resolved against the current manifest rather
    /// than the enclosing JUMBF container. Inverse of [`is_absolute`].
    ///
    /// [`is_absolute`]: Self::is_absolute
    pub fn is_relative(&self) -> bool {
        !self.is_absolute()
    }

    /// Return the first path segment parseable as a `urn:c2pa:` URN, if any.
    pub fn embedded_c2pa_urn(&self) -> Option<Urn> {
        self.segments()
            .filter(|s| s.starts_with("urn:c2pa:"))
            .find_map(|s| s.parse::<Urn>().ok())
    }
}

/// Reasons a JUMBF URI string can fail to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JumbfUriParseError {
    MissingPrefix,
    EmptyPath,
    InvalidChar(char),
    InvalidFirstChar(char),
    InvalidLastChar(char),
}

impl fmt::Display for JumbfUriParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPrefix => write!(f, "JUMBF URI is missing the `self#jumbf=` prefix"),
            Self::EmptyPath => write!(f, "JUMBF URI path is empty"),
            Self::InvalidChar(c) => write!(f, "JUMBF URI path contains invalid character {c:?}"),
            Self::InvalidFirstChar(c) => {
                write!(f, "JUMBF URI path begins with invalid character {c:?}")
            }
            Self::InvalidLastChar(c) => {
                write!(f, "JUMBF URI path ends with invalid character {c:?}")
            }
        }
    }
}

impl std::error::Error for JumbfUriParseError {}

impl FromStr for JumbfUri {
    type Err = JumbfUriParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let path = s
            .strip_prefix(PREFIX)
            .ok_or(JumbfUriParseError::MissingPrefix)?;
        validate_path(path)?;
        Ok(JumbfUri {
            path: path.to_string(),
        })
    }
}

impl fmt::Display for JumbfUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}{}", self.path)
    }
}

impl Serialize for JumbfUri {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for JumbfUri {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Validate against the regex
/// `[\w\d/][\w\d./:-]+[\w\d]`, interpreting `\w` as `[A-Za-z0-9_]`.
fn validate_path(path: &str) -> Result<(), JumbfUriParseError> {
    if path.is_empty() {
        return Err(JumbfUriParseError::EmptyPath);
    }
    for c in path.chars() {
        if !is_path_char(c) {
            return Err(JumbfUriParseError::InvalidChar(c));
        }
    }
    let first = path.chars().next().unwrap();
    if !is_first_char(first) {
        return Err(JumbfUriParseError::InvalidFirstChar(first));
    }
    let last = path.chars().last().unwrap();
    if !is_last_char(last) {
        return Err(JumbfUriParseError::InvalidLastChar(last));
    }
    Ok(())
}

/// Middle of the path: `[\w\d./:-]` = alnum / `_` / `.` / `/` / `:` / `-`.
fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | ':' | '-')
}

/// First char: `[\w\d/]` = alnum / `_` / `/`.
fn is_first_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '/')
}

/// Last char: `[\w\d]` = alnum / `_`.
fn is_last_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let u: JumbfUri = "self#jumbf=c2pa.assertions/c2pa.actions.v1"
            .parse()
            .unwrap();
        assert_eq!(u.path(), "c2pa.assertions/c2pa.actions.v1");
        assert_eq!(u.to_string(), "self#jumbf=c2pa.assertions/c2pa.actions.v1");
    }

    #[test]
    fn parse_leading_slash() {
        let u: JumbfUri = "self#jumbf=/c2pa/c2pa.assertions/c2pa.hash.data"
            .parse()
            .unwrap();
        let segments: Vec<_> = u.segments().collect();
        assert_eq!(segments, vec!["c2pa", "c2pa.assertions", "c2pa.hash.data"]);
    }

    #[test]
    fn absolute_vs_relative() {
        let abs: JumbfUri = "self#jumbf=/c2pa/c2pa.assertions/c2pa.hash.data"
            .parse()
            .unwrap();
        assert!(abs.is_absolute());
        assert!(!abs.is_relative());

        let rel: JumbfUri = "self#jumbf=c2pa.assertions/c2pa.hash.data".parse().unwrap();
        assert!(!rel.is_absolute());
        assert!(rel.is_relative());
    }

    #[test]
    fn extract_embedded_urn() {
        let u: JumbfUri =
            "self#jumbf=/c2pa/urn:c2pa:12345678-1234-1234-1234-123456789abc/c2pa.assertions/c2pa.actions"
                .parse()
                .unwrap();
        let urn = u.embedded_c2pa_urn().expect("should find urn");
        assert_eq!(urn.uuid, "12345678-1234-1234-1234-123456789abc");
    }

    #[test]
    fn no_embedded_urn() {
        let u: JumbfUri = "self#jumbf=c2pa.assertions/c2pa.hash.data".parse().unwrap();
        assert!(u.embedded_c2pa_urn().is_none());
    }

    #[test]
    fn missing_prefix() {
        let err: JumbfUriParseError = "c2pa.assertions".parse::<JumbfUri>().unwrap_err();
        assert_eq!(err, JumbfUriParseError::MissingPrefix);
    }

    #[test]
    fn empty_path() {
        let err: JumbfUriParseError = "self#jumbf=".parse::<JumbfUri>().unwrap_err();
        assert_eq!(err, JumbfUriParseError::EmptyPath);
    }

    #[test]
    fn space_is_invalid() {
        let err: JumbfUriParseError = "self#jumbf=has space".parse::<JumbfUri>().unwrap_err();
        assert_eq!(err, JumbfUriParseError::InvalidChar(' '));
    }

    #[test]
    fn trailing_hyphen_rejected() {
        let err: JumbfUriParseError = "self#jumbf=foo-".parse::<JumbfUri>().unwrap_err();
        assert_eq!(err, JumbfUriParseError::InvalidLastChar('-'));
    }

    #[test]
    fn serde_roundtrip() {
        let s = "self#jumbf=/c2pa/c2pa.assertions/c2pa.actions";
        let u: JumbfUri = s.parse().unwrap();
        let json = serde_json::to_string(&u).unwrap();
        assert_eq!(json, format!("\"{s}\""));
        let back: JumbfUri = serde_json::from_str(&json).unwrap();
        assert_eq!(back, u);
    }
}
