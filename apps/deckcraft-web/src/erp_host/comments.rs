//! The `erpHost` comment calls (M5): the shell's handlers `anchorOfSelection`, `showComments`,
//! `focusAnchor` and `resolveAnchors`, and the app's call `commentClicked`
//! ([`deckcraft_ui_egui::comments`]). Anchors cross as JavaScript objects and are JSON strings on
//! the app's side.

use deckcraft_ui_egui::comments::{CommentEvent, CommentHost, CommentInbox, ResolvedThread, SelectionAnchor, Thread};
use js_sys::{Array, Function, JSON, Object, Promise, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};

/// Whether the shell has Cloud comments (detected by feature, like v3 regions).
pub(super) fn offered(host: &JsValue) -> bool {
    Reflect::get(host, &JsValue::from_str("commentClicked")).is_ok_and(|f| f.is_function())
}

pub(super) struct JsCommentHost {
    host: JsValue,
}

impl JsCommentHost {
    pub(super) fn new(host: JsValue) -> Self {
        JsCommentHost { host }
    }
}

impl CommentHost for JsCommentHost {
    fn comment_clicked(&self, thread: &str) {
        // A shell without the call gets nothing.
        let Ok(f) = Reflect::get(&self.host, &JsValue::from_str("commentClicked")).and_then(|f| f.dyn_into::<Function>()) else { return };
        if f.call1(&self.host, &JsValue::from_str(thread)).is_err() {
            log::warn!("erpHost.commentClicked failed");
        }
    }
}

fn field(value: &JsValue, key: &str) -> Option<JsValue> {
    Reflect::get(value, &JsValue::from_str(key)).ok().filter(|v| !v.is_undefined() && !v.is_null())
}

/// An anchor as JSON: a string stays as it is, an object is stringified.
fn anchor_json(value: &JsValue) -> Option<String> {
    if let Some(s) = value.as_string() {
        return Some(s);
    }
    value.is_object().then(|| JSON::stringify(value).ok().and_then(|s| s.as_string())).flatten()
}

/// An id the shell may give as a string or a number.
fn id(value: &JsValue) -> Option<String> {
    value.as_string().or_else(|| value.as_f64().filter(|n| n.is_finite()).map(|n| n.to_string())).filter(|s| !s.is_empty())
}

/// `[{ id, anchor, status, count }]`; entries without an id or an anchor are skipped.
fn threads(value: &JsValue) -> Vec<Thread> {
    let Some(array) = value.dyn_ref::<Array>() else { return Vec::new() };
    array
        .iter()
        .filter_map(|t| {
            Some(Thread {
                id: id(&field(&t, "id")?)?,
                anchor: anchor_json(&field(&t, "anchor")?)?,
                status: field(&t, "status").and_then(|s| s.as_string()).unwrap_or_default(),
                count: field(&t, "count")
                    .and_then(|c| c.as_f64())
                    .filter(|c| c.is_finite() && *c >= 0.0)
                    .map_or(0, |c| c.min(f64::from(u32::MAX)) as u32),
            })
        })
        .collect()
}

/// An anchor's JSON back as an object; a string that won't parse stays a string.
fn parsed(json: &str) -> JsValue {
    JSON::parse(json).unwrap_or_else(|_| JsValue::from_str(json))
}

fn resolved(list: Vec<ResolvedThread>) -> JsValue {
    let out = Array::new();
    for r in list {
        let o = Object::new();
        let _ = Reflect::set(&o, &JsValue::from_str("id"), &JsValue::from_str(&r.id));
        let _ = Reflect::set(&o, &JsValue::from_str("anchor"), &parsed(&r.anchor));
        let _ = Reflect::set(&o, &JsValue::from_str("status"), &JsValue::from_str(r.status.as_str()));
        out.push(&o);
    }
    out.into()
}

/// The comment handlers for `ready`.
pub(super) fn handlers(inbox: &CommentInbox, selection: &SelectionAnchor, ctx: &egui::Context) -> Vec<(&'static str, JsValue)> {
    let queue = |inbox: &CommentInbox, ctx: &egui::Context| {
        let (inbox, ctx) = (inbox.clone(), ctx.clone());
        move |event: CommentEvent| {
            match inbox.try_borrow_mut() {
                Ok(mut q) => q.push(event),
                Err(_) => log::warn!("erpHost: busy, a call was dropped"),
            }
            ctx.request_repaint();
        }
    };
    let selection = selection.clone();
    // Answered at once from the last frame: the shell may ask while its own button has the focus.
    let anchor_of_selection = Closure::<dyn FnMut() -> JsValue>::new(move || {
        selection.try_borrow().ok().and_then(|s| s.clone()).map_or(JsValue::NULL, |json| parsed(&json))
    })
    .into_js_value();
    let push = queue(inbox, ctx);
    let show_comments = Closure::<dyn FnMut(JsValue)>::new(move |list: JsValue| push(CommentEvent::Show(threads(&list)))).into_js_value();
    let push = queue(inbox, ctx);
    let focus_anchor = Closure::<dyn FnMut(JsValue)>::new(move |anchor: JsValue| {
        if let Some(json) = anchor_json(&anchor) {
            push(CommentEvent::Focus(json));
        }
    })
    .into_js_value();
    let push = queue(inbox, ctx);
    let resolve_anchors = Closure::<dyn FnMut(JsValue) -> Promise>::new(move |list: JsValue| {
        let threads = threads(&list);
        Promise::new(&mut |resolve: Function, _reject: Function| {
            let reply = Box::new(move |list: Vec<ResolvedThread>| {
                let _ = resolve.call1(&JsValue::UNDEFINED, &resolved(list));
            });
            push(CommentEvent::Resolve { threads: threads.clone(), reply });
        })
    })
    .into_js_value();
    vec![
        ("anchorOfSelection", anchor_of_selection),
        ("showComments", show_comments),
        ("focusAnchor", focus_anchor),
        ("resolveAnchors", resolve_anchors),
    ]
}
