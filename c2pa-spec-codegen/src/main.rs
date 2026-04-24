//! `refresh-spec`: regenerates the committed Rust sources in `c2pa-spec`
//! from the CDDL / YAML / ABNF files that accompany the C2PA technical
//! specification.
//!
//! Typical workflow when bumping the spec:
//!
//! ```text
//! cargo run -p c2pa-spec-codegen -- --download
//! cargo build -p c2pa-spec
//! git diff c2pa-spec/
//! ```

mod codegen;

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use clap::Parser;

/// Specifications index on `spec.c2pa.org`. The page is an HTML
/// redirect that names the latest version in its canonical link and
/// meta-refresh target (e.g.
/// `.../specifications/specifications/2.4/index.html`); we scrape the
/// version out of there to build the schemas-bundle URL for the current
/// spec.
const SPEC_INDEX_URL: &str = "https://spec.c2pa.org/specifications/";

/// CDDL files that fail to parse with `cddl` 0.10.5 and are skipped
/// when building the master schema. Drop a corrected copy into
/// `c2pa-spec/schema-patches/` to un-exclude a file. See
/// `c2pa-spec/CDDL_ISSUES.md`.
const EXCLUDED_CDDLS: &[&str] = &[
    "jsonld.cddl",
    // Self-reference: this is the master file the codegen writes.
    "c2pa.cddl",
];

/// Regenerate c2pa-spec source files from the CDDL, YAML, and ABNF
/// schemas that accompany the C2PA technical specification.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Download the schemas bundle first, overwriting the vendored
    /// CDDL / YAML / ABNF files in `--spec-dir`.
    #[arg(short, long)]
    download: bool,

    /// Schemas bundle URL. Defaults to whichever version
    /// `https://spec.c2pa.org` currently redirects to.
    #[arg(long)]
    url: Option<String>,

    /// Path to the `c2pa-spec` crate root. Defaults to the sibling of
    /// this crate inside the workspace.
    #[arg(long, default_value_os_t = default_spec_dir())]
    spec_dir: PathBuf,

    /// Directory of manually patched CDDL files that override the
    /// downloaded / vendored versions. Any `<patches-dir>/foo.cddl`
    /// replaces `<spec-dir>/schemas/cddl/foo.cddl` when building the
    /// master schema, and automatically un-excludes its file from
    /// `EXCLUDED_CDDLS`. Defaults to `<spec-dir>/schema-patches/`.
    #[arg(long)]
    patches_dir: Option<PathBuf>,
}

fn default_spec_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("codegen manifest should have a parent workspace")
        .join("c2pa-spec")
}

/// Discover the schemas bundle URL by fetching the `spec.c2pa.org`
/// specifications index (which is a static HTML page with a
/// meta-refresh / canonical link pointing at the current versioned
/// index), extracting the version segment from the body, and pasting it
/// into the known bundle path template.
fn resolve_latest_spec_url() -> Result<String, String> {
    let body = ureq::get(SPEC_INDEX_URL)
        .call()
        .map_err(|e| format!("GET {SPEC_INDEX_URL}: {e}"))?
        .into_body()
        .read_to_string()
        .map_err(|e| format!("read body of {SPEC_INDEX_URL}: {e}"))?;

    let version = extract_spec_version(&body).ok_or_else(|| {
        format!("no `/specifications/specifications/<version>/` link found at {SPEC_INDEX_URL}")
    })?;

    Ok(format!(
        "https://spec.c2pa.org/specifications/specifications/{version}/specs/_attachments/C2PA_Schemas.zip"
    ))
}

/// Pull the first `specifications/specifications/<VERSION>/` version
/// segment out of the given HTML body. Accepts `<VERSION>` as a sequence
/// of digits and dots (e.g. `2`, `2.4`, `2.4.1`).
fn extract_spec_version(body: &str) -> Option<String> {
    let marker = "/specifications/specifications/";
    let mut cursor = 0;
    while let Some(rel) = body[cursor..].find(marker) {
        let start = cursor + rel + marker.len();
        let tail = &body[start..];
        let end = tail.find('/')?;
        let candidate = &tail[..end];
        if !candidate.is_empty() && candidate.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return Some(candidate.to_string());
        }
        cursor = start + end;
    }
    None
}

#[cfg(test)]
mod version_tests {
    use super::extract_spec_version;

    #[test]
    fn finds_version_in_canonical_link() {
        let body = r#"<link rel="canonical" href="https://spec.c2pa.org/specifications/specifications/2.4/index.html">"#;
        assert_eq!(extract_spec_version(body).as_deref(), Some("2.4"));
    }

    #[test]
    fn handles_patch_versions() {
        let body = "/specifications/specifications/2.4.1/index.html";
        assert_eq!(extract_spec_version(body).as_deref(), Some("2.4.1"));
    }

    #[test]
    fn skips_non_numeric_segments() {
        let body =
            "/specifications/specifications/draft/foo /specifications/specifications/3.0/bar";
        assert_eq!(extract_spec_version(body).as_deref(), Some("3.0"));
    }

    #[test]
    fn returns_none_when_absent() {
        assert!(extract_spec_version("nothing interesting here").is_none());
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("refresh-spec: {msg}");
    std::process::exit(2);
}

fn main() {
    let cli = Cli::parse();
    if !cli.spec_dir.is_dir() {
        fail(&format!(
            "spec dir {} does not exist",
            cli.spec_dir.display()
        ));
    }

    let schemas_dir = cli.spec_dir.join("schemas");
    let cddl_dir = schemas_dir.join("cddl");
    let patches_dir = cli
        .patches_dir
        .clone()
        .unwrap_or_else(|| cli.spec_dir.join("schema-patches"));
    let metadata_yaml = schemas_dir.join("valid_metadata_fields.yml");
    let urn_abnf = schemas_dir.join("c2pa_urn.abnf");
    let generated_rs = cli.spec_dir.join("src/generated.rs");
    let fields_rs = cli.spec_dir.join("src/valid_metadata_fields.rs");
    let cargo_toml = cli.spec_dir.join("Cargo.toml");

    // Record the spec version we start from so the workflow can tell the
    // user about it. The version lives in SemVer build metadata on
    // `c2pa-spec`'s crate version (`0.1.0+spec-2.4`).
    let old_spec = read_spec_version(&cargo_toml);

    if cli.download {
        let url = match cli.url {
            Some(u) => u,
            None => {
                let resolved = resolve_latest_spec_url()
                    .unwrap_or_else(|e| fail(&format!("could not resolve latest spec URL: {e}")));
                eprintln!("refresh-spec: resolved latest spec URL to {resolved}");
                resolved
            }
        };
        let new_spec = extract_version_from_url(&url);
        refresh_from_spec(&url, &cddl_dir, &metadata_yaml, &urn_abnf);

        if let Some(ver) = &new_spec {
            if let Err(e) = write_spec_version(&cargo_toml, ver) {
                fail(&format!("update Cargo.toml spec version: {e}"));
            }
        }

        // Emit structured lines the refresh workflow parses to build
        // the PR title.
        eprintln!(
            "refresh-spec: old_spec_version={}",
            old_spec.as_deref().unwrap_or("unknown")
        );
        eprintln!(
            "refresh-spec: new_spec_version={}",
            new_spec.as_deref().unwrap_or("unknown")
        );
    }

    let master = build_master_cddl(&cddl_dir, &patches_dir);
    let master_path = cddl_dir.join("c2pa.cddl");
    fs::write(&master_path, &master)
        .unwrap_or_else(|e| fail(&format!("write {}: {e}", master_path.display())));
    eprintln!(
        "refresh-spec: wrote {}",
        relative(&master_path, &cli.spec_dir)
    );

    emit_generated_rs(&master, &generated_rs);
    eprintln!(
        "refresh-spec: wrote {}",
        relative(&generated_rs, &cli.spec_dir)
    );

    emit_valid_metadata_fields(&metadata_yaml, &fields_rs);
    eprintln!(
        "refresh-spec: wrote {}",
        relative(&fields_rs, &cli.spec_dir)
    );
}

fn relative(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

/// Concatenate every `.cddl` file in `cddl_dir` (except the excluded
/// ones and the generated master) into a single string. Drops duplicate
/// `name = ...` rule definitions so `font-weight-range` doesn't appear
/// twice. If `patches_dir` exists, any `<patches_dir>/foo.cddl` replaces
/// `<cddl_dir>/foo.cddl` in the output and un-excludes that filename
/// from `EXCLUDED_CDDLS`.
fn build_master_cddl(cddl_dir: &Path, patches_dir: &Path) -> String {
    let patched = collect_patch_names(patches_dir);
    for name in &patched {
        eprintln!("refresh-spec: applying patch {name}");
    }

    let mut entries: Vec<PathBuf> = fs::read_dir(cddl_dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", cddl_dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "cddl"))
        .filter(|p| {
            let name = p.file_name().unwrap().to_str().unwrap();
            patched.contains(name) || !EXCLUDED_CDDLS.contains(&name)
        })
        .collect();
    entries.sort();

    // Warn about patches that don't correspond to any file in cddl_dir
    // so typos / drift from upstream surface instead of silently adding
    // a new file to the master schema.
    let cddl_names: HashSet<String> = entries
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    for name in &patched {
        if !cddl_names.contains(name) {
            eprintln!(
                "refresh-spec: warning: patch {name} has no matching file in {}",
                cddl_dir.display()
            );
        }
    }

    let excluded_report: Vec<&str> = EXCLUDED_CDDLS[..EXCLUDED_CDDLS.len() - 1]
        .iter()
        .filter(|name| !patched.contains(**name))
        .copied()
        .collect();

    let mut out = String::new();
    out.push_str("; C2PA master CDDL schema\n");
    out.push_str("; Auto-generated by `cargo run -p c2pa-spec-codegen -- --help`.\n");
    out.push_str("; Do not edit by hand.\n");
    if !excluded_report.is_empty() {
        out.push_str("; Excluded files (see CDDL_ISSUES.md): ");
        out.push_str(&excluded_report.join(", "));
        out.push('\n');
    }
    if !patched.is_empty() {
        let mut names: Vec<&str> = patched.iter().map(String::as_str).collect();
        names.sort_unstable();
        out.push_str("; Patched files (from schema-patches/): ");
        out.push_str(&names.join(", "));
        out.push('\n');
    }
    out.push('\n');

    let mut seen_eq: HashSet<String> = HashSet::new();

    for path in &entries {
        let name = path.file_name().unwrap().to_string_lossy();
        let source_path = if patched.contains(name.as_ref()) {
            patches_dir.join(name.as_ref())
        } else {
            path.clone()
        };
        let contents = fs::read_to_string(&source_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", source_path.display()));
        out.push_str("\n; ================================================================\n");
        if patched.contains(name.as_ref()) {
            out.push_str(&format!("; From {name} (patched)\n"));
        } else {
            out.push_str(&format!("; From {name}\n"));
        }
        out.push_str("; ================================================================\n");
        // Blank line so the separator comments above do not attach as
        // doc-comments to the first rule in the following file.
        out.push('\n');
        for line in contents.lines() {
            if let Some(rule_name) = parse_eq_rule_name(line) {
                if !seen_eq.insert(rule_name.to_string()) {
                    out.push_str(&format!(
                        "; [refresh-spec dedup] skipped duplicate `=` definition of {rule_name}\n"
                    ));
                    continue;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
    }

    out
}

/// Return the set of `*.cddl` file names present in `patches_dir`.
/// A missing directory yields an empty set (patches are optional).
fn collect_patch_names(patches_dir: &Path) -> HashSet<String> {
    let Ok(read) = fs::read_dir(patches_dir) else {
        return HashSet::new();
    };
    read.filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "cddl"))
        .filter_map(|p| p.file_name().and_then(|s| s.to_str()).map(String::from))
        .collect()
}

fn parse_eq_rule_name(line: &str) -> Option<&str> {
    if line.is_empty() || line.starts_with(char::is_whitespace) {
        return None;
    }
    let first = line.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '$') {
        return None;
    }
    let eq_pos = line.find('=')?;
    if line[..eq_pos].ends_with('/') {
        return None;
    }
    let before = line[..eq_pos].trim_end();
    if before.contains(char::is_whitespace) {
        return None;
    }
    Some(before)
}

/// Parse the master CDDL, run codegen, and write the output to
/// `generated.rs` with the autogen header.
fn emit_generated_rs(master_cddl: &str, dest: &Path) {
    let cddl_ast = cddl::parser::cddl_from_str(master_cddl, true)
        .unwrap_or_else(|e| panic!("parse master CDDL: {e}"));
    let body = codegen::generate_all_types(&cddl_ast, master_cddl)
        .unwrap_or_else(|e| panic!("codegen: {e}"));

    let mut out = String::new();
    out.push_str("// Auto-generated by `cargo run -p c2pa-spec-codegen`.\n");
    out.push_str("// Do not edit by hand.\n\n");
    out.push_str(&body);

    fs::write(dest, out).unwrap_or_else(|e| panic!("write {}: {e}", dest.display()));
}

/// Parse `valid_metadata_fields.yml` and write the
/// `pub mod valid_metadata_fields { ... }` contents to `dest`.
fn emit_valid_metadata_fields(yaml_path: &Path, dest: &Path) {
    let contents = fs::read_to_string(yaml_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", yaml_path.display()));
    let groups = parse_metadata_yaml(&contents);

    let mut out = String::new();
    out.push_str(
        "// Auto-generated by `cargo run -p c2pa-spec-codegen` from valid_metadata_fields.yml.\n",
    );
    out.push_str("// Do not edit by hand.\n\n");

    for (name, fields) in &groups {
        let const_name = to_upper_snake(name);
        out.push_str(&format!(
            "/// Valid metadata field names in the `{name}` group.\n"
        ));
        out.push_str(&format!("pub const {const_name}: &[&str] = &[\n"));
        for field in fields {
            out.push_str(&format!("    {},\n", rust_string_literal(field)));
        }
        out.push_str("];\n\n");
    }

    out.push_str("/// Every group as a `(group_name, fields)` pair, in source order.\n");
    out.push_str("pub const ALL: &[(&str, &[&str])] = &[\n");
    for (name, _) in &groups {
        let const_name = to_upper_snake(name);
        out.push_str(&format!(
            "    ({}, {const_name}),\n",
            rust_string_literal(name)
        ));
    }
    out.push_str("];\n");

    fs::write(dest, out).unwrap_or_else(|e| panic!("write {}: {e}", dest.display()));
}

/// Line-based parser for the subset of YAML used in
/// `valid_metadata_fields.yml` (top-level `Group:` headers with
/// `- field` list items). Returns groups in source order.
fn parse_metadata_yaml(contents: &str) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for line in contents.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let value = rest.trim().to_string();
            match current.as_mut() {
                Some((_, items)) => items.push(value),
                None => panic!("YAML list item `{value}` appeared before any group header"),
            }
        } else if let Some(name) = trimmed.strip_suffix(':') {
            if line.starts_with(|c: char| c.is_whitespace()) {
                panic!("unexpected indentation on header line: {line:?}");
            }
            if let Some(prev) = current.take() {
                groups.push(prev);
            }
            current = Some((name.trim().to_string(), Vec::new()));
        } else {
            panic!("unrecognized YAML line: {line:?}");
        }
    }
    if let Some(prev) = current.take() {
        groups.push(prev);
    }
    groups
}

fn to_upper_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev_was_lower = false;
    for c in name.chars() {
        if matches!(c, '_' | '-' | ' ' | '.') {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            prev_was_lower = false;
        } else if c.is_ascii_uppercase() {
            if prev_was_lower && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c);
            prev_was_lower = false;
        } else if c.is_ascii_lowercase() {
            out.push(c.to_ascii_uppercase());
            prev_was_lower = true;
        } else if c.is_ascii_digit() {
            out.push(c);
            prev_was_lower = false;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("UNKNOWN");
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn rust_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Download the schemas zip and refresh every `cddl/*.cddl`,
/// `valid_metadata_fields.yml`, and `c2pa_urn.abnf` in place.
fn refresh_from_spec(url: &str, cddl_dir: &Path, metadata_yaml: &Path, urn_abnf: &Path) {
    eprintln!("refresh-spec: downloading {url}");
    let response = ureq::get(url)
        .call()
        .unwrap_or_else(|e| panic!("GET {url}: {e}"));
    let mut bytes: Vec<u8> = Vec::new();
    response
        .into_body()
        .into_reader()
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("read body from {url}: {e}"));

    let cursor = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).unwrap_or_else(|e| panic!("open zip from {url}: {e}"));

    fs::create_dir_all(cddl_dir).unwrap_or_else(|e| panic!("create {}: {e}", cddl_dir.display()));

    let mut cddl_count = 0usize;
    let mut yaml_count = 0usize;
    let mut abnf_count = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .unwrap_or_else(|e| panic!("read zip entry {i}: {e}"));
        if entry.is_dir() {
            continue;
        }
        let entry_path = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };
        let file_name = match entry_path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let dest: Option<PathBuf> = if file_name.ends_with(".cddl") {
            Some(cddl_dir.join(&file_name))
        } else if file_name == "valid_metadata_fields.yml" {
            Some(metadata_yaml.to_path_buf())
        } else if file_name == "c2pa_urn.abnf" {
            Some(urn_abnf.to_path_buf())
        } else {
            None
        };
        let Some(dest) = dest else { continue };

        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut buf)
            .unwrap_or_else(|e| panic!("read {file_name} from zip: {e}"));
        let mut f =
            fs::File::create(&dest).unwrap_or_else(|e| panic!("create {}: {e}", dest.display()));
        f.write_all(&buf)
            .unwrap_or_else(|e| panic!("write {}: {e}", dest.display()));
        if !buf.ends_with(b"\n") {
            f.write_all(b"\n")
                .unwrap_or_else(|e| panic!("write trailing newline to {}: {e}", dest.display()));
        }
        if file_name.ends_with(".cddl") {
            cddl_count += 1;
        } else if file_name.ends_with(".abnf") {
            abnf_count += 1;
        } else {
            yaml_count += 1;
        }
    }

    if cddl_count == 0 {
        panic!("no .cddl entries found in archive at {url}");
    }
    eprintln!(
        "refresh-spec: refreshed {cddl_count} CDDL file(s), {yaml_count} metadata YAML, {abnf_count} ABNF file(s)"
    );
}

/// Pull the version segment out of a schemas-bundle URL
/// (`.../specifications/specifications/<VERSION>/...`).
fn extract_version_from_url(url: &str) -> Option<String> {
    let marker = "/specifications/specifications/";
    let start = url.find(marker)? + marker.len();
    let rest = &url[start..];
    let end = rest.find('/')?;
    let candidate = &rest[..end];
    if !candidate.is_empty()
        && candidate.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        Some(candidate.to_string())
    } else {
        None
    }
}

/// Read the spec version out of the `version = "X.Y.Z+spec-N"` line in
/// `c2pa-spec/Cargo.toml`. Returns `None` if the file has no
/// `+spec-<version>` build metadata.
fn read_spec_version(cargo_toml: &Path) -> Option<String> {
    let content = fs::read_to_string(cargo_toml).ok()?;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("version") || !trimmed.contains("+spec-") {
            continue;
        }
        let plus = trimmed.find("+spec-")? + "+spec-".len();
        let tail = &trimmed[plus..];
        let end = tail.find('"')?;
        return Some(tail[..end].to_string());
    }
    None
}

/// Rewrite the `+spec-<version>` build metadata on the first
/// `version = ...` line of `Cargo.toml` that carries it, leaving
/// everything else untouched.
fn write_spec_version(cargo_toml: &Path, new_version: &str) -> Result<(), String> {
    let content = fs::read_to_string(cargo_toml)
        .map_err(|e| format!("read {}: {e}", cargo_toml.display()))?;
    let mut out = String::with_capacity(content.len());
    let mut replaced = false;

    for (idx, line) in content.lines().enumerate() {
        let needs_rewrite = !replaced
            && line.trim_start().starts_with("version")
            && line.contains("+spec-");
        if needs_rewrite {
            if let (Some(plus), Some(close)) =
                (line.find("+spec-"), line.rfind('"'))
            {
                let head = &line[..plus];
                // close is the index of the closing quote; everything
                // between plus+1 and close is the spec version segment.
                if close > plus {
                    let tail = &line[close..];
                    out.push_str(head);
                    out.push_str("+spec-");
                    out.push_str(new_version);
                    out.push_str(tail);
                    out.push('\n');
                    replaced = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
        let _ = idx;
    }

    if replaced {
        fs::write(cargo_toml, out)
            .map_err(|e| format!("write {}: {e}", cargo_toml.display()))?;
    }
    Ok(())
}
