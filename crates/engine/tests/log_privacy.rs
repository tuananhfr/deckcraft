//! Log records never name the user's documents (spec §07): a `log::` macro in the apps, the UI or
//! the engine that interpolates a name, path, title or URL must go through `log_safe` instead.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

const MACROS: [&str; 5] = ["error!(", "warn!(", "info!(", "debug!(", "trace!("];
/// Identifiers that hold a document's name, path, URL or server id in this code base.
const SUSPECT: [&str; 13] = ["name", "path", "url", "uri", "href", "title", "file", "filename", "file_name", "f", "p", "dir", "doc_id"];
/// Error values: a record that names a document (`log_name`) may be quoting it in its error too.
const ERRORS: [&str; 4] = ["e", "err", "error", "detail"];
/// Wrapping a value in one of these makes it safe to log.
const SAFE: [&str; 3] = ["log_name(", "redact(", "tilde_home("];
/// (file, text of the record) that interpolate a suspect identifier which is not the user's.
const ALLOWED: [(&str, &str); 2] = [
    // The account hook's JavaScript method name.
    ("apps/deckcraft-web/src/account.rs", "\"erpAccount.{name} failed: {e:?}\""),
    // Interface font families (`f` is a face).
    ("crates/ui-egui/src/uifonts.rs", "\"interface fallback fonts: {}\""),
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if path.is_dir() {
            if name != "tests" && name != "target" {
                sources(&path, out);
            }
        } else if name.ends_with(".rs") && !name.ends_with("tests.rs") {
            out.push(path);
        }
    }
}

/// The text between the macro's parentheses, honouring string literals.
fn macro_args(text: &str) -> &str {
    let (mut depth, mut in_str, mut escaped) = (1usize, false, false);
    for (i, c) in text.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_str => escaped = true,
            '"' => in_str = !in_str,
            '(' | '[' | '{' if !in_str => depth += 1,
            ')' | ']' | '}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return &text[..i];
                }
            }
            _ => {}
        }
    }
    text
}

/// Top-level comma-separated arguments.
fn split_args(args: &str) -> Vec<&str> {
    let (mut out, mut start, mut depth, mut in_str, mut escaped) = (vec![], 0, 0usize, false, false);
    for (i, c) in args.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_str => escaped = true,
            '"' => in_str = !in_str,
            '(' | '[' | '{' if !in_str => depth += 1,
            ')' | ']' | '}' if !in_str => depth = depth.saturating_sub(1),
            ',' if !in_str && depth == 0 => {
                out.push(args[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(args[start..].trim());
    out
}

fn identifiers(expr: &str) -> impl Iterator<Item = &str> {
    expr.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|s| !s.is_empty())
}

/// The suspect identifiers a record interpolates.
fn suspects(args: &str) -> Vec<String> {
    let parts = split_args(args);
    let mut found = vec![];
    let Some(format) = parts.first().filter(|f| f.starts_with('"')) else { return found };
    let names_document = args.contains("log_name(");
    let mut rest = *format;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        if let Some(escaped) = rest.strip_prefix('{') {
            rest = escaped;
            continue;
        }
        let capture = rest.split(['}', ':']).next().unwrap_or("");
        if SUSPECT.contains(&capture) || (names_document && ERRORS.contains(&capture)) {
            found.push(format!("{{{capture}}}"));
        }
    }
    for arg in parts.iter().skip(1) {
        if SAFE.iter().any(|safe| arg.contains(safe)) {
            continue;
        }
        if identifiers(arg).any(|id| SUSPECT.contains(&id) || id == "display" || (names_document && ERRORS.contains(&id))) {
            found.push(arg.to_string());
        }
    }
    found
}

#[test]
fn log_records_do_not_interpolate_document_names_paths_or_urls() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = vec![];
    for dir in ["apps", "crates/ui-egui/src", "crates/engine/src"] {
        sources(&root.join(dir), &mut files);
    }
    assert!(files.len() > 100, "only {} source files found; is the scan still pointed at the sources?", files.len());
    let (mut records, mut offending, mut allowed_used) = (0, vec![], vec![false; ALLOWED.len()]);
    for path in files {
        let rel = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        let text = std::fs::read_to_string(&path).unwrap();
        for mac in MACROS {
            for (at, _) in text.match_indices(mac) {
                if text[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                records += 1;
                let args = macro_args(&text[at + mac.len()..]);
                let found = suspects(args);
                if found.is_empty() {
                    continue;
                }
                if let Some(i) = ALLOWED.iter().position(|(file, record)| rel == *file && args.contains(record)) {
                    allowed_used[i] = true;
                    continue;
                }
                let line = text[..at].lines().count();
                offending.push(format!("{rel}:{line}: {} ({})", args.split_whitespace().collect::<Vec<_>>().join(" "), found.join(", ")));
            }
        }
    }
    assert!(records > 40, "only {records} log records found");
    assert!(offending.is_empty(), "log records naming documents; use deckcraft_engine::log_safe: {offending:#?}");
    for ((file, record), used) in ALLOWED.iter().zip(allowed_used) {
        assert!(used, "stale allowlist entry {file}: {record}");
    }
}

#[test]
fn the_scan_catches_names_paths_and_titles() {
    assert_eq!(suspects(r#""{f}: {e}""#), ["{f}"]);
    assert_eq!(suspects(r#""couldn't save {name}: {e}""#), ["{name}"]);
    assert_eq!(suspects(r#""log file {}", path.display()"#), ["path.display()"]);
    assert_eq!(suspects(r#""closed {} is too large", c.title"#), ["c.title"]);
    assert!(suspects(r#""draft {} of {n} slides", log_name(&e.current.name, None)"#).is_empty());
    assert_eq!(suspects(r#""draft {}: {err}", log_name(&e.current.name, None)"#), ["{err}"]);
    assert!(suspects(r#""command `{id}` panicked: {msg}""#).is_empty());
    assert!(suspects(r#""{{name}} is literal""#).is_empty());
    assert_eq!(suspects(r#""couldn't open a {}: {e}", log_name(path, None)"#), ["{e}"]);
    assert_eq!(suspects(r#""a {}: {}", log_name(path, None), e.to_string()"#), ["e.to_string()"]);
    assert!(suspects(r#""a {}: {}", log_name(path, None), redact(&e, path)"#).is_empty());
    assert!(suspects(r#""recovery: {e}""#).is_empty(), "an error alone names no document");
}
