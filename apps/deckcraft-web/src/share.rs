//! The office shell's share sheet: `window.erpCanShareFiles()` and `window.erpShare(bytes, name,
//! mime)`. Without them (standalone pages) Share… is not offered.

use wasm_bindgen::{JsCast as _, JsValue};

fn shell_function(name: &str) -> Option<(web_sys::Window, js_sys::Function)> {
    let window = web_sys::window()?;
    let function = js_sys::Reflect::get(&window, &JsValue::from_str(name)).ok()?.dyn_into::<js_sys::Function>().ok()?;
    Some((window, function))
}

/// The shell can share a .pptx from this browser and page (asked once, at start-up). Chromium's
/// Web Share refuses Office types, so there it says no.
pub(crate) fn available() -> bool {
    let mime = JsValue::from_str(crate::file_types::PPTX_MIME);
    shell_function("erpCanShareFiles").and_then(|(window, can_share)| can_share.call1(&window, &mime).ok()).is_some_and(|v| v.as_bool() == Some(true))
}

/// Hands the file to the share sheet; false when the shell didn't take it, so the caller downloads.
pub(crate) fn share(name: &str, bytes: &[u8]) -> bool {
    let Some((window, share)) = shell_function("erpShare") else { return false };
    let mime = JsValue::from_str(crate::file_types::mime_for(name));
    share.call3(&window, &js_sys::Uint8Array::from(bytes), &JsValue::from_str(name), &mime).is_ok_and(|v| v.as_bool() == Some(true))
}
