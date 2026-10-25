//! Narrow windows (phones in portrait): below [`NARROW_WIDTH`] the ribbon folds to its tab row, the
//! slide thumbnails hide, task panes float over the slide instead of squeezing it, and the title
//! and status bars keep only what fits.

use egui::{Rect, Ui, pos2, vec2};

/// Window content width, in points, under which the layout compacts. A tablet in portrait
/// (~768 points) stays wide.
pub const NARROW_WIDTH: f32 = 600.0;

pub fn is_narrow(width: f32) -> bool {
    width < NARROW_WIDTH
}

/// Edge-triggered compacting: the ribbon folds and the thumbnails hide once when the width drops
/// below the breakpoint, and the user's wide-window choices come back when it rises above it. In
/// between, the user's own expand, collapse, show or hide is left alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NarrowLayout {
    /// `None` until the first frame measured the window.
    narrow: Option<bool>,
    /// The slide thumbnail pane is showing (Normal view).
    pub thumbs: bool,
    /// The choices the user had before the window went narrow; restored when it widens.
    wide_ribbon_collapsed: Option<bool>,
    wide_thumbs: Option<bool>,
}

impl Default for NarrowLayout {
    fn default() -> Self {
        NarrowLayout { narrow: None, thumbs: true, wide_ribbon_collapsed: None, wide_thumbs: None }
    }
}

impl NarrowLayout {
    /// Feeds this frame's content width; changes `ribbon_collapsed` and the thumbnails only on a
    /// crossing.
    pub fn update(&mut self, width: f32, ribbon_collapsed: &mut bool) {
        let narrow = is_narrow(width);
        let was = self.narrow.replace(narrow);
        if was == Some(narrow) {
            return;
        }
        if narrow {
            self.wide_ribbon_collapsed = Some(*ribbon_collapsed);
            self.wide_thumbs = Some(self.thumbs);
            *ribbon_collapsed = true;
            self.thumbs = false;
        } else {
            if let Some(wide) = self.wide_ribbon_collapsed.take() {
                *ribbon_collapsed = wide;
            }
            if let Some(wide) = self.wide_thumbs.take() {
                self.thumbs = wide;
            }
        }
    }

    pub fn is_narrow(&self) -> bool {
        self.narrow == Some(true)
    }

    /// The ribbon state worth saving: while narrow, the choice the user made for a wide window,
    /// so a phone-sized session doesn't leave the desktop ribbon folded.
    pub fn saved_ribbon_collapsed(&self, current: bool) -> bool {
        self.wide_ribbon_collapsed.unwrap_or(current)
    }
}

/// Slide width, in points, a floating pane leaves uncovered so a tap there still reaches the slide.
const SLIDE_STRIP: f32 = 48.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A pane floating over the slide area at `side` of `room`, `width` wide at most: a narrow window
/// keeps the slide its full width under it.
pub fn floating_pane(ui: &Ui, id: &'static str, side: Side, width: f32, frame: egui::Frame, add: impl FnOnce(&mut Ui)) {
    let room = ui.available_rect_before_wrap();
    let width = width.min((room.width() - SLIDE_STRIP).max(room.width() * 0.5)).max(0.0);
    let x = match side {
        Side::Left => room.left(),
        Side::Right => room.right() - width,
    };
    let rect = Rect::from_min_size(pos2(x, room.top()), vec2(width, room.height()));
    let margin = frame.total_margin().sum();
    let inner = vec2((width - margin.x).max(0.0), (room.height() - margin.y).max(0.0));
    let shadow = ui.visuals().popup_shadow;
    egui::Area::new(egui::Id::new((id, "floating"))).order(egui::Order::Middle).fixed_pos(rect.min).constrain(false).show(ui.ctx(), |ui| {
        frame.shadow(shadow).show(ui, |ui| {
            ui.set_width(inner.x);
            ui.set_min_height(inner.y);
            ui.set_max_height(inner.y);
            add(ui);
        });
    });
}

/// Title-bar controls a narrow window may hide, in the order they give way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleItem {
    Appearance,
    Comments,
    /// The PI ID account button; without an account hook it takes no room. File › Account stays
    /// reachable when it gives way.
    Account,
    PlayFromStart,
    Home,
}

pub const TITLE_DROP_ORDER: [TitleItem; 5] =
    [TitleItem::Appearance, TitleItem::Comments, TitleItem::Account, TitleItem::PlayFromStart, TitleItem::Home];

/// How many of [`TITLE_DROP_ORDER`] a narrow title bar hides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TitleFit {
    hidden: usize,
}

impl TitleFit {
    /// Everything shown (a wide window).
    pub const ALL: TitleFit = TitleFit { hidden: 0 };

    /// Hides optional controls until `fixed` (the controls that always show, the title's minimum
    /// and the gaps) plus the remaining optional widths fit in `available`.
    pub fn new(available: f32, fixed: f32, width: impl Fn(TitleItem) -> f32) -> TitleFit {
        let mut needed = fixed + TITLE_DROP_ORDER.iter().map(|&o| width(o)).sum::<f32>();
        let mut hidden = 0;
        let fits = |needed: f32| needed.is_finite() && available.is_finite() && needed <= available;
        while hidden < TITLE_DROP_ORDER.len() && !fits(needed) {
            needed -= width(TITLE_DROP_ORDER[hidden]);
            hidden += 1;
        }
        TitleFit { hidden }
    }

    pub fn shows(self, item: TitleItem) -> bool {
        TITLE_DROP_ORDER.iter().position(|&o| o == item).is_none_or(|at| at >= self.hidden)
    }
}

/// `text` cut with "…" to fit `width` in `font`; `None` when not even a few letters fit.
pub fn elide(ui: &Ui, text: &str, font: egui::FontId, color: egui::Color32, width: f32) -> Option<std::sync::Arc<egui::Galley>> {
    if !width.is_finite() || width < 24.0 {
        return None;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    Some(ui.fonts_mut(|f| f.layout_job(job)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: f32 = 390.0;
    const TABLET: f32 = 768.0;

    #[test]
    fn going_narrow_folds_the_ribbon_and_hides_the_thumbnails_once() {
        let mut layout = NarrowLayout::default();
        let mut collapsed = false;
        layout.update(TABLET, &mut collapsed);
        assert!(!collapsed && layout.thumbs && !layout.is_narrow(), "a tablet in portrait stays wide");
        layout.update(PHONE, &mut collapsed);
        assert!(collapsed && !layout.thumbs && layout.is_narrow());
    }

    #[test]
    fn a_manual_expand_or_show_sticks_while_narrow() {
        let mut layout = NarrowLayout::default();
        let mut collapsed = false;
        layout.update(TABLET, &mut collapsed);
        layout.update(PHONE, &mut collapsed);
        collapsed = false;
        layout.thumbs = true;
        for width in [PHONE, 360.0, 420.0, 599.0, PHONE] {
            layout.update(width, &mut collapsed);
            assert!(!collapsed && layout.thumbs, "resizing within the narrow range keeps the user's choice ({width})");
        }
        assert!(!layout.saved_ribbon_collapsed(collapsed));
    }

    #[test]
    fn widening_restores_the_wide_choices() {
        for wide_choice in [false, true] {
            let mut layout = NarrowLayout::default();
            let mut collapsed = wide_choice;
            layout.thumbs = !wide_choice;
            layout.update(1366.0, &mut collapsed);
            layout.update(PHONE, &mut collapsed);
            assert!(collapsed && !layout.thumbs);
            assert_eq!(layout.saved_ribbon_collapsed(collapsed), wide_choice, "saving while narrow keeps the wide choice");
            // Whatever the user did while narrow, the wide window gets its own choices back.
            collapsed = !wide_choice;
            layout.thumbs = wide_choice;
            layout.update(TABLET, &mut collapsed);
            assert_eq!((collapsed, layout.thumbs), (wide_choice, !wide_choice));
            assert_eq!(layout.saved_ribbon_collapsed(collapsed), wide_choice);
            // A manual change while wide holds too.
            collapsed = !wide_choice;
            layout.thumbs = wide_choice;
            layout.update(1366.0, &mut collapsed);
            assert_eq!((collapsed, layout.thumbs), (!wide_choice, wide_choice));
        }
    }

    #[test]
    fn starting_narrow_compacts_and_widening_restores() {
        let mut layout = NarrowLayout::default();
        let mut collapsed = false;
        layout.update(PHONE, &mut collapsed);
        assert!(collapsed && !layout.thumbs);
        layout.update(TABLET, &mut collapsed);
        assert!(!collapsed && layout.thumbs);
    }

    #[test]
    fn the_breakpoint_itself_is_wide() {
        assert!(!is_narrow(NARROW_WIDTH));
        assert!(is_narrow(NARROW_WIDTH - 0.5));
    }

    fn widths(_: TitleItem) -> f32 {
        26.0
    }

    #[test]
    fn title_controls_give_way_lowest_priority_first() {
        assert!(TITLE_DROP_ORDER.iter().all(|&o| TitleFit::new(1366.0, 250.0, widths).shows(o)));
        let fit = TitleFit::new(304.0, 250.0, widths);
        assert!(!fit.shows(TitleItem::Appearance) && !fit.shows(TitleItem::Comments) && fit.shows(TitleItem::PlayFromStart));
        let mut last = 0;
        for available in (150..=600).rev().step_by(5) {
            let fit = TitleFit::new(available as f32, 250.0, widths);
            let shown: Vec<bool> = TITLE_DROP_ORDER.iter().map(|&o| fit.shows(o)).collect();
            let hidden = shown.iter().take_while(|s| !**s).count();
            assert!(shown[hidden..].iter().all(|s| *s), "hidden ones are a prefix of the order at {available}");
            assert!(hidden >= last, "narrower never shows more");
            last = hidden;
        }
        assert!(TITLE_DROP_ORDER.iter().all(|&o| !TitleFit::new(f32::NAN, 250.0, widths).shows(o)));
    }
}
