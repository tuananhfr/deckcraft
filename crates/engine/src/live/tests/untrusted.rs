//! Payloads and comment anchors come from other people's editors through the room, so they are
//! untrusted: one refused leaves the presentation exactly as it was, and none of it may panic,
//! whatever it holds.

use super::*;
use crate::anchors::Anchor;

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

// ── fuzzing ───────────────────────────────────────────────────────────────────────────────────

/// A small deterministic generator (xorshift64*): the same cases on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n.max(1) as u64).unwrap_or(0)
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

const NUMBERS: [&str; 12] =
    ["0", "1", "-1", "7", "4294967295", "4294967296", "18446744073709551615", "-9223372036854775808", "1e308", "-1e308", "0.5", "NaN"];

/// `json` with one random change: a byte flipped or dropped, a cut, numbers swapped for extreme
/// ones, deep nesting, or an injected key.
fn mutate(rng: &mut Rng, json: &str) -> String {
    let mut bytes = json.as_bytes().to_vec();
    match rng.below(7) {
        0 if !bytes.is_empty() => {
            let i = rng.below(bytes.len());
            bytes[i] = *rng.pick(b"{}[]\":,0123456789-.etrfnul\\ ");
        }
        1 if !bytes.is_empty() => {
            let i = rng.below(bytes.len());
            bytes.remove(i);
        }
        2 => bytes.truncate(rng.below(bytes.len() + 1)),
        3 => {
            let s = String::from_utf8_lossy(&bytes).into_owned();
            let mut out = String::new();
            let mut chars = s.chars().peekable();
            while let Some(c) = chars.next() {
                if c.is_ascii_digit() && rng.below(3) == 0 {
                    while chars.peek().is_some_and(char::is_ascii_digit) {
                        chars.next();
                    }
                    out.push_str(rng.pick(&NUMBERS));
                } else {
                    out.push(c);
                }
            }
            return out;
        }
        4 => {
            let depth = 1 + rng.below(300);
            let s = String::from_utf8_lossy(&bytes).into_owned();
            return s.replacen("\"v\":1", &format!("\"v\":1,\"x\":{}0{}", "[".repeat(depth), "]".repeat(depth)), 1);
        }
        5 => {
            let s = String::from_utf8_lossy(&bytes).into_owned();
            let key = rng.pick(&[
                "\"at\":18446744073709551615",
                "\"del\":1",
                "\"v\":2",
                "\"order\":[1,1,1,1]",
                "\"new\":[]",
                "\"media\":[{\"id\":4294967295,\"data\":\"\"}]",
            ]);
            return s.replacen("\"v\":1", &format!("\"v\":1,{key}"), 1);
        }
        _ => {}
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Thousands of mutated genuine payloads under genuine and hostile region ids: none panics, and
/// every one refused leaves the presentation as it was.
#[test]
fn mutated_payloads_never_panic_and_a_refused_one_changes_nothing() {
    let (mut a, _) = pair();
    let genuine: Vec<(String, String)> = a.all().into_iter().collect();
    let star = (Region::Document.to_string(), a.s.live_payload(a.uid, &Region::Document).unwrap());
    let (s1, s0) = (a.slide_id(1), a.slide_id(0));
    let hostile = [
        "*".to_owned(),
        "slides".to_owned(),
        format!("slide:{s0}"),
        format!("shape:{s0}:{}", a.shape_ids(1)[0].0),
        format!("shape:{s1}:4294967295"),
        "slide:4294967295".to_owned(),
        "shape:1:2:3".to_owned(),
        String::new(),
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (_, mut b) = pair();
    let (mut applied, mut refused_count) = (0, 0);
    for round in 0..1500 {
        if round % 100 == 99 {
            b = pair().1;
        }
        let (region, payload) = if round % 50 == 0 { star.clone() } else { rng.pick(&genuine).clone() };
        let region = if rng.below(4) == 0 { rng.pick(&hostile).clone() } else { region };
        let mut payload = payload;
        for _ in 0..=rng.below(3) {
            payload = mutate(&mut rng, &payload);
        }
        let before = b.doc().clone();
        match b.s.apply_remote(b.uid, &region, &payload) {
            Ok(_) => applied += 1,
            Err(_) => {
                refused_count += 1;
                assert_eq!(*b.doc(), before, "round {round}: {region} refused but the presentation changed");
            }
        }
    }
    assert!(applied > 0 && refused_count > 0, "both kinds were tried: {applied} applied, {refused_count} refused");
}

/// Random and mutated anchors: reading, placing and resolving them never panics and never
/// touches the presentation.
#[test]
fn mutated_anchors_never_panic_and_never_change_the_presentation() {
    let (_, b) = pair();
    let (s1, shape) = (b.slide_id(1), b.shape_ids(1)[0]);
    let genuine = [
        format!(
            r#"{{"v":1,"region":"shape:{s1}:{shape}","pos":{{"box":[10,10,50,50],"text":"x"}},"soft":{{"index":1,"title":"","name":"Rectangle"}},"label":"Slide 2"}}"#
        ),
        format!(r#"{{"v":1,"region":"slide:{s1}","pos":null,"soft":{{"index":1,"title":"Budget","name":""}},"label":"Slide 2"}}"#),
    ];
    let before = b.doc().clone();
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut anchors = Vec::new();
    for round in 0..3000 {
        let mut json = rng.pick(&genuine).clone();
        for _ in 0..=rng.below(3) {
            json = mutate(&mut rng, &json);
        }
        if round % 500 == 0 {
            json = format!(r#"{{"v":1,"region":"slide:{s1}","soft":{{"index":0,"title":"{}"}}}}"#, "é".repeat(5000));
        }
        if let Ok(anchor) = Anchor::parse(&json) {
            let _ = crate::anchors::place(b.doc(), &anchor);
            anchors.push(anchor);
        }
    }
    assert!(!anchors.is_empty());
    let _ = b.s.resolve_anchors(b.uid, &anchors);
    assert_eq!(*b.doc(), before, "anchors changed the presentation");
}

/// Someone else's change to a group ends the text editing of a shape inside it: the caret's
/// paragraph and offset may no longer exist.
#[test]
fn a_remote_change_to_a_group_ends_text_editing_inside_it() {
    let (mut a, mut b) = pair();
    a.go(1);
    let ids: Vec<u32> = a.shape_ids(1).iter().map(|s| s.0).collect();
    let group = ShapeId(u32::try_from(a.run("arrange.group", json!({"ids": ids})).unwrap()["id"].as_u64().unwrap()).unwrap());
    relay(&mut a, &mut b);
    b.go(1);
    b.run("text.edit", json!({"id": ids[0]})).unwrap();
    assert!(b.s.active().unwrap().selection.text.is_some(), "editing the child's text");
    a.nudge(1, group, 5.0).unwrap();
    relay(&mut a, &mut b);
    assert_eq!(b.s.active().unwrap().selection.text, None);
}
