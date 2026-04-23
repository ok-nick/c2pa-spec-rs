//! Internal code generation logic for converting CDDL AST to Rust source code.
//!
//! A few helpers and the `CodegenError::ParseError` variant are legacy
//! from the original proc-macro fork and are kept for parity, even though
//! the `refresh-spec` binary only uses `generate_all_types`.
#![allow(dead_code)]

use cddl::ast::{
  Group, GroupChoice, GroupEntry, MemberKey, Occur, Rule, Type, Type1, Type2, TypeChoice, TypeRule,
  ValueMemberKeyEntry, CDDL,
};
use std::fmt::Write;

/// Extracts comments from raw CDDL source and exposes them keyed by line
/// number. The `cddl` crate's pest-based parser always sets the AST
/// `comments_*` fields to `None`, but it does preserve line spans — we
/// use those together with this index to attach comments to generated
/// items as Rust doc comments.
pub(crate) struct CommentIndex<'a> {
  lines: Vec<&'a str>,
  /// Trailing `; ...` text for each line, without the leading `;` / space.
  trailing: Vec<Option<String>>,
  /// Whether each line is only a comment (optionally indented, nothing
  /// else before the `;`).
  is_comment_only: Vec<bool>,
  /// The comment text for comment-only lines.
  comment_only_text: Vec<Option<String>>,
}

impl<'a> CommentIndex<'a> {
  pub(crate) fn build(source: &'a str) -> Self {
    let lines: Vec<&str> = source.lines().collect();
    let mut trailing = Vec::with_capacity(lines.len());
    let mut is_comment_only = Vec::with_capacity(lines.len());
    let mut comment_only_text = Vec::with_capacity(lines.len());

    for line in &lines {
      match split_comment(line) {
        (Some(text), true) => {
          trailing.push(None);
          is_comment_only.push(true);
          comment_only_text.push(Some(text));
        }
        (Some(text), false) => {
          trailing.push(Some(text));
          is_comment_only.push(false);
          comment_only_text.push(None);
        }
        (None, _) => {
          trailing.push(None);
          is_comment_only.push(false);
          comment_only_text.push(None);
        }
      }
    }

    Self {
      lines,
      trailing,
      is_comment_only,
      comment_only_text,
    }
  }

  /// Empty index used when source text is not available.
  pub(crate) fn empty() -> Self {
    Self {
      lines: Vec::new(),
      trailing: Vec::new(),
      is_comment_only: Vec::new(),
      comment_only_text: Vec::new(),
    }
  }

  /// Comment text trailing on the given 1-indexed line, if any.
  pub(crate) fn trailing(&self, line: usize) -> Option<&str> {
    if line == 0 || line > self.lines.len() {
      return None;
    }
    self.trailing[line - 1].as_deref()
  }

  /// Block of standalone comment lines immediately preceding the given
  /// 1-indexed line, in source order. Stops at the first blank or
  /// non-comment line.
  pub(crate) fn preceding(&self, line: usize) -> Vec<&str> {
    if line <= 1 || line > self.lines.len() + 1 {
      return Vec::new();
    }
    let mut result = Vec::new();
    let mut idx = line as isize - 2;
    while idx >= 0 {
      let i = idx as usize;
      if self.is_comment_only[i] {
        result.push(self.comment_only_text[i].as_deref().unwrap_or(""));
      } else {
        break;
      }
      idx -= 1;
    }
    result.reverse();
    result
  }
}

/// Split a CDDL source line at the first `;` outside a string literal.
/// Returns `(comment_text, is_comment_only)`. `comment_text` has one
/// leading space stripped (so `; foo` yields `"foo"`).
fn split_comment(line: &str) -> (Option<String>, bool) {
  let mut in_string = false;
  let mut escape = false;
  for (byte_idx, ch) in line.char_indices() {
    if in_string {
      if escape {
        escape = false;
      } else if ch == '\\' {
        escape = true;
      } else if ch == '"' {
        in_string = false;
      }
      continue;
    }
    if ch == '"' {
      in_string = true;
      continue;
    }
    if ch == ';' {
      let before = &line[..byte_idx];
      let after = &line[byte_idx + 1..];
      let text = after.strip_prefix(' ').unwrap_or(after).trim_end();
      let is_only = before.trim().is_empty();
      return (Some(text.to_string()), is_only);
    }
  }
  (None, false)
}

/// Return the 1-indexed starting line number of a CDDL span.
fn span_line(span: &cddl::ast::Span) -> usize {
  span.2
}

/// Errors that can occur during code generation.
#[derive(Debug)]
pub(crate) enum CodegenError {
  /// CDDL parsing failed.
  ParseError(String),
  /// Formatting error.
  FmtError(std::fmt::Error),
}

impl std::fmt::Display for CodegenError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      CodegenError::ParseError(msg) => write!(f, "CDDL parse error: {}", msg),
      CodegenError::FmtError(e) => write!(f, "formatting error: {}", e),
    }
  }
}

impl From<std::fmt::Error> for CodegenError {
  fn from(e: std::fmt::Error) -> Self {
    CodegenError::FmtError(e)
  }
}

/// A generated Rust type definition.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RustTypeDef {
  Struct {
    name: String,
    fields: Vec<RustField>,
    docs: Vec<String>,
  },
  TypeAlias {
    name: String,
    target: String,
    docs: Vec<String>,
  },
  Enum {
    name: String,
    variants: Vec<RustEnumVariant>,
    docs: Vec<String>,
  },
}

/// A field within a generated Rust struct.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RustField {
  pub name: String,
  pub original_name: String,
  pub rust_type: String,
  pub is_optional: bool,
  pub docs: Vec<String>,
}

/// A variant within a generated Rust enum.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RustEnumVariant {
  pub name: String,
  pub inner_type: Option<String>,
  /// Original CDDL identifier or literal value to use for `#[serde(rename)]`.
  /// Only set when it differs from `name`.
  pub serde_rename: Option<String>,
  pub docs: Vec<String>,
}

/// Generate Rust source code for all rules in a parsed CDDL AST.
pub(crate) fn generate_all_types(
  cddl: &CDDL<'_>,
  source: &str,
) -> Result<String, CodegenError> {
  let comments = CommentIndex::build(source);
  let type_defs = collect_type_defs(cddl, &comments)?;
  render_type_defs(&type_defs)
}

/// Generate Rust source code for a single named rule in a parsed CDDL AST.
///
/// If `output_name` is provided, the generated type will use that name instead
/// of the name derived from the CDDL rule. This allows the `#[cddl]` attribute
/// macro to preserve the user's chosen struct name.
pub(crate) fn generate_single_type(
  cddl: &CDDL<'_>,
  source: &str,
  rule_name: &str,
  output_name: Option<&str>,
) -> Result<String, CodegenError> {
  let comments = CommentIndex::build(source);
  let type_defs = collect_type_defs(cddl, &comments)?;
  let matching = type_defs
    .into_iter()
    .find(|d| match d {
      RustTypeDef::Struct { name, .. }
      | RustTypeDef::TypeAlias { name, .. }
      | RustTypeDef::Enum { name, .. } => name == rule_name,
    })
    .ok_or_else(|| {
      CodegenError::ParseError(format!(
        "no rule matching '{}' found in CDDL definition",
        rule_name
      ))
    })?;

  let mut output = String::new();
  match &matching {
    RustTypeDef::Struct { name, fields, docs } => {
      let emit_name = output_name.unwrap_or(name);
      render_struct(&mut output, emit_name, fields, docs)?;
    }
    RustTypeDef::TypeAlias { name, target, docs } => {
      let emit_name = output_name.unwrap_or(name);
      render_type_alias(&mut output, emit_name, target, docs)?;
    }
    RustTypeDef::Enum {
      name,
      variants,
      docs,
    } => {
      let emit_name = output_name.unwrap_or(name);
      render_enum(&mut output, emit_name, variants, docs)?;
    }
  }
  Ok(output)
}

/// Convert a PascalCase struct name to a CDDL-style kebab-case identifier.
pub(crate) fn pascal_to_cddl_name(pascal: &str) -> String {
  let mut result = String::with_capacity(pascal.len() + 4);
  for (i, c) in pascal.chars().enumerate() {
    if c.is_uppercase() {
      if i > 0 {
        result.push('-');
      }
      result.push(c.to_lowercase().next().unwrap());
    } else {
      result.push(c);
    }
  }
  result
}

// --- Internal helpers (unchanged from original codegen) ---

fn collect_type_defs(
  cddl: &CDDL<'_>,
  comments: &CommentIndex<'_>,
) -> Result<Vec<RustTypeDef>, CodegenError> {
  let mut defs: Vec<RustTypeDef> = Vec::new();
  for rule in &cddl.rules {
    match rule {
      Rule::Type {
        rule: type_rule,
        span,
        ..
      } => {
        // Skip rules that have a hand-written Rust substitute; the
        // consumer crate provides the real type.
        if rule_substitution(type_rule.name.ident).is_some() {
          continue;
        }
        let docs = preceding_docs_for(comments, span);
        if let Some(def) = type_rule_to_rust_def(type_rule, comments, docs)? {
          push_or_merge(&mut defs, def);
        }
      }
      Rule::Group {
        rule: group_rule,
        span,
        ..
      } => {
        if rule_substitution(group_rule.name.ident).is_some() {
          continue;
        }
        let name = to_pascal_case(group_rule.name.ident);
        let docs = preceding_docs_for(comments, span);
        if let Some(fields) = group_entry_to_fields(&group_rule.entry, comments)? {
          push_or_merge(
            &mut defs,
            RustTypeDef::Struct {
              name,
              fields,
              docs,
            },
          );
        }
      }
    }
  }
  for def in &mut defs {
    if let RustTypeDef::Struct { fields, .. } = def {
      disambiguate_field_names(fields);
    }
  }
  Ok(defs)
}

fn preceding_docs_for(comments: &CommentIndex<'_>, span: &cddl::ast::Span) -> Vec<String> {
  let line = span_line(span);
  comments
    .preceding(line)
    .into_iter()
    .map(String::from)
    .collect()
}

/// Push a RustTypeDef, merging into any existing definition with the same
/// name. CDDL socket/plug extensions (`$foo /= A` then `$foo /= B`) produce
/// multiple `Rule`s with the same identifier; emitting duplicate Rust items
/// would not compile. This merges them by folding additional type aliases /
/// structs / enums into an untagged enum.
fn push_or_merge(defs: &mut Vec<RustTypeDef>, new_def: RustTypeDef) {
  let new_name = def_name(&new_def).to_string();
  if let Some(idx) = defs.iter().position(|d| def_name(d) == new_name) {
    let existing = defs.remove(idx);
    defs.insert(idx, merge_defs(existing, new_def));
  } else {
    defs.push(new_def);
  }
}

fn def_name(def: &RustTypeDef) -> &str {
  match def {
    RustTypeDef::Struct { name, .. }
    | RustTypeDef::TypeAlias { name, .. }
    | RustTypeDef::Enum { name, .. } => name,
  }
}

fn merge_defs(existing: RustTypeDef, new: RustTypeDef) -> RustTypeDef {
  let name = def_name(&existing).to_string();
  let existing_docs = def_docs(&existing);
  let new_docs = def_docs(&new);

  let mut variants = Vec::new();
  extend_variants_from(&mut variants, existing);
  extend_variants_from(&mut variants, new);

  let mut seen: Vec<(String, Option<String>)> = Vec::new();
  variants.retain(|v| {
    let key = (v.name.clone(), v.inner_type.clone());
    if seen.contains(&key) {
      false
    } else {
      seen.push(key);
      true
    }
  });

  let mut name_counts: std::collections::HashMap<String, usize> =
    std::collections::HashMap::new();
  for v in &mut variants {
    let count = name_counts.entry(v.name.clone()).or_insert(0);
    if *count > 0 {
      v.name = format!("{}{}", v.name, *count + 1);
    }
    *count += 1;
  }

  // Keep the earliest preceding-comment block as the enum-level docs.
  let docs = if !existing_docs.is_empty() {
    existing_docs
  } else {
    new_docs
  };
  RustTypeDef::Enum {
    name,
    variants,
    docs,
  }
}

fn def_docs(def: &RustTypeDef) -> Vec<String> {
  match def {
    RustTypeDef::Struct { docs, .. }
    | RustTypeDef::TypeAlias { docs, .. }
    | RustTypeDef::Enum { docs, .. } => docs.clone(),
  }
}

fn extend_variants_from(variants: &mut Vec<RustEnumVariant>, def: RustTypeDef) {
  match def {
    RustTypeDef::TypeAlias { target, docs, .. } => {
      let variant_name = variant_name_from_type(&target);
      variants.push(RustEnumVariant {
        name: variant_name,
        inner_type: Some(target),
        serde_rename: None,
        docs,
      });
    }
    RustTypeDef::Enum {
      variants: mut inner,
      docs,
      ..
    } => {
      // If the enum has rule-level docs and exactly one variant, carry them
      // to that variant so merged `/=` extensions keep their line comments.
      if !docs.is_empty() && inner.len() == 1 && inner[0].docs.is_empty() {
        inner[0].docs = docs;
      }
      variants.append(&mut inner);
    }
    RustTypeDef::Struct { name, docs, .. } => {
      variants.push(RustEnumVariant {
        name: name.clone(),
        inner_type: Some(name),
        serde_rename: None,
        docs,
      });
    }
  }
}

fn variant_name_from_type(target: &str) -> String {
  let base = target.rsplit("::").next().unwrap_or(target);
  let base = base.split('<').next().unwrap_or(base);
  if base.is_empty() {
    "Variant".to_string()
  } else {
    to_pascal_case(base)
  }
}

fn disambiguate_field_names(fields: &mut [RustField]) {
  let mut counts: std::collections::HashMap<String, usize> =
    std::collections::HashMap::new();
  for f in fields.iter_mut() {
    let entry = counts.entry(f.name.clone()).or_insert(0);
    if *entry > 0 {
      f.name = format!("{}_{}", f.name, *entry + 1);
      // After disambiguation there is no single "original" key to honor;
      // align original_name with the new Rust name so the serde rename
      // attribute is not emitted. Multiple fields claiming the same
      // rename produce unreachable match arms in serde's deserializer.
      f.original_name = f.name.clone();
    }
    *entry += 1;
  }
}

fn type_rule_to_rust_def(
  rule: &TypeRule<'_>,
  comments: &CommentIndex<'_>,
  docs: Vec<String>,
) -> Result<Option<RustTypeDef>, CodegenError> {
  let name = to_pascal_case(rule.name.ident);
  let ty = &rule.value;

  if ty.type_choices.len() > 1 {
    return Ok(Some(type_choices_to_enum(&name, ty, docs)?));
  }

  if let Some(tc) = ty.type_choices.first() {
    let type1 = &tc.type1;
    // Range expression: `a..b` / `a...b`. Alias to the primitive of the
    // left-hand value so socket/plug merging can combine ranges with other
    // type aliases under a single enum.
    if type1.operator.is_some() {
      let target = range_primitive(&type1.type2);
      return Ok(Some(RustTypeDef::TypeAlias {
        name,
        target,
        docs,
      }));
    }
    match &type1.type2 {
      Type2::Map { group, .. } => {
        let fields = group_to_fields(group, comments)?;
        Ok(Some(RustTypeDef::Struct { name, fields, docs }))
      }
      Type2::Array { group, .. } => {
        let rust_type = array_group_to_type(group)?;
        Ok(Some(RustTypeDef::TypeAlias {
          name,
          target: rust_type,
          docs,
        }))
      }
      Type2::Typename { ident, .. } => {
        let target = cddl_ident_to_rust_type(ident.ident);
        Ok(Some(RustTypeDef::TypeAlias { name, target, docs }))
      }
      Type2::ParenthesizedType { pt, .. } => {
        if pt.type_choices.len() > 1 {
          return Ok(Some(type_choices_to_enum(&name, pt, docs)?));
        }
        let target = type_to_rust_string(pt)?;
        Ok(Some(RustTypeDef::TypeAlias { name, target, docs }))
      }
      Type2::Unwrap { ident, .. } => {
        let target = to_pascal_case(ident.ident);
        Ok(Some(RustTypeDef::TypeAlias { name, target, docs }))
      }
      // Single literal-value rules become a single-variant enum so that
      // repeated `$foo /= "literal"` socket extensions can be folded into
      // one enum by `push_or_merge`. The rule-level docs attach to the
      // variant, so that merging extensions preserves each line's comment.
      Type2::TextValue { value, .. } => {
        let variant_name = to_pascal_case(value);
        let original = value.to_string();
        let serde_rename = if variant_name == original {
          None
        } else {
          Some(original)
        };
        Ok(Some(RustTypeDef::Enum {
          name,
          variants: vec![RustEnumVariant {
            name: variant_name,
            inner_type: None,
            serde_rename,
            docs: docs.clone(),
          }],
          docs,
        }))
      }
      Type2::IntValue { value, .. } => Ok(Some(RustTypeDef::Enum {
        name,
        variants: vec![RustEnumVariant {
          name: if *value < 0 {
            format!("Neg{}", value.unsigned_abs())
          } else {
            format!("N{}", value)
          },
          inner_type: None,
          serde_rename: None,
          docs: docs.clone(),
        }],
        docs,
      })),
      Type2::UintValue { value, .. } => Ok(Some(RustTypeDef::Enum {
        name,
        variants: vec![RustEnumVariant {
          name: format!("N{}", value),
          inner_type: None,
          serde_rename: None,
          docs: docs.clone(),
        }],
        docs,
      })),
      Type2::FloatValue { value, .. } => Ok(Some(RustTypeDef::Enum {
        name,
        variants: vec![RustEnumVariant {
          name: format!("F{}", value.to_string().replace(['.', '-'], "_")),
          inner_type: None,
          serde_rename: None,
          docs: docs.clone(),
        }],
        docs,
      })),
      Type2::ChoiceFromInlineGroup { group, .. } => {
        let variants = group_to_enum_variants(group)?;
        Ok(Some(RustTypeDef::Enum {
          name,
          variants,
          docs,
        }))
      }
      Type2::ChoiceFromGroup { ident, .. } => {
        let target = to_pascal_case(ident.ident);
        Ok(Some(RustTypeDef::TypeAlias { name, target, docs }))
      }
      Type2::TaggedData { t, .. } => {
        let target = type_to_rust_string(t)?;
        Ok(Some(RustTypeDef::TypeAlias { name, target, docs }))
      }
      _ => Ok(None),
    }
  } else {
    Ok(None)
  }
}

fn range_primitive(lhs: &Type2<'_>) -> String {
  match lhs {
    Type2::IntValue { .. } => "i64".to_string(),
    Type2::UintValue { .. } => "u64".to_string(),
    Type2::FloatValue { .. } => "f64".to_string(),
    Type2::Typename { ident, .. } => cddl_ident_to_rust_type(ident.ident),
    _ => "i64".to_string(),
  }
}

fn type_choices_to_enum(
  name: &str,
  ty: &Type<'_>,
  docs: Vec<String>,
) -> Result<RustTypeDef, CodegenError> {
  let mut variants = Vec::new();
  for tc in &ty.type_choices {
    let variant = type_choice_to_variant(tc)?;
    variants.push(variant);
  }
  Ok(RustTypeDef::Enum {
    name: name.to_string(),
    variants,
    docs,
  })
}

fn type_choice_to_variant(tc: &TypeChoice<'_>) -> Result<RustEnumVariant, CodegenError> {
  let type1 = &tc.type1;
  match &type1.type2 {
    Type2::Typename { ident, .. } => {
      let ident_str = ident.ident;
      let variant_name = to_pascal_case(ident_str);
      let inner = if is_prelude_type(ident_str) {
        Some(cddl_ident_to_rust_type(ident_str))
      } else {
        Some(variant_name.clone())
      };
      Ok(RustEnumVariant {
        name: variant_name,
        inner_type: inner,
        serde_rename: None,
        docs: Vec::new(),
      })
    }
    Type2::TextValue { value, .. } => {
      let variant_name = to_pascal_case(value);
      let original = value.to_string();
      let serde_rename = if variant_name == original {
        None
      } else {
        Some(original)
      };
      Ok(RustEnumVariant {
        name: variant_name,
        inner_type: None,
        serde_rename,
        docs: Vec::new(),
      })
    }
    Type2::IntValue { value, .. } => {
      let variant_name = if *value < 0 {
        format!("Neg{}", value.unsigned_abs())
      } else {
        format!("N{}", value)
      };
      Ok(RustEnumVariant {
        name: variant_name,
        inner_type: None,
        serde_rename: None,
        docs: Vec::new(),
      })
    }
    Type2::UintValue { value, .. } => Ok(RustEnumVariant {
      name: format!("N{}", value),
      inner_type: None,
      serde_rename: None,
      docs: Vec::new(),
    }),
    Type2::FloatValue { value, .. } => Ok(RustEnumVariant {
      name: format!("F{}", value.to_string().replace(['.', '-'], "_")),
      inner_type: None,
      serde_rename: None,
      docs: Vec::new(),
    }),
    Type2::Map { group, .. } => {
      let fields = group_to_fields(group, &CommentIndex::empty())?;
      let variant_name = if fields.is_empty() {
        "Empty".to_string()
      } else {
        fields
          .iter()
          .map(|f| to_pascal_case(&f.original_name))
          .collect::<Vec<_>>()
          .join("")
      };
      Ok(RustEnumVariant {
        name: variant_name,
        inner_type: None,
        serde_rename: None,
        docs: Vec::new(),
      })
    }
    Type2::Array { .. } => Ok(RustEnumVariant {
      name: "Array".to_string(),
      inner_type: Some("Vec<()>".to_string()),
      serde_rename: None,
      docs: Vec::new(),
    }),
    Type2::ParenthesizedType { pt, .. } => {
      let rust_type = type_to_rust_string(pt)?;
      let variant_name = to_pascal_case(&rust_type);
      Ok(RustEnumVariant {
        name: variant_name,
        inner_type: Some(rust_type),
        serde_rename: None,
        docs: Vec::new(),
      })
    }
    _ => Ok(RustEnumVariant {
      name: "Unknown".to_string(),
      inner_type: None,
      serde_rename: None,
      docs: Vec::new(),
    }),
  }
}

fn group_to_fields(
  group: &Group<'_>,
  comments: &CommentIndex<'_>,
) -> Result<Vec<RustField>, CodegenError> {
  let mut fields = Vec::new();
  for gc in &group.group_choices {
    let gc_fields = group_choice_to_fields(gc, comments)?;
    fields.extend(gc_fields);
  }
  Ok(fields)
}

fn group_choice_to_fields(
  gc: &GroupChoice<'_>,
  comments: &CommentIndex<'_>,
) -> Result<Vec<RustField>, CodegenError> {
  let mut fields = Vec::new();
  for (entry, _optional_comma) in &gc.group_entries {
    if let Some(mut entry_fields) = group_entry_to_fields(entry, comments)? {
      fields.append(&mut entry_fields);
    }
  }
  Ok(fields)
}

fn group_entry_to_fields(
  entry: &GroupEntry<'_>,
  comments: &CommentIndex<'_>,
) -> Result<Option<Vec<RustField>>, CodegenError> {
  match entry {
    GroupEntry::ValueMemberKey { ge, span, .. } => {
      let docs = field_docs_for(comments, span);
      if let Some(field) = value_member_key_to_field(ge, docs)? {
        Ok(Some(vec![field]))
      } else {
        Ok(None)
      }
    }
    GroupEntry::TypeGroupname { ge, span, .. } => {
      let ident = ge.name.ident;
      let rust_type = cddl_ident_to_rust_type(ident);
      let field_name = to_snake_case(ident);
      let is_optional = ge
        .occur
        .as_ref()
        .map(|o| matches!(o.occur, Occur::Optional { .. }))
        .unwrap_or(false);
      let docs = field_docs_for(comments, span);
      Ok(Some(vec![RustField {
        name: field_name,
        original_name: ident.to_string(),
        rust_type,
        is_optional,
        docs,
      }]))
    }
    GroupEntry::InlineGroup { group, occur, .. } => {
      let is_optional = occur
        .as_ref()
        .map(|o| matches!(o.occur, Occur::Optional { .. }))
        .unwrap_or(false);
      let mut fields = group_to_fields(group, comments)?;
      if is_optional {
        for f in &mut fields {
          f.is_optional = true;
        }
      }
      Ok(Some(fields))
    }
  }
}

fn field_docs_for(comments: &CommentIndex<'_>, span: &cddl::ast::Span) -> Vec<String> {
  let line = span_line(span);
  let mut docs: Vec<String> = comments
    .preceding(line)
    .into_iter()
    .map(String::from)
    .collect();
  if let Some(trailing) = comments.trailing(line) {
    docs.push(trailing.to_string());
  }
  docs
}

fn value_member_key_to_field(
  vmke: &ValueMemberKeyEntry<'_>,
  docs: Vec<String>,
) -> Result<Option<RustField>, CodegenError> {
  let (field_name, original_name) = match &vmke.member_key {
    Some(MemberKey::Bareword { ident, .. }) => {
      (to_snake_case(ident.ident), ident.ident.to_string())
    }
    Some(MemberKey::Value { value, .. }) => {
      let s = value.to_string();
      let s = s.trim_matches('"');
      (to_snake_case(s), s.to_string())
    }
    Some(MemberKey::Type1 { t1, .. }) => {
      let key_type = type1_to_rust_string(t1)?;
      let value_type = type_to_rust_string(&vmke.entry_type)?;
      let rust_type = format!("std::collections::HashMap<{}, {}>", key_type, value_type);
      return Ok(Some(RustField {
        name: "entries".to_string(),
        original_name: "entries".to_string(),
        rust_type,
        is_optional: false,
        docs,
      }));
    }
    None => {
      let rust_type = type_to_rust_string(&vmke.entry_type)?;
      return Ok(Some(RustField {
        name: "value".to_string(),
        original_name: "value".to_string(),
        rust_type,
        is_optional: false,
        docs,
      }));
    }
    _ => return Ok(None),
  };

  let is_optional = vmke
    .occur
    .as_ref()
    .map(|o| matches!(o.occur, Occur::Optional { .. }))
    .unwrap_or(false);

  let is_vec = vmke
    .occur
    .as_ref()
    .map(|o| is_vec_occurrence(&o.occur))
    .unwrap_or(false);

  let rust_type = type_to_rust_string(&vmke.entry_type)?;

  let final_type = if is_vec {
    format!("Vec<{}>", rust_type)
  } else {
    rust_type
  };

  Ok(Some(RustField {
    name: field_name,
    original_name,
    rust_type: final_type,
    is_optional,
    docs,
  }))
}

fn is_vec_occurrence(occur: &Occur) -> bool {
  match occur {
    Occur::ZeroOrMore { .. } | Occur::OneOrMore { .. } => true,
    Occur::Exact { upper: Some(u), .. } => *u > 1,
    _ => false,
  }
}

fn array_group_to_type(group: &Group<'_>) -> Result<String, CodegenError> {
  if group.group_choices.len() == 1 {
    let gc = &group.group_choices[0];
    if gc.group_entries.len() == 1 {
      let (entry, _) = &gc.group_entries[0];
      match entry {
        GroupEntry::ValueMemberKey { ge, .. } => {
          let element_type = type_to_rust_string(&ge.entry_type)?;
          return Ok(format!("Vec<{}>", element_type));
        }
        GroupEntry::TypeGroupname { ge, .. } => {
          let element_type = cddl_ident_to_rust_type(ge.name.ident);
          return Ok(format!("Vec<{}>", element_type));
        }
        _ => {}
      }
    }
    if !gc.group_entries.is_empty() {
      let mut types = Vec::new();
      for (entry, _) in &gc.group_entries {
        match entry {
          GroupEntry::ValueMemberKey { ge, .. } => {
            types.push(type_to_rust_string(&ge.entry_type)?);
          }
          GroupEntry::TypeGroupname { ge, .. } => {
            types.push(cddl_ident_to_rust_type(ge.name.ident));
          }
          _ => types.push("()".to_string()),
        }
      }
      if types.len() == 1 {
        return Ok(format!("Vec<{}>", types[0]));
      }
      return Ok(format!("({})", types.join(", ")));
    }
  }
  Ok("Vec<()>".to_string())
}

fn type_to_rust_string(ty: &Type<'_>) -> Result<String, CodegenError> {
  if ty.type_choices.len() == 1 {
    return type1_to_rust_string(&ty.type_choices[0].type1);
  }
  if ty.type_choices.len() == 2 {
    let (a, b) = (&ty.type_choices[0].type1, &ty.type_choices[1].type1);
    if is_null_type(&b.type2) {
      let inner = type1_to_rust_string(a)?;
      return Ok(format!("Option<{}>", inner));
    }
    if is_null_type(&a.type2) {
      let inner = type1_to_rust_string(b)?;
      return Ok(format!("Option<{}>", inner));
    }
  }
  Ok("ciborium::Value".to_string())
}

fn type1_to_rust_string(type1: &Type1<'_>) -> Result<String, CodegenError> {
  type2_to_rust_string(&type1.type2)
}

fn type2_to_rust_string(type2: &Type2<'_>) -> Result<String, CodegenError> {
  match type2 {
    Type2::Typename { ident, .. } => Ok(cddl_ident_to_rust_type(ident.ident)),
    Type2::Map { group, .. } => {
      if let Some(field) = detect_table_type(group)? {
        Ok(field)
      } else {
        Ok("ciborium::Value".to_string())
      }
    }
    Type2::Array { group, .. } => array_group_to_type(group),
    Type2::TextValue { .. } => Ok("String".to_string()),
    Type2::IntValue { .. } => Ok("i64".to_string()),
    Type2::UintValue { .. } => Ok("u64".to_string()),
    Type2::FloatValue { .. } => Ok("f64".to_string()),
    Type2::UTF8ByteString { .. } | Type2::B16ByteString { .. } | Type2::B64ByteString { .. } => {
      // CDDL byte strings -> CBOR major type 2. `serde_bytes::ByteBuf`
      // wraps a `Vec<u8>` and serializes as a byte string; a raw
      // `Vec<u8>` would encode as a CBOR array of `u8`s.
      Ok("serde_bytes::ByteBuf".to_string())
    }
    Type2::ParenthesizedType { pt, .. } => type_to_rust_string(pt),
    Type2::Unwrap { ident, .. } => Ok(to_pascal_case(ident.ident)),
    Type2::TaggedData { t, .. } => type_to_rust_string(t),
    Type2::Any { .. } => Ok("ciborium::Value".to_string()),
    Type2::ChoiceFromInlineGroup { .. } | Type2::ChoiceFromGroup { .. } => {
      Ok("ciborium::Value".to_string())
    }
    Type2::DataMajorType { mt, .. } => Ok(major_type_to_rust(*mt)),
  }
}

fn detect_table_type(group: &Group<'_>) -> Result<Option<String>, CodegenError> {
  if group.group_choices.len() != 1 {
    return Ok(None);
  }
  let gc = &group.group_choices[0];
  if gc.group_entries.len() != 1 {
    return Ok(None);
  }
  let (entry, _) = &gc.group_entries[0];
  if let GroupEntry::ValueMemberKey { ge, .. } = entry {
    if let Some(MemberKey::Type1 { t1, .. }) = &ge.member_key {
      let key_type = type1_to_rust_string(t1)?;
      let value_type = type_to_rust_string(&ge.entry_type)?;
      return Ok(Some(format!(
        "std::collections::HashMap<{}, {}>",
        key_type, value_type
      )));
    }
  }
  Ok(None)
}

fn is_null_type(type2: &Type2<'_>) -> bool {
  matches!(type2, Type2::Typename { ident, .. } if ident.ident == "null" || ident.ident == "nil")
}

fn group_to_enum_variants(group: &Group<'_>) -> Result<Vec<RustEnumVariant>, CodegenError> {
  let mut variants = Vec::new();
  for gc in &group.group_choices {
    for (entry, _) in &gc.group_entries {
      let variant = group_entry_to_variant(entry)?;
      variants.push(variant);
    }
  }
  Ok(variants)
}

fn group_entry_to_variant(entry: &GroupEntry<'_>) -> Result<RustEnumVariant, CodegenError> {
  match entry {
    GroupEntry::ValueMemberKey { ge, .. } => {
      let variant_name = match &ge.member_key {
        Some(MemberKey::Bareword { ident, .. }) => to_pascal_case(ident.ident),
        Some(MemberKey::Value { value, .. }) => {
          let s = value.to_string();
          to_pascal_case(s.trim_matches('"'))
        }
        _ => "Variant".to_string(),
      };
      let inner = type_to_rust_string(&ge.entry_type)?;
      Ok(RustEnumVariant {
        name: variant_name,
        inner_type: Some(inner),
        serde_rename: None,
        docs: Vec::new(),
      })
    }
    GroupEntry::TypeGroupname { ge, .. } => {
      let variant_name = to_pascal_case(ge.name.ident);
      let inner = cddl_ident_to_rust_type(ge.name.ident);
      Ok(RustEnumVariant {
        name: variant_name.clone(),
        inner_type: if is_prelude_type(ge.name.ident) {
          Some(inner)
        } else {
          Some(variant_name)
        },
        serde_rename: None,
        docs: Vec::new(),
      })
    }
    GroupEntry::InlineGroup { .. } => Ok(RustEnumVariant {
      name: "Group".to_string(),
      inner_type: None,
      serde_rename: None,
      docs: Vec::new(),
    }),
  }
}

/// CDDL rule names that should be replaced with hand-written Rust types.
/// Mapping is from CDDL identifier to an in-scope Rust type name at the
/// macro's call site. Rules in this table are not emitted as generated
/// type aliases or structs (the consumer provides the real definition).
const RULE_SUBSTITUTIONS: &[(&str, &str)] = &[
  ("jumbf-uri-type", "crate::jumbf_uri::JumbfUri"),
];

pub(crate) fn rule_substitution(cddl_ident: &str) -> Option<&'static str> {
  RULE_SUBSTITUTIONS
    .iter()
    .find(|(k, _)| *k == cddl_ident)
    .map(|(_, v)| *v)
}

fn cddl_ident_to_rust_type(ident: &str) -> String {
  if let Some(sub) = rule_substitution(ident) {
    return sub.to_string();
  }
  match ident {
    "bool" | "true" | "false" => "bool".to_string(),
    "uint" | "unsigned" => "u64".to_string(),
    "nint" => "i64".to_string(),
    "int" | "integer" => "i64".to_string(),
    "float16" | "float32" | "float64" | "float16-32" | "float32-64" | "float" => "f64".to_string(),
    "number" => "f64".to_string(),
    "tstr" | "text" => "String".to_string(),
    "bstr" | "bytes" => "serde_bytes::ByteBuf".to_string(),
    "null" | "nil" => "()".to_string(),
    "any" => "ciborium::Value".to_string(),
    "undefined" => "()".to_string(),
    "tdate" => "String".to_string(),
    "time" => "i64".to_string(),
    "uri" => "String".to_string(),
    "b64url" | "b64legacy" => "String".to_string(),
    "regexp" => "String".to_string(),
    "biguint" | "bignint" | "bigint" => "serde_bytes::ByteBuf".to_string(),
    _ => to_pascal_case(ident),
  }
}

fn major_type_to_rust(mt: u8) -> String {
  match mt {
    0 => "u64".to_string(),
    1 => "i64".to_string(),
    2 => "serde_bytes::ByteBuf".to_string(),
    3 => "String".to_string(),
    4 => "Vec<ciborium::Value>".to_string(),
    5 => "std::collections::HashMap<String, ciborium::Value>".to_string(),
    7 => "bool".to_string(),
    _ => "ciborium::Value".to_string(),
  }
}

fn is_prelude_type(ident: &str) -> bool {
  matches!(
    ident,
    "bool"
      | "true"
      | "false"
      | "uint"
      | "unsigned"
      | "nint"
      | "int"
      | "integer"
      | "float16"
      | "float32"
      | "float64"
      | "float16-32"
      | "float32-64"
      | "float"
      | "number"
      | "tstr"
      | "text"
      | "bstr"
      | "bytes"
      | "null"
      | "nil"
      | "any"
      | "undefined"
      | "tdate"
      | "time"
      | "uri"
      | "b64url"
      | "b64legacy"
      | "regexp"
      | "biguint"
      | "bignint"
      | "bigint"
  )
}

fn render_type_defs(defs: &[RustTypeDef]) -> Result<String, CodegenError> {
  let mut output = String::new();

  for (idx, def) in defs.iter().enumerate() {
    if idx > 0 {
      output.push('\n');
    }
    match def {
      RustTypeDef::Struct { name, fields, docs } => {
        render_struct(&mut output, name, fields, docs)?;
      }
      RustTypeDef::TypeAlias { name, target, docs } => {
        render_type_alias(&mut output, name, target, docs)?;
      }
      RustTypeDef::Enum {
        name,
        variants,
        docs,
      } => {
        render_enum(&mut output, name, variants, docs)?;
      }
    }
  }

  Ok(output)
}

fn render_docs(output: &mut String, indent: &str, docs: &[String]) -> Result<(), CodegenError> {
  for line in docs {
    for sub in line.split('\n') {
      // Rust's doc-comment parser is lenient, but writing the slash-star
      // sequence inside a `///` line is still fine. We trim trailing
      // whitespace to keep the generated output tidy.
      let trimmed = sub.trim_end();
      if trimmed.is_empty() {
        writeln!(output, "{}///", indent)?;
      } else {
        writeln!(output, "{}/// {}", indent, trimmed)?;
      }
    }
  }
  Ok(())
}

fn render_struct(
  output: &mut String,
  name: &str,
  fields: &[RustField],
  docs: &[String],
) -> Result<(), CodegenError> {
  render_docs(output, "", docs)?;
  writeln!(
    output,
    "#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]"
  )?;
  writeln!(output, "pub struct {} {{", name)?;
  for field in fields {
    render_docs(output, "    ", &field.docs)?;
    if field.name != field.original_name {
      writeln!(output, "    #[serde(rename = \"{}\")]", field.original_name)?;
    }
    if field.is_optional {
      writeln!(
        output,
        "    #[serde(skip_serializing_if = \"Option::is_none\")]"
      )?;
      writeln!(
        output,
        "    pub {}: Option<{}>,",
        field.name, field.rust_type
      )?;
    } else {
      writeln!(output, "    pub {}: {},", field.name, field.rust_type)?;
    }
  }
  writeln!(output, "}}")?;
  Ok(())
}

fn render_type_alias(
  output: &mut String,
  name: &str,
  target: &str,
  docs: &[String],
) -> Result<(), CodegenError> {
  render_docs(output, "", docs)?;
  writeln!(output, "pub type {} = {};", name, target)?;
  Ok(())
}

fn render_enum(
  output: &mut String,
  name: &str,
  variants: &[RustEnumVariant],
  docs: &[String],
) -> Result<(), CodegenError> {
  render_docs(output, "", docs)?;
  writeln!(
    output,
    "#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]"
  )?;
  writeln!(output, "#[serde(untagged)]")?;
  writeln!(output, "pub enum {} {{", name)?;
  for variant in variants {
    render_docs(output, "    ", &variant.docs)?;
    if let Some(rename) = &variant.serde_rename {
      writeln!(
        output,
        "    #[serde(rename = \"{}\")]",
        rename.replace('\\', "\\\\").replace('"', "\\\"")
      )?;
    }
    if let Some(inner) = &variant.inner_type {
      writeln!(output, "    {}({}),", variant.name, inner)?;
    } else {
      writeln!(output, "    {},", variant.name)?;
    }
  }
  writeln!(output, "}}")?;
  Ok(())
}

pub(crate) fn to_pascal_case(s: &str) -> String {
  // Split into chunks on explicit separators and on lowercase->uppercase
  // transitions, then title-case each chunk. This lets ALL-CAPS CDDL
  // rule names like `EXCLUSION_RANGE-map` collapse to `ExclusionRangeMap`
  // while camelCase inputs like `validationResults` still break at the
  // `R` and yield `ValidationResults`.
  let mut chunks: Vec<String> = Vec::new();
  let mut current = String::new();
  let mut prev_was_lower = false;
  for c in s.chars() {
    if matches!(c, '-' | '_' | '.' | ' ' | ':' | '/' | '@') {
      if !current.is_empty() {
        chunks.push(std::mem::take(&mut current));
      }
      prev_was_lower = false;
    } else if c.is_ascii_alphanumeric() {
      if c.is_ascii_uppercase() && prev_was_lower && !current.is_empty() {
        chunks.push(std::mem::take(&mut current));
      }
      current.push(c);
      prev_was_lower = c.is_ascii_lowercase();
    } else {
      // Any other punctuation acts as a separator.
      if !current.is_empty() {
        chunks.push(std::mem::take(&mut current));
      }
      prev_was_lower = false;
    }
  }
  if !current.is_empty() {
    chunks.push(current);
  }

  let mut result = String::new();
  for chunk in chunks {
    let mut chars = chunk.chars();
    if let Some(first) = chars.next() {
      result.extend(first.to_uppercase());
      for c in chars {
        result.extend(c.to_lowercase());
      }
    }
  }

  if let Some(first) = result.chars().next() {
    if first.is_ascii_digit() {
      result.insert(0, '_');
    }
  }
  if result.is_empty() {
    return "Unknown".to_string();
  }
  result
}

fn to_snake_case(s: &str) -> String {
  let mut result = String::with_capacity(s.len() + 4);
  let mut prev_was_upper = false;
  let mut prev_was_separator = false;
  for (i, c) in s.chars().enumerate() {
    if c == '-' || c == '.' || c == ' ' || c == ':' || c == '/' || c == '@' {
      if !result.is_empty() && !prev_was_separator {
        result.push('_');
      }
      prev_was_separator = true;
      prev_was_upper = false;
    } else if c.is_uppercase() {
      if i > 0 && !prev_was_upper && !prev_was_separator {
        result.push('_');
      }
      result.push(c.to_lowercase().next().unwrap());
      prev_was_upper = true;
      prev_was_separator = false;
    } else if c.is_ascii_alphanumeric() || c == '_' {
      result.push(c);
      prev_was_upper = false;
      // Treat a literal underscore as a separator so a following upper
      // case letter does not trigger a second insert (e.g. the CDDL
      // key `informational_URI` should snake-case to `informational_uri`,
      // not `informational__uri`).
      prev_was_separator = c == '_';
    } else {
      // Drop any other punctuation so we end up with a valid Rust ident.
      if !result.is_empty() && !prev_was_separator {
        result.push('_');
      }
      prev_was_separator = true;
      prev_was_upper = false;
    }
  }
  // Trim trailing underscores that resulted from the separator handling.
  while result.ends_with('_') && result.len() > 1 {
    result.pop();
  }
  // Ensure the identifier starts with a letter or underscore (Rust rule).
  if let Some(first) = result.chars().next() {
    if first.is_ascii_digit() {
      result.insert(0, '_');
    }
  }
  if is_rust_keyword(&result) {
    result.push('_');
  }
  if result.is_empty() {
    return "value".to_string();
  }
  result
}

fn is_rust_keyword(s: &str) -> bool {
  matches!(
    s,
    "as"
      | "async"
      | "await"
      | "break"
      | "const"
      | "continue"
      | "crate"
      | "dyn"
      | "else"
      | "enum"
      | "extern"
      | "false"
      | "fn"
      | "for"
      | "if"
      | "impl"
      | "in"
      | "let"
      | "loop"
      | "match"
      | "mod"
      | "move"
      | "mut"
      | "pub"
      | "ref"
      | "return"
      | "self"
      | "Self"
      | "static"
      | "struct"
      | "super"
      | "trait"
      | "true"
      | "type"
      | "unsafe"
      | "use"
      | "where"
      | "while"
      | "yield"
      | "box"
  )
}

#[cfg(test)]
mod tests {
  use super::*;
  use cddl::parser::cddl_from_str;

  fn generate(input: &str) -> String {
    let cddl = cddl_from_str(input, true).unwrap();
    generate_all_types(&cddl, input).unwrap()
  }

  #[test]
  fn test_to_pascal_case() {
    assert_eq!(to_pascal_case("my-type"), "MyType");
    assert_eq!(to_pascal_case("my_type"), "MyType");
    assert_eq!(to_pascal_case("person"), "Person");
    assert_eq!(to_pascal_case("http-request"), "HttpRequest");
  }

  #[test]
  fn test_to_snake_case() {
    assert_eq!(to_snake_case("myType"), "my_type");
    assert_eq!(to_snake_case("my-type"), "my_type");
    assert_eq!(to_snake_case("type"), "type_");
    assert_eq!(to_snake_case("self"), "self_");
  }

  #[test]
  fn test_pascal_to_cddl_name() {
    assert_eq!(pascal_to_cddl_name("Person"), "person");
    assert_eq!(pascal_to_cddl_name("MyType"), "my-type");
    assert_eq!(pascal_to_cddl_name("HttpRequest"), "http-request");
  }

  #[test]
  fn test_simple_struct() {
    let result = generate(
      r#"
      person = {
        name: tstr,
        age: uint,
      }
    "#,
    );
    assert!(result.contains("pub struct Person"));
    assert!(result.contains("pub name: String,"));
    assert!(result.contains("pub age: u64,"));
  }

  #[test]
  fn test_optional_fields() {
    let result = generate(
      r#"
      person = {
        name: tstr,
        ? nickname: tstr,
      }
    "#,
    );
    assert!(result.contains("pub nickname: Option<String>,"));
    assert!(result.contains("skip_serializing_if"));
  }

  #[test]
  fn test_type_choices_enum() {
    let result = generate(r#"value = int / tstr / bool"#);
    assert!(result.contains("pub enum Value"));
    assert!(result.contains("Int(i64)"));
    assert!(result.contains("Tstr(String)"));
  }

  #[test]
  fn test_type_alias() {
    let result = generate(r#"name = tstr"#);
    assert!(result.contains("pub type Name = String;"));
  }

  #[test]
  fn test_single_type_generation() {
    let source = r#"
      address = { street: tstr, city: tstr }
      person = { name: tstr, home: address }
    "#;
    let cddl = cddl_from_str(source, true).unwrap();
    let result = generate_single_type(&cddl, source, "Person", None).unwrap();
    assert!(result.contains("pub struct Person"));
    assert!(!result.contains("pub struct Address"));
  }

  #[test]
  fn test_single_type_with_output_name_override() {
    let source = r#"
      address = { street: tstr, city: tstr }
    "#;
    let cddl = cddl_from_str(source, true).unwrap();
    let result = generate_single_type(&cddl, source, "Address", Some("Addr")).unwrap();
    assert!(result.contains("pub struct Addr"));
    assert!(!result.contains("pub struct Address"));
  }

  #[test]
  fn test_hyphenated_names_serde_rename() {
    let result = generate(
      r#"
      my-record = {
        first-name: tstr,
      }
    "#,
    );
    assert!(result.contains("#[serde(rename = \"first-name\")]"));
    assert!(result.contains("pub first_name: String,"));
  }

  #[test]
  fn test_nullable_type() {
    let result = generate(
      r#"
      record = {
        value: tstr / null,
      }
    "#,
    );
    assert!(result.contains("pub value: Option<String>,"));
  }

  #[test]
  fn test_keyword_escaping() {
    let result = generate(
      r#"
      my-record = {
        type: tstr,
      }
    "#,
    );
    assert!(result.contains("pub type_: String,"));
    assert!(result.contains("#[serde(rename = \"type\")]"));
  }
}
