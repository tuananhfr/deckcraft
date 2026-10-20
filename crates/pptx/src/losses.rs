//! Opaque object classification shared with the writer's fallback decision.

pub(crate) fn preserved(root: &crate::xml::El) -> bool {
    !crate::read::has_rel_refs(root) && matches!(root.local(), "graphicFrame" | "sp" | "grpSp" | "cxnSp")
}

/// The kind of an opaque object reduced to a preview by the PPTX writer.
pub fn opaque_loss_kind(xml: &str) -> Option<&'static str> {
    match crate::xml::parse(xml.as_bytes()) {
        Ok(doc) if preserved(&doc.root) => None,
        Ok(doc) if doc.root.find("oleObj").is_some() => Some("oleObject"),
        _ => Some("opaqueObject"),
    }
}
