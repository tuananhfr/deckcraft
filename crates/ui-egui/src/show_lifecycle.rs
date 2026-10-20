//! Shared entry and exit paths for slide shows and explicit Presenter View requests.

use serde_json::Value;

use crate::{SlideApp, media, show::Show};

fn auto_presenter(enabled: bool, reading: bool, narrow: bool, web: bool) -> bool {
    enabled && !reading && !narrow && !web
}

impl SlideApp {
    pub fn start_show(&mut self, from: usize, reading: bool) {
        self.media.stop_owned(media::Owner::Editor);
        let presenter = auto_presenter(self.ui.presenter_view, reading, self.layout.is_narrow(), cfg!(target_arch = "wasm32"));
        if let Some(d) = self.session.active() {
            self.show = Some(Show::new(&d.doc, from, reading, presenter));
            if let Some(fullscreen) = self.services.show_fullscreen.as_mut() {
                fullscreen(!reading);
            }
        }
    }

    pub fn end_show(&mut self) {
        self.show = None;
        self.media.stop_owned(media::Owner::Show);
        if let Some(fullscreen) = self.services.show_fullscreen.as_mut() {
            fullscreen(false);
        }
    }

    pub(crate) fn presenter_view(&mut self, params: &Value) {
        self.presenter_view_for(params, cfg!(target_arch = "wasm32"));
    }

    fn presenter_view_for(&mut self, params: &Value, web: bool) {
        if web {
            let on = self.flag(params, self.show.as_ref().is_some_and(|s| s.presenter));
            if on && self.show.is_none() {
                let from = self.session.active().map(|d| d.selection.slide).unwrap_or(0);
                self.start_show(from, false);
            }
            if let Some(show) = self.show.as_mut() {
                show.presenter = on;
            }
        } else {
            self.ui.presenter_view = self.flag(params, self.ui.presenter_view);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::{cell::RefCell, rc::Rc};

    use super::*;
    use crate::Services;

    #[test]
    fn only_desktop_wide_shows_automatically_open_presenter_view() {
        assert!(auto_presenter(true, false, false, false));
        assert!(!auto_presenter(true, false, false, true));
        assert!(!auto_presenter(true, false, true, false));
        assert!(!auto_presenter(true, true, false, false));
        assert!(!auto_presenter(false, false, false, false));
    }

    #[test]
    fn show_commands_notify_the_host_synchronously_and_reading_stays_windowed() {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let output = requests.clone();
        let services = Services { show_fullscreen: Some(Box::new(move |on| output.borrow_mut().push(on))), ..Default::default() };
        let mut app = SlideApp::new(deckcraft_engine::Session::with_new(), services);
        app.run("show.start", serde_json::json!({})).unwrap();
        assert_eq!(*requests.borrow(), vec![true]);
        app.run("show.end", serde_json::json!({})).unwrap();
        assert!(app.show.is_none());
        assert_eq!(*requests.borrow(), vec![true, false]);
        app.start_show(0, true);
        assert_eq!(*requests.borrow(), vec![true, false, false]);
    }

    #[test]
    fn web_presenter_view_is_available_only_when_explicitly_requested() {
        let mut app = SlideApp::new(deckcraft_engine::Session::with_new(), Services::default());
        app.presenter_view_for(&serde_json::json!({"on": false}), true);
        assert!(app.show.is_none());
        app.presenter_view_for(&serde_json::json!({}), true);
        assert!(app.show.as_ref().unwrap().presenter);
        app.presenter_view_for(&serde_json::json!({}), true);
        assert!(!app.show.as_ref().unwrap().presenter);
    }

    #[test]
    fn desktop_presenter_command_keeps_its_existing_preference_toggle() {
        let mut app = SlideApp::new(deckcraft_engine::Session::with_new(), Services::default());
        assert!(app.ui.presenter_view);
        app.presenter_view_for(&serde_json::json!({}), false);
        assert!(!app.ui.presenter_view);
        assert!(app.show.is_none());
        app.presenter_view_for(&serde_json::json!({"on": true}), false);
        assert!(app.ui.presenter_view);
        assert!(app.show.is_none());
    }
}
