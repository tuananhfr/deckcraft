//! Brief notices over the window's bottom edge: shown once, gone after a few seconds or when
//! closed. Used to say a presentation's fonts aren't installed when it is first opened.

use egui::{Align2, CornerRadius, Margin, Stroke, vec2};

use crate::SlideApp;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// How long a toast stays, in milliseconds.
const SHOW_MS: f64 = 8000.0;

pub struct Toast {
    pub text: String,
    pub at: f64,
}

/// The notice for fonts a presentation uses that aren't installed, or `None` when all are.
pub fn missing_fonts_message(fonts: &[String]) -> Option<String> {
    let names = match fonts {
        [] => return None,
        [a] => a.clone(),
        [a, b] => crate::i18n::fmt(crate::tl!("{a} and {b}"), &[("a", a), ("b", b)]),
        [a, b, c] => crate::i18n::fmt(crate::tl!("{a}, {b} and {c}"), &[("a", a), ("b", b), ("c", c)]),
        [a, b, c, rest @ ..] => {
            crate::i18n::fmt(crate::tl!("{a}, {b}, {c} and {n} more"), &[("a", a), ("b", b), ("c", c), ("n", &rest.len().to_string())])
        }
    };
    let template = if fonts.len() == 1 {
        crate::tl!("{names} isn't installed on this computer. Text that uses it is shown in a similar font.")
    } else {
        crate::tl!("{names} aren't installed on this computer. Text that uses them is shown in a similar font.")
    };
    Some(crate::i18n::fmt(template, &[("names", &names)]))
}

/// Toast the active presentation's missing fonts, once per opened document.
pub fn note_missing_fonts(app: &mut SlideApp) {
    let Some(d) = app.session.active() else { return };
    if !app.fonts_noted.insert(d.uid) {
        return;
    }
    if let Some(text) = missing_fonts_message(&d.missing_fonts) {
        app.toast = Some(Toast { text, at: crate::now_ms() });
    }
}

/// The card a notice over the window's bottom edge is drawn in.
pub(crate) fn frame(t: &Tokens) -> egui::Frame {
    egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(14, 10))
        .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 8, spread: 0, color: t.shadow })
}

pub fn show(app: &mut SlideApp, ctx: &egui::Context) {
    let Some(toast) = &app.toast else { return };
    if crate::now_ms() - toast.at > SHOW_MS {
        app.toast = None;
        return;
    }
    let t = Tokens::get(ctx);
    let text = toast.text.clone();
    let mut close = false;
    egui::Area::new(egui::Id::new("toast")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -44.0)).order(egui::Order::Foreground).show(ctx, |ui| {
        frame(&t).show(ui, |ui| {
            // A phone's window is narrower than the notice's usual width; keep a 16-point gutter.
            ui.set_max_width((ctx.content_rect().width() - 2.0 * 16.0 - 2.0 * 14.0).clamp(120.0, 520.0));
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(egui::RichText::new(crate::tl!(&text)).font(theme::font(13.0)).color(t.text)).wrap());
                let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close"));
                if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
                }
                icons::paint(ui.painter(), r.shrink(3.0), Icon::Close, t.text_dim, false);
                if resp.on_hover_text(crate::tl!("Close")).clicked() {
                    close = true;
                }
            });
        });
    });
    if close {
        app.toast = None;
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn the_message_names_the_fonts() {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let f = |v: &[&str]| missing_fonts_message(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(f(&[]), None);
        assert_eq!(f(&["Aptos"]).unwrap(), "Aptos isn't installed on this computer. Text that uses it is shown in a similar font.");
        assert!(f(&["Aptos", "Sora"]).unwrap().starts_with("Aptos and Sora aren't installed"));
        assert!(f(&["A", "B", "C"]).unwrap().starts_with("A, B and C aren't"));
        assert!(f(&["A", "B", "C", "D", "E"]).unwrap().starts_with("A, B, C and 2 more aren't"));
    }

    #[test]
    fn missing_font_toast_is_dismissible_once_per_opened_presentation() {
        use egui_kittest::{Harness, kittest::Queryable};
        use serde_json::json;

        let mut app = SlideApp::new(deckcraft_engine::Session::with_new(), crate::Services::default());
        app.ui.language = "vi".into();
        let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        h.run_steps(2);
        let bytes = deckcraft_engine::cmd::file::save_bytes(&deckcraft_model::defaults::new_presentation(None), "pptx").unwrap();
        let params = json!({"name": "fonts.pptx", "data": deckcraft_engine::cmd::base64_encode(&bytes)});
        h.state_mut().run("file.openBytes", params.clone()).unwrap();
        let fonts: Vec<String> = ["Missing Alpha", "Missing Beta", "Missing Gamma", "Missing Delta", "Missing Epsilon"].map(String::from).into();
        h.state_mut().session.active_mut().unwrap().missing_fonts = fonts.clone();
        let first = h.state().session.active_index().unwrap();
        h.run_steps(2);
        let text = h.state().toast.as_ref().unwrap().text.clone();
        assert!(text.starts_with("Missing Alpha, Missing Beta, Missing Gamma và 2 khác"), "{text}");
        assert!(!text.contains("Missing Delta"));
        assert!(!text.contains("Missing Epsilon"));
        h.get_by_label("Close").click();
        h.run_steps(2);
        assert!(h.state().toast.is_none());
        h.state_mut().session.set_active(0);
        h.run_steps(2);
        h.state_mut().session.set_active(first);
        h.run_steps(2);
        assert!(h.state().toast.is_none());
        h.state_mut().run("file.openBytes", params).unwrap();
        h.state_mut().session.active_mut().unwrap().missing_fonts = fonts;
        h.run_steps(2);
        assert!(h.state().toast.is_some(), "a separate opening has its own notice");
    }
}
