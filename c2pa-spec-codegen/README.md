# c2pa-spec-codegen

Binary that regenerates the committed Rust sources in
[`c2pa-spec`](../c2pa-spec) from the CDDL, YAML, and ABNF files in
the C2PA schemas bundle.

Fixes various issues and implements various features needed to work
with the published C2PA schemas. Most of this is expected to be
upstreamed to [`cddl-derive`][cddl] and [`cddl`][cddl] over
time.

Regenerate from the committed schemas:

```sh
cargo run -p c2pa-spec-codegen
```

Fetch the latest schemas bundle first, then regenerate:

```sh
cargo run -p c2pa-spec-codegen -- --download
```

[cddl]: https://github.com/anweiss/cddl
