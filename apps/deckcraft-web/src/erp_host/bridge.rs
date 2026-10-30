//! `window.erpHost` itself: the version check at start-up, `ready({open, flush, acknowledged})` (v2 adds
//! `setMode` and `setUser`) and the editor → shell calls. Its records never carry a document's id,
//! title or bytes, nor the account's name.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};

use super::autosave::{Outcome, Reason};
use super::documents::{HostApi, HostDocs};
use super::mode::Mode;
use super::open::OpenRequest;
use super::request::Request;
use super::{ErpHost, outcome};

/// `flush` resolves by then even with saves still in flight.
const FLUSH_TIMEOUT_MS: i32 = 10_000;

/// The ids as the shell gave them (a string or a number), by their string form.
type Ids = Rc<RefCell<HashMap<String, JsValue>>>;

/// `erpHost` when the page offers version 1 or 2; checked once, at start-up.
pub(crate) fn install(ctx: egui::Context) -> Option<ErpHost> {
    let window = web_sys::window()?;
    let host = Reflect::get(&window, &JsValue::from_str("erpHost")).ok().filter(JsValue::is_object)?;
    let v2 = match Reflect::get(&host, &JsValue::from_str("version")).ok().and_then(|v| v.as_f64()) {
        Some(1.0) => false,
        Some(2.0) => true,
        _ => return None,
    };
    let ids = Ids::default();
    let wake = ctx.clone();
    let docs = HostDocs::new(
        Box::new(JsHost { host: host.clone(), ids: ids.clone() }),
        Box::new(js_sys::Date::now),
        Box::new(move || wake.request_repaint()),
    );
    let opens = Rc::new(RefCell::new(Vec::new()));
    let register = {
        let (docs, opens) = (docs.clone(), opens.clone());
        Box::new(move || {
            if let Err(e) = ready(&host, v2, docs, opens, ids, ctx) {
                log::warn!("erpHost.ready failed: {e}");
            }
        })
    };
    Some(ErpHost::new(docs, opens, register, v2))
}

type Requests = Rc<RefCell<Vec<Request>>>;

fn ready(host: &JsValue, v2: bool, docs: HostDocs, opens: Requests, ids: Ids, ctx: egui::Context) -> Result<(), String> {
    let docs_for_mode = docs.clone();
    let acknowledged = {
        let (docs, ids) = (docs.clone(), ids.clone());
        Closure::<dyn FnMut(JsValue, JsValue, JsValue)>::new(move |id: JsValue, revision: JsValue, version: JsValue| {
            let known = id_key(&id).filter(|key| ids.try_borrow().is_ok_and(|ids| ids.contains_key(key)));
            let (Some(key), Some(revision)) = (known, whole(&revision)) else { return };
            docs.acknowledged(&key, revision, whole(&version));
        })
    };
    let queue = {
        let (opens, ctx) = (opens.clone(), ctx.clone());
        move |request: Request| {
            match opens.try_borrow_mut() {
                Ok(mut opens) => opens.push(request),
                Err(_) => log::warn!("erpHost: busy, a call was dropped"),
            }
            ctx.request_repaint();
        }
    };
    let open = {
        let (ids, queue) = (ids.clone(), queue.clone());
        Closure::<dyn FnMut(JsValue)>::new(move |doc: JsValue| match read_request(&doc, &ids, v2) {
            Some(request) => queue(request.into()),
            None => log::warn!("erpHost open: a document without a docId"),
        })
    };
    let flush = Closure::<dyn FnMut() -> Promise>::new(move || {
        Promise::new(&mut |resolve: Function, _reject: Function| {
            let settle = resolve.clone();
            docs.flush(Box::new(move || {
                let _ = settle.call0(&JsValue::UNDEFINED);
            }));
            // Resolving twice is harmless; this one keeps the shell from waiting past the limit.
            let timeout = Closure::once_into_js(move || {
                let _ = resolve.call0(&JsValue::UNDEFINED);
            });
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(timeout.unchecked_ref(), FLUSH_TIMEOUT_MS);
            }
        })
    });
    let handlers = Object::new();
    let set = |key: &str, value: &JsValue| Reflect::set(&handlers, &JsValue::from_str(key), value).map_err(|e| format!("{e:?}"));
    set("open", open.as_ref())?;
    set("flush", flush.as_ref())?;
    set("acknowledged", acknowledged.as_ref())?;
    if v2 {
        let set_mode = {
            let queue = queue.clone();
            Closure::<dyn FnMut(JsValue, JsValue, JsValue)>::new(move |id: JsValue, mode: JsValue, can_download: JsValue| {
                let known = id_key(&id).filter(|key| ids.try_borrow().is_ok_and(|ids| ids.contains_key(key)));
                let Some(doc_id) = known else { return };
                let mode = Mode::parse(mode.as_string().as_deref());
                // Saves stop at once, not a frame later: the shell may hide the page in between.
                if let Some(uid) = docs_for_mode.uid_for(&doc_id) {
                    docs_for_mode.set_read_only(uid, mode != Mode::Edit);
                }
                queue(Request::SetMode { doc_id, mode, can_download: can_download.as_bool().unwrap_or(true) });
            })
        };
        let set_user = Closure::<dyn FnMut(JsValue)>::new(move |user: JsValue| queue(Request::SetUser(user_name(&user))));
        set("setMode", set_mode.as_ref())?;
        set("setUser", set_user.as_ref())?;
        set_mode.forget();
        set_user.forget();
    }
    // The shell may call them at any time while the page lives.
    open.forget();
    flush.forget();
    acknowledged.forget();
    call(host, "ready", &Array::of1(&handlers)).map(|_| ()).map_err(|e| format!("{e:?}"))
}

/// `user`: `{id, name}`, or `null` for the local preference (an anonymous link viewer).
fn user_name(user: &JsValue) -> Option<String> {
    let name = Reflect::get(user, &JsValue::from_str("name")).ok()?.as_string()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// `doc`: `{docId, version, title, format, bytes: Uint8Array}` and, from a v2 shell, `mode`,
/// `canDownload` and `preserveView`; `None` without a usable `docId`.
fn read_request(doc: &JsValue, ids: &Ids, v2: bool) -> Option<OpenRequest> {
    let field = |key: &str| Reflect::get(doc, &JsValue::from_str(key)).ok();
    let id = field("docId")?;
    let key = id_key(&id)?;
    if let Ok(mut ids) = ids.try_borrow_mut() {
        ids.insert(key.clone(), id);
    }
    let version = field("version").as_ref().and_then(whole).unwrap_or(0);
    let text = |key: &str| field(key).and_then(|v| v.as_string()).unwrap_or_default();
    let bytes = field("bytes").and_then(|v| v.dyn_into::<Uint8Array>().ok()).map(|bytes| bytes.to_vec());
    let flag = |key: &str| field(key).and_then(|v| v.as_bool());
    let (mode, can_download, preserve_view) = if v2 {
        (
            Mode::parse(field("mode").and_then(|v| v.as_string()).as_deref()),
            flag("canDownload").unwrap_or(true),
            flag("preserveView").unwrap_or(false),
        )
    } else {
        (Mode::Edit, true, false)
    };
    Some(OpenRequest { doc_id: key, version, title: text("title"), format: text("format"), bytes, mode, can_download, preserve_view })
}

/// A non-negative number as a `u64`.
fn whole(value: &JsValue) -> Option<u64> {
    value.as_f64().filter(|v| v.is_finite() && *v >= 0.0).map(|v| v as u64)
}

fn id_key(id: &JsValue) -> Option<String> {
    id.as_string().filter(|s| !s.is_empty()).or_else(|| id.as_f64().filter(|n| n.is_finite()).map(|n| n.to_string()))
}

fn call(target: &JsValue, method: &str, args: &Array) -> Result<JsValue, JsValue> {
    let function: Function = Reflect::get(target, &JsValue::from_str(method))?.dyn_into()?;
    function.apply(target, args)
}

struct JsHost {
    host: JsValue,
    ids: Ids,
}

impl JsHost {
    fn id(&self, doc_id: &str) -> JsValue {
        self.ids.try_borrow().ok().and_then(|ids| ids.get(doc_id).cloned()).unwrap_or_else(|| JsValue::from_str(doc_id))
    }

    fn notify(&self, method: &str, args: &Array) {
        if call(&self.host, method, args).is_err() {
            log::warn!("erpHost.{method} failed");
        }
    }
}

impl HostApi for JsHost {
    fn save(&mut self, doc_id: &str, bytes: Vec<u8>, revision: u64, reason: Reason, done: Box<dyn FnOnce(Outcome)>) {
        let options = Object::new();
        let _ = Reflect::set(&options, &JsValue::from_str("revision"), &JsValue::from_f64(revision as f64));
        let _ = Reflect::set(&options, &JsValue::from_str("reason"), &JsValue::from_str(reason.as_str()));
        // A copy the shell owns, never a view into wasm memory (which moves when it grows).
        let result = call(&self.host, "save", &Array::of3(&self.id(doc_id), &Uint8Array::from(bytes.as_slice()), &options));
        drop(bytes);
        // Answered from a task even when the call failed at once: `done` never runs inside `save`.
        wasm_bindgen_futures::spawn_local(async move {
            let answer = match result.map(|v| v.dyn_into::<Promise>()) {
                Ok(Ok(promise)) => wasm_bindgen_futures::JsFuture::from(promise).await.ok(),
                _ => None,
            };
            let outcome = match answer {
                Some(result) => read_result(&result),
                None => {
                    log::warn!("erpHost.save didn't answer with a SaveResult");
                    Outcome::Retry
                }
            };
            done(outcome);
        });
    }

    fn dirty(&mut self, doc_id: &str, dirty: bool) {
        self.notify("dirty", &Array::of2(&self.id(doc_id), &JsValue::from_bool(dirty)));
    }

    fn opened(&mut self, doc_id: &str, ok: bool, reason: Option<&str>) {
        let args = Array::of2(&self.id(doc_id), &JsValue::from_bool(ok));
        if let Some(reason) = reason {
            args.push(&JsValue::from_str(reason));
        }
        self.notify("opened", &args);
    }

    fn closed(&mut self, doc_id: &str) {
        self.notify("closed", &Array::of1(&self.id(doc_id)));
        if let Ok(mut ids) = self.ids.try_borrow_mut() {
            ids.remove(doc_id);
        }
    }
}

fn read_result(result: &JsValue) -> Outcome {
    let field = |key: &str| Reflect::get(result, &JsValue::from_str(key)).ok();
    outcome::outcome(
        field("status").and_then(|v| v.as_string()).as_deref(),
        field("version").and_then(|v| v.as_f64()),
        field("retryable").and_then(|v| v.as_bool()),
        field("message").and_then(|v| v.as_string()),
    )
}
