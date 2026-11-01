//! Co-editing by region between two sessions, with the room played by the test: payloads of one
//! side's changes applied on the other converge, touch nothing else, carry slide operations with
//! their ids, survive a save and a reopen, and per-user undo never reverts the other side.

use std::collections::BTreeMap;

use deckcraft_model::{ShapeId, SlideId};
use serde_json::json;

use super::*;
use crate::{ClientPrefix, cmd};

/// Two slides; the second has two rectangles.
fn deck() -> Vec<u8> {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("slide.new", &json!({"layout": "blank"})).unwrap();
    s.execute("shape.insert", &json!({"preset": "rect", "rect": [10, 10, 50, 50]})).unwrap();
    s.execute("shape.insert", &json!({"preset": "ellipse", "rect": [100, 100, 40, 40]})).unwrap();
    cmd::file::save_bytes(&s.active().unwrap().doc, "pptx").unwrap()
}

struct Side {
    s: Session,
    uid: u64,
}

fn open(bytes: &[u8], client: &str) -> Side {
    let mut s = Session::new();
    s.set_client_prefix(ClientPrefix::parse(client).unwrap());
    s.execute("file.openBytes", &json!({"name": "deck.pptx", "data": cmd::base64_encode(bytes)})).unwrap();
    let uid = s.active().unwrap().uid;
    s.set_live(uid, true).unwrap();
    Side { s, uid }
}

fn pair() -> (Side, Side) {
    let bytes = deck();
    (open(&bytes, "0000aa"), open(&bytes, "0000bb"))
}

impl Side {
    fn doc(&self) -> &Presentation {
        &self.s.active().unwrap().doc
    }
    fn run(&mut self, cmd: &str, params: serde_json::Value) -> crate::Result<serde_json::Value> {
        self.s.execute(cmd, &params)
    }
    fn go(&mut self, index: usize) {
        self.run("slide.go", json!({"index": index})).unwrap();
    }
    fn slide_id(&self, index: usize) -> SlideId {
        self.doc().slides[index].id
    }
    fn shape_ids(&self, index: usize) -> Vec<ShapeId> {
        self.doc().slides[index].shapes.iter().map(|s| s.id).collect()
    }
    fn nudge(&mut self, slide: usize, shape: ShapeId, dx: f64) -> crate::Result<serde_json::Value> {
        self.go(slide);
        self.run("shape.move", json!({"ids": [shape.0], "dx": dx}))
    }
    fn x(&self, slide: usize, shape: ShapeId) -> f64 {
        self.doc().slides[slide].shape(shape).and_then(|s| s.xfrm.as_ref()).map(|x| x.x).unwrap()
    }
    fn regions(&mut self) -> Vec<String> {
        self.s.take_live_changes(self.uid).into_iter().flat_map(|c| c.regions).map(|r| r.to_string()).collect()
    }
    /// Every region's payload, by id.
    fn all(&mut self) -> BTreeMap<String, String> {
        let mut regions = vec![Region::Slides];
        for slide in &self.doc().slides {
            regions.push(Region::Slide(slide.id));
            regions.extend(slide.shapes.iter().map(|sh| Region::Shape(slide.id, sh.id)));
        }
        let uid = self.uid;
        regions.into_iter().map(|r| (r.to_string(), self.s.live_payload(uid, &r).unwrap())).collect()
    }
}

/// Send every change `from` made since the last call to `to`, as the room would.
fn relay(from: &mut Side, to: &mut Side) -> Vec<String> {
    let mut sent = Vec::new();
    for change in from.s.take_live_changes(from.uid) {
        for region in change.regions {
            let payload = from.s.live_payload(from.uid, &region).unwrap();
            to.s.apply_remote(to.uid, &region.to_string(), &payload).unwrap();
            sent.push(region.to_string());
        }
    }
    sent
}

#[test]
fn edits_name_their_regions() {
    let (mut a, _) = pair();
    let (slide, shapes) = (a.slide_id(1), a.shape_ids(1));
    a.nudge(1, shapes[0], 5.0).unwrap();
    assert_eq!(a.regions(), [format!("shape:{slide}:{}", shapes[0])]);
    a.run("arrange.bringToFront", json!({"ids": [shapes[0].0]})).unwrap();
    assert_eq!(a.regions(), [format!("slide:{slide}")]);
    a.run("slide.hide", json!({})).unwrap();
    assert_eq!(a.regions(), [format!("slide:{slide}")]);
    a.run("slide.new", json!({})).unwrap();
    assert_eq!(a.regions(), ["slides"]);
    let theme = a.run("design.themes", json!({})).unwrap()["themes"][1]["name"].as_str().unwrap().to_owned();
    a.run("design.theme", json!({"name": theme})).unwrap();
    assert_eq!(a.regions(), ["*"]);
}

#[test]
fn concurrent_edits_of_different_regions_converge() {
    let (mut a, mut b) = pair();
    let shapes = a.shape_ids(1);
    a.nudge(1, shapes[0], 30.0).unwrap();
    b.nudge(1, shapes[1], -20.0).unwrap();
    b.go(0);
    b.run("slide.notes", json!({"text": "Ghi chú"})).unwrap();
    relay(&mut a, &mut b);
    relay(&mut b, &mut a);
    assert_eq!(a.all(), b.all());
    assert_eq!(a.x(1, shapes[0]), b.x(1, shapes[0]));
    assert_eq!(b.doc().slides[0].notes_text(), "Ghi chú");
    assert!(b.s.active().unwrap().is_dirty(), "the leader saves; every side holds the change");
}

#[test]
fn applying_a_region_leaves_every_other_region_byte_identical() {
    let (mut a, mut b) = pair();
    let shapes = a.shape_ids(1);
    let before = b.all();
    a.nudge(1, shapes[1], 12.0).unwrap();
    let sent = relay(&mut a, &mut b);
    assert_eq!(sent.len(), 1);
    let after = b.all();
    for (region, payload) in &before {
        if *region == sent[0] {
            assert_ne!(after[region], *payload);
            assert_eq!(after[region], a.all()[region], "the region is exactly the sender's");
        } else {
            assert_eq!(after[region], *payload, "{region} untouched");
        }
    }
    assert!(b.s.active().unwrap().history.undo.is_empty(), "someone else's change isn't an undo step");
}

#[test]
fn a_region_someone_else_holds_refuses_the_edit_untouched() {
    let (_, mut b) = pair();
    let (slide, shapes) = (b.slide_id(1), b.shape_ids(1));
    b.go(1);
    let before = b.doc().clone();
    b.s.set_live_blocked(b.uid, Blocked { regions: [Region::Shape(slide, shapes[0])].into(), document: false });
    let refused = b.run("shape.move", json!({"ids": [shapes[0].0], "dx": 5}));
    assert!(matches!(refused, Err(EngineError::RegionLocked(ref r)) if *r == format!("shape:{slide}:{}", shapes[0])), "{refused:?}");
    assert_eq!(*b.doc(), before);
    assert!(b.s.active().unwrap().history.undo.is_empty());
    assert!(b.regions().is_empty());
    b.nudge(1, shapes[1], 5.0).unwrap();
    // `*` held by someone blocks everything; holding anything blocks `*`.
    b.s.set_live_blocked(b.uid, Blocked { regions: HashSet::new(), document: true });
    assert!(matches!(b.nudge(1, shapes[1], 5.0), Err(EngineError::RegionLocked(_))));
}

#[test]
fn slide_operations_apply_on_the_other_side_with_the_same_ids_both_ways() {
    let (mut a, mut b) = pair();
    a.go(1);
    a.run("slide.new", json!({})).unwrap();
    a.run("shape.insert", json!({"preset": "rect", "rect": [1, 2, 3, 4]})).unwrap();
    a.run("slide.move", json!({"from": 2, "to": 0})).unwrap();
    relay(&mut a, &mut b);
    let ids: Vec<SlideId> = a.doc().slides.iter().map(|s| s.id).collect();
    assert_eq!(b.doc().slides.iter().map(|s| s.id).collect::<Vec<_>>(), ids);
    assert_eq!(b.shape_ids(0), a.shape_ids(0), "the new slide came whole, with its shape");
    b.go(1);
    b.run("slide.duplicate", json!({})).unwrap();
    b.go(0);
    b.run("slide.delete", json!({})).unwrap();
    relay(&mut b, &mut a);
    assert_eq!(a.doc().slides.iter().map(|s| s.id).collect::<Vec<_>>(), b.doc().slides.iter().map(|s| s.id).collect::<Vec<_>>());
    assert_eq!(a.all(), b.all());
}

#[test]
fn undo_skips_steps_someone_else_changed_since() {
    let (mut a, mut b) = pair();
    let shapes = a.shape_ids(1);
    a.nudge(1, shapes[0], 30.0).unwrap();
    relay(&mut a, &mut b);
    b.nudge(1, shapes[0], 7.0).unwrap();
    relay(&mut b, &mut a);
    let theirs = a.x(1, shapes[0]);
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(a.x(1, shapes[0]), theirs, "B's move stays");
    assert_eq!(a.s.take_live_skipped(a.uid), 1);
    assert!(a.regions().is_empty(), "nothing to send");
    b.run("edit.undo", json!({})).unwrap();
    relay(&mut b, &mut a);
    assert_eq!(a.x(1, shapes[0]), b.x(1, shapes[0]));
}

#[test]
fn undo_after_someone_else_changed_another_region_puts_back_only_its_own() {
    let (mut a, mut b) = pair();
    let shapes = a.shape_ids(1);
    let start = a.x(1, shapes[0]);
    a.nudge(1, shapes[0], 30.0).unwrap();
    relay(&mut a, &mut b);
    b.nudge(1, shapes[1], 9.0).unwrap();
    relay(&mut b, &mut a);
    let moved = a.x(1, shapes[1]);
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(a.x(1, shapes[0]), start);
    assert_eq!(a.x(1, shapes[1]), moved, "B's change stays");
    relay(&mut a, &mut b);
    assert_eq!(a.all(), b.all());
    a.run("edit.redo", json!({})).unwrap();
    relay(&mut a, &mut b);
    assert_eq!(b.x(1, shapes[0]), start + 30.0);
    assert_eq!(a.all(), b.all());
}

#[test]
fn a_refused_change_is_taken_back_keeping_the_others() {
    let (mut a, mut b) = pair();
    let shapes = a.shape_ids(1);
    let start = a.x(1, shapes[0]);
    a.nudge(1, shapes[0], 30.0).unwrap();
    let mine = a.s.take_live_changes(a.uid);
    b.nudge(1, shapes[1], 9.0).unwrap();
    relay(&mut b, &mut a);
    a.s.revert_live_change(a.uid, &mine[0]).unwrap();
    assert_eq!(a.x(1, shapes[0]), start);
    assert_eq!(a.x(1, shapes[1]), b.x(1, shapes[1]));
    assert!(a.s.active().unwrap().history.undo.is_empty());
    // A change the holder overwrote meanwhile stays theirs.
    a.nudge(1, shapes[1], 1.0).unwrap();
    let mine = a.s.take_live_changes(a.uid);
    b.nudge(1, shapes[1], 4.0).unwrap();
    relay(&mut b, &mut a);
    a.s.revert_live_change(a.uid, &mine[0]).unwrap();
    assert_eq!(a.x(1, shapes[1]), b.x(1, shapes[1]));
}

#[test]
fn ids_survive_a_save_and_a_reopen_after_remote_changes() {
    let (mut a, mut b) = pair();
    let shapes = a.shape_ids(1);
    a.go(1);
    a.run("slide.new", json!({})).unwrap();
    b.nudge(1, shapes[0], 3.0).unwrap();
    relay(&mut a, &mut b);
    relay(&mut b, &mut a);
    // A latecomer opens the leader's save.
    let saved = cmd::file::save_bytes(&a.doc().clone(), "pptx").unwrap();
    let mut c = open(&saved, "0000cc");
    assert_eq!(c.doc().slides.iter().map(|s| s.id).collect::<Vec<_>>(), a.doc().slides.iter().map(|s| s.id).collect::<Vec<_>>());
    assert_eq!(c.shape_ids(1), a.shape_ids(1));
    a.nudge(1, shapes[1], 11.0).unwrap();
    relay(&mut a, &mut c);
    let region = Region::Shape(a.slide_id(1), shapes[1]).to_string();
    assert_eq!(c.all()[&region], a.all()[&region]);
}

#[test]
fn a_document_change_travels_whole() {
    let (mut a, mut b) = pair();
    let theme = a.run("design.themes", json!({})).unwrap()["themes"][1]["name"].as_str().unwrap().to_owned();
    a.run("design.theme", json!({"name": theme})).unwrap();
    assert_eq!(relay(&mut a, &mut b), ["*"]);
    assert_eq!(a.doc().masters, b.doc().masters);
    assert_eq!(a.all(), b.all());
}

#[test]
fn payloads_from_a_newer_version_or_garbage_are_refused() {
    let (_, mut b) = pair();
    let region = Region::Shape(b.slide_id(1), b.shape_ids(1)[0]).to_string();
    let before = b.doc().clone();
    assert_eq!(b.s.apply_remote(b.uid, &region, r#"{"v":2,"del":1}"#), Err(LiveError::Version(2)));
    assert!(matches!(b.s.apply_remote(b.uid, &region, "{"), Err(LiveError::Malformed(_))));
    assert!(matches!(b.s.apply_remote(b.uid, &region, r#"{"v":1,"shape":{"id":"x"}}"#), Err(LiveError::Malformed(_))));
    assert!(matches!(b.s.apply_remote(b.uid, "shape:1", r#"{"v":1,"del":1}"#), Err(LiveError::Malformed(_))));
    assert_eq!(*b.doc(), before);
}

mod untrusted;
