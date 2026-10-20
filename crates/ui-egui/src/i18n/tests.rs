#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::catalog::{Catalog, parse_entries};
use super::*;

#[test]
fn bundled_catalogs_parse_without_errors() {
    for lang in &LANGUAGES {
        let (_, errors) = Catalog::parse(lang.source);
        assert!(errors.is_empty(), "{}: {errors:#?}", lang.code);
    }
}

#[test]
fn vietnamese_product_name_is_localized_except_the_original_copyright() {
    let (entries, errors) = parse_entries(include_str!("vi.tsv"));
    assert!(errors.is_empty(), "{errors:#?}");
    // Copyright names the original contributors rather than the localized product.
    let credit = "© 2026 ArtCraft Team and the DeckCraft contributors";
    for entry in entries {
        if entry.source == credit {
            assert_eq!(entry.translation, "© 2026 nhóm ArtCraft và những người đóng góp cho DeckCraft");
        } else {
            assert!(!entry.translation.contains("DeckCraft"), "unlocalized product name: {}", entry.source);
        }
    }
    let vi = Lang::from_code("vi").unwrap();
    assert_eq!(tr(vi, "About DeckCraft"), "Giới thiệu Trình chiếu");
    assert_eq!(tr(vi, "DeckCraft can't export “{format}”"), "Trình chiếu không xuất được “{format}”");
}

#[test]
fn vietnamese_is_registered_and_english_is_the_fallback() {
    let vi = Lang::from_code("VI").expect("vi registered");
    assert_eq!(vi.name(), "Tiếng Việt");
    assert_eq!(Lang::from_pref("ja"), Lang::EN);
    assert_eq!(tr(vi, "No such interface string"), "No such interface string");
}

#[test]
fn slide_show_end_and_touch_exit_have_vietnamese_labels() {
    let vi = Lang::from_code("vi").unwrap();
    assert_eq!(tr(vi, "End"), "Kết thúc trình chiếu");
    assert_eq!(tr(vi, "End of slide show"), "Kết thúc trình chiếu");
    assert_eq!(tr(vi, "End of slide show, click to exit."), "Kết thúc trình chiếu, bấm để thoát.");
    assert_eq!(tr(vi, "Exit slide show"), "Thoát trình chiếu");
}

#[test]
fn fmt_fills_placeholders_once() {
    assert_eq!(fmt("Slide {n} of {total}", &[("n", "2"), ("total", "{n}")]), "Slide 2 of {n}");
    assert_eq!(fmt("{missing} stays", &[]), "{missing} stays");
}

#[test]
fn catalog_rejects_mismatched_placeholders_and_ellipses() {
    let (entries, errors) = parse_entries("\tSlide {n}\tTrang chiếu\n\tOpen…\tMở\n\tSave\tLưu\n");
    assert_eq!(entries.len(), 1);
    assert_eq!(errors.len(), 2);
}

/// Undoes Rust string escapes in a `tl!` literal so it can be looked up as the program sees it.
fn unescape(lit: &str) -> String {
    let mut out = String::new();
    let mut chars = lit.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex: String = chars.by_ref().skip(1).take_while(|c| *c != '}').collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Every literal passed to `tl!` in the crate has a Vietnamese entry, so new UI text cannot ship
/// in English unnoticed.
#[test]
fn every_tl_literal_is_translated() {
    let vi = Lang::from_code("vi").unwrap();
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src];
    let mut missing = Vec::new();
    let mut seen = 0;
    while let Some(path) = files.pop() {
        if path.is_dir() {
            files.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") || path.ends_with("i18n/tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (at, _) in text.match_indices("tl!(\"") {
            let body = &text[at + 5..];
            let mut end = 0;
            let mut escaped = false;
            for (i, c) in body.char_indices() {
                match c {
                    _ if escaped => escaped = false,
                    '\\' => escaped = true,
                    '"' => {
                        end = i;
                        break;
                    }
                    _ => {}
                }
            }
            let s = unescape(&body[..end]);
            seen += 1;
            if s.chars().any(char::is_alphabetic) && !has(vi, &s) {
                missing.push(format!("{}: {s:?}", path.file_name().unwrap().to_string_lossy()));
            }
        }
    }
    assert!(seen > 300, "only {seen} tl! literals found; is the scan still pointed at src/?");
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "untranslated: {missing:#?}");
}

/// The ribbon tabs and every command's label and menu location are translated, so the ribbon,
/// command search and undo names never fall back to English.
#[test]
fn every_tab_and_command_is_translated() {
    let vi = Lang::from_code("vi").unwrap();
    let session = deckcraft_engine::Session::with_new();
    let contextual = ["Shape Format", "Picture Format", "Table Design", "Layout", "Chart Design", "Slide Master"];
    let mut keys: Vec<&str> = crate::ribbon::TABS.iter().chain(&contextual).copied().collect();
    let commands = session.commands();
    keys.extend(commands.iter().flat_map(|c| std::iter::once(c.label).chain(c.menu.iter().copied())));
    keys.extend(crate::UI_COMMANDS.iter().map(|c| c.1));
    let mut missing: Vec<&str> = keys.into_iter().filter(|s| s.chars().any(char::is_alphabetic) && !has(vi, s)).collect();
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "untranslated: {missing:#?}");
}

/// The prompts the canvas draws in empty placeholders ("Click to add title"…) are translated.
#[test]
fn every_placeholder_prompt_is_translated() {
    use deckcraft_model::PhType::*;
    let vi = Lang::from_code("vi").unwrap();
    let kinds = [Title, CtrTitle, SubTitle, Body, Obj, Chart, Table, ClipArt, Diagram, Media, Picture, Date, Footer, SlideNum, Header, SlideImage];
    let missing: Vec<&str> = kinds.iter().filter(|k| !k.is_footer_kind()).map(|k| k.prompt()).filter(|s| !s.is_empty() && !has(vi, s)).collect();
    assert!(missing.is_empty(), "untranslated: {missing:#?}");
}

#[test]
fn the_language_setting_is_a_ui_command_and_persists() {
    let mut app = crate::SlideApp::new(deckcraft_engine::Session::with_new(), Default::default());
    assert_eq!(app.ui.language, "en");
    app.run("view.language", serde_json::json!({"language": "VI"})).unwrap();
    assert_eq!(app.ui.language, "vi");
    assert!(app.run("view.language", serde_json::json!({"language": "xx"})).is_err());
    assert_eq!(app.ui.language, "vi", "an unknown code keeps the setting");
    let restored: crate::UiState = serde_json::from_str(&serde_json::to_string(&app.ui).unwrap()).unwrap();
    assert_eq!(restored.language, "vi");
    let legacy: crate::UiState = serde_json::from_str("{}").unwrap();
    assert_eq!(legacy.language, "en", "settings saved before the option existed stay English");
}
