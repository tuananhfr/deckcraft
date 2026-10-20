//! Inventory parts whose source payload is not retained by the importer.

use std::collections::{BTreeSet, HashMap};

use deckcraft_model::import_losses::ImportLosses;

use crate::opc::Package;

pub(super) fn inventory(pkg: &Package) -> ImportLosses {
    // Snapshot before inspecting relationships: inspection must not mark payloads as imported.
    let read = pkg.read_parts();
    let mut kinds: HashMap<String, BTreeSet<String>> = HashMap::new();
    for name in pkg.names().filter(|n| n.ends_with(".rels")) {
        let owner = if name == "_rels/.rels" {
            String::new()
        } else if let Some((dir, file)) = name.rsplit_once("/_rels/") {
            format!("{dir}/{}", file.strip_suffix(".rels").unwrap_or(file))
        } else {
            continue;
        };
        for rel in pkg.rels(&owner).items.into_iter().filter(|r| !r.external && pkg.original_name(&r.target).is_some()) {
            kinds.entry(crate::opc::norm(&rel.target)).or_default().insert(rel.kind);
        }
    }
    let mut facts = ImportLosses::default();
    let mut dropped = BTreeSet::new();
    for name in pkg.names() {
        if name.starts_with("_xmlsignatures/") {
            facts.signatures = true;
            continue;
        }
        let part_kinds = kinds.get(name);
        if name.ends_with("vbaproject.bin")
            || name.ends_with("vbaprojectsignature.bin")
            || part_kinds.is_some_and(|ks| ks.contains("vbaProject") || ks.contains("vbaProjectSignature"))
        {
            facts.macros = true;
            continue;
        }
        if name == "[content_types].xml" || name.ends_with(".rels") {
            continue;
        }
        // View preferences are UI state rather than presentation content; the writer regenerates them.
        if part_kinds.is_some_and(|ks| ks.contains("viewProps")) {
            continue;
        }
        // These are read for previews or chart data, but their original parts are never written.
        let discarded = part_kinds
            .is_some_and(|ks| ks.iter().any(|k| matches!(k.as_str(), "oleObject" | "package" | "vmlDrawing" | "diagramData" | "diagramDrawing")));
        if discarded || !read.contains(name) {
            if let Some(ks) = part_kinds {
                dropped.extend(ks.iter().cloned());
            } else {
                dropped.insert(pkg.content_type(name).unwrap_or_else(|| name.rsplit('/').next().unwrap_or(name).to_string()));
            }
        }
    }
    facts.parts = dropped.into_iter().collect();
    facts
}
