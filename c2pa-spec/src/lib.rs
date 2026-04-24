//! Rust types for the [C2PA] core specification, generated from the
//! [upstream CDDL schemas][schemas-zip] by the `c2pa-spec-codegen`
//! workspace tool and committed in place.
//!
//! C2PA defines a CBOR data model for provenance and authenticity
//! metadata on media assets. This crate is that model, in Rust.
//!
//! # What's inside
//!
//! The generated types cover the spec's assertions (actions, ingredient,
//! hashes, soft binding, asset type, training and mining, etc.), the
//! claim and manifest structures, hashed and external URI references,
//! validation results and status codes, and the shared metadata and
//! choice enums the other items reference. What's actually in the crate
//! is whatever shows up below in the module type listing. It tracks the
//! CDDL directly, so the set grows and shrinks with the upstream spec.
//!
//! Every generated type derives [`Clone`], [`Debug`],
//! [`serde::Serialize`], and [`serde::Deserialize`], so values
//! round-trip through any serde-compatible format. CBOR via
//! [`ciborium`] is the wire format. CDDL comments on rules and fields
//! carry over as Rust doc comments on the items they describe.
//!
//! Alongside the generated types:
//!
//! - [`valid_metadata_fields`]: valid XMP, Exif, IPTC, TIFF, PLUS, etc.
//!   metadata field names from the spec's `valid_metadata_fields.yml`,
//!   as string-slice constants.
//! - [`urn`]: a parser for `urn:c2pa:` identifiers, matching the
//!   grammar in `c2pa_urn.abnf`.
//! - [`jumbf_uri`]: a parser for `self#jumbf=...` references. Wherever
//!   the CDDL uses `jumbf-uri-type`, the generated field holds the
//!   parsed type from this module instead of a raw `String`, so callers
//!   can reach an embedded URN without parsing the string themselves.
//!
//! # The doc comments are non-normative
//!
//! The `///` text on generated types and fields is copied verbatim from
//! the CDDL. Treat it as a reminder of what a field is for, not a
//! precise definition. For authoritative semantics like value ranges,
//! version-dependent behavior, cross-field constraints, or deprecation
//! rules, go read the [C2PA technical specification][C2PA]. When the
//! docstring and the spec disagree, the spec is right.
//!
//! # How the types are generated
//!
//! The CDDL files in `cddl/`, `valid_metadata_fields.yml`, and
//! `c2pa_urn.abnf` are committed copies of the schemas that ship in the
//! C2PA [schemas bundle][schemas-zip]. The `c2pa-spec-codegen` binary
//! concatenates the CDDLs, parses them, and writes
//! `c2pa-spec/src/generated.rs` plus `c2pa-spec/src/valid_metadata_fields.rs`.
//! Running it with `--download` fetches a fresh bundle first. Regenerate
//! with:
//!
//! ```text
//! cargo run -p c2pa-spec-codegen -- --download
//! ```
//!
//! [C2PA]: https://c2pa.org
//! [schemas-zip]: https://spec.c2pa.org/specifications/specifications/2.4/specs/_attachments/C2PA_Schemas.zip
//! [`ciborium`]: https://docs.rs/ciborium

pub use ciborium::Value;

pub type CoseKey = ciborium::Value;
pub type CoseSign1 = ciborium::Value;
pub type CoseSign1Tagged = ciborium::Value;

/// Valid metadata field names grouped by vocabulary (XMP, Exif, IPTC,
/// TIFF, PLUS, Photoshop, Dublin Core, Camera Raw, and PDF), generated
/// from the spec's `valid_metadata_fields.yml`.
pub mod valid_metadata_fields {
    include!("valid_metadata_fields.rs");
}

pub mod urn;

pub mod jumbf_uri;

#[allow(rustdoc::bare_urls, rustdoc::invalid_html_tags)]
mod generated {
    use crate::{CoseKey, CoseSign1, CoseSign1Tagged};
    include!("generated.rs");
}
pub use generated::*;
