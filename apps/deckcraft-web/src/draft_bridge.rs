//! Bridge to the office site's draft store, `window.erpDrafts` (`put`, `remove`, `ready`):
//! unsaved presentations are put there as they change and when the page is hidden, and drafts it
//! hands back are reopened. Without the store (DeckCraft opened on its own) all of this is off.

use std::cell::RefCell;
use std::rc::Rc;

use deckcraft_ui_egui::SlideApp;
use deckcraft_ui_egui::drafts::{DocInput, DraftStore, Drafts, FORMAT};

use crate::draft_state::{Message, States};
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
    tracking: Rc<RefCell<Tracking>>,
}

impl DraftBridge {
    pub fn new() -> Self {
        let drafts = Rc::new(RefCell::new(Drafts::default()));
        let tracking = Rc::new(RefCell::new(Tracking::default()));
        if let Err(e) = flush_when_hidden(&drafts, &tracking) {
            log::warn!("drafts aren't flushed when the page is hidden: {e}");
        }
        DraftBridge { drafts, restores: Rc::default(), ready: false, tracking }
    }

    /// After the app's `logic`: tell the store the app can open drafts (once, after the fonts, as
    /// for any file), and open those it handed back.
    pub fn logic(&mut self, app: &mut SlideApp, ctx: &egui::Context) {
        if !self.ready
            && app.fonts_ready()
            && let Some(store) = JsStore::lookup(self.tracking.clone())
        {
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
            } else if let Some(d) = app.session.active()
                && let Ok(mut tracking) = self.tracking.try_borrow_mut()
            {
                tracking.keys.insert(d.uid, r.key.clone());
                if let Some(input) = app.draft_inputs().into_iter().find(|input| input.uid == d.uid) {
                    tracking.states.observe(&r.key, &input.name, false, true);
                    tracking.states.checkpoint(&r.key);
                    tracking.states.complete(&r.key, 0, true);
                    tracking.snapshots.insert(d.uid, input);
                }
            }
        }
    }

    /// At the end of every frame: put and remove what is due.
    pub fn after_frame(&mut self, app: &SlideApp, ctx: &egui::Context) {
        let Some(mut store) = JsStore::lookup(self.tracking.clone()) else { return };
        let Ok(mut drafts) = self.drafts.try_borrow_mut() else { return };
        if let Ok(mut tracking) = self.tracking.try_borrow_mut() {
            let inputs = app.draft_inputs();
            let active = app.session.active().map(|d| tracking.key(d.uid));
            let messages = tracking.states.switch(active.as_deref());
            store.send(messages);
            let closed: Vec<_> = tracking.snapshots.keys().filter(|uid| !inputs.iter().any(|d| d.uid == **uid)).copied().collect();
            for uid in closed {
                let key = tracking.key(uid);
                let messages = tracking.states.close(&key);
                store.send(messages);
                tracking.snapshots.remove(&uid);
                tracking.keys.remove(&uid);
            }
            for d in inputs {
                let key = tracking.key(d.uid);
                let edited = tracking.snapshots.get(&d.uid).is_some_and(|old| !std::sync::Arc::ptr_eq(&old.doc, &d.doc));
                let messages = tracking.states.observe(&key, &d.name, edited, d.unsaved);
                store.send(messages);
                tracking.snapshots.insert(d.uid, d);
            }
        }
        if drafts.tick(app, js_sys::Date::now(), &js_sys::Date::now, &mut store) {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }
}

/// Hiding the page may be the last chance (mobile browsers discard hidden tabs without
/// `beforeunload`): put every changed draft at once.
fn flush_when_hidden(drafts: &Rc<RefCell<Drafts>>, tracking: &Rc<RefCell<Tracking>>) -> Result<(), String> {
    let js = |e: JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let tracking = tracking.clone();
    let flush = move |drafts: &RefCell<Drafts>| {
        let (Some(mut store), Ok(mut drafts)) = (JsStore::lookup(tracking.clone()), drafts.try_borrow_mut()) else { return };
        drafts.flush(&js_sys::Date::now, &mut store);
    };
    let visibility_flush = flush.clone();
    let (d, doc) = (drafts.clone(), document.clone());
    let on_visibility = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        if doc.visibility_state() == web_sys::VisibilityState::Hidden {
            visibility_flush(&d);
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

#[derive(Default)]
struct Tracking {
    states: States,
    keys: std::collections::BTreeMap<u64, String>,
    snapshots: std::collections::BTreeMap<u64, DocInput>,
}

impl Tracking {
    fn key(&self, uid: u64) -> String {
        self.keys.get(&uid).cloned().unwrap_or_else(|| format!("doc-{uid}"))
    }
}

/// `window.erpDrafts`, looked up at each use: the site may not provide it.
struct JsStore(JsValue, Rc<RefCell<Tracking>>);

impl JsStore {
    fn lookup(tracking: Rc<RefCell<Tracking>>) -> Option<Self> {
        let window = web_sys::window()?;
        js_sys::Reflect::get(&window, &JsValue::from_str("erpDrafts")).ok().filter(JsValue::is_object).map(|value| JsStore(value, tracking))
    }

    fn send(&self, messages: Vec<Message>) {
        let Ok(function) = js_sys::Reflect::get(&self.0, &JsValue::from_str("state")) else { return };
        let Some(function) = function.dyn_ref::<js_sys::Function>() else { return };
        for m in messages {
            let value = js_sys::Object::new();
            let fields = [
                ("docId", JsValue::from_str(&m.doc_id)),
                ("name", JsValue::from_str(&m.name)),
                ("revision", JsValue::from_f64(m.revision as f64)),
                ("dirty", JsValue::from_bool(m.dirty)),
                ("pending", JsValue::from_bool(m.pending)),
                ("committed", JsValue::from_bool(m.committed)),
                ("failed", JsValue::from_bool(m.failed)),
            ];
            if fields.into_iter().all(|(key, v)| js_sys::Reflect::set(&value, &JsValue::from_str(key), &v).is_ok()) {
                let _ = function.call1(&self.0, &value);
            }
        }
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
        let args =
            js_sys::Array::of4(&JsValue::from_str(key), &JsValue::from_str(name), &JsValue::from_str(format), &js_sys::Uint8Array::from(bytes));
        let revision = if let Ok(mut tracking) = self.1.try_borrow_mut() {
            let messages = tracking.states.checkpoint(key);
            self.send(messages);
            tracking.states.revision(key)
        } else {
            None
        };
        let result = self.call("put", &args);
        let Some(revision) = revision else {
            if let Err(e) = result {
                log::warn!("erpDrafts.put({key}) failed: {e}");
            }
            return;
        };
        // Legacy stores have no state method and may return undefined.
        let state_available = js_sys::Reflect::get(&self.0, &JsValue::from_str("state")).ok().is_some_and(|v| v.is_function());
        if !state_available {
            return;
        }
        let store = JsStore(self.0.clone(), self.1.clone());
        let key = key.to_owned();
        wasm_bindgen_futures::spawn_local(async move {
            let success = match result {
                Ok(value) => match value.dyn_into::<js_sys::Promise>() {
                    Ok(promise) => wasm_bindgen_futures::JsFuture::from(promise).await.ok().and_then(|v| v.as_bool()) == Some(true),
                    Err(_) => false,
                },
                Err(_) => false,
            };
            if let Ok(mut tracking) = store.1.try_borrow_mut() {
                let messages = tracking.states.complete(&key, revision, success);
                store.send(messages);
            }
        });
    }

    fn remove(&mut self, key: &str) {
        if let Err(e) = self.call("remove", &js_sys::Array::of1(&JsValue::from_str(key))) {
            log::warn!("erpDrafts.remove({key}) failed: {e}");
        }
    }
}
