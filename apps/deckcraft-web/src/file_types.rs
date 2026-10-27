//! The presentation types the browser's open picker offers, and the MIME type a file is handed to
//! the browser with.

/// What opens as a presentation (or an outline). The legacy .ppt and .odp only get the conversion
/// notice, so the picker leaves them out; dropped ones still show it.
pub(crate) const PRESENTATION_EXTS: &[&str] = &["deckcraft", "pptx", "potx", "ppsx", "pptm", "txt", "md"];

/// What Save As and Share… write on the web.
pub(crate) const PPTX_MIME: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

pub(crate) fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain",
        Some("pptx") => PPTX_MIME,
        Some("deckcraft") => "application/zip",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckcraft_engine::legacy_format::{LegacyFormat, detect};

    #[test]
    fn the_open_picker_offers_only_what_opens() {
        for ext in PRESENTATION_EXTS {
            let refused = matches!(detect(&format!("deck.{ext}"), &[]), Some(LegacyFormat::Ppt | LegacyFormat::Odp));
            assert!(!refused, ".{ext} only opens the conversion notice");
        }
        assert!(PRESENTATION_EXTS.contains(&"pptx") && PRESENTATION_EXTS.contains(&"pptm"));
    }

    #[test]
    fn every_type_the_open_picker_offers_opens() {
        let deck = deckcraft_engine::model::defaults::new_presentation(None);
        for ext in PRESENTATION_EXTS {
            let bytes = match *ext {
                "txt" | "md" => b"Title\n\tPoint".to_vec(),
                "deckcraft" => deckcraft_engine::cmd::file::save_bytes(&deck, "deckcraft").unwrap_or_default(),
                _ => deckcraft_engine::cmd::file::save_bytes(&deck, "pptx").unwrap_or_default(),
            };
            let mut app = deckcraft_ui_egui::SlideApp::new(deckcraft_engine::Session::with_new(), Default::default());
            assert!(app.open_or_insert_bytes(&format!("Báo cáo.{ext}"), &bytes), ".{ext} didn't open");
            let opened = if matches!(*ext, "txt" | "md") { 1 } else { 2 };
            assert_eq!(app.session.documents().len(), opened, ".{ext}");
        }
    }

    #[test]
    fn shared_presentations_carry_their_mime_type() {
        assert_eq!(mime_for("Báo cáo.PPTX"), "application/vnd.openxmlformats-officedocument.presentationml.presentation");
        assert_eq!(mime_for("deck.deckcraft"), "application/zip");
        assert_eq!(mime_for("no extension"), "application/octet-stream");
    }
}
