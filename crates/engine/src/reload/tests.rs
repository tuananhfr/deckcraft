use serde_json::json;

use crate::*;

/// A deck of three slides, each with a rectangle, as a PPTX round trip gives it back.
fn reopened(slides: usize) -> model::Presentation {
    let mut s = Session::with_new();
    for i in 1..slides {
        s.execute("slide.new", &json!({})).unwrap();
        s.execute("shape.insert", &json!({"preset": "rect", "rect": [10 * i, 10, 50, 50]})).unwrap();
    }
    let bytes = cmd::file::save_bytes(&s.active().unwrap().doc, "pptx").unwrap();
    cmd::file::open_presentation("d.pptx", &bytes).unwrap()
}

#[test]
fn a_newer_version_keeps_the_slide_and_the_selection_in_the_same_tab() {
    let mut s = Session::new();
    cmd::file::add_opened(&mut s, "d.pptx", None, reopened(3)).unwrap();
    s.execute("file.new", &json!({})).unwrap();
    s.set_active(0);
    s.execute("slide.go", &json!({"index": 2})).unwrap();
    let shape = s.active().unwrap().current_slide().unwrap().shapes.last().unwrap().id;
    s.execute("edit.select", &json!({"ids": [shape.0]})).unwrap();
    s.execute("text.set", &json!({"id": shape.0, "text": "local"})).unwrap();
    let uid = s.active().unwrap().uid;
    let revision = s.active().unwrap().revision;
    assert!(s.replace_content(uid, reopened(3)));
    assert_eq!(s.active_index(), Some(0), "the same tab, still active");
    let d = s.active().unwrap();
    assert_eq!((d.uid, d.selection.slide), (uid, 2));
    assert_eq!(d.selection.shapes, vec![shape]);
    assert!(!d.is_dirty(), "the new version is what the server has");
    assert!(d.history.undo.is_empty(), "undo can't bring the old version back");
    assert!(d.revision > revision);
    assert_eq!(s.documents().len(), 2);
}

#[test]
fn a_shorter_version_keeps_the_last_slide_and_drops_what_is_gone() {
    let mut s = Session::new();
    cmd::file::add_opened(&mut s, "d.pptx", None, reopened(3)).unwrap();
    s.execute("slide.go", &json!({"index": 2})).unwrap();
    let shape = s.active().unwrap().current_slide().unwrap().shapes.last().unwrap().id;
    s.execute("edit.select", &json!({"ids": [shape.0]})).unwrap();
    let uid = s.active().unwrap().uid;
    assert!(s.replace_content(uid, reopened(1)));
    let d = s.active().unwrap();
    assert_eq!(d.selection.slide, 0);
    assert!(d.selection.shapes.iter().all(|id| d.shape(*id).is_some()));
    assert!(!s.replace_content(u64::MAX, reopened(1)));
}

#[test]
fn a_read_only_presentation_takes_a_newer_version_and_stays_read_only() {
    let mut s = Session::new();
    cmd::file::add_opened(&mut s, "d.pptx", None, reopened(2)).unwrap();
    let uid = s.active().unwrap().uid;
    s.set_access(uid, Access::read_only(false));
    assert!(s.replace_content(uid, reopened(3)));
    assert_eq!(s.active().unwrap().doc.slides.len(), 3);
    assert_eq!(s.active().unwrap().access, Access::read_only(false));
}
