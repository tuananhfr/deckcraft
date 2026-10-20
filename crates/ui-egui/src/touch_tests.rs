//! One finger on the slide canvas and in the slide show, driven through the touch and pointer
//! events eframe sends for a touch screen; the mouse keeps its own behaviour.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use deckcraft_engine::{Session, tools::Hit};
use deckcraft_geom::Point;
use egui::{Event, Modifiers, PointerButton, Pos2, TouchDeviceId, TouchId, TouchPhase, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::narrow_tests::sample_harness;
use crate::{Services, SlideApp};

fn touch(phase: TouchPhase, pos: Pos2) -> Event {
    Event::Touch { device_id: TouchDeviceId(7), id: TouchId(0), phase, pos, force: None }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// One frame with `events` together, as eframe delivers a touch event (kittest's queue would
/// spread them over a frame each).
fn frame(h: &mut Harness<'_, SlideApp>, events: Vec<Event>) {
    h.input_mut().events.extend(events);
    h.run_steps(1);
}

fn finger_down(h: &mut Harness<'_, SlideApp>, pos: Pos2) {
    frame(h, vec![Event::PointerMoved(pos), button(pos, true), touch(TouchPhase::Start, pos)]);
}

fn finger_move(h: &mut Harness<'_, SlideApp>, pos: Pos2) {
    frame(h, vec![Event::PointerMoved(pos), touch(TouchPhase::Move, pos)]);
}

fn finger_up(h: &mut Harness<'_, SlideApp>, pos: Pos2) {
    frame(h, vec![button(pos, false), Event::PointerGone, touch(TouchPhase::End, pos)]);
}

fn finger_drag(h: &mut Harness<'_, SlideApp>, from: Pos2, by: egui::Vec2) {
    finger_down(h, from);
    for i in 1..=4 {
        finger_move(h, from + by * (i as f32 / 4.0));
    }
    finger_up(h, from + by);
    h.run_steps(2);
}

fn tap(h: &mut Harness<'_, SlideApp>, at: Pos2) {
    finger_down(h, at);
    finger_up(h, at);
    // Past the double-click delay.
    h.run_steps(2);
}

/// The sample deck at desktop size, without snapping, so moves are exact.
fn canvas(zoom: Option<f32>) -> Harness<'static, SlideApp> {
    let mut h = sample_harness(vec2(1200.0, 800.0), "en");
    let app = h.state_mut();
    app.ui.zoom = zoom;
    app.session.prefs.snap_to_grid = false;
    app.session.prefs.smart_guides = false;
    h.run_steps(2);
    h
}

fn screen(h: &Harness<'_, SlideApp>, x: f32, y: f32) -> Pos2 {
    let app = h.state();
    app.slide_rect.unwrap().min + vec2(x, y) * app.canvas_scale
}

fn selected(h: &Harness<'_, SlideApp>) -> Vec<u32> {
    h.state().session.active().unwrap().selection.shapes.iter().map(|s| s.0).collect()
}

/// The sample title slide's gradient band: a plain rectangle across the bottom.
fn band(h: &Harness<'_, SlideApp>) -> (u32, f64) {
    let sh = &h.state().session.active().unwrap().current_slide().unwrap().shapes[2];
    (sh.id.0, sh.xfrm.unwrap().x)
}

fn band_x(h: &Harness<'_, SlideApp>) -> f64 {
    let id = band(h).0;
    h.state().session.active().unwrap().current_slide().unwrap().shapes.iter().find(|s| s.id.0 == id).unwrap().xfrm.unwrap().x
}

#[test]
fn a_finger_on_an_empty_spot_pans_a_zoomed_slide_and_selects_nothing() {
    let mut h = canvas(Some(2.0));
    assert_eq!(h.state().session.hit_test(Point::new(40.0, 40.0), 4.0), None, "the spot is empty");
    let before = h.state().slide_rect.unwrap().min;
    let at = screen(&h, 40.0, 40.0);
    finger_drag(&mut h, at, vec2(-200.0, -120.0));
    let moved = h.state().slide_rect.unwrap().min - before;
    assert!(moved.x < -150.0 && moved.y < -80.0, "the slide follows the finger: {moved:?}");
    assert!(selected(&h).is_empty());
    assert!(h.state().session.tool.marquee.is_none() && !h.state().session.tool.dragging());
    assert!(!h.state().session.active().unwrap().is_dirty());
}

#[test]
fn a_finger_on_an_unselected_shape_pans_instead_of_moving_it() {
    let mut h = canvas(Some(2.0));
    let title = h.state().session.active().unwrap().current_slide().unwrap().shapes[0].xfrm.unwrap();
    let at = screen(&h, title.x as f32 + 30.0, (title.y + title.h / 2.0) as f32);
    assert!(h.state().canvas_rect.unwrap().contains(at), "{at:?}");
    let before = h.state().slide_rect.unwrap().min;
    finger_drag(&mut h, at, vec2(-160.0, 0.0));
    let after = h.state().session.active().unwrap().current_slide().unwrap().shapes[0].xfrm.unwrap();
    assert_eq!((after.x, after.y), (title.x, title.y), "the title stays put");
    assert!(selected(&h).is_empty());
    assert!((h.state().slide_rect.unwrap().min - before).x < -100.0);
}

#[test]
fn a_tap_selects_a_shape_and_a_drag_on_it_then_moves_it() {
    let mut h = canvas(None);
    let (id, x0) = band(&h);
    let at = screen(&h, 480.0, 505.0);
    tap(&mut h, at);
    assert_eq!(selected(&h), vec![id]);
    assert!(h.state().session.active().unwrap().selection.text.is_none());
    let k = h.state().canvas_scale as f64;
    let at = screen(&h, 480.0, 505.0);
    finger_drag(&mut h, at, vec2(40.0, 0.0));
    let x = band_x(&h);
    assert!((x - (x0 + 40.0 / k)).abs() < 1.5, "{x} vs {x0} + {}", 40.0 / k);
    assert_eq!(selected(&h), vec![id]);
    // A tap on an empty spot clears the selection, as a click does.
    let at = screen(&h, 40.0, 40.0);
    tap(&mut h, at);
    assert!(selected(&h).is_empty());
}

#[test]
fn a_long_press_selects_the_shape_under_the_finger_and_opens_its_menu() {
    let mut h = canvas(None);
    let (id, x0) = band(&h);
    let at = screen(&h, 480.0, 505.0);
    finger_down(&mut h, at);
    // Held still past egui's long-press time (0.8 s; a step is 0.25 s).
    for _ in 0..4 {
        frame(&mut h, vec![touch(TouchPhase::Move, at)]);
    }
    finger_up(&mut h, at);
    h.run_steps(1);
    assert_eq!(selected(&h), vec![id]);
    assert!(h.query_by_label("Edit Text").is_some(), "the shape menu is open");
    assert_eq!(band_x(&h), x0);
    assert!(h.state().session.active().unwrap().selection.text.is_none(), "a long press doesn't start typing");
}

#[test]
fn two_fingers_pinch_zoom_without_moving_the_selection() {
    let mut h = canvas(None);
    let (id, x0) = band(&h);
    let at = screen(&h, 480.0, 505.0);
    tap(&mut h, at);
    let zoom0 = h.state().canvas_scale;
    let a = screen(&h, 480.0, 505.0);
    let b = a + vec2(100.0, 0.0);
    let second = |phase, pos| Event::Touch { device_id: TouchDeviceId(7), id: TouchId(1), phase, pos, force: None };
    finger_down(&mut h, a);
    frame(&mut h, vec![second(TouchPhase::Start, b)]);
    for i in 1..=4 {
        let d = 25.0 * i as f32;
        let f = a - vec2(d, 0.0);
        frame(&mut h, vec![Event::PointerMoved(f), touch(TouchPhase::Move, f), second(TouchPhase::Move, b + vec2(d, 0.0))]);
    }
    frame(&mut h, vec![second(TouchPhase::End, b + vec2(100.0, 0.0))]);
    finger_up(&mut h, a - vec2(100.0, 0.0));
    h.run_steps(1);
    assert!(h.state().ui.zoom.is_some_and(|z| z > zoom0 * 1.5), "{:?} from {zoom0}", h.state().ui.zoom);
    assert_eq!(band_x(&h), x0);
    assert_eq!(selected(&h), vec![id]);
}

#[test]
fn a_mouse_drag_on_an_empty_spot_still_draws_a_selection_box() {
    let mut h = canvas(Some(2.0));
    let before = h.state().slide_rect.unwrap().min;
    let from = screen(&h, 40.0, 40.0);
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    let mut boxed = false;
    for i in 1..=4 {
        h.hover_at(from + vec2(30.0 * i as f32, 30.0 * i as f32));
        h.run_steps(1);
        boxed |= h.state().session.tool.marquee.is_some();
    }
    h.drop_at(from + vec2(120.0, 120.0));
    h.run_steps(2);
    assert!(boxed, "the mouse draws a marquee");
    assert_eq!(h.state().slide_rect.unwrap().min, before, "and doesn't pan");
    assert!(!h.state().touch.active);
}

#[test]
fn the_hit_tolerance_grows_for_a_finger() {
    let mut h = canvas(None);
    let (id, _) = band(&h);
    let at = screen(&h, 480.0, 505.0);
    tap(&mut h, at);
    // 9 points inside the band's right-middle handle: a mouse misses it, a finger takes it.
    let k = h.state().canvas_scale;
    let near = screen(&h, 960.0, 505.0) - vec2(9.0, 0.0);
    let p = Point::new(960.0 - (9.0 / k) as f64, 505.0);
    assert!(matches!(h.state().session.hit_test(p, (4.0 / k) as f64), Some(Hit::Shape { .. })));
    let w0 = h.state().session.active().unwrap().current_slide().unwrap().shapes[2].xfrm.unwrap().w;
    finger_drag(&mut h, near, vec2(-40.0, 0.0));
    let st = h.state().session.active().unwrap();
    let sh = st.current_slide().unwrap().shapes.iter().find(|s| s.id.0 == id).unwrap();
    assert!(sh.xfrm.unwrap().w < w0 - 30.0, "the finger narrowed the band: {} from {w0}", sh.xfrm.unwrap().w);
}

/// A slide show on a phone: three blank slides, no Presenter View (a narrow window has none).
fn show_harness() -> Harness<'static, SlideApp> {
    let mut session = Session::with_new();
    for _ in 0..3 {
        session.execute("slide.new", &json!({})).unwrap();
    }
    let mut h = Harness::builder().with_size(vec2(390.0, 844.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SlideApp::new(session, Services::default()),
    );
    h.run_steps(3);
    assert!(h.state().ui.presenter_view);
    h.state_mut().run("show.start", json!({})).unwrap();
    h.run_steps(3);
    assert!(!h.state().show.as_ref().unwrap().presenter, "no Presenter View over the show on a phone");
    h
}

fn show_slide(h: &Harness<'_, SlideApp>) -> usize {
    h.state().show.as_ref().unwrap().state.slide
}

#[test]
fn a_swipe_in_the_slide_show_changes_slides() {
    let mut h = show_harness();
    assert_eq!(show_slide(&h), 0);
    finger_drag(&mut h, pos2(330.0, 420.0), vec2(-250.0, 10.0));
    assert_eq!(show_slide(&h), 1, "right to left: the next slide");
    finger_drag(&mut h, pos2(60.0, 420.0), vec2(250.0, -10.0));
    assert_eq!(show_slide(&h), 0, "left to right: the previous one");
    tap(&mut h, pos2(200.0, 420.0));
    assert_eq!(show_slide(&h), 1, "a tap still advances");
    // A mouse drag is no swipe.
    h.hover_at(pos2(330.0, 420.0));
    h.drag_at(pos2(330.0, 420.0));
    h.run_steps(1);
    h.hover_at(pos2(60.0, 420.0));
    h.run_steps(1);
    h.drop_at(pos2(60.0, 420.0));
    h.run_steps(2);
    assert_eq!(show_slide(&h), 1);
}

#[test]
fn a_long_press_in_the_slide_show_opens_its_menu() {
    let mut h = show_harness();
    let at = pos2(200.0, 420.0);
    finger_down(&mut h, at);
    for _ in 0..4 {
        frame(&mut h, vec![touch(TouchPhase::Move, at)]);
    }
    finger_up(&mut h, at);
    h.run_steps(1);
    assert_eq!(show_slide(&h), 0, "holding is not a click");
    h.get_by_label("Next Slide").click();
    h.run_steps(2);
    assert_eq!(show_slide(&h), 1, "the menu's Next Slide, and only that");
    assert!(h.query_by_label("Next Slide").is_none(), "the menu closed");
    // The mouse's right-click still goes back a slide without a menu.
    h.hover_at(at);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(show_slide(&h), 0);
    assert!(h.query_by_label("Next Slide").is_none());
}
