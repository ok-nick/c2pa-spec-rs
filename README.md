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
