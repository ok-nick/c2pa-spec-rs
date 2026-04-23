# c2pa-spec-rs

Rust types for the [C2PA core specification schemas bundle][schemas-zip],
with the generator that produces them.

[schemas-zip]: https://spec.c2pa.org/specifications/specifications/2.4/specs/_attachments/C2PA_Schemas.zip

Two crates:

- [`c2pa-spec`](./c2pa-spec) — the committed Rust types. Downstream
  consumers depend on this crate and don't pull in `cddl`, proc macros,
  or any network access.
- [`c2pa-spec-codegen`](./c2pa-spec-codegen) — a binary that
  regenerates the committed sources from the schemas bundle. Runs
  occasionally when bumping the spec.

`c2pa-spec/schemas/` mirrors the upstream bundle (`cddl/*.cddl`,
`valid_metadata_fields.yml`, `c2pa_urn.abnf`). The Rust sources emitted
from those live under `c2pa-spec/src/`: `generated.rs` and
`valid_metadata_fields.rs` are both written by `c2pa-spec-codegen` and
should not be edited by hand. Everything else in that crate is
hand-maintained.

## Excluded CDDL files

A few files in `c2pa-spec/schemas/cddl/` don't parse with the `cddl`
0.10.5 crate and are skipped when building the master schema. See
[`c2pa-spec/CDDL_ISSUES.md`](./c2pa-spec/CDDL_ISSUES.md) for the
details. Symbols from those files that other schemas still reference
(`RegionMap`, `CoseKey`, `CoseSign1`, `CoseSign1Tagged`,
`ValidationResultsMap`, `StatusMap`, `SemverString`) are aliased to
`ciborium::Value` (or `String`) stubs in `c2pa-spec/src/lib.rs` so the
crate still compiles.

## Regenerating

```sh
cargo run -p c2pa-spec-codegen
```

Reads the committed CDDL, YAML, and ABNF files under
`c2pa-spec/schemas/` and writes `c2pa-spec/src/generated.rs`,
`c2pa-spec/src/valid_metadata_fields.rs`, and
`c2pa-spec/schemas/cddl/c2pa.cddl`. Pass `--download` to fetch a fresh
copy of the schemas bundle first:

```sh
cargo run -p c2pa-spec-codegen -- --download
```

Point at a different schemas bundle URL with `--url`, or run against a
different crate root with `--spec-dir PATH`. See
[`c2pa-spec-codegen/src/main.rs`](./c2pa-spec-codegen/src/main.rs) for
the codegen details — identifier conversion, socket/plug rule merging,
literal-to-enum promotion, the `jumbf-uri-type` substitution that swaps
`String` for `jumbf_uri::JumbfUri`, and so on.
