//! `erpHost` v3, region co-editing: the app's calls into the shell (`regionWillEdit`,
//! `regionChanged`, `regionDidEndEdit`, `selectionChanged`) and the shell's handlers, queued for
//! the next frame ([`deckcraft_ui_egui::live`]).

use deckcraft_ui_egui::live::{Choice, Cursor, LiveEvent, LiveInbox, LockView, Member, RegionHost};
use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};

/// Whether the shell offers region co-editing (detected by feature: `erpHost.version` stays 2).
pub(super) fn offered(host: &JsValue) -> bool {
    Reflect::get(host, &JsValue::from_str("regionWillEdit")).is_ok_and(|f| f.is_function())
}

pub(super) struct JsRegionHost {
    host: JsValue,
    ctx: egui::Context,
}

impl JsRegionHost {
    pub(super) fn new(host: JsValue, ctx: egui::Context) -> Self {
        JsRegionHost { host, ctx }
    }

    fn call(&self, method: &str, args: &[&str]) -> Result<JsValue, JsValue> {
        let f: Function = Reflect::get(&self.host, &JsValue::from_str(method))?.dyn_into()?;
        let args: Array = args.iter().map(|a| JsValue::from_str(a)).collect();
        f.apply(&self.host, &args)
    }

    fn notify(&self, method: &str, args: &[&str]) {
        if self.call(method, args).is_err() {
            log::warn!("erpHost.{method} failed");
        }
    }
}

impl RegionHost for JsRegionHost {
    fn region_will_edit(&self, region: &str, reply: Box<dyn FnOnce(bool)>) {
        let called = self.call("regionWillEdit", &[region]);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            // A throw, a rejection or anything but `true` is a refusal: the change is taken back.
            let granted = match called {
                Ok(value) => wasm_bindgen_futures::JsFuture::from(Promise::resolve(&value)).await.is_ok_and(|v| v.as_bool() == Some(true)),
                Err(_) => false,
            };
            reply(granted);
            ctx.request_repaint();
        });
    }

    fn region_changed(&self, region: &str, payload: &str) {
        self.notify("regionChanged", &[region, payload]);
    }

    fn region_did_end_edit(&self, region: &str) {
        self.notify("regionDidEndEdit", &[region]);
    }

    fn selection_changed(&self, region: &str, range: &str) {
        self.notify("selectionChanged", &[region, range]);
    }

    fn now_ms(&self) -> f64 {
        js_sys::Date::now()
    }
}

fn field(value: &JsValue, key: &str) -> Option<JsValue> {
    Reflect::get(value, &JsValue::from_str(key)).ok().filter(|v| !v.is_undefined() && !v.is_null())
}

fn text(value: &JsValue, key: &str) -> Option<String> {
    field(value, key).and_then(|v| v.as_string())
}

fn flag(value: &JsValue, key: &str) -> bool {
    field(value, key).and_then(|v| v.as_bool()) == Some(true)
}

/// The entries of a v3 list: an array, or an object whose keys name its entries.
fn entries(value: &JsValue) -> Vec<(Option<String>, JsValue)> {
    if let Some(array) = value.dyn_ref::<Array>() {
        return array.iter().filter(JsValue::is_object).map(|e| (None, e)).collect();
    }
    if !value.is_object() {
        return Vec::new();
    }
    Object::entries(value.unchecked_ref())
        .iter()
        .filter_map(|pair| {
            let pair: Array = pair.dyn_into().ok()?;
            let entry = pair.get(1);
            entry.is_object().then(|| (pair.get(0).as_string(), entry))
        })
        .collect()
}

/// `showPresence`'s `[{ id, name, colour, you, … }]`.
fn members(value: &JsValue) -> Vec<Member> {
    entries(value)
        .iter()
        .filter_map(|(key, m)| {
            Some(Member { id: text(m, "id").or_else(|| key.clone())?, name: text(m, "name"), colour: text(m, "colour"), me: flag(m, "you") })
        })
        .collect()
}

/// `showLocks`' `[{ region, holder, name, colour, mine }]`.
fn locks(value: &JsValue) -> Vec<LockView> {
    entries(value)
        .iter()
        .filter_map(|(key, l)| {
            Some(LockView {
                region: text(l, "region").or_else(|| key.clone())?,
                holder: text(l, "holder").unwrap_or_default(),
                colour: text(l, "colour"),
                name: text(l, "name"),
                mine: flag(l, "mine"),
            })
        })
        .collect()
}

/// `showCursors`' `[{ author, name, colour, region, range }]`.
fn cursors(value: &JsValue) -> Vec<Cursor> {
    entries(value)
        .iter()
        .filter_map(|(key, c)| {
            Some(Cursor {
                author: text(c, "author").or_else(|| key.clone()).unwrap_or_default(),
                region: text(c, "region")?,
                range: text(c, "range").unwrap_or_default(),
                name: text(c, "name"),
                colour: text(c, "colour"),
            })
        })
        .collect()
}

/// The v3 handlers for `ready`: `applyRemote`, `showPresence`, `showLocks`, `showCursors`,
/// `lockDenied`, `isLeader` and `regionConflict`.
pub(super) fn handlers(inbox: &LiveInbox, ctx: &egui::Context) -> Vec<(&'static str, JsValue)> {
    let queue = |inbox: &LiveInbox, ctx: &egui::Context| {
        let (inbox, ctx) = (inbox.clone(), ctx.clone());
        move |event: Option<LiveEvent>| {
            if let Some(event) = event {
                match inbox.try_borrow_mut() {
                    Ok(mut q) => q.push(event),
                    Err(_) => log::warn!("erpHost: busy, a call was dropped"),
                }
            }
            ctx.request_repaint();
        }
    };
    let push = queue(inbox, ctx);
    let apply_remote = Closure::<dyn FnMut(JsValue, JsValue, JsValue)>::new(move |region: JsValue, payload: JsValue, author: JsValue| {
        push(region.as_string().zip(payload.as_string()).map(|(region, payload)| LiveEvent::ApplyRemote {
            region,
            payload,
            author: author.as_string(),
        }));
    })
    .into_js_value();
    let push = queue(inbox, ctx);
    let show_presence = Closure::<dyn FnMut(JsValue)>::new(move |v: JsValue| push(Some(LiveEvent::Presence(members(&v))))).into_js_value();
    let push = queue(inbox, ctx);
    let show_locks = Closure::<dyn FnMut(JsValue)>::new(move |v: JsValue| push(Some(LiveEvent::Locks(locks(&v))))).into_js_value();
    let push = queue(inbox, ctx);
    let show_cursors = Closure::<dyn FnMut(JsValue)>::new(move |v: JsValue| push(Some(LiveEvent::Cursors(cursors(&v))))).into_js_value();
    let push = queue(inbox, ctx);
    let lock_denied = Closure::<dyn FnMut(JsValue)>::new(move |region: JsValue| push(region.as_string().map(LiveEvent::LockDenied))).into_js_value();
    let push = queue(inbox, ctx);
    let is_leader =
        Closure::<dyn FnMut(JsValue)>::new(move |flag: JsValue| push(Some(LiveEvent::Leader(flag.as_bool() == Some(true))))).into_js_value();
    let push = queue(inbox, ctx);
    let region_conflict = Closure::<dyn FnMut(JsValue) -> Promise>::new(move |region: JsValue| {
        let region = region.as_string().unwrap_or_default();
        Promise::new(&mut |resolve: Function, _reject: Function| {
            let reply = Box::new(move |choice: Choice| {
                let _ = resolve.call1(&JsValue::UNDEFINED, &JsValue::from_str(choice.as_str()));
            });
            push(Some(LiveEvent::Conflict { region: region.clone(), reply }));
        })
    })
    .into_js_value();
    vec![
        ("applyRemote", apply_remote),
        ("showPresence", show_presence),
        ("showLocks", show_locks),
        ("showCursors", show_cursors),
        ("lockDenied", lock_denied),
        ("isLeader", is_leader),
        ("regionConflict", region_conflict),
    ]
}
