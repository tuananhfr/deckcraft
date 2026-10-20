use wasm_bindgen::{JsCast as _, JsValue};

pub(crate) fn notify() {
    let Some(window) = web_sys::window() else { return };
    let Some(hook) = js_sys::Reflect::get(&window, &JsValue::from_str("erpEditorClosed")).ok().and_then(|v| v.dyn_into::<js_sys::Function>().ok())
    else {
        return;
    };
    let _ = hook.call0(&window);
}
