//! A file refused by the decompression limits fails with one message the interface translates,
//! and opens nothing.
use deckcraft_engine::cmd::base64_encode;
use deckcraft_engine::cmd::file::open_presentation;
use deckcraft_engine::model::inflate::{self, LIMITS, TOO_LARGE};
use deckcraft_engine::model::{Presentation, defaults};
use deckcraft_engine::{EngineError, Session};
use serde_json::json;

/// A native file whose one medium is 64 MiB of zeros, deflated to a few dozen KiB.
fn bomb() -> Vec<u8> {
    let mut p = defaults::new_presentation(None);
    p.add_media("a.bmp", "image/bmp", vec![0; 64 << 20]);
    deckcraft_format::save(&p).unwrap()
}

#[test]
fn a_zip_bomb_is_refused_with_the_translatable_message_and_opens_nothing() {
    let bytes = bomb();
    assert!(bytes.len() < 1 << 20);
    let e = open_presentation("bomb.deckcraft", &bytes).unwrap_err();
    assert!(matches!(e, EngineError::FileTooLarge));
    assert_eq!(e.to_string(), TOO_LARGE);
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let before = s.documents().len();
    let e = s.execute("file.openBytes", &json!({"name": "bomb.deckcraft", "data": base64_encode(&bytes)})).unwrap_err();
    assert!(matches!(e, EngineError::FileTooLarge));
    assert_eq!(s.documents().len(), before, "no presentation, not even a partial one");
}

/// Rewrites the inflated size `name` declares, in its local header and the central directory.
fn declare_size(zip: &mut [u8], name: &str, size: u32) {
    for i in 0..zip.len().saturating_sub(4) {
        let (name_at, size_at) = match zip[i..i + 4] {
            [0x50, 0x4b, 0x03, 0x04] => (30, 22),
            [0x50, 0x4b, 0x01, 0x02] => (46, 24),
            _ => continue,
        };
        if zip.get(i + name_at..i + name_at + name.len()) == Some(name.as_bytes()) {
            zip[i + size_at..i + size_at + 4].copy_from_slice(&size.to_le_bytes());
        }
    }
}

#[test]
fn a_required_part_declared_over_the_cap_says_too_large_not_damaged() {
    let deck = defaults::new_presentation(None);
    let cases = [
        ("big.deckcraft", deckcraft_format::save(&deck).unwrap(), "presentation.json", LIMITS.total),
        ("big.pptx", deckcraft_pptx::export(&deck).unwrap(), "ppt/presentation.xml", LIMITS.entry),
        ("types.pptx", deckcraft_pptx::export(&deck).unwrap(), "[Content_Types].xml", LIMITS.entry),
    ];
    for (name, mut bytes, part, cap) in cases {
        declare_size(&mut bytes, part, u32::try_from(cap + 1).unwrap_or(u32::MAX - 1));
        let e = open_presentation(name, &bytes).unwrap_err();
        assert!(matches!(e, EngineError::FileTooLarge), "{name}: {e}");
    }
}

#[test]
fn a_large_legitimate_presentation_still_opens() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let outline: String = (0..100).map(|i| format!("Báo cáo công trình {i}\n\tHạng mục {i}: chi phí {} triệu đồng\n", i * 13)).collect();
    s.execute("slide.fromOutline", &json!({"text": outline})).unwrap();
    let mut deck: Presentation = (*s.doc().unwrap().doc).clone();
    assert!(deck.slides.len() >= 100);
    // A 24 MiB picture deflating about 66:1: past the ratio floor, well under the ratio.
    let picture: Vec<u8> = (0..24u32 << 20).map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8).collect();
    deck.add_media("photo.bmp", "image/bmp", picture);
    for (name, bytes) in [("big.deckcraft", deckcraft_format::save(&deck).unwrap()), ("big.pptx", deckcraft_pptx::export(&deck).unwrap())] {
        let back = open_presentation(name, &bytes).unwrap();
        assert_eq!(back.slides.len(), deck.slides.len(), "{name}");
    }
    let back = open_presentation("big.deckcraft", &deckcraft_format::save(&deck).unwrap()).unwrap();
    assert!(back.media.iter().any(|m| m.data.len() == 24 << 20), "the picture came back whole");
}

#[test]
fn the_web_build_selects_the_smaller_limits() {
    assert_eq!(inflate::for_target(true), inflate::WEB);
    assert_eq!(inflate::for_target(false), inflate::NATIVE);
    assert_eq!(LIMITS, inflate::for_target(cfg!(target_arch = "wasm32")));
    assert_eq!((inflate::WEB.entry, inflate::WEB.total), (256 << 20, 512 << 20));
    assert_eq!((inflate::NATIVE.entry, inflate::NATIVE.total), (512 << 20, 2048 << 20));
}
