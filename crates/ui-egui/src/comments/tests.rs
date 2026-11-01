//! The comment calls against a fake shell, natively: the selection's anchor, markers for open
//! threads on the slide shown, a click reported, going to an anchor, finding anchors again (also
//! after another person's change through the fake room of the co-editing tests), and none of it
//! touching the presentation.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use deckcraft_engine::model::ShapeId;
use deckcraft_engine::{Session, Target};
use serde_json::{Value, json};

use super::markers::{MAX_MARKERS, markers};
use super::*;
use crate::canvas::Xf;
use crate::live::IDLE_END_MS;
use crate::live::tests::{at, pair};
use crate::{Services, SlideApp};

#[derive(Clone, Default)]
struct Shell(Rc<RefCell<Vec<String>>>);

impl CommentHost for Shell {
    fn comment_clicked(&self, thread: &str) {
        self.0.borrow_mut().push(thread.to_owned());
    }
}

const XF: Xf = Xf { origin: egui::pos2(0.0, 0.0), scale: 1.0 };

/// Slide 1 from `file.new`; slide 2 "Budget" with a rectangle; slide 3 "Plan".
fn app() -> (SlideApp, u64, Shell, CommentInbox, SelectionAnchor) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("slide.new", &json!({"layout": "title", "title": "Budget"})).unwrap();
    s.execute("shape.insert", &json!({"preset": "rect", "rect": [10, 20, 50, 40]})).unwrap();
    s.execute("slide.new", &json!({"layout": "title", "title": "Plan"})).unwrap();
    s.execute("slide.go", &json!({"index": 0})).unwrap();
    let uid = s.active().unwrap().uid;
    let mut app = SlideApp::new(s, Services::default());
    let (shell, inbox, selection) = with_comments(&mut app, uid);
    (app, uid, shell, inbox, selection)
}

fn with_comments(app: &mut SlideApp, uid: u64) -> (Shell, CommentInbox, SelectionAnchor) {
    let shell = Shell::default();
    let comments = Comments::new(Box::new(shell.clone()));
    let (inbox, selection) = (comments.inbox(), comments.selection());
    app.comments = Some(comments);
    app.comments_bind(Some(uid));
    app.comments_step();
    (shell, inbox, selection)
}

fn json_of(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

fn rect(app: &SlideApp, slide: usize) -> ShapeId {
    app.session.active().unwrap().doc.slides[slide].shapes.last().unwrap().id
}

/// The anchor of the rectangle on slide `slide`, as the shell got it from `anchorOfSelection`.
fn anchor_of_rect(app: &mut SlideApp, selection: &SelectionAnchor, slide: usize) -> String {
    app.run("slide.go", json!({"index": slide})).unwrap();
    let id = rect(app, slide);
    app.run("edit.select", json!({"ids": [id.0]})).unwrap();
    app.comments_step();
    let anchor = selection.borrow().clone().unwrap();
    app.run("edit.deselect", json!({})).unwrap();
    anchor
}

fn thread(id: &str, anchor: &str, status: &str) -> Thread {
    Thread { id: id.into(), anchor: anchor.into(), status: status.into(), count: 3 }
}

fn resolve(app: &mut SlideApp, inbox: &CommentInbox, threads: Vec<Thread>) -> Vec<ResolvedThread> {
    let out = Rc::new(RefCell::new(None));
    let slot = out.clone();
    inbox.borrow_mut().push(CommentEvent::Resolve { threads, reply: Box::new(move |r| *slot.borrow_mut() = Some(r)) });
    app.comments_step();
    out.borrow_mut().take().expect("answered on the next step")
}

/// The thread list comes from the server and anyone who may comment can pile threads on one
/// slide: a bounded number is read and resolved, and a slide draws a bounded number of bubbles
/// (placing each checks the earlier ones, every frame).
#[test]
fn bounded_threads_and_markers() {
    let (mut app, _uid, _shell, inbox, selection) = app();
    let slide = selection.borrow().clone().unwrap();
    let many: Vec<Thread> = (0..MAX_THREADS + 500).map(|i| thread(&format!("t{i}"), &slide, "open")).collect();
    inbox.borrow_mut().push(CommentEvent::Show(many.clone()));
    app.comments_step();
    assert_eq!(app.comments.as_ref().unwrap().threads.len(), MAX_THREADS);
    assert_eq!(markers(&app, XF).len(), MAX_MARKERS);
    assert!(resolve(&mut app, &inbox, many).len() <= MAX_THREADS);
}

#[test]
fn the_selection_anchor_is_the_selected_shape_else_the_slide() {
    let (mut app, _uid, _shell, _inbox, selection) = app();
    let first = json_of(selection.borrow().as_deref().unwrap());
    assert_eq!((first["region"].as_str().map(|r| r.starts_with("slide:")), first["label"].clone()), (Some(true), "Slide 1".into()));
    let shape = json_of(&anchor_of_rect(&mut app, &selection, 1));
    let slide_id = app.session.active().unwrap().doc.slides[1].id;
    assert_eq!(shape["region"], format!("shape:{slide_id}:{}", rect(&app, 1)));
    assert_eq!((shape["label"].clone(), shape["soft"]["title"].clone()), ("Slide 2 · Budget".into(), "Budget".into()));
    app.comments_step();
    assert_eq!(json_of(selection.borrow().as_deref().unwrap())["region"], format!("slide:{slide_id}"), "deselected: the slide");
    app.session.select(|_, sel| sel.target = Target::Master { master: 0 }).unwrap();
    app.comments_step();
    assert_eq!(*selection.borrow(), None, "Slide Master view anchors nothing");
}

#[test]
fn open_threads_get_markers_on_their_slide_and_a_click_reaches_the_shell() {
    let (mut app, _uid, shell, inbox, selection) = app();
    let shape = anchor_of_rect(&mut app, &selection, 1);
    app.run("slide.go", json!({"index": 2})).unwrap();
    app.comments_step();
    let plan = selection.borrow().clone().unwrap();
    inbox.borrow_mut().push(CommentEvent::Show(vec![
        thread("t1", &shape, "open"),
        thread("t2", &shape, "resolved"),
        thread("t3", &plan, "open"),
        thread("t4", "nonsense", "open"),
    ]));
    assert!(app.comments_step());
    assert_eq!(markers(&app, XF).iter().map(|m| m.thread.as_str()).collect::<Vec<_>>(), ["t3"], "slide 3 shown");
    app.run("slide.go", json!({"index": 1})).unwrap();
    let on_budget = markers(&app, XF);
    assert_eq!(on_budget.iter().map(|m| (m.thread.as_str(), m.count)).collect::<Vec<_>>(), [("t1", 3)]);
    let m = &on_budget[0];
    assert_eq!(egui::Rect::from_points(&m.corners), egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(50.0, 40.0)));
    assert!((m.bubble.left() - 64.0).abs() < 0.01, "right of the shape: {:?}", m.bubble);
    assert!(shell.0.borrow().is_empty());
    app.comments.as_mut().unwrap().clicked = Some("t1".into());
    app.comments_step();
    assert_eq!(*shell.0.borrow(), ["t1"]);
}

#[test]
fn focusing_an_anchor_goes_to_its_slide_and_selects_its_shape() {
    let (mut app, _uid, _shell, inbox, selection) = app();
    let shape = anchor_of_rect(&mut app, &selection, 1);
    app.run("slide.go", json!({"index": 0})).unwrap();
    inbox.borrow_mut().push(CommentEvent::Focus(shape));
    app.comments_step();
    let st = app.session.active().unwrap();
    assert_eq!((st.selection.slide, st.selection.shapes.clone()), (1, vec![rect(&app, 1)]));
    inbox.borrow_mut().push(CommentEvent::Focus("{}".into()));
    app.comments_step();
    assert_eq!(app.session.active().unwrap().selection.slide, 1, "an anchor that can't be read goes nowhere");
}

#[test]
fn resolving_answers_each_readable_thread_kept_or_orphaned() {
    let (mut app, _uid, _shell, inbox, selection) = app();
    let shape = anchor_of_rect(&mut app, &selection, 1);
    let newer = r#"{"v":7,"region":"slide:256","soft":{"index":0}}"#;
    app.run("slide.move", json!({"from": 1, "to": 2})).unwrap();
    let out = resolve(&mut app, &inbox, vec![thread("t1", &shape, "open"), thread("t2", newer, "open")]);
    assert_eq!(out.len(), 1, "a newer editor's anchor is left out");
    let moved = json_of(&out[0].anchor);
    assert_eq!((out[0].status, moved["soft"]["index"].clone(), moved["label"].clone()), (AnchorStatus::Open, 2.into(), "Slide 3 · Budget".into()));
    app.run("slide.go", json!({"index": 2})).unwrap();
    let id = rect(&app, 2);
    app.run("edit.delete", json!({"ids": [id.0]})).unwrap();
    let out = resolve(&mut app, &inbox, vec![thread("t1", &shape, "open")]);
    assert_eq!((out[0].status, out[0].anchor.as_str()), (AnchorStatus::Orphaned, shape.as_str()), "an orphan keeps its anchor");
}

#[test]
fn markers_focus_and_resolving_never_change_the_presentation() {
    let (mut app, _uid, _shell, inbox, selection) = app();
    let shape = anchor_of_rect(&mut app, &selection, 1);
    let st = app.session.active().unwrap();
    let (doc, dirty, undo) = (st.doc.clone(), st.is_dirty(), st.history.undo.len());
    inbox.borrow_mut().push(CommentEvent::Show(vec![thread("t1", &shape, "open")]));
    inbox.borrow_mut().push(CommentEvent::Focus(shape.clone()));
    resolve(&mut app, &inbox, vec![thread("t1", &shape, "open")]);
    assert_eq!(markers(&app, XF).len(), 1);
    let st = app.session.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc));
    assert_eq!((st.is_dirty(), st.history.undo.len()), (dirty, undo));
}

#[test]
fn without_the_comment_calls_nothing_changes() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let mut app = SlideApp::new(s, Services::default());
    app.comments_bind(Some(1));
    assert!(!app.comments_step());
    assert!(markers(&app, XF).is_empty());
    assert!(app.comments.is_none());
}

/// Another person moves the slide, then deletes the shape; this side finds the anchor on the
/// moved slide, its marker follows, then it is orphaned.
#[test]
fn another_persons_changes_move_the_anchor_through_the_room() {
    let (mut a, mut b, room) = pair();
    let (_shell, inbox, selection) = with_comments(&mut a.app, a.uid);
    let shape = anchor_of_rect(&mut a.app, &selection, 1);
    inbox.borrow_mut().push(CommentEvent::Show(vec![thread("t1", &shape, "open")]));
    a.app.comments_step();
    a.go(1);
    assert_eq!(markers(&a.app, XF).len(), 1);

    b.go(1);
    b.app.run("slide.move", json!({"from": 1, "to": 0})).unwrap();
    b.step();
    a.step();
    let order = |c: &crate::live::tests::Client| c.app.session.active().unwrap().doc.slides.iter().map(|s| s.id).collect::<Vec<_>>();
    assert_eq!(order(&a), order(&b), "the move reached this side");
    let out = resolve(&mut a.app, &inbox, vec![thread("t1", &shape, "open")]);
    let moved = json_of(&out[0].anchor);
    assert_eq!(
        (out[0].status, moved["region"].clone(), moved["soft"]["index"].clone()),
        (AnchorStatus::Open, json_of(&shape)["region"].clone(), 0.into())
    );
    assert_eq!(moved["label"], "Slide 1");
    a.go(0);
    assert_eq!(markers(&a.app, XF).iter().map(|m| m.thread.as_str()).collect::<Vec<_>>(), ["t1"], "the marker follows its slide");

    at(&room, IDLE_END_MS + 1.0);
    b.step();
    a.step();
    b.go(0);
    let id = b.app.session.active().unwrap().doc.slides[0].shapes.last().unwrap().id;
    b.app.run("edit.delete", json!({"ids": [id.0]})).unwrap();
    at(&room, 2.0 * IDLE_END_MS + 3.0);
    b.step();
    a.step();
    assert!(a.app.session.active().unwrap().doc.slides[0].shape(id).is_none(), "the deletion reached this side");
    let gone = resolve(&mut a.app, &inbox, vec![thread("t1", &out[0].anchor, "open")]);
    assert_eq!(gone[0].status, AnchorStatus::Orphaned);
    assert!(markers(&a.app, XF).is_empty());
}
