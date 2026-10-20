//! Several open presentations: the open-documents limit on every path that adds one, switching,
//! and closing one that isn't active.

use serde_json::json;

use crate::*;

fn full() -> Session {
    let mut s = Session::new();
    for _ in 0..MAX_DOCUMENTS {
        s.execute("file.new", &json!({})).unwrap();
    }
    s
}

fn titles(s: &Session) -> Vec<String> {
    s.documents().iter().map(DocState::title).collect()
}

fn refused(r: Result<serde_json::Value>) -> bool {
    matches!(r, Err(EngineError::TooManyDocuments))
}

#[test]
fn new_is_refused_past_the_limit() {
    let mut s = full();
    assert_eq!(s.documents().len(), 10);
    s.set_active(3);
    assert!(refused(s.execute("file.new", &json!({}))));
    assert_eq!(s.documents().len(), 10);
    assert_eq!(s.active_index(), Some(3), "the refusal leaves the active presentation");
    s.close_document(0);
    s.execute("file.new", &json!({})).unwrap();
    assert_eq!(s.documents().last().unwrap().title(), "Presentation11", "no untitled number was used by the refusal");
    assert_eq!(EngineError::TooManyDocuments.to_string(), "You can have up to 10 documents open. Close one to open another.");
}

#[test]
fn opening_bytes_or_a_file_is_refused_past_the_limit() {
    let bytes = cmd::file::save_bytes(&model::defaults::new_presentation(None), "pptx").unwrap();
    let mut s = full();
    assert!(refused(s.execute("file.openBytes", &json!({"name": "a.pptx", "data": cmd::base64_encode(&bytes)}))));
    let path = std::env::temp_dir().join(format!("deckcraft-limit-{}.pptx", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    assert!(refused(s.execute("file.open", &json!({"path": path.to_string_lossy()}))));
    let _ = std::fs::remove_file(&path);
    assert!(refused(sample::open_sample(&mut s).map(|_| json!(null))));
    assert!(matches!(s.add_document(DocState::new(model::defaults::new_presentation(None), None, "x".into())), Err(EngineError::TooManyDocuments)));
    assert_eq!(s.documents().len(), 10);
}

#[test]
fn recovery_opens_up_to_the_limit_and_keeps_the_rest() {
    let dir = std::env::temp_dir().join(format!("deckcraft-limit-recovery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut a = Session::new();
    a.recovery_dir = Some(dir.clone());
    for _ in 0..3 {
        a.execute("file.new", &json!({})).unwrap();
        a.execute("slide.new", &json!({})).unwrap();
    }
    a.execute("file.recovery.save", &json!({})).unwrap();
    let mut b = Session::new();
    b.recovery_dir = Some(dir.clone());
    for _ in 0..MAX_DOCUMENTS - 2 {
        b.execute("file.new", &json!({})).unwrap();
    }
    assert!(refused(b.execute("file.recovery.open", &json!({}))));
    assert_eq!(b.documents().len(), 10);
    assert_eq!(recovery::list(&dir).len(), 1, "the one that didn't fit stays recoverable");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closing_another_presentation_keeps_the_active_one() {
    let mut s = Session::new();
    for _ in 0..4 {
        s.execute("file.new", &json!({})).unwrap();
    }
    s.execute("window.next", &json!({"index": 2})).unwrap();
    assert_eq!(s.doc().unwrap().title(), "Presentation3");
    s.execute("file.close", &json!({"index": 0})).unwrap();
    assert_eq!(s.doc().unwrap().title(), "Presentation3", "a tab before the active one");
    s.execute("file.close", &json!({"index": 2})).unwrap();
    assert_eq!(s.doc().unwrap().title(), "Presentation3", "a tab after it");
    assert_eq!(titles(&s), ["Presentation2", "Presentation3"]);
    assert!(s.execute("file.close", &json!({"index": 5})).is_err());
    s.execute("file.close", &json!({})).unwrap();
    assert_eq!(titles(&s), ["Presentation2"], "no index closes the active one");
    assert_eq!(s.active_index(), Some(0));
    let journal: Vec<&str> = s.journal.iter().map(|(id, _)| id.as_str()).collect();
    assert!(journal.contains(&"window.next") && journal.contains(&"file.close"), "{journal:?}");
}

fn open_named(s: &mut Session, name: &str) -> Result<serde_json::Value> {
    let bytes = cmd::file::save_bytes(&model::defaults::new_presentation(None), "pptx").unwrap();
    s.execute("file.openBytes", &json!({"name": format!("{name}.pptx"), "data": cmd::base64_encode(&bytes)}))
}

fn names(prefix: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{prefix}{i}")).collect()
}

#[test]
fn an_untouched_sample_never_takes_a_slot() {
    let mut s = Session::new();
    sample::open_sample(&mut s).unwrap();
    for name in names("f", MAX_DOCUMENTS) {
        open_named(&mut s, &name).unwrap();
    }
    assert_eq!(titles(&s), names("f", MAX_DOCUMENTS), "the first one opened replaced the sample");

    // Beside other presentations it stays until the limit, then gives way.
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    sample::open_sample(&mut s).unwrap();
    for name in names("f", MAX_DOCUMENTS - 2) {
        open_named(&mut s, &name).unwrap();
    }
    assert_eq!(s.documents().len(), MAX_DOCUMENTS);
    assert_eq!(titles(&s)[1], "Northern Lights");
    s.set_active(0);
    open_named(&mut s, "last").unwrap();
    assert_eq!(s.documents().len(), MAX_DOCUMENTS);
    assert!(!titles(&s).contains(&"Northern Lights".to_string()));
    assert_eq!(s.doc().unwrap().title(), "last", "the new one is active");
    assert!(refused(s.execute("file.new", &json!({}))), "{} user presentations fill every slot", MAX_DOCUMENTS);
}

#[test]
fn an_edited_sample_counts() {
    let mut s = Session::new();
    sample::open_sample(&mut s).unwrap();
    s.execute("slide.new", &json!({})).unwrap();
    for name in names("f", MAX_DOCUMENTS - 1) {
        open_named(&mut s, &name).unwrap();
    }
    assert!(refused(open_named(&mut s, "more")));
    assert_eq!(titles(&s)[0], "Northern Lights");

    // An undone edit leaves something to redo, so it still counts.
    let mut s = Session::new();
    sample::open_sample(&mut s).unwrap();
    s.execute("slide.new", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
    open_named(&mut s, "f").unwrap();
    assert_eq!(titles(&s), ["Northern Lights", "f"]);
}

#[test]
fn only_a_placeholder_blank_is_replaced() {
    let mut s = Session::with_new();
    open_named(&mut s, "f").unwrap();
    assert_eq!(titles(&s), ["Presentation1", "f"], "a blank the user made stays");

    let mut s = Session::with_new();
    s.active_mut().unwrap().placeholder = true;
    s.execute("file.new", &json!({})).unwrap();
    assert_eq!(titles(&s), ["Presentation2"], "the start blank gives way to a new one too");
}
