//! Cloud comment anchors (ERPCons M5): made from the selection, read back from their JSON, and
//! found again after edits and in another version: by slide id, else by slide title then shape
//! id, else orphaned. Resolving never changes the presentation.

use std::sync::Arc;

use deckcraft_model::{ShapeId, SlideId};
use serde_json::{Value, json};

use crate::anchors::{Anchor, AnchorError, MAX_ANCHOR_BYTES, QUOTE_CHARS, Resolution, resolve};
use crate::live::Region;
use crate::{Session, Target, TextSel};

/// Slide 1 from `file.new`; slide 2 "Budget" with a rectangle; slide 3 "Plan".
fn deck() -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("slide.new", &json!({"layout": "title", "title": "Budget"})).unwrap();
    s.execute("shape.insert", &json!({"preset": "rect", "rect": [10, 20, 50, 40]})).unwrap();
    s.execute("slide.new", &json!({"layout": "title", "title": "Plan"})).unwrap();
    let uid = s.active().unwrap().uid;
    (s, uid)
}

fn slide(s: &Session, index: usize) -> SlideId {
    s.active().unwrap().doc.slides[index].id
}

/// The rectangle on slide 2 (its last shape).
fn rect(s: &Session) -> ShapeId {
    s.active().unwrap().doc.slides[1].shapes.last().unwrap().id
}

fn select_rect(s: &mut Session) -> ShapeId {
    s.execute("slide.go", &json!({"index": 1})).unwrap();
    let id = rect(s);
    s.execute("edit.select", &json!({"ids": [id.0]})).unwrap();
    id
}

fn open(r: &Resolution) -> &Anchor {
    match r {
        Resolution::Open(a) => a,
        Resolution::Orphaned => panic!("orphaned"),
    }
}

#[test]
fn anchors_round_trip_through_their_json_and_bad_ones_are_refused() {
    let (mut s, uid) = deck();
    let id = select_rect(&mut s);
    let a = s.anchor_of_selection(uid).unwrap();
    let value: Value = serde_json::from_str(&a.to_json("Trang chiếu 2 · Budget")).unwrap();
    assert_eq!(value["v"], 1);
    assert_eq!(value["region"], Region::Shape(slide(&s, 1), id).to_string());
    assert_eq!(value["pos"]["box"], json!([10.0, 20.0, 50.0, 40.0]));
    assert_eq!(value["soft"]["index"], 1);
    assert_eq!(value["soft"]["title"], "Budget");
    assert_eq!(value["label"], "Trang chiếu 2 · Budget");
    assert_eq!(Anchor::parse(&a.to_json("")).unwrap(), a);

    assert_eq!(Anchor::parse(r#"{"v":2,"region":"slide:256","soft":{"index":0}}"#), Err(AnchorError::Version));
    for bad in ["", "{}", r#"{"v":1,"soft":{"index":0}}"#, r#"{"v":1,"region":"slides","soft":{"index":0}}"#, r#"{"v":1,"region":"slide:256"}"#] {
        assert!(matches!(Anchor::parse(bad), Err(AnchorError::Malformed(_))), "{bad}");
    }
    assert!(Anchor::parse(&format!(r#"{{"v":1,"region":"slide:256","soft":{{"index":0,"title":"{}"}}}}"#, "x".repeat(MAX_ANCHOR_BYTES))).is_err());
    // A slide anchor keeps no box or text; a bad box is dropped.
    let slide = Anchor::parse(r#"{"v":1,"region":"slide:300","pos":{"box":[1,2,3,4]},"soft":{"index":4,"title":"  A \n b "}}"#).unwrap();
    assert_eq!((slide.shape, slide.bounds, slide.index, slide.title.as_str()), (None, None, 4, "A b"));
    let shape =
        Anchor::parse(r#"{"v":1,"region":"shape:300:7","pos":{"box":[1,"x",3,4],"text":{"from":[0,1],"to":[0,4]}},"soft":{"index":0}}"#).unwrap();
    assert_eq!((shape.bounds, shape.text), (None, Some(((0, 1), (0, 4)))));
}

#[test]
fn the_selection_gives_a_shape_its_text_or_the_slide() {
    let (mut s, uid) = deck();
    s.execute("slide.go", &json!({"index": 2})).unwrap();
    let plan = s.anchor_of_selection(uid).unwrap();
    assert_eq!((plan.region(), plan.index, plan.title.as_str()), (Region::Slide(slide(&s, 2)), 2, "Plan"));
    let id = select_rect(&mut s);
    assert_eq!(s.anchor_of_selection(uid).unwrap().shape, Some(id));
    // Text being selected in the slide's title placeholder.
    let title = s.active().unwrap().doc.slides[1].shapes[0].id;
    s.select(|_, sel| sel.text = Some(TextSel { shape: title, anchor: (0, 4), caret: (0, 1), ..TextSel::default() })).unwrap();
    let text = s.anchor_of_selection(uid).unwrap();
    assert_eq!((text.shape, text.text), (Some(title), Some(((0, 1), (0, 4)))));
    s.select(|_, sel| sel.target = Target::Master { master: 0 }).unwrap();
    assert_eq!(s.anchor_of_selection(uid), None, "Slide Master view anchors nothing");
}

#[test]
fn a_moved_slide_keeps_the_anchor_and_a_deleted_shape_or_slide_orphans_it() {
    let (mut s, uid) = deck();
    select_rect(&mut s);
    let shape = s.anchor_of_selection(uid).unwrap();
    s.execute("slide.go", &json!({"index": 2})).unwrap();
    let plan = s.anchor_of_selection(uid).unwrap();
    s.execute("slide.move", &json!({"from": 1, "to": 0})).unwrap();
    let out = s.resolve_anchors(uid, &[shape.clone(), plan.clone()]);
    assert_eq!((open(&out[0]).region(), open(&out[0]).index), (shape.region(), 0));
    assert_eq!(open(&out[1]).index, 2);
    let moved = open(&out[0]).clone();
    s.execute("slide.go", &json!({"index": 0})).unwrap();
    s.execute("edit.delete", &json!({"ids": [rect_at(&s, 0).0]})).unwrap();
    assert_eq!(s.resolve_anchors(uid, std::slice::from_ref(&moved)), [Resolution::Orphaned], "the shape is gone, its slide isn't");
    s.execute("slide.delete", &json!({"index": 2})).unwrap();
    assert_eq!(s.resolve_anchors(uid, std::slice::from_ref(&plan)), [Resolution::Orphaned]);
}

fn rect_at(s: &Session, index: usize) -> ShapeId {
    s.active().unwrap().doc.slides[index].shapes.last().unwrap().id
}

#[test]
fn an_unknown_slide_id_is_found_by_title_nearest_its_place_then_by_shape_id() {
    let (mut s, uid) = deck();
    let id = select_rect(&mut s);
    let mut a = s.anchor_of_selection(uid).unwrap();
    // Another version numbered the slides afresh.
    a.slide = SlideId(9_999);
    let found = s.resolve_anchors(uid, std::slice::from_ref(&a));
    assert_eq!(open(&found[0]).region(), Region::Shape(slide(&s, 1), id));
    // Two slides titled "Budget" holding the shape id (as another version may have): the one
    // nearest the old place wins. A duplicate made here would get fresh shape ids.
    let mut doc = (*s.active().unwrap().doc).clone();
    let mut copy = (*doc.slides[1]).clone();
    copy.id = SlideId(9_998);
    doc.slides.push(Arc::new(copy));
    a.index = 3;
    assert_eq!(open(&resolve(&doc, &a)).index, 3);
    a.index = 0;
    assert_eq!(open(&resolve(&doc, &a)).index, 1);
    a.title = "Nowhere".into();
    assert_eq!(s.resolve_anchors(uid, std::slice::from_ref(&a)), [Resolution::Orphaned]);
    a.title = String::new();
    assert_eq!(s.resolve_anchors(uid, std::slice::from_ref(&a)), [Resolution::Orphaned], "no title, nothing to look for");
}

#[test]
fn resolving_changes_nothing_in_the_presentation() {
    let (mut s, uid) = deck();
    select_rect(&mut s);
    let a = s.anchor_of_selection(uid).unwrap();
    let (doc, revision) = (s.active().unwrap().doc.clone(), s.active().unwrap().revision);
    s.resolve_anchors(uid, std::slice::from_ref(&a));
    resolve(&s.active().unwrap().doc, &a);
    let st = s.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc));
    assert_eq!(st.revision, revision);
    assert_eq!(s.resolve_anchors(uid + 99, std::slice::from_ref(&a)), [Resolution::Orphaned]);
}

/// The shell's position key, compared as `online-comments-state.js` does: element by element, a
/// shorter key first when one is the other's start.
fn shell_order(a: &Anchor) -> Vec<f64> {
    let value: Value = serde_json::from_str(&a.to_json("")).unwrap();
    value["order"].as_array().expect("order").iter().map(|n| n.as_f64().expect("a number")).collect()
}

fn shell_cmp(a: &[f64], b: &[f64]) -> std::cmp::Ordering {
    a.iter().zip(b).find_map(|(x, y)| (x != y).then(|| x.total_cmp(y))).unwrap_or(a.len().cmp(&b.len()))
}

#[test]
fn threads_sort_by_slide_then_top_then_left_then_text() {
    let (mut s, uid) = deck();
    let low = select_rect(&mut s);
    s.execute("shape.insert", &json!({"preset": "rect", "rect": [300, 5, 20, 10]})).unwrap();
    let high = rect(&s);
    s.execute("shape.insert", &json!({"preset": "rect", "rect": [200, 20, 20, 10]})).unwrap();
    let right = rect(&s);
    s.execute("text.set", &json!({"id": low.0, "text": "one\ntwo"})).unwrap();
    let pick = |s: &mut Session, id: ShapeId| {
        s.execute("edit.select", &json!({"ids": [id.0]})).unwrap();
        s.anchor_of_selection(uid).unwrap()
    };
    let text_at = |s: &mut Session, from: (usize, usize)| {
        s.select(|_, sel| sel.text = Some(TextSel { shape: low, anchor: from, caret: (from.0, from.1 + 2), ..TextSel::default() })).unwrap();
        s.anchor_of_selection(uid).unwrap()
    };
    let (low_a, high_a, right_a) = (pick(&mut s, low), pick(&mut s, high), pick(&mut s, right));
    let (second_para, first_para) = (text_at(&mut s, (1, 0)), text_at(&mut s, (0, 1)));
    s.select(|_, sel| {
        sel.text = None;
        sel.shapes.clear();
    })
    .unwrap();
    let slide2 = s.anchor_of_selection(uid).unwrap();
    s.execute("slide.go", &json!({"index": 0})).unwrap();
    let slide1 = s.anchor_of_selection(uid).unwrap();
    s.execute("slide.go", &json!({"index": 2})).unwrap();
    let slide3 = s.anchor_of_selection(uid).unwrap();

    assert_eq!(shell_order(&slide2), [1.0]);
    assert_eq!(shell_order(&low_a), [1.0, 20.0, 10.0]);
    assert_eq!(shell_order(&first_para), [1.0, 20.0, 10.0, 0.0, 1.0]);
    let mut shuffled = [&slide3, &right_a, &second_para, &slide1, &low_a, &high_a, &first_para, &slide2];
    shuffled.sort_by(|a, b| shell_cmp(&shell_order(a), &shell_order(b)));
    let expected = [&slide1, &slide2, &high_a, &low_a, &first_para, &second_para, &right_a, &slide3];
    assert_eq!(shuffled.map(shell_order), expected.map(shell_order));
}

#[test]
fn the_selected_text_is_quoted_collapsed_and_cut_like_wordcraft() {
    let (mut s, uid) = deck();
    let id = select_rect(&mut s);
    assert_eq!(s.anchor_of_selection(uid).unwrap().quote, "", "a whole shape quotes nothing");
    let long = format!("  Doanh\tthu \u{1}quý\n\nba {}", "x".repeat(300));
    s.execute("text.set", &json!({"id": id.0, "text": long})).unwrap();
    s.select(|_, sel| sel.text = Some(TextSel { shape: id, anchor: (0, 0), caret: (9, 9_999), ..TextSel::default() })).unwrap();
    let a = s.anchor_of_selection(uid).unwrap();
    assert_eq!(a.quote.chars().count(), QUOTE_CHARS);
    assert!(a.quote.starts_with("Doanh thu quý ba xxx") && a.quote.ends_with("x…"), "{}", a.quote);
    let value: Value = serde_json::from_str(&a.to_json("")).unwrap();
    assert_eq!(value["quote"], a.quote.as_str());
    assert_eq!(Anchor::parse(&a.to_json("")).unwrap(), a);
    s.select(|_, sel| sel.text = Some(TextSel { shape: id, anchor: (0, 2), caret: (0, 2), ..TextSel::default() })).unwrap();
    let caret = s.anchor_of_selection(uid).unwrap();
    assert_eq!(caret.quote, "");
    assert!(serde_json::from_str::<Value>(&caret.to_json("")).unwrap().get("quote").is_none(), "no quote, no key");
}

/// The quote is what was commented on: resolving keeps it while the shape's box, and so the
/// order, follow the shape.
#[test]
fn resolving_keeps_the_quote_and_updates_the_order() {
    let (mut s, uid) = deck();
    let id = select_rect(&mut s);
    s.execute("text.set", &json!({"id": id.0, "text": "Doanh thu"})).unwrap();
    s.select(|_, sel| sel.text = Some(TextSel { shape: id, anchor: (0, 0), caret: (0, 5), ..TextSel::default() })).unwrap();
    let a = s.anchor_of_selection(uid).unwrap();
    assert_eq!(a.quote, "Doanh");
    s.select(|_, sel| sel.text = None).unwrap();
    s.execute("text.set", &json!({"id": id.0, "text": "Tổng doanh thu"})).unwrap();
    s.execute("shape.move", &json!({"ids": [id.0], "dx": 0, "dy": 30})).unwrap();
    let now = open(&s.resolve_anchors(uid, std::slice::from_ref(&a))[0]).clone();
    assert_eq!(now.quote, "Doanh");
    assert_eq!(shell_order(&now), [1.0, 50.0, 10.0, 0.0, 0.0]);
}

/// Anchors written before `quote` and `order` still read and resolve, and come back with an
/// order.
#[test]
fn an_anchor_without_quote_or_order_still_resolves() {
    let (mut s, uid) = deck();
    let id = select_rect(&mut s);
    let old = format!(
        r#"{{"v":1,"region":"shape:{}:{id}","pos":{{"box":[10,20,50,40],"text":{{"from":[0,0],"to":[0,0]}}}},"soft":{{"index":1,"title":"Budget","name":""}},"label":"Trang chiếu 2 · Budget"}}"#,
        slide(&s, 1)
    );
    let a = Anchor::parse(&old).unwrap();
    assert_eq!(a.quote, "");
    let now = open(&s.resolve_anchors(uid, std::slice::from_ref(&a))[0]).clone();
    assert_eq!(shell_order(&now), [1.0, 20.0, 10.0, 0.0, 0.0]);
}

/// The shell refuses anchors over 8 KiB of UTF-8 and the server any `\u0000`: the longest title,
/// name and quote, in four-byte characters and with control characters, still fit.
#[test]
fn the_largest_anchor_fits_the_shells_cap_and_carries_no_control_characters() {
    let (mut s, uid) = deck();
    let id = select_rect(&mut s);
    let wide = "\u{0}\u{1F600}\u{7}".repeat(600);
    s.execute("shape.rename", &json!({"id": id.0, "name": wide})).unwrap();
    s.execute("text.set", &json!({"id": id.0, "text": wide})).unwrap();
    s.select(|_, sel| sel.text = Some(TextSel { shape: id, anchor: (0, 0), caret: (0, 9_999), ..TextSel::default() })).unwrap();
    let mut a = s.anchor_of_selection(uid).unwrap();
    a.title = crate::anchors::clean_text(&wide);
    let label = format!("Trang chiếu 99999 · “{}…”", "\u{1F600}".repeat(60));
    let json = a.to_json(&label);
    assert!(json.len() <= 8 * 1024, "{} bytes", json.len());
    assert!(!json.contains(r"\u0000"), "{json}");
    assert_eq!(Anchor::parse(&json).unwrap(), a);
}
