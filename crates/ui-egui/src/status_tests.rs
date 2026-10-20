//! The status bar of windows wider than a phone: lower-priority items give way instead of
//! overlapping when translated labels or a message need the room.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use egui::{Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::SlideApp;
use crate::i18n::{self, Lang};
use crate::narrow_tests::{assert_apart, rects_in, sample_harness};

const HEIGHT: f32 = 768.0;
const BUTTONS: [&str; 9] =
    ["Fit slide to current window", "Zoom In", "Zoom Out", "Slide Show", "Reading View", "Slide Sorter", "Normal", "Comments", "Notes"];

/// The sample deck with eleven slides (a two-digit counter), at `width`.
fn bar_harness(width: f32, code: &str, message: Option<&str>) -> Harness<'static, SlideApp> {
    let mut h = sample_harness(vec2(width, HEIGHT), code);
    for _ in 0..3 {
        h.state_mut().run("slide.new", json!({})).unwrap();
    }
    if let Some(m) = message {
        h.state_mut().set_status(m);
    }
    h.run_steps(2);
    h
}

fn band(width: f32) -> Rect {
    Rect::from_min_max(pos2(0.0, HEIGHT - 28.0), pos2(width, HEIGHT))
}

/// Everything in the status bar with its rect: buttons, the zoom percentage and slider, and the
/// counter, language and message text.
fn status_items(h: &Harness<'_, SlideApp>, lang: Lang, width: f32) -> Vec<(String, Rect)> {
    let bottom = band(width);
    let mut items = rects_in(h, lang, bottom, &BUTTONS);
    let counter = i18n::fmt(i18n::tr(lang, "Slide {n} of {total}"), &[("n", "4"), ("total", "11")]);
    let counter = h.query_by_label(&counter).unwrap_or_else(|| panic!("the slide counter shows at {width} ({})", lang.code()));
    items.push(("counter".into(), counter.rect()));
    items.extend(rects_in(h, lang, bottom, &["English (United States)"]));
    let zoom = h.query_all_by_label_contains("%").map(|n| n.rect()).find(|r| bottom.intersects(*r));
    items.push(("zoom".into(), zoom.expect("the zoom percentage stays")));
    items.extend(h.query_all_by_role(egui::accesskit::Role::Slider).map(|n| ("slider".to_string(), n.rect())).filter(|(_, r)| bottom.intersects(*r)));
    items
}

#[test]
fn wide_status_bars_never_overlap() {
    let message = "The file is too large or abnormally compressed, so it can't be opened.";
    for code in ["vi", "en"] {
        let lang = Lang::from_code(code).unwrap();
        for width in [640.0, 768.0, 1024.0, 1366.0] {
            for msg in [None, Some(message)] {
                let h = bar_harness(width, code, msg);
                let mut items = status_items(&h, lang, width);
                for must in ["Fit slide to current window", "Comments", "Notes"] {
                    assert!(items.iter().any(|(l, _)| l == i18n::tr(lang, must)), "{must} stays at {width} ({code})");
                }
                if msg.is_some() {
                    let start: String = i18n::tr(lang, message).chars().take(10).collect();
                    let shown = h.query_all_by_label_contains(&start).map(|n| n.rect()).find(|r| band(width).intersects(*r));
                    items.push(("message".into(), shown.unwrap_or_else(|| panic!("the message shows at {width} ({code})"))));
                }
                assert_apart(&format!("status bar ({code}, message: {})", msg.is_some()), width, &items);
            }
        }
    }
}

#[test]
fn a_desktop_window_keeps_every_status_item() {
    for code in ["vi", "en"] {
        let lang = Lang::from_code(code).unwrap();
        let h = bar_harness(1366.0, code, None);
        let items = status_items(&h, lang, 1366.0);
        // The nine buttons, the counter, the language, the zoom percentage and the slider.
        assert_eq!(items.len(), 13, "{items:#?}");
        let notes = items.iter().find(|(l, _)| l == i18n::tr(lang, "Notes")).unwrap().1;
        assert!(notes.width() > 40.0, "the Notes switch keeps its word: {notes:?}");
    }
}

#[test]
fn a_tablet_drops_the_language_before_any_control() {
    let lang = Lang::from_code("vi").unwrap();
    let h = bar_harness(768.0, "vi", None);
    let items = status_items(&h, lang, 768.0);
    assert!(!items.iter().any(|(l, _)| l == i18n::tr(lang, "English (United States)")), "{items:#?}");
    for b in BUTTONS {
        assert!(items.iter().any(|(l, _)| l == i18n::tr(lang, b)), "{b} stays at 768");
    }
}

/// Renders the status bar's widths in Vietnamese into `DECKCRAFT_SHOTS` (run with `--ignored`;
/// needs a GPU or software wgpu adapter).
#[test]
#[ignore]
fn status_bar_screenshots() {
    let Some(dir) = std::env::var_os("DECKCRAFT_SHOTS").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let message = "The file is too large or abnormally compressed, so it can't be opened.";
    let shots = [
        ((768.0, 1024.0), None, "status-768x1024-vi.png"),
        ((768.0, 1024.0), Some(message), "status-768x1024-message-vi.png"),
        ((1024.0, 768.0), None, "status-1024x768-vi.png"),
        ((1024.0, 768.0), Some(message), "status-1024x768-message-vi.png"),
        ((1366.0, 768.0), None, "status-1366x768-vi.png"),
        ((390.0, 844.0), None, "status-390x844-vi.png"),
    ];
    for ((w, hgt), msg, file) in shots {
        let mut h = sample_harness(vec2(w, hgt), "vi");
        for _ in 0..3 {
            h.state_mut().run("slide.new", json!({})).unwrap();
        }
        if let Some(m) = msg {
            h.state_mut().set_status(m);
        }
        h.run_steps(4);
        match h.render() {
            Ok(img) => img.save(dir.join(file)).unwrap(),
            Err(e) => eprintln!("no renderer: {e}"),
        }
    }
}
