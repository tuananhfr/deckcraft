//! Payloads and comment anchors come from other people's editors through the room, so they are
//! untrusted: one refused leaves the presentation exactly as it was, and none of it may panic,
//! whatever it holds.

use super::*;

/// `payload` under `region` is refused and the presentation is unchanged.
fn refused(side: &mut Side, region: &str, payload: &str) -> LiveError {
    let before = side.doc().clone();
    let err = side.s.apply_remote(side.uid, region, payload).expect_err(region);
    assert_eq!(*side.doc(), before, "{region}: a refused payload changed the presentation");
    err
}

/// A payload larger than any editor sends is refused before it is read.
#[test]
fn an_oversized_payload_is_refused_unread() {
    let (_, mut b) = pair();
    let pad = "A".repeat(payload::MAX_PAYLOAD_BYTES);
    for region in ["*".to_owned(), "slides".to_owned(), Region::Slide(b.slide_id(1)).to_string()] {
        assert_eq!(refused(&mut b, &region, &format!(r#"{{"v":1,"deck":"{pad}"}}"#)), LiveError::TooLarge);
    }
}

/// A whole-presentation payload fits the room's 1 MiB but may inflate a few hundred times: what
/// it may inflate to is bounded far below what opening a file allows.
#[test]
fn a_whole_presentation_that_inflates_too_far_is_refused() {
    let (_, mut b) = pair();
    let mut doc = b.doc().clone();
    // Long runs with a varying letter every 300 bytes: about 100 times smaller deflated, under the
    // inflater's ratio check, so only the total stops it.
    let mut name = String::with_capacity(payload::MAX_DECK_INFLATED as usize + 1_000_000);
    let mut seed = 7u32;
    while name.len() <= payload::MAX_DECK_INFLATED as usize {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        name.push(char::from(b'b' + (seed >> 16) as u8 % 20));
        name.push_str(&"a".repeat(299));
    }
    std::sync::Arc::make_mut(&mut doc.slides[1]).shapes[0].name = name;
    let bytes = deckcraft_format::save(&doc).unwrap();
    let payload = format!(r#"{{"v":1,"deck":"{}"}}"#, cmd::base64_encode(&bytes));
    assert!(payload.len() <= payload::MAX_PAYLOAD_BYTES, "the room carries it: {} bytes", payload.len());
    refused(&mut b, "*", &payload);
}
