//! Legacy format detection and safe output paths, independent of the host.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyFormat {
    Ppt,
    Odp,
    Pptm,
}

impl LegacyFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Ppt => ".ppt",
            Self::Odp => ".odp",
            Self::Pptm => ".pptm",
        }
    }
}

pub fn detect(name: &str, bytes: &[u8]) -> Option<LegacyFormat> {
    if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        return Some(LegacyFormat::Ppt);
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".ppt") {
        Some(LegacyFormat::Ppt)
    } else if lower.ends_with(".odp") {
        Some(LegacyFormat::Odp)
    } else if lower.ends_with(".pptm") {
        Some(LegacyFormat::Pptm)
    } else {
        None
    }
}

pub fn output_path(path: &str) -> String {
    if detect(path, &[]).is_some() {
        std::path::Path::new(path).with_extension("pptx").to_string_lossy().into_owned()
    } else {
        path.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ole2_even_without_a_legacy_extension() {
        let magic = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
        assert_eq!(detect("renamed.pptx", &magic), Some(LegacyFormat::Ppt));
        for length in 0..magic.len() {
            assert_eq!(detect("unknown", &magic[..length]), None);
        }
    }

    #[test]
    fn detects_extensions_case_insensitively_without_matching_suffixes() {
        for (name, format) in [("a.PPT", LegacyFormat::Ppt), ("a.ODP", LegacyFormat::Odp), ("a.PpTm", LegacyFormat::Pptm)] {
            assert_eq!(detect(name, &[]), Some(format));
        }
        for name in ["a.pptx", "a.odp.zip", "pptm", ""] {
            assert_eq!(detect(name, &[]), None);
        }
    }

    #[test]
    fn legacy_output_paths_always_use_pptx() {
        for path in ["a.ppt", "a.ODP", "a.PPTM"] {
            assert_eq!(output_path(path), "a.pptx");
        }
        assert_eq!(output_path("a.deckcraft"), "a.deckcraft");
    }

    #[test]
    fn unsupported_formats_are_refused_before_import() {
        for (name, bytes) in [
            ("old.ppt", &[][..]),
            ("old.odp", &[][..]),
            ("renamed.pptx", &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1][..]),
        ] {
            assert!(matches!(crate::cmd::file::open_presentation(name, bytes), Err(crate::EngineError::LegacyFormat(_))));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[allow(clippy::unwrap_used)]
    fn saving_and_saving_as_never_overwrite_the_pptm_original() {
        use serde_json::json;

        let mut session = crate::Session::with_new();
        let uid = session.active().unwrap().uid;
        let dir = std::env::temp_dir().join(format!("deckcraft-legacy-save-{uid}"));
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("source.PPTM");
        let output = dir.join("source.pptx");
        let bytes = crate::cmd::file::save_bytes(&session.active().unwrap().doc, "pptx").unwrap();
        std::fs::write(&original, &bytes).unwrap();
        session.execute("file.open", &json!({"path": original})).unwrap();
        session.execute("slide.new", &json!({"title": "Changed"})).unwrap();
        session.execute("file.save", &json!({})).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        let saved = std::fs::read(&output).unwrap();
        assert!(deckcraft_pptx::sniff(&saved));
        assert_ne!(saved, bytes);
        session.execute("file.saveAs", &json!({"path": original, "format": "deckcraft"})).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        assert!(deckcraft_pptx::sniff(&std::fs::read(&output).unwrap()));
        assert_eq!(session.active().unwrap().path.as_deref(), output.to_str());
        assert!(session.execute("file.export", &json!({"path": original, "format": "pptx"})).is_err());
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        std::fs::remove_file(original).unwrap();
        std::fs::remove_file(output).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
