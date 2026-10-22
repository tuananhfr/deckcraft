//! Web Audio output: the mixer's samples (resampled to the context's rate) are queued as short
//! `AudioBufferSourceNode`s scheduled back to back on the context clock, pumped from the host frame.

use std::cell::RefCell;
use std::rc::Rc;

use deckcraft_media::AudioOut;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{AudioBufferSourceNode, AudioContext, AudioContextState, Event};

type Fill = Box<dyn FnMut(&mut [f32], usize, u32) + Send>;

/// Touch input only grants user activation on `pointerup`, a mouse on `pointerdown`.
const GESTURES: [&str; 3] = ["pointerdown", "pointerup", "keydown"];
const FRAMES: u32 = 2048;
/// Seconds queued ahead of the context clock: rides out a janky frame without making Pause lag much.
const HORIZON: f64 = 0.15;
const MAX_QUEUED: usize = 8;
/// A fresh or starved queue starts this far ahead so its first buffer isn't clipped.
const LEAD: f64 = 0.02;
/// Firefox and Safari start a resumed context asynchronously; only a resume still pending after
/// this long counts as blocked.
const RESUME_GRACE_MS: f64 = 1500.0;
const UNAVAILABLE: &str = "browser audio unavailable";
const SUSPENDED: &str = "browser audio suspended";

#[derive(Default)]
struct Device {
    context: Option<AudioContext>,
    /// When the latest `resume()` was requested, until its promise settles.
    resuming_since: Option<f64>,
}

impl Device {
    /// The context when it is running (`Ok(None)` while a gesture's resume is still starting it).
    fn running(&self) -> Result<Option<&AudioContext>, String> {
        let context = self.context.as_ref().ok_or(UNAVAILABLE)?;
        if context.state() == AudioContextState::Running {
            Ok(Some(context))
        } else if self.resuming_since.is_some_and(|t| js_sys::Date::now() - t < RESUME_GRACE_MS) {
            Ok(None)
        } else {
            Err(SUSPENDED.into())
        }
    }
}

/// Create or resume the context inside a trusted gesture: eframe only runs the Play click on a
/// later animation frame, after the browser's user activation is gone.
fn unlock(device: &Rc<RefCell<Device>>, event: &Event) {
    if !event.is_trusted() {
        return;
    }
    let Ok(mut d) = device.try_borrow_mut() else { return };
    if d.context.as_ref().is_none_or(|c| c.state() == AudioContextState::Closed) {
        d.context = AudioContext::new().ok();
    }
    let Some(context) = d.context.clone() else { return };
    if context.state() == AudioContextState::Running {
        d.resuming_since = None;
        return;
    }
    let Ok(promise) = context.resume() else { return };
    let token = js_sys::Date::now();
    d.resuming_since = Some(token);
    drop(d);
    let settled = device.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
        if let Ok(mut d) = settled.try_borrow_mut()
            && d.resuming_since == Some(token)
        {
            d.resuming_since = None;
        }
    });
}

pub struct WebAudio {
    device: Rc<RefCell<Device>>,
    gesture: Closure<dyn FnMut(Event)>,
    fill: Option<Fill>,
    /// Scheduled sources and when each ends, on the context clock.
    queued: Vec<(f64, AudioBufferSourceNode)>,
    next: f64,
}

impl WebAudio {
    pub fn new() -> Self {
        let device = Rc::new(RefCell::new(Device::default()));
        let shared = device.clone();
        let gesture = Closure::new(move |event: Event| unlock(&shared, &event));
        if let Some(window) = web_sys::window() {
            for name in GESTURES {
                let _ = window.add_event_listener_with_callback_and_bool(name, gesture.as_ref().unchecked_ref(), true);
            }
        }
        Self { device, gesture, fill: None, queued: Vec::new(), next: 0.0 }
    }

    fn pump(&mut self) -> Result<(), String> {
        let device = self.device.try_borrow().map_err(|_| UNAVAILABLE.to_string())?;
        let Some(context) = device.running()? else { return Ok(()) };
        let now = context.current_time();
        self.queued.retain(|(end, _)| *end > now);
        if self.next < now {
            self.next = now + LEAD;
        }
        let Some(fill) = self.fill.as_mut() else { return Ok(()) };
        let rate = context.sample_rate();
        if !rate.is_finite() || !(8_000.0..=192_000.0).contains(&rate) {
            return Err("unsupported browser sample rate".into());
        }
        // Bounded even after a background-tab stall, so a catch-up never allocates without limit.
        while self.next < now + HORIZON && self.queued.len() < MAX_QUEUED {
            let buffer = context.create_buffer(2, FRAMES, rate).map_err(js_error)?;
            let source = context.create_buffer_source().map_err(js_error)?;
            source.set_buffer(Some(&buffer));
            source.connect_with_audio_node(&context.destination()).map_err(js_error)?;
            let mut interleaved = vec![0.0; FRAMES as usize * 2];
            fill(&mut interleaved, 2, rate as u32);
            for channel in 0..2 {
                let plane: Vec<f32> = interleaved.iter().skip(channel).step_by(2).copied().collect();
                buffer.copy_to_channel(&plane, channel as i32).map_err(js_error)?;
            }
            source.start_with_when(self.next).map_err(js_error)?;
            self.next += f64::from(FRAMES) / f64::from(rate);
            self.queued.push((self.next, source));
        }
        Ok(())
    }

    fn stop_sources(&mut self) {
        for (_, source) in self.queued.drain(..) {
            let _ = source.unchecked_ref::<web_sys::AudioScheduledSourceNode>().stop();
            let _ = source.disconnect();
        }
        self.next = 0.0;
    }
}

fn js_error(_: wasm_bindgen::JsValue) -> String {
    UNAVAILABLE.into()
}

impl AudioOut for WebAudio {
    fn start(&mut self, fill: Fill) -> Result<(), String> {
        self.stop_sources();
        self.device.try_borrow().map_err(|_| UNAVAILABLE.to_string())?.running()?;
        self.fill = Some(fill);
        Ok(())
    }

    fn stop(&mut self) {
        self.stop_sources();
        self.fill = None;
    }

    fn requires_output(&self) -> bool {
        true
    }

    fn check(&mut self) -> Result<(), String> {
        self.pump()
    }
}

impl Drop for WebAudio {
    fn drop(&mut self) {
        self.stop();
        if let Some(window) = web_sys::window() {
            for name in GESTURES {
                let _ = window.remove_event_listener_with_callback_and_bool(name, self.gesture.as_ref().unchecked_ref(), true);
            }
        }
        if let Ok(mut device) = self.device.try_borrow_mut()
            && let Some(context) = device.context.take()
        {
            let _ = context.close();
        }
    }
}
