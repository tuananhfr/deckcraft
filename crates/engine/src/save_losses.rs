//! Pure facts for a host to confirm before a lossy save or export.

use std::collections::BTreeSet;

use deckcraft_model::{Presentation, ShapeKind, walk};
use serde::{Deserialize, Serialize};

use crate::Session;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LossItem {
    OleObjects { count: usize },
    DigitalSignatures,
    Macros,
    UnsupportedParts { kinds: Vec<String> },
}

impl Session {
    pub fn save_losses(&self, format: &str) -> Vec<LossItem> {
        self.active().map(|d| losses_for(&d.doc, format)).unwrap_or_default()
    }
}

pub fn losses_for(doc: &Presentation, format: &str) -> Vec<LossItem> {
    let mut losses = Vec::new();
    let mut ole = 0usize;
    let mut kinds: BTreeSet<String> = doc.import_losses.parts.iter().cloned().collect();
    let mut inspect = |shape: &deckcraft_model::Shape, _: usize| {
        if let ShapeKind::Opaque { xml, label, .. } = &shape.kind {
            match deckcraft_pptx::opaque_loss_kind(xml) {
                Some("oleObject") => ole = ole.saturating_add(1),
                Some(_) if format != "deckcraft" => {
                    kinds.insert(label.clone());
                }
                _ => {}
            }
        }
    };
    for slide in &doc.slides {
        walk(&slide.shapes, &mut inspect);
    }
    for master in &doc.masters {
        walk(&master.shapes, &mut inspect);
        for layout in &master.layouts {
            walk(&layout.shapes, &mut inspect);
        }
    }
    for master in doc.notes_master.iter().chain(doc.handout_master.iter()) {
        walk(&master.shapes, &mut inspect);
    }
    if ole > 0 {
        // Unmodelled OLE payloads still need a warning when no counted shape survived import.
        kinds.remove("oleObject");
        losses.push(LossItem::OleObjects { count: ole });
    }
    if doc.import_losses.signatures {
        losses.push(LossItem::DigitalSignatures);
    }
    if doc.import_losses.macros {
        losses.push(LossItem::Macros);
    }
    if !kinds.is_empty() {
        losses.push(LossItem::UnsupportedParts { kinds: kinds.into_iter().collect() });
    }
    losses
}

#[cfg(test)]
mod tests;
