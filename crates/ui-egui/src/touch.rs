//! Touch input: whether the user is on a touch screen right now, what a one-finger press on the
//! slide canvas turns into, the fling that keeps a canvas pan going after the finger lifts, and
//! horizontal swipes in the slide show.

use egui::{Event, PointerButton, Pos2, Vec2};

/// Hit tolerance for a finger on the canvas, in screen points (a mouse gets 4).
pub const FINGER_TOLERANCE: f32 = 8.0;

/// Horizontal travel, in points, that makes a release a swipe.
const SWIPE_MIN: f32 = 60.0;

/// Fling speeds, in points per second (GridCraft's sheet fling, so the editors feel alike).
const FLING_MIN_SPEED: f32 = 250.0;
const FLING_STOP_SPEED: f32 = 20.0;
/// Fraction of the fling speed lost per second.
const FLING_DECAY: f32 = 4.0;

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
    /// A released pan still moving, and the canvas scroll it moves (each presentation has its own).
    pub fling: Option<(egui::Id, Fling)>,
}

impl CanvasTouch {
    pub fn panning(&self) -> bool {
        self.press.is_some_and(|p| p.owner == Owner::Pan)
    }
}

/// A one-finger pan carried on after the finger lifts, slowing down to a stop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fling {
    /// The finger's velocity, screen points per second.
    velocity: Vec2,
}

impl Fling {
    /// The fling a pan released at `velocity` starts; `None` for a slow release.
    pub fn from_release(velocity: Vec2) -> Option<Fling> {
        (velocity.is_finite() && velocity.length() >= FLING_MIN_SPEED).then_some(Fling { velocity })
    }

    /// Moves the scroll `offset` (kept within `0..=max`) on for `dt` seconds the way the finger
    /// went, and slows down. An axis that reaches its edge stops there; `false` once it all stopped.
    pub fn scroll(&mut self, offset: &mut Vec2, max: Vec2, dt: f32) -> bool {
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.1) } else { 0.0 };
        let max = max.max(Vec2::ZERO);
        let want = *offset - self.velocity * dt;
        if !(0.0..=max.x).contains(&want.x) {
            self.velocity.x = 0.0;
        }
        if !(0.0..=max.y).contains(&want.y) {
            self.velocity.y = 0.0;
        }
        *offset = want.clamp(Vec2::ZERO, max);
        self.velocity *= (1.0 - FLING_DECAY * dt).max(0.0);
        self.velocity.length() >= FLING_STOP_SPEED
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
    use egui::{Modifiers, TouchDeviceId, TouchId, TouchPhase, pos2, vec2};

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

    const FRAME: f32 = 1.0 / 60.0;

    /// Flings `f` from `offset` until it stops (at most 10 s); the offsets it went through.
    fn run(mut f: Fling, mut offset: Vec2, max: Vec2) -> Vec<Vec2> {
        let mut path = vec![];
        for _ in 0..600 {
            let moving = f.scroll(&mut offset, max, FRAME);
            path.push(offset);
            if !moving {
                break;
            }
        }
        path
    }

    #[test]
    fn a_quick_release_keeps_scrolling_and_slows_to_a_stop() {
        let max = vec2(5000.0, 5000.0);
        let start = vec2(2500.0, 2500.0);
        // The finger went left and up: the content follows it, so the scroll grows.
        let path = run(Fling::from_release(vec2(-1200.0, -600.0)).unwrap(), start, max);
        assert!(path.len() > 10 && path.len() < 600, "it stops by itself after {} frames", path.len());
        let steps: Vec<Vec2> = path.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(steps.iter().all(|d| d.x >= 0.0 && d.y >= 0.0), "it keeps the finger's direction");
        assert!(steps.windows(2).all(|w| w[1].length() <= w[0].length() + 1e-3), "and only slows down");
        let gone = *path.last().unwrap() - start;
        assert!(gone.x > 200.0 && gone.x < 400.0 && (gone.y - gone.x / 2.0).abs() < 1.0, "{gone:?}");
    }

    #[test]
    fn a_slow_or_hostile_release_doesnt_fling() {
        assert_eq!(Fling::from_release(vec2(150.0, 100.0)), None);
        assert_eq!(Fling::from_release(Vec2::ZERO), None);
        assert_eq!(Fling::from_release(vec2(f32::NAN, 900.0)), None);
        assert_eq!(Fling::from_release(vec2(f32::INFINITY, 0.0)), None);
        let mut f = Fling::from_release(vec2(900.0, 0.0)).unwrap();
        let mut offset = vec2(100.0, 0.0);
        f.scroll(&mut offset, vec2(500.0, 0.0), f32::NAN);
        assert_eq!(offset, vec2(100.0, 0.0), "a broken frame time doesn't move it");
    }

    #[test]
    fn a_fling_stops_at_the_content_edge() {
        // Toward the right edge, 40 points away: it stops there, not past it.
        let path = run(Fling::from_release(vec2(-3000.0, 0.0)).unwrap(), vec2(260.0, 50.0), vec2(300.0, 100.0));
        assert_eq!(*path.last().unwrap(), vec2(300.0, 50.0));
        assert!(path.len() < 5, "it stops when it gets there ({} frames)", path.len());
        // Along one edge it still slides along the other axis.
        let mut f = Fling::from_release(vec2(2000.0, -2000.0)).unwrap();
        let mut offset = vec2(0.0, 0.0);
        assert!(f.scroll(&mut offset, vec2(300.0, 1000.0), FRAME));
        assert!(offset.x == 0.0 && offset.y > 20.0, "{offset:?}");
        // With nothing to scroll, nothing moves.
        let path = run(Fling::from_release(vec2(800.0, 800.0)).unwrap(), Vec2::ZERO, Vec2::ZERO);
        assert_eq!(path, vec![Vec2::ZERO]);
    }
}
