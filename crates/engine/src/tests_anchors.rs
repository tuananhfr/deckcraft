//! Cloud comment anchors (ERPCons M5): made from the selection, read back from their JSON, and
//! found again after edits and in another version: by slide id, else by slide title then shape
//! id, else orphaned. Resolving never changes the presentation.

use std::sync::Arc;

use deckcraft_model::{ShapeId, SlideId};
use serde_json::{Value, json};

use crate::anchors::{Anchor, AnchorError, MAX_ANCHOR_BYTES, Resolution, resolve};
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
