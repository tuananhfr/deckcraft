//! Tells the office shell how each dropped file ended (`window.erpIntakeResult(name, size, ok)`),
//! so a file it handed over shows as opened or refused. Files from the open picker aren't reported.

use std::cell::RefCell;
use std::rc::Rc;

/// Dropped files read into the inbox and not yet reported, as (name, byte length).
#[derive(Clone, Default)]
pub(crate) struct DroppedFiles(Rc<RefCell<Vec<(String, usize)>>>);

impl DroppedFiles {
    pub(crate) fn record(&self, name: &str, size: usize) {
        if let Ok(mut files) = self.0.try_borrow_mut() {
            files.push((name.to_string(), size));
        }
    }

    /// Whether `(name, size)` was dropped; each drop is claimed once.
    pub(crate) fn claim(&self, name: &str, size: usize) -> bool {
        let Ok(mut files) = self.0.try_borrow_mut() else { return false };
        let Some(i) = files.iter().position(|(n, s)| n == name && *s == size) else { return false };
        files.remove(i);
        true
    }
}

/// A browser `File.size` as a byte length (0 for anything that isn't one).
pub(crate) fn byte_length(size: f64) -> usize {
    if size.is_finite() && size > 0.0 { size as usize } else { 0 }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn report(name: &str, size: usize, ok: bool) {
    use wasm_bindgen::{JsCast as _, JsValue};
    let Some(window) = web_sys::window() else { return };
    let Some(hook) = js_sys::Reflect::get(&window, &JsValue::from_str("erpIntakeResult")).ok().and_then(|v| v.dyn_into::<js_sys::Function>().ok())
    else {
        return;
    };
    let _ = hook.call3(&window, &JsValue::from_str(name), &JsValue::from_f64(size as f64), &JsValue::from_bool(ok));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dropped_files_are_claimed_and_each_once() {
        let dropped = DroppedFiles::default();
        dropped.record("Báo cáo.pptx", 10);
        dropped.record("Báo cáo.pptx", 10);
        assert!(!dropped.claim("Báo cáo.pptx", 11), "a picked file of the same name");
        assert!(!dropped.claim("other.pptx", 10));
        assert!(dropped.claim("Báo cáo.pptx", 10));
        assert!(dropped.claim("Báo cáo.pptx", 10), "two drops of the same file report twice");
        assert!(!dropped.claim("Báo cáo.pptx", 10));
    }

    #[test]
    fn unreadable_files_report_the_browser_size() {
        assert_eq!(byte_length(12345.0), 12345);
        for odd in [f64::NAN, -1.0, f64::INFINITY] {
            assert_eq!(byte_length(odd), 0);
        }
    }
}
