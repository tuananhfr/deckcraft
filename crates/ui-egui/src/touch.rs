//! Touch input: whether the user is on a touch screen right now, what a one-finger press on the
//! slide canvas turns into, and horizontal swipes in the slide show.

use egui::{Event, PointerButton, Pos2};

/// Hit tolerance for a finger on the canvas, in screen points (a mouse gets 4).
pub const FINGER_TOLERANCE: f32 = 8.0;

/// Horizontal travel, in points, that makes a release a swipe.
const SWIPE_MIN: f32 = 60.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TouchInput {
    /// The last pointer input came from a finger.
    pub active: bool,
}

impl TouchInput {
    /// Feeds one frame's events. eframe sends `Event::Touch` next to the pointer events a finger
    /// makes, and a press can arrive a frame before its touch, so only a mouse move or wheel
    /// without touches switches back to mouse.
    pub fn observe(&mut self, events: &[Event]) {
        if events.iter().any(|e| matches!(e, Event::Touch { .. })) {
            self.active = true;
        } else if events.iter().any(|e| matches!(e, Event::PointerMoved(_) | Event::MouseWheel { .. })) {
            self.active = false;
        }
    }
}

/// Who acts on a finger's press on the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The press is on the selection or its handles (or text being edited): moving the finger
    /// moves, sizes or selects there, as a mouse drag would.
    Engine,
    /// Anywhere else: moving the finger pans, so a shape is never moved by accident.
    Pan,
}

/// A finger down on the canvas. Nothing reaches the engine until the finger moves or lifts, so a
/// tap and a pan can be told apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasPress {
    pub at: Pos2,
    pub owner: Owner,
    /// The engine has had the press (the finger moved on the selection).
    pub started: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasTouch {
    pub press: Option<CanvasPress>,
}

impl CanvasTouch {
    pub fn panning(&self) -> bool {
        self.press.is_some_and(|p| p.owner == Owner::Pan)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swipe {
    /// The finger moved right to left: the next slide.
    Next,
    Previous,
}

/// Tracks one finger from press to release to spot a horizontal swipe.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwipeTracker {
    origin: Option<Pos2>,
}

impl SwipeTracker {
    /// Feeds one frame's events; returns the swipe a release ended. The release position comes
    /// from the event itself: eframe follows a lifted finger with `PointerGone` in the same frame.
    pub fn feed(&mut self, events: &[Event]) -> Option<Swipe> {
        let mut swipe = None;
        for e in events {
            match e {
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } => {
                    self.origin.get_or_insert(*pos);
                }
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, .. } => {
                    if let Some(o) = self.origin.take() {
                        let d = *pos - o;
                        if d.x.abs() >= SWIPE_MIN && d.x.abs() > d.y.abs() * 1.5 {
                            swipe = Some(if d.x < 0.0 { Swipe::Next } else { Swipe::Previous });
                        }
                    }
                }
                _ => {}
            }
        }
        swipe
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Modifiers, TouchDeviceId, TouchId, TouchPhase, pos2};

    fn touch(phase: TouchPhase) -> Event {
        Event::Touch { device_id: TouchDeviceId(1), id: TouchId(0), phase, pos: pos2(10.0, 10.0), force: None }
    }

    fn button(x: f32, y: f32, pressed: bool) -> Event {
        Event::PointerButton { pos: pos2(x, y), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
    }

    #[test]
    fn a_finger_is_touch_until_the_mouse_moves() {
        let mut t = TouchInput::default();
        t.observe(&[Event::PointerMoved(pos2(1.0, 1.0)), touch(TouchPhase::Start)]);
        assert!(t.active);
        // The press eframe sends from `pointerdown` comes alone, and the release is followed by
        // `PointerGone`: neither means a mouse.
        t.observe(&[button(1.0, 1.0, true)]);
        t.observe(&[Event::PointerGone]);
        assert!(t.active);
        t.observe(&[Event::PointerMoved(pos2(5.0, 5.0))]);
        assert!(!t.active);
    }

    #[test]
    fn a_horizontal_release_far_enough_is_a_swipe() {
        let mut s = SwipeTracker::default();
        assert_eq!(s.feed(&[button(300.0, 400.0, true)]), None);
        assert_eq!(s.feed(&[Event::PointerMoved(pos2(200.0, 410.0))]), None);
        assert_eq!(s.feed(&[button(150.0, 410.0, false), Event::PointerGone]), Some(Swipe::Next));
        // A second release (eframe sends one from `pointerup` and one from `touchend`) is nothing.
        assert_eq!(s.feed(&[button(150.0, 410.0, false)]), None);
        assert_eq!(s.feed(&[button(100.0, 400.0, true), button(100.0, 400.0, true)]), None);
        assert_eq!(s.feed(&[button(200.0, 380.0, false)]), Some(Swipe::Previous));
        // Too short, or mostly vertical: not a swipe.
        s.feed(&[button(100.0, 400.0, true)]);
        assert_eq!(s.feed(&[button(140.0, 400.0, false)]), None);
        s.feed(&[button(100.0, 400.0, true)]);
        assert_eq!(s.feed(&[button(170.0, 520.0, false)]), None);
    }
}
