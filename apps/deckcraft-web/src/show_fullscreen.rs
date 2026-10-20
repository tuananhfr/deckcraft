//! Browser fullscreen follows the show's lifetime, including late browser responses.

#[derive(Clone, Copy, Debug, Default)]
struct State {
    showing: bool,
    fullscreen: bool,
    pending: bool,
}

#[derive(Clone, Copy)]
enum Event {
    Show(bool),
    Fullscreen(bool),
    Rejected,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    None,
    Request,
    Exit,
    EndShow,
}

impl State {
    fn update(&mut self, event: Event) -> Action {
        match event {
            Event::Show(showing) => {
                if self.showing == showing {
                    return Action::None;
                }
                self.showing = showing;
                if showing && !self.fullscreen {
                    self.pending = true;
                    Action::Request
                } else if !showing && self.fullscreen {
                    Action::Exit
                } else {
                    Action::None
                }
            }
            Event::Fullscreen(fullscreen) => {
                self.fullscreen = fullscreen;
                let pending = self.pending;
                self.pending = false;
                if !fullscreen && self.showing {
                    self.showing = false;
                    Action::EndShow
                } else if fullscreen && !self.showing && pending {
                    Action::Exit
                } else {
                    Action::None
                }
            }
            Event::Rejected => {
                self.pending = false;
                Action::None
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::FullscreenBridge;

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::{cell::Cell, rc::Rc};

    use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};

    use super::{Action, Event, State};

    #[derive(Clone)]
    pub struct FullscreenBridge {
        state: Rc<Cell<State>>,
        end_show: Rc<Cell<bool>>,
        document: web_sys::Document,
        ctx: egui::Context,
    }

    impl FullscreenBridge {
        pub fn install(ctx: egui::Context) -> Result<Self, String> {
            let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
            let bridge = Self {
                state: Rc::new(Cell::new(State { fullscreen: document.fullscreen_element().is_some(), ..Default::default() })),
                end_show: Rc::default(),
                document,
                ctx,
            };
            let events = bridge.clone();
            let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                events.dispatch(Event::Fullscreen(events.document.fullscreen_element().is_some()));
            });
            bridge.document.add_event_listener_with_callback("fullscreenchange", listener.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
            // Like the unload guard, the listener belongs to the page's lifetime.
            listener.forget();
            Ok(bridge)
        }

        pub fn set_showing(&self, showing: bool) {
            self.dispatch(Event::Show(showing));
        }

        pub fn take_end_show(&self) -> bool {
            self.end_show.replace(false)
        }

        fn dispatch(&self, event: Event) {
            let mut state = self.state.get();
            let action = state.update(event);
            self.state.set(state);
            match action {
                Action::None => {}
                Action::Request => {
                    // web-sys types these methods as returning void, hiding rejected promises.
                    let promise = self.document.document_element().and_then(|element| call(&element, "requestFullscreen"));
                    if let Some(promise) = promise {
                        let bridge = self.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            if wasm_bindgen_futures::JsFuture::from(promise).await.is_err() {
                                bridge.dispatch(Event::Rejected);
                            }
                        });
                    } else {
                        self.dispatch(Event::Rejected);
                    }
                }
                Action::Exit => {
                    if self.document.fullscreen_element().is_some()
                        && let Some(promise) = call(&self.document, "exitFullscreen")
                    {
                        wasm_bindgen_futures::spawn_local(async move {
                            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
                        });
                    }
                }
                Action::EndShow => {
                    self.end_show.set(true);
                    self.ctx.request_repaint();
                }
            }
        }
    }

    fn call(target: &JsValue, method: &str) -> Option<js_sys::Promise> {
        let function = js_sys::Reflect::get(target, &JsValue::from_str(method)).ok()?.dyn_into::<js_sys::Function>().ok()?;
        function.call0(target).ok()?.dyn_into().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_requests_fullscreen_once_and_rejection_keeps_the_show() {
        let mut state = State::default();
        assert_eq!(state.update(Event::Show(true)), Action::Request);
        assert_eq!(state.update(Event::Show(true)), Action::None);
        assert_eq!(state.update(Event::Rejected), Action::None);
        assert!(state.showing);
        assert_eq!(state.update(Event::Show(false)), Action::None);
        assert_eq!(state.update(Event::Show(true)), Action::Request);
    }

    #[test]
    fn every_show_exit_releases_fullscreen() {
        let mut state = State::default();
        assert_eq!(state.update(Event::Show(true)), Action::Request);
        assert_eq!(state.update(Event::Fullscreen(true)), Action::None);
        assert_eq!(state.update(Event::Show(false)), Action::Exit);
        assert_eq!(state.update(Event::Fullscreen(false)), Action::None);
        assert!(!state.showing);
    }

    #[test]
    fn browser_escape_or_browser_ui_exit_ends_the_show() {
        let mut state = State::default();
        state.update(Event::Show(true));
        state.update(Event::Fullscreen(true));
        assert_eq!(state.update(Event::Fullscreen(false)), Action::EndShow);
        assert!(!state.showing);
        assert_eq!(state.update(Event::Show(false)), Action::None);
    }

    #[test]
    fn fullscreen_arriving_after_the_show_ended_is_released() {
        let mut state = State::default();
        state.update(Event::Show(true));
        assert_eq!(state.update(Event::Show(false)), Action::None);
        assert_eq!(state.update(Event::Fullscreen(true)), Action::Exit);
        assert_eq!(state.update(Event::Fullscreen(false)), Action::None);
        assert!(!state.showing);
    }

    #[test]
    fn windowed_and_preexisting_browser_fullscreen_are_supported() {
        let mut state = State::default();
        assert_eq!(state.update(Event::Show(false)), Action::None);
        assert_eq!(state.update(Event::Fullscreen(true)), Action::None);
        assert_eq!(state.update(Event::Show(true)), Action::None);
        assert_eq!(state.update(Event::Show(false)), Action::Exit);
    }
}
