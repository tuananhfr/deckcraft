//! Two apps co-editing through a fake shell that keeps the lock table, grants or refuses locks at
//! once and relays each region to the other side, natively.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use deckcraft_engine::model::ShapeId;
use deckcraft_engine::{ClientPrefix, Session, cmd};
use serde_json::json;

use super::*;
use crate::{Services, SlideApp};
use deckcraft_engine::live::Region;

const NAMES: [&str; 2] = ["Ada", "Bea"];

#[derive(Default)]
pub(crate) struct Room {
    inboxes: Vec<LiveInbox>,
    /// (region, seat)
    locks: Vec<(String, usize)>,
    log: Vec<String>,
    now: Rc<Cell<f64>>,
}

/// `*` clashes with every region, any other region only with itself.
fn clash(a: &str, b: &str) -> bool {
    a == b || a == "*" || b == "*"
}

impl Room {
    fn push(&self, seat: usize, event: LiveEvent) {
        self.inboxes[seat].borrow_mut().push(event);
    }

    fn broadcast_locks(&self) {
        for seat in 0..self.inboxes.len() {
            let locks = self
                .locks
                .iter()
                .map(|(region, holder)| LockView {
                    region: region.clone(),
                    holder: format!("m{holder}"),
                    colour: Some("#e11d48".into()),
                    name: None,
                    mine: *holder == seat,
                })
                .collect();
            self.push(seat, LiveEvent::Locks(locks));
        }
    }
}

struct Seat {
    room: Rc<RefCell<Room>>,
    me: usize,
}

impl RegionHost for Seat {
    fn region_will_edit(&self, region: &str, reply: Box<dyn FnOnce(bool)>) {
        let granted = {
            let mut room = self.room.borrow_mut();
            room.log.push(format!("{}: willEdit {region}", self.me));
            let free = !room.locks.iter().any(|(r, h)| *h != self.me && clash(r, region));
            if free && !room.locks.iter().any(|(r, h)| *h == self.me && r == region) {
                room.locks.push((region.to_owned(), self.me));
                room.broadcast_locks();
            }
            free
        };
        reply(granted);
    }
    fn region_changed(&self, region: &str, payload: &str) {
        let mut room = self.room.borrow_mut();
        room.log.push(format!("{}: changed {region}", self.me));
        for seat in (0..room.inboxes.len()).filter(|s| *s != self.me) {
            room.push(seat, LiveEvent::ApplyRemote { region: region.into(), payload: payload.into(), author: Some(format!("m{}", self.me)) });
        }
    }
    fn region_did_end_edit(&self, region: &str) {
        let mut room = self.room.borrow_mut();
        room.log.push(format!("{}: didEndEdit {region}", self.me));
        let me = self.me;
        room.locks.retain(|(r, h)| !(*h == me && r == region));
        room.broadcast_locks();
    }
    fn selection_changed(&self, region: &str, range: &str) {
        let room = self.room.borrow();
        for seat in (0..room.inboxes.len()).filter(|s| *s != self.me) {
            let cursor =
                Cursor { author: format!("m{}", self.me), region: region.into(), range: range.into(), name: None, colour: Some("#2563eb".into()) };
            room.push(seat, LiveEvent::Cursors(vec![cursor]));
        }
    }
    fn now_ms(&self) -> f64 {
        self.room.borrow().now.get()
    }
}

/// Two slides; the second has two rectangles.
fn deck() -> Vec<u8> {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("slide.new", &json!({"layout": "blank"})).unwrap();
    s.execute("shape.insert", &json!({"preset": "rect", "rect": [10, 10, 50, 50]})).unwrap();
    s.execute("shape.insert", &json!({"preset": "ellipse", "rect": [100, 100, 40, 40]})).unwrap();
    cmd::file::save_bytes(&s.active().unwrap().doc, "pptx").unwrap()
}

pub(crate) struct Client {
    pub(crate) app: SlideApp,
    pub(crate) uid: u64,
}

impl Client {
    pub(crate) fn step(&mut self) -> bool {
        self.app.live_step()
    }
    pub(crate) fn go(&mut self, index: usize) {
        self.app.run("slide.go", json!({"index": index})).unwrap();
    }
    fn shapes(&self) -> Vec<ShapeId> {
        self.app.session.active().unwrap().doc.slides[1].shapes.iter().map(|s| s.id).collect()
    }
    fn nudge(&mut self, shape: ShapeId, dx: f64) -> bool {
        self.go(1);
        self.app.run("shape.move", json!({"ids": [shape.0], "dx": dx})).is_ok()
    }
    fn x(&self, shape: ShapeId) -> f64 {
        self.app.session.active().unwrap().doc.slides[1].shape(shape).and_then(|s| s.xfrm.as_ref()).map(|x| x.x).unwrap()
    }
    fn slide_ids(&self) -> Vec<u32> {
        self.app.session.active().unwrap().doc.slides.iter().map(|s| s.id.0).collect()
    }
    fn toast(&self) -> Option<&str> {
        self.app.toast.as_ref().map(|t| t.text.as_str())
    }
    fn live(&self) -> &Live {
        self.app.live.as_ref().unwrap()
    }
}

/// Two apps with the same presentation open, in one room, after their first frame.
pub(crate) fn pair() -> (Client, Client, Rc<RefCell<Room>>) {
    let room = Rc::new(RefCell::new(Room::default()));
    let bytes = deck();
    let mut clients = Vec::new();
    for (me, prefix) in ["0000aa", "0000bb"].into_iter().enumerate() {
        let mut session = Session::new();
        session.set_client_prefix(ClientPrefix::parse(prefix).unwrap());
        session.execute("file.openBytes", &json!({"name": "deck.pptx", "data": cmd::base64_encode(&bytes)})).unwrap();
        let uid = session.active().unwrap().uid;
        let mut app = SlideApp::new(session, Services::default());
        app.live = Some(Live::new(Box::new(Seat { room: room.clone(), me })));
        room.borrow_mut().inboxes.push(app.live.as_ref().unwrap().inbox());
        app.live_bind(Some(uid));
        clients.push(Client { app, uid });
    }
    for seat in 0..2 {
        let members = (0..2).map(|m| Member { id: format!("m{m}"), name: Some(NAMES[m].into()), colour: None, me: m == seat }).collect();
        room.borrow().push(seat, LiveEvent::Presence(members));
    }
    let mut b = clients.pop().unwrap();
    let mut a = clients.pop().unwrap();
    a.step();
    b.step();
    log(&room);
    (a, b, room)
}

pub(crate) fn log(room: &Rc<RefCell<Room>>) -> Vec<String> {
    std::mem::take(&mut room.borrow_mut().log)
}

pub(crate) fn at(room: &Rc<RefCell<Room>>, now: f64) {
    room.borrow().now.set(now);
}

#[test]
fn an_edit_takes_the_lock_sends_the_region_and_lets_it_go_once_left() {
    let (mut a, mut b, room) = pair();
    assert!(a.app.session.is_live(a.uid) && b.app.session.is_live(b.uid));
    let shapes = a.shapes();
    let slide = a.app.session.active().unwrap().doc.slides[1].id;
    assert!(a.nudge(shapes[0], 20.0));
    at(&room, 100.0);
    a.step();
    let region = format!("shape:{slide}:{}", shapes[0]);
    assert_eq!(log(&room), [format!("0: willEdit {region}"), format!("0: changed {region}")]);
    b.step();
    assert_eq!(b.x(shapes[0]), a.x(shapes[0]));
    assert!(b.app.session.active().unwrap().is_dirty());
    assert!(b.app.session.active().unwrap().history.undo.is_empty(), "someone else's change isn't B's to undo");
    at(&room, 100.0 + IDLE_END_MS + 1.0);
    a.step();
    assert_eq!(log(&room), [format!("0: didEndEdit {region}")]);
    assert!(room.borrow().locks.is_empty());
}

#[test]
fn a_shape_someone_else_holds_refuses_the_edit_and_names_them() {
    let (mut a, mut b, _room) = pair();
    let shapes = a.shapes();
    assert!(a.nudge(shapes[0], 20.0));
    a.step();
    b.step();
    let x = b.x(shapes[0]);
    assert!(!b.nudge(shapes[0], 5.0), "refused before it changes anything");
    b.step();
    assert_eq!(b.x(shapes[0]), x);
    assert_eq!(b.toast(), Some("Ada is editing this part."));
    assert!(b.nudge(shapes[1], 5.0), "another shape is free");
}

/// The status bar shows the refusal as stored and translates it when drawn, as WordCraft words it.
#[test]
fn a_refused_edit_is_told_in_the_interface_language() {
    let (mut a, mut b, _room) = pair();
    let shapes = a.shapes();
    assert!(a.nudge(shapes[0], 20.0));
    a.step();
    b.step();
    assert!(!b.nudge(shapes[0], 5.0));
    let status = b.app.status.clone().map(|s| s.0).unwrap_or_default();
    let vi = crate::i18n::Lang::from_code("vi").unwrap();
    assert_eq!(crate::i18n::tr(vi, &status), "Có người khác đang sửa phần này nên thay đổi của bạn chưa được thực hiện.");
}

#[test]
fn a_refused_lock_takes_the_change_back_and_keeps_the_holders_version() {
    let (mut a, mut b, room) = pair();
    let shapes = a.shapes();
    assert!(a.nudge(shapes[0], 20.0));
    assert!(b.nudge(shapes[0], -7.0), "B hasn't heard of A's lock yet");
    a.step();
    b.step();
    assert_eq!(b.x(shapes[0]), a.x(shapes[0]), "A's version, not the one before B's change");
    assert_eq!(b.toast(), Some("Ada is editing this part."));
    assert_eq!(log(&room).iter().filter(|l| l.contains("changed")).count(), 1, "nothing of B's was sent");
    assert!(b.app.session.active().unwrap().history.undo.is_empty(), "the refused step left B's history");
}

#[test]
fn slide_operations_reach_the_other_side_with_the_same_ids_both_ways() {
    let (mut a, mut b, room) = pair();
    a.go(1);
    a.app.run("slide.new", json!({})).unwrap();
    a.app.run("slide.move", json!({"from": 2, "to": 0})).unwrap();
    a.step();
    b.step();
    assert_eq!(b.slide_ids(), a.slide_ids());
    assert!(b.app.run("slide.delete", json!({})).is_err(), "A still holds the slide order");
    at(&room, IDLE_END_MS + 1.0);
    a.step();
    b.step();
    b.go(0);
    b.app.run("slide.delete", json!({})).unwrap();
    at(&room, IDLE_END_MS + 2.0);
    b.step();
    a.step();
    assert_eq!(a.slide_ids(), b.slide_ids());
    assert_eq!(a.slide_ids().len(), 2);
}

#[test]
fn undo_skips_a_step_someone_else_changed_since_and_never_reverts_them() {
    let (mut a, mut b, room) = pair();
    let shapes = a.shapes();
    assert!(a.nudge(shapes[0], 20.0));
    a.step();
    b.step();
    at(&room, IDLE_END_MS + 1.0);
    a.step();
    b.step();
    assert!(b.nudge(shapes[0], 9.0));
    b.step();
    a.step();
    let theirs = a.x(shapes[0]);
    log(&room);
    a.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(a.x(shapes[0]), theirs, "B's move stays");
    a.step();
    assert_eq!(a.toast(), Some("Skipped a change someone else has edited since."));
    assert!(log(&room).iter().all(|l| !l.contains("changed")), "nothing to send");
}

#[test]
fn a_conflict_asks_the_user_keep_mine_puts_this_sides_version_back_use_theirs_keeps_the_rooms() {
    let (mut a, mut b, room) = pair();
    let shapes = a.shapes();
    let slide = a.app.session.active().unwrap().doc.slides[1].id;
    let region = format!("shape:{slide}:{}", shapes[0]);
    assert!(a.nudge(shapes[0], 20.0));
    a.step();
    b.step();
    let mine = a.x(shapes[0]);
    at(&room, IDLE_END_MS + 1.0);
    a.step();
    b.step();
    assert!(b.nudge(shapes[0], 9.0));
    b.step();
    a.step();
    let theirs = a.x(shapes[0]);
    let answered = Rc::new(RefCell::new(Vec::new()));
    let reply = |answered: &Rc<RefCell<Vec<Choice>>>| -> ChoiceReply {
        let answered = answered.clone();
        Box::new(move |c| answered.borrow_mut().push(c))
    };
    room.borrow().push(0, LiveEvent::Conflict { region: region.clone(), reply: reply(&answered) });
    assert!(a.step(), "waits for the user");
    assert!(answered.borrow().is_empty());
    a.app.answer_live_conflict(Choice::Theirs);
    assert_eq!(a.x(shapes[0]), theirs);
    room.borrow().push(0, LiveEvent::Conflict { region, reply: reply(&answered) });
    a.step();
    log(&room);
    a.app.answer_live_conflict(Choice::Mine);
    assert_eq!(a.x(shapes[0]), mine, "this side's last sent version");
    a.step();
    assert!(log(&room).iter().all(|l| !l.contains("changed")), "the shell sends it, not the app");
    assert_eq!(*answered.borrow(), [Choice::Theirs, Choice::Mine]);
}

#[test]
fn a_lost_lock_ends_the_text_edit_and_says_so() {
    let (mut a, _b, room) = pair();
    let shapes = a.shapes();
    a.go(1);
    a.app.run("text.edit", json!({"id": shapes[0].0})).unwrap();
    a.step();
    assert!(log(&room).iter().any(|l| l.contains("willEdit shape:")), "the lock is asked for before typing");
    room.borrow().push(0, LiveEvent::LockDenied(Region::Shape(a.app.session.active().unwrap().doc.slides[1].id, shapes[0]).to_string()));
    a.step();
    assert!(a.app.session.active().unwrap().selection.text.is_none());
    assert_eq!(a.toast(), Some("Someone else is editing this part."));
}

#[test]
fn others_locks_and_selections_are_drawn_on_the_slide() {
    let (mut a, mut b, _room) = pair();
    let shapes = a.shapes();
    assert!(a.nudge(shapes[0], 20.0));
    a.step();
    b.go(1);
    b.step();
    let xf = crate::canvas::Xf { origin: egui::pos2(0.0, 0.0), scale: 1.0 };
    let marks = marks::marks(&b.app, xf);
    let lock = marks.iter().find(|m| m.lock).expect("A's lock outlined");
    assert_eq!((lock.name.as_str(), lock.colour), ("Ada", egui::Color32::from_rgb(0xe1, 0x1d, 0x48)));
    assert!(lock.corners.is_some());
    assert!(marks::marks(&a.app, xf).iter().all(|m| !m.lock), "one's own lock isn't drawn");
}

#[test]
fn the_selection_is_reported_throttled() {
    let (mut a, mut b, room) = pair();
    let first = a.app.session.active().unwrap().doc.slides[0].id;
    assert_eq!(b.live().cursors.iter().map(|c| c.region.clone()).collect::<Vec<_>>(), [format!("slide:{first}")]);
    a.go(1);
    assert!(a.step(), "waits for the throttle");
    b.step();
    assert_eq!(b.live().cursors[0].region, format!("slide:{first}"));
    at(&room, SELECTION_MS + 1.0);
    a.step();
    b.step();
    let second = a.app.session.active().unwrap().doc.slides[1].id;
    assert_eq!(b.live().cursors[0].region, format!("slide:{second}"));
}

#[test]
fn leadership_is_only_noted() {
    let (mut a, _b, room) = pair();
    room.borrow().push(0, LiveEvent::Leader(true));
    a.step();
    assert!(a.live().is_leader());
}

#[test]
fn without_erp_host_v3_nothing_is_live() {
    let mut app = SlideApp::new(Session::with_new(), Services::default());
    let uid = app.session.active().unwrap().uid;
    assert!(!app.live_step());
    app.live_bind(Some(uid));
    assert!(!app.session.is_live(uid));
    app.run("slide.new", json!({})).unwrap();
    assert!(app.session.take_live_changes(uid).is_empty());
}
