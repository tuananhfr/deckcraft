//! Bridge to the office site's draft store, `window.erpDrafts` (`put`, `remove`, `ready`):
//! unsaved presentations are put there as they change and when the page is hidden, and drafts it
//! hands back are reopened. Without the store (DeckCraft opened on its own) all of this is off.

use std::cell::RefCell;
use std::rc::Rc;

use deckcraft_ui_egui::SlideApp;
use deckcraft_ui_egui::drafts::{DraftStore, Drafts, FORMAT};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;

/// A draft the store handed back, opened on the next frame.
struct Restore {
    key: String,
    name: String,
    format: String,
    bytes: Vec<u8>,
}

pub struct DraftBridge {
    /// Shared with the page-hide listeners, which can't reach the app: what they flush is what
    /// the last frame saw.
    drafts: Rc<RefCell<Drafts>>,
    restores: Rc<RefCell<Vec<Restore>>>,
    ready: bool,
}

impl DraftBridge {
    pub fn new() -> Self {
        let drafts = Rc::new(RefCell::new(Drafts::default()));
        if let Err(e) = flush_when_hidden(&drafts) {
            log::warn!("drafts aren't flushed when the page is hidden: {e}");
        }
        DraftBridge { drafts, restores: Rc::default(), ready: false }
    }

    /// After the app's `logic`: tell the store the app can open drafts (once, after the fonts, as
    /// for any file), and open those it handed back.
    pub fn logic(&mut self, app: &mut SlideApp, ctx: &egui::Context) {
        if !self.ready && app.fonts_ready() && let Some(store) = JsStore::lookup() {
            self.ready = true;
            if let Err(e) = store.ready(self.restores.clone(), ctx.clone()) {
                log::warn!("erpDrafts.ready failed: {e}");
            }
        }
        let Ok(mut drafts) = self.drafts.try_borrow_mut() else { return };
        let restores = match self.restores.try_borrow_mut() {
            Ok(mut r) => std::mem::take(&mut *r),
            Err(_) => return,
        };
        for r in restores {
            if r.format != FORMAT {
                log::warn!("draft {} is {}, not a {FORMAT} file", r.key, r.format);
                continue;
            }
            if let Err(e) = drafts.restore(app, &r.key, &r.name, &r.bytes, js_sys::Date::now()) {
                log::warn!("couldn't open draft {}: {e}", r.key);
            }
        }
    }

    /// At the end of every frame: put and remove what is due.
    pub fn after_frame(&mut self, app: &SlideApp, ctx: &egui::Context) {
        let Some(mut store) = JsStore::lookup() else { return };
        let Ok(mut drafts) = self.drafts.try_borrow_mut() else { return };
        if drafts.tick(app, js_sys::Date::now(), &js_sys::Date::now, &mut store) {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }
}

/// Hiding the page may be the last chance (mobile browsers discard hidden tabs without
/// `beforeunload`): put every changed draft at once.
fn flush_when_hidden(drafts: &Rc<RefCell<Drafts>>) -> Result<(), String> {
    let js = |e: JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let flush = |drafts: &RefCell<Drafts>| {
        let (Some(mut store), Ok(mut drafts)) = (JsStore::lookup(), drafts.try_borrow_mut()) else { return };
        drafts.flush(&js_sys::Date::now, &mut store);
    };
    let (d, doc) = (drafts.clone(), document.clone());
    let on_visibility = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        if doc.visibility_state() == web_sys::VisibilityState::Hidden {
            flush(&d);
        }
    });
    document.add_event_listener_with_callback("visibilitychange", on_visibility.as_ref().unchecked_ref()).map_err(js)?;
    let d = drafts.clone();
    let on_pagehide = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| flush(&d));
    window.add_event_listener_with_callback("pagehide", on_pagehide.as_ref().unchecked_ref()).map_err(js)?;
    // The listeners live as long as the page.
    on_visibility.forget();
    on_pagehide.forget();
    Ok(())
}

/// `window.erpDrafts`, looked up at each use: the site may not provide it.
struct JsStore(JsValue);

impl JsStore {
    fn lookup() -> Option<Self> {
        let window = web_sys::window()?;
        js_sys::Reflect::get(&window, &JsValue::from_str("erpDrafts")).ok().filter(JsValue::is_object).map(JsStore)
    }

    fn call(&self, method: &str, args: &js_sys::Array) -> Result<JsValue, String> {
        let f: js_sys::Function = js_sys::Reflect::get(&self.0, &JsValue::from_str(method))
            .map_err(|e| format!("{e:?}"))?
            .dyn_into()
            .map_err(|_| format!("erpDrafts.{method} is not a function"))?;
        f.apply(&self.0, args).map_err(|e| format!("{e:?}"))
    }

    /// Hands the store `restore(key, name, format, bytes)`, which queues the draft for the next
    /// frame (opening it inside the store's callback would reach into the running app).
    fn ready(&self, restores: Rc<RefCell<Vec<Restore>>>, ctx: egui::Context) -> Result<(), String> {
        let restore = wasm_bindgen::closure::Closure::<dyn FnMut(JsValue, JsValue, JsValue, JsValue)>::new(
            move |key: JsValue, name: JsValue, format: JsValue, bytes: JsValue| {
                let (Some(key), Some(name), Some(format)) = (key.as_string(), name.as_string(), format.as_string()) else {
                    log::warn!("erpDrafts restore: key, name and format must be strings");
                    return;
                };
                let Ok(bytes) = bytes.dyn_into::<js_sys::Uint8Array>() else {
                    log::warn!("erpDrafts restore: bytes must be a Uint8Array");
                    return;
                };
                match restores.try_borrow_mut() {
                    Ok(mut r) => r.push(Restore { key, name, format, bytes: bytes.to_vec() }),
                    Err(_) => log::warn!("erpDrafts restore: busy, draft {key} dropped"),
                }
                ctx.request_repaint();
            },
        );
        let r = self.call("ready", &js_sys::Array::of1(restore.as_ref()));
        // The store may call it at any time while the page lives.
        restore.forget();
        r.map(|_| ())
    }
}

impl DraftStore for JsStore {
    fn put(&mut self, key: &str, name: &str, format: &str, bytes: &[u8]) {
        // A copy the store owns, never a view into wasm memory (which moves when it grows).
        let args = js_sys::Array::of4(&JsValue::from_str(key), &JsValue::from_str(name), &JsValue::from_str(format), &js_sys::Uint8Array::from(bytes));
        if let Err(e) = self.call("put", &args) {
            log::warn!("erpDrafts.put({key}) failed: {e}");
        }
    }

    fn remove(&mut self, key: &str) {
        if let Err(e) = self.call("remove", &js_sys::Array::of1(&JsValue::from_str(key))) {
            log::warn!("erpDrafts.remove({key}) failed: {e}");
        }
    }
}
