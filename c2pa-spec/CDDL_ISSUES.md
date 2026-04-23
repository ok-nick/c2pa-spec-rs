# CDDL Parser Issues

These 4 CDDL files in `schemas/cddl/` fail to parse with the `cddl` crate
v0.10.5. They are currently excluded from `schemas/cddl/c2pa.cddl` and
therefore no Rust types are generated for them. Review each and decide
whether to patch our local copies, fix upstream, or leave excluded.

## ai-disclosure.cddl:16

```
$scientific-domain-list /= 1* $scientific-domain-string
```

The `1*` occurrence prefix is only valid inside an array `[...]` or group
`(...)`. Wrapping as `[1* $scientific-domain-string]` would resolve it.

## jsonld.cddl:63

```
"@container": $$container-group-choice, ; ...
```

`$$container-group-choice` is a group socket (double `$`) being used in a
type position (as the value of a map entry). The parser expects an
identifier or type socket (`$name`) here, not a group socket.

## regions-of-interest.cddl:54

```
? "start": tstr .regexp "^(?:\d+(?:\.\d*)?|[\d]+\:[0-5]?\d\:[0-5]?\d(?:\.\d*)?)$", ; ...
```

The regex string contains unescaped backslash sequences (`\d`, `\:`) that
the CDDL parser does not accept. In CDDL string literals, the backslashes
need to be doubled (`\\d`, `\\:`) or the ranges re-expressed without escapes.

## time-stamp.cddl:4

```
* $$time-stamp-entry => bstr
```

`$$time-stamp-entry` is a group socket being used as the key of a map
entry. Like the jsonld case above, the parser does not accept `$$name`
in a map key type position.

---

Raw parser output for reference:

```
=== ai-disclosure.cddl ===
error: parser errors
   ┌─ input:16:29
   │
16 │ $scientific-domain-list /= 1* $scientific-domain-string
   │                             ^ expected one of: rule definition, type choice operator '/', range operator ('..' or '...'), control operator
   │
   = At line 16, column 29: Expected rule definition or type choice operator '/' or range operator ('..' or '...') or control operator.

=== jsonld.cddl ===
error: parser errors
   ┌─ input:63:16
   │
63 │ "@container": $$container-group-choice, ; Used to set the default container type for a term
   │                ^^^^^^^^^^^^^^^^^^^^^^^ expected identifier
   │
   = At line 63, column 16: Expected identifier.

=== regions-of-interest.cddl ===
error: parser errors
   ┌─ input:54:33
   │
54 │     ? "start":     tstr .regexp "^(?:\d+(?:\.\d*)?|[\d]+\:[0-5]?\d\:[0-5]?\d(?:\.\d*)?)$",     ; start time (or beginning of asset if not present). Inclusive.
   │                                 ^ expected type value
   │
   = At line 54, column 33: Expected type value.

=== time-stamp.cddl ===
error: parser errors
  ┌─ input:4:24
  │
4 │   * $$time-stamp-entry => bstr
  │                        ^ expected one of: generic arguments '<...>', group_choice_op, group entry
  │
  = At line 4, column 24: Expected generic arguments '<...>' or group_choice_op or group entry.
```
