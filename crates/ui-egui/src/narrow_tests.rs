//! A phone-width window (`narrow`): what compacts and when, and title and status bars whose
//! controls never overlap.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use deckcraft_engine::Session;
use egui::{Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::i18n::{self, Lang};
use crate::{Services, SlideApp};

const HEIGHT: f32 = 844.0;

pub(crate) fn sample_harness(size: egui::Vec2, language: &str) -> Harness<'static, SlideApp> {
    let mut session = Session::new();
    deckcraft_engine::sample::open_sample(&mut session).unwrap();
    let mut app = SlideApp::new(session, Services::default());
    app.ui.language = language.into();
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

fn canvas_width(h: &Harness<'_, SlideApp>) -> f32 {
    h.state().canvas_rect.unwrap().width()
}

fn select_subtitle(h: &mut Harness<'_, SlideApp>) {
    let id = h.state().session.active().unwrap().current_slide().unwrap().shapes[1].id.0;
    h.state_mut().run("edit.select", json!({"ids": [id]})).unwrap();
}

#[test]
fn a_phone_folds_the_ribbon_hides_the_thumbnails_and_floats_panes() {
    let mut h = sample_harness(vec2(390.0, HEIGHT), "en");
    let app = h.state();
    assert!(app.layout.is_narrow() && app.ui.ribbon_collapsed && !app.layout.thumbs);
    let full = canvas_width(&h);
    assert!(full > 340.0, "the slide keeps the window's width: {full}");

    select_subtitle(&mut h);
    h.state_mut().run("view.pane", json!({"pane": "format"})).unwrap();
    h.run_steps(2);
    assert!(h.query_by_label("Format Shape").is_some());
    assert!((canvas_width(&h) - full).abs() < 1.0, "the pane floats over the slide");

    // The thumbnails can be reopened, over the slide as well.
    h.get_by_label("Slide Thumbnails").click();
    h.run_steps(2);
    assert!(h.state().layout.thumbs);
    assert!((canvas_width(&h) - full).abs() < 1.0);

    // A wide window gets its side panels and its ribbon back.
    h.set_size(vec2(1366.0, 768.0));
    h.run_steps(3);
    let app = h.state();
    assert!(!app.layout.is_narrow() && !app.ui.ribbon_collapsed && app.layout.thumbs);
    assert!(canvas_width(&h) < 1366.0 - 236.0 - 240.0, "thumbnails and pane sit beside the slide: {}", canvas_width(&h));
}

#[test]
fn a_tab_tapped_on_a_phone_unfolds_the_ribbon_until_the_width_crosses_again() {
    let mut h = sample_harness(vec2(390.0, HEIGHT), "en");
    assert!(h.state().ui.ribbon_collapsed);
    h.get_by_label("Insert").click();
    h.run_steps(4);
    assert!(!h.state().ui.ribbon_collapsed, "the user's expand sticks while narrow");
    h.set_size(vec2(420.0, HEIGHT));
    h.run_steps(2);
    assert!(!h.state().ui.ribbon_collapsed, "resizing within the narrow range keeps it");
    h.set_size(vec2(768.0, 1024.0));
    h.run_steps(2);
    assert!(!h.state().ui.ribbon_collapsed);
    h.set_size(vec2(390.0, HEIGHT));
    h.run_steps(2);
    assert!(h.state().ui.ribbon_collapsed, "crossing below the breakpoint folds it again");
}

#[test]
fn a_tablet_in_portrait_keeps_the_wide_layout() {
    let h = sample_harness(vec2(768.0, 1024.0), "vi");
    let app = h.state();
    assert!(!app.layout.is_narrow() && !app.ui.ribbon_collapsed && app.layout.thumbs);
}

#[test]
fn the_desktop_saves_its_own_ribbon_choice_while_narrow() {
    let mut h = sample_harness(vec2(1366.0, 768.0), "en");
    h.set_size(vec2(390.0, HEIGHT));
    h.run_steps(2);
    assert!(h.state().ui.ribbon_collapsed);
    assert!(!h.state().ui_state_to_save().ribbon_collapsed);
}

/// The rects of the nodes labelled `labels` (in `lang`) inside `band`.
pub(crate) fn rects_in(h: &Harness<'_, SlideApp>, lang: Lang, band: Rect, labels: &[&str]) -> Vec<(String, Rect)> {
    let mut out = vec![];
    for l in labels {
        let shown = i18n::tr(lang, l);
        for n in h.query_all_by_label(shown) {
            if band.contains_rect(n.rect()) || band.intersects(n.rect()) {
                out.push((shown.to_string(), n.rect()));
            }
        }
    }
    out
}

pub(crate) fn assert_apart(what: &str, width: f32, rects: &[(String, Rect)]) {
    for (i, (a, ra)) in rects.iter().enumerate() {
        assert!(ra.min.x >= -0.5 && ra.max.x <= width + 0.5, "{what} at {width}: {a} leaves the window: {ra:?}");
        for (b, rb) in &rects[i + 1..] {
            let overlap = ra.intersect(*rb);
            assert!(!overlap.is_positive() || overlap.area() < 0.5, "{what} at {width}: {a} {ra:?} overlaps {b} {rb:?}");
        }
    }
}

#[test]
fn phone_title_and_status_bars_never_overlap() {
    for (width, code) in [(390.0, "vi"), (360.0, "vi"), (320.0, "vi"), (390.0, "en"), (599.0, "vi")] {
        let lang = Lang::from_code(code).unwrap();
        let mut h = sample_harness(vec2(width, HEIGHT), code);
        // A longer title ("— Edited") and a two-digit slide count.
        for _ in 0..3 {
            h.state_mut().run("slide.new", json!({})).unwrap();
        }
        h.run_steps(2);
        let title = format!("Northern Lights — {}", i18n::tr(lang, "Edited"));
        let top = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(width, crate::ribbon::TITLE_H));
        let mut bar = rects_in(
            &h,
            lang,
            top,
            &["Home", "Save (⌘S)", "Undo (⌘Z)", "Redo (⌘Y)", "Play from Start (F5)", "Search commands (⇧⌘P)", "Comments", "Light/Dark appearance"],
        );
        let present = h.query_all_by_label_contains(i18n::tr(lang, "Present")).map(|n| n.rect()).find(|r| top.intersects(*r));
        bar.push(("Present".into(), present.expect("the Present button stays")));
        let shown_title = h.query_by_label(&title).map(|n| n.rect());
        bar.push(("title".into(), shown_title.unwrap_or_else(|| panic!("the title shows at {width}"))));
        for must in ["Save (⌘S)", "Undo (⌘Z)", "Search commands (⇧⌘P)"] {
            assert!(bar.iter().any(|(l, _)| l == i18n::tr(lang, must)), "{must} stays at {width}");
        }
        assert_apart("title bar", width, &bar);

        let bottom = Rect::from_min_max(egui::pos2(0.0, HEIGHT - 28.0), egui::pos2(width, HEIGHT));
        let counter = i18n::fmt(i18n::tr(lang, "Slide {n} of {total}"), &[("n", "4"), ("total", "11")]);
        let mut status = rects_in(&h, lang, bottom, &["Slide Thumbnails", "Notes", "Comments", "Fit slide to current window"]);
        assert_eq!(status.len(), 4, "{status:?}");
        status.push(("counter".into(), h.get_by_label(&counter).rect()));
        let zoom = h.query_all_by_label_contains("%").map(|n| n.rect()).find(|r| bottom.intersects(*r));
        status.push(("zoom".into(), zoom.expect("the zoom percentage stays")));
        assert_apart("status bar", width, &status);
    }
}
