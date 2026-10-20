//! Which status-bar items a window too narrow for all of them keeps: the optional ones give way,
//! lowest priority first, until the rest fits without overlapping (translated labels are longer).
//! Below `narrow::NARROW_WIDTH` the phone status bar takes over.

/// Status-bar items that may be hidden or shortened, in the order they give way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusItem {
    /// The proofing language ("English (United States)"); it only informs.
    Language,
    ZoomSlider,
    /// The Notes and Comments switches' words; their icons stay.
    ToggleLabels,
    /// Normal, Slide Sorter, Reading View and Slide Show (the View tab has them too).
    ViewButtons,
    /// Zoom In and Zoom Out (the zoom percentage still opens the Zoom dialog).
    ZoomButtons,
}

pub const STATUS_DROP_ORDER: [StatusItem; 5] =
    [StatusItem::Language, StatusItem::ZoomSlider, StatusItem::ToggleLabels, StatusItem::ViewButtons, StatusItem::ZoomButtons];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusFit {
    /// How many of [`STATUS_DROP_ORDER`] are hidden.
    hidden: usize,
}

impl StatusFit {
    /// Everything shown.
    pub const ALL: StatusFit = StatusFit { hidden: 0 };

    /// Hides optional items until `fixed` (what always shows, with its spacing, and room for a
    /// fresh message) plus the remaining optional widths fit in `available`.
    pub fn new(available: f32, fixed: f32, width: impl Fn(StatusItem) -> f32) -> StatusFit {
        let mut needed = fixed + STATUS_DROP_ORDER.iter().map(|&o| width(o)).sum::<f32>();
        let mut hidden = 0;
        let fits = |needed: f32| needed.is_finite() && available.is_finite() && needed <= available;
        while hidden < STATUS_DROP_ORDER.len() && !fits(needed) {
            needed -= width(STATUS_DROP_ORDER[hidden]);
            hidden += 1;
        }
        StatusFit { hidden }
    }

    pub fn shows(self, item: StatusItem) -> bool {
        STATUS_DROP_ORDER.iter().position(|&o| o == item).is_none_or(|at| at >= self.hidden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(o: StatusItem) -> f32 {
        match o {
            StatusItem::Language => 125.0,
            StatusItem::ZoomSlider => 114.0,
            StatusItem::ToggleLabels => 100.0,
            StatusItem::ViewButtons => 128.0,
            StatusItem::ZoomButtons => 60.0,
        }
    }

    #[test]
    fn a_wide_bar_keeps_everything() {
        let fit = StatusFit::new(1366.0, 300.0, widths);
        assert_eq!(fit, StatusFit::ALL);
        assert!(STATUS_DROP_ORDER.iter().all(|&o| fit.shows(o)));
    }

    #[test]
    fn items_give_way_lowest_priority_first() {
        // 300 fixed + 527 optional is 59 over 768: the language goes, nothing else.
        let fit = StatusFit::new(768.0, 300.0, widths);
        assert!(!fit.shows(StatusItem::Language) && fit.shows(StatusItem::ZoomSlider) && fit.shows(StatusItem::ZoomButtons));
        let mut last = 0;
        for available in (250..=900).rev().step_by(10) {
            let fit = StatusFit::new(available as f32, 300.0, widths);
            let shown: Vec<bool> = STATUS_DROP_ORDER.iter().map(|&o| fit.shows(o)).collect();
            let hidden = shown.iter().take_while(|s| !**s).count();
            assert!(shown[hidden..].iter().all(|s| *s), "hidden ones are a prefix of the order at {available}");
            assert!(hidden >= last, "narrower never shows more");
            last = hidden;
        }
        assert_eq!(last, STATUS_DROP_ORDER.len());
    }

    #[test]
    fn hostile_widths_hide_every_optional_item() {
        let fit = StatusFit::new(f32::NAN, 300.0, widths);
        assert!(STATUS_DROP_ORDER.iter().all(|&o| !fit.shows(o)));
    }
}
