//! C2PA URN parsing and construction.
//!
//! The grammar is the one given in `c2pa_urn.abnf` at the crate root,
//! vendored verbatim from the upstream schemas bundle:
//!
//! ```text
//! urn:c2pa:<uuid>[:<claim-generator>[:<version>_<reason>]]
//! ```
//!
//! - `<uuid>` is an 8-4-4-4-12 hex UUID (case-insensitive; the original
//!   case is preserved on round-trip).
//! - `<claim-generator>` is 0 to 32 visible non-space characters
//!   (`%x21-7E` or `%x80-FF`). Empty is valid per the ABNF.
//! - `<version>` and `<reason>` are non-negative decimal integers.
//!
//! # Examples
//!
//! ```
//! use c2pa_spec::urn::{Urn, Generator, VersionReason};
//!
//! let parsed: Urn = "urn:c2pa:12345678-1234-1234-1234-123456789abc:acme:2_1"
//!     .parse()
//!     .unwrap();
//!
//! assert_eq!(parsed.uuid, "12345678-1234-1234-1234-123456789abc");
//! assert_eq!(
//!     parsed.generator,
//!     Some(Generator {
//!         identifier: "acme".into(),
//!         version_reason: Some(VersionReason { version: 2, reason: 1 }),
//!     }),
//! );
//! assert_eq!(
//!     parsed.to_string(),
//!     "urn:c2pa:12345678-1234-1234-1234-123456789abc:acme:2_1",
//! );
//! ```

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Maximum length (in chars) of a claim-generator identifier. Pulled
/// straight from the ABNF: `claim-generator-identifier = 0*32...`.
pub const MAX_GENERATOR_LEN: usize = 32;

const PREFIX: &str = "urn:c2pa:";
const UUID_LEN: usize = 36;

/// A parsed `urn:c2pa:` URN.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Urn {
    /// The UUID portion, as it appeared in the source (case preserved).
    pub uuid: String,
    /// The claim-generator and optional version/reason, if present.
    pub generator: Option<Generator>,
}

/// A claim-generator identifier (possibly empty) plus optional version/reason.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Generator {
    /// 0..=[`MAX_GENERATOR_LEN`] characters, each visible-non-space.
    pub identifier: String,
    pub version_reason: Option<VersionReason>,
}

/// The `<version>_<reason>` trailer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VersionReason {
    pub version: u32,
    pub reason: u32,
}

/// Reasons a URN string can fail to parse against the ABNF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrnParseError {
    /// Input did not start with `urn:c2pa:`.
    MissingPrefix,
    /// UUID portion was not exactly 36 chars in `8-4-4-4-12` hex form.
    InvalidUuid,
    /// Claim-generator identifier exceeded 32 characters.
    GeneratorTooLong { length: usize },
    /// Claim-generator identifier contained a space or control character.
    InvalidGeneratorChar(char),
    /// `<version>_<reason>` trailer was malformed (empty half, non-digit,
    /// missing underscore, or overflowed `u32`).
    InvalidVersionReason,
    /// Input had extra content after the recognized URN.
    TrailingContent,
}

impl fmt::Display for UrnParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPrefix => write!(f, "URN is missing the `urn:c2pa:` prefix"),
            Self::InvalidUuid => write!(f, "URN does not contain a valid 8-4-4-4-12 UUID"),
            Self::GeneratorTooLong { length } => write!(
                f,
                "claim-generator identifier is {length} chars, max is {MAX_GENERATOR_LEN}"
            ),
            Self::InvalidGeneratorChar(c) => {
                write!(f, "claim-generator contains invalid character {c:?}")
            }
            Self::InvalidVersionReason => {
                write!(f, "version_reason trailer is not `<digits>_<digits>`")
            }
            Self::TrailingContent => write!(f, "URN has unexpected trailing content"),
        }
    }
}

impl std::error::Error for UrnParseError {}

impl Urn {
    /// Build a URN from its UUID part.
    pub fn from_uuid(uuid: impl Into<String>) -> Result<Self, UrnParseError> {
        let uuid = uuid.into();
        validate_uuid(&uuid)?;
        Ok(Self {
            uuid,
            generator: None,
        })
    }

    /// Attach a claim-generator identifier (validated).
    pub fn with_generator(mut self, generator: Generator) -> Result<Self, UrnParseError> {
        validate_generator_identifier(&generator.identifier)?;
        self.generator = Some(generator);
        Ok(self)
    }
}

impl FromStr for Urn {
    type Err = UrnParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s.strip_prefix(PREFIX).ok_or(UrnParseError::MissingPrefix)?;

        if rest.len() < UUID_LEN {
            return Err(UrnParseError::InvalidUuid);
        }
        let uuid_str = &rest[..UUID_LEN];
        validate_uuid(uuid_str)?;
        let rest = &rest[UUID_LEN..];

        if rest.is_empty() {
            return Ok(Urn {
                uuid: uuid_str.to_string(),
                generator: None,
            });
        }

        let rest = rest
            .strip_prefix(':')
            .ok_or(UrnParseError::TrailingContent)?;

        let (generator_id, vr_rest) = match rest.find(':') {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };
        validate_generator_identifier(generator_id)?;

        let version_reason = match vr_rest {
            Some(vr) => Some(parse_version_reason(vr)?),
            None => None,
        };

        Ok(Urn {
            uuid: uuid_str.to_string(),
            generator: Some(Generator {
                identifier: generator_id.to_string(),
                version_reason,
            }),
        })
    }
}

impl fmt::Display for Urn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}{}", self.uuid)?;
        if let Some(g) = &self.generator {
            write!(f, ":{}", g.identifier)?;
            if let Some(vr) = &g.version_reason {
                write!(f, ":{}_{}", vr.version, vr.reason)?;
            }
        }
        Ok(())
    }
}

impl Serialize for Urn {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Urn {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

fn validate_uuid(s: &str) -> Result<(), UrnParseError> {
    if s.len() != UUID_LEN {
        return Err(UrnParseError::InvalidUuid);
    }
    let groups: Vec<&str> = s.split('-').collect();
    if groups.len() != 5 {
        return Err(UrnParseError::InvalidUuid);
    }
    let expected = [8, 4, 4, 4, 12];
    for (g, &want) in groups.iter().zip(expected.iter()) {
        if g.len() != want || !g.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(UrnParseError::InvalidUuid);
        }
    }
    Ok(())
}

fn validate_generator_identifier(s: &str) -> Result<(), UrnParseError> {
    let char_count = s.chars().count();
    if char_count > MAX_GENERATOR_LEN {
        return Err(UrnParseError::GeneratorTooLong { length: char_count });
    }
    for c in s.chars() {
        if !is_visible_non_space(c) {
            return Err(UrnParseError::InvalidGeneratorChar(c));
        }
    }
    Ok(())
}

/// ABNF: `visible-char-except-space = %x21-7E / %x80-FF`.
fn is_visible_non_space(c: char) -> bool {
    let v = c as u32;
    (0x21..=0x7E).contains(&v) || (0x80..=0xFF).contains(&v)
}

fn parse_version_reason(s: &str) -> Result<VersionReason, UrnParseError> {
    let (v, r) = s
        .split_once('_')
        .ok_or(UrnParseError::InvalidVersionReason)?;
    if v.is_empty() || r.is_empty() {
        return Err(UrnParseError::InvalidVersionReason);
    }
    let version = v
        .parse::<u32>()
        .map_err(|_| UrnParseError::InvalidVersionReason)?;
    let reason = r
        .parse::<u32>()
        .map_err(|_| UrnParseError::InvalidVersionReason)?;
    Ok(VersionReason { version, reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "12345678-1234-1234-1234-123456789abc";

    fn urn_bare() -> String {
        format!("urn:c2pa:{UUID}")
    }

    #[test]
    fn parse_uuid_only() {
        let u: Urn = urn_bare().parse().unwrap();
        assert_eq!(u.uuid, UUID);
        assert!(u.generator.is_none());
        assert_eq!(u.to_string(), urn_bare());
    }

    #[test]
    fn parse_with_generator() {
        let s = format!("urn:c2pa:{UUID}:acme");
        let u: Urn = s.parse().unwrap();
        let g = u.generator.clone().unwrap();
        assert_eq!(g.identifier, "acme");
        assert!(g.version_reason.is_none());
        assert_eq!(u.to_string(), s);
    }

    #[test]
    fn parse_with_empty_generator() {
        let s = format!("urn:c2pa:{UUID}:");
        let u: Urn = s.parse().unwrap();
        let g = u.generator.clone().unwrap();
        assert_eq!(g.identifier, "");
        assert_eq!(u.to_string(), s);
    }

    #[test]
    fn parse_full() {
        let s = format!("urn:c2pa:{UUID}:acme:2_1");
        let u: Urn = s.parse().unwrap();
        let g = u.generator.clone().unwrap();
        let vr = g.version_reason.clone().unwrap();
        assert_eq!(g.identifier, "acme");
        assert_eq!(vr.version, 2);
        assert_eq!(vr.reason, 1);
        assert_eq!(u.to_string(), s);
    }

    #[test]
    fn parse_uppercase_uuid_preserved() {
        let s = "urn:c2pa:12345678-ABCD-1234-1234-123456789ABC";
        let u: Urn = s.parse().unwrap();
        assert_eq!(u.uuid, "12345678-ABCD-1234-1234-123456789ABC");
        assert_eq!(u.to_string(), s);
    }

    #[test]
    fn missing_prefix() {
        let err: UrnParseError = "c2pa:uuid".parse::<Urn>().unwrap_err();
        assert_eq!(err, UrnParseError::MissingPrefix);
    }

    #[test]
    fn bad_uuid_shape() {
        let err: UrnParseError = "urn:c2pa:not-a-uuid-really-no-way-long-enough"
            .parse::<Urn>()
            .unwrap_err();
        assert_eq!(err, UrnParseError::InvalidUuid);
    }

    #[test]
    fn bad_uuid_non_hex() {
        let err: UrnParseError = "urn:c2pa:gggggggg-1234-1234-1234-123456789abc"
            .parse::<Urn>()
            .unwrap_err();
        assert_eq!(err, UrnParseError::InvalidUuid);
    }

    #[test]
    fn generator_too_long() {
        let long = "x".repeat(33);
        let s = format!("urn:c2pa:{UUID}:{long}");
        match s.parse::<Urn>() {
            Err(UrnParseError::GeneratorTooLong { length }) => assert_eq!(length, 33),
            other => panic!("expected GeneratorTooLong, got {other:?}"),
        }
    }

    #[test]
    fn generator_rejects_space() {
        let s = format!("urn:c2pa:{UUID}:ac me");
        match s.parse::<Urn>() {
            Err(UrnParseError::InvalidGeneratorChar(' ')) => {}
            other => panic!("expected InvalidGeneratorChar(' '), got {other:?}"),
        }
    }

    #[test]
    fn bad_version_reason_missing_underscore() {
        let s = format!("urn:c2pa:{UUID}:acme:21");
        assert_eq!(
            s.parse::<Urn>().unwrap_err(),
            UrnParseError::InvalidVersionReason
        );
    }

    #[test]
    fn bad_version_reason_empty_half() {
        let s = format!("urn:c2pa:{UUID}:acme:_1");
        assert_eq!(
            s.parse::<Urn>().unwrap_err(),
            UrnParseError::InvalidVersionReason
        );
    }

    #[test]
    fn bad_version_reason_non_digit() {
        let s = format!("urn:c2pa:{UUID}:acme:a_1");
        assert_eq!(
            s.parse::<Urn>().unwrap_err(),
            UrnParseError::InvalidVersionReason
        );
    }

    #[test]
    fn serde_roundtrip() {
        let s = format!("urn:c2pa:{UUID}:acme:2_1");
        let u: Urn = s.parse().unwrap();
        let json = serde_json::to_string(&u).unwrap();
        assert_eq!(json, format!("\"{s}\""));
        let back: Urn = serde_json::from_str(&json).unwrap();
        assert_eq!(back, u);
    }

    #[test]
    fn builder_from_uuid() {
        let u = Urn::from_uuid(UUID)
            .unwrap()
            .with_generator(Generator {
                identifier: "acme".into(),
                version_reason: Some(VersionReason {
                    version: 2,
                    reason: 1,
                }),
            })
            .unwrap();
        assert_eq!(u.to_string(), format!("urn:c2pa:{UUID}:acme:2_1"));
    }

    #[test]
    fn builder_rejects_long_generator() {
        let long = "x".repeat(33);
        let err = Urn::from_uuid(UUID)
            .unwrap()
            .with_generator(Generator {
                identifier: long,
                version_reason: None,
            })
            .unwrap_err();
        assert!(matches!(
            err,
            UrnParseError::GeneratorTooLong { length: 33 }
        ));
    }
}
