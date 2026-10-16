//! The browser shell: web `Services`, drag-and-drop, the unsaved-changes guard and the eframe
//! web runner.

use std::cell::Cell;
use std::rc::Rc;

use deckcraft_engine::Session;
use deckcraft_ui_egui::{Inbox, SaveDone, SaveOutcome, Services, SlideApp};
use wasm_bindgen::JsCast as _;

const PRESENTATION_EXTS: &[&str] = &["deckcraft", "pptx", "potx", "ppsx", "txt", "md"];
const PICTURE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"];
const AUDIO_EXTS: &[&str] = &["wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "aif", "aiff"];
const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mov", "webm"];
const CANVAS_ID: &str = "deckcraft_canvas";
const LOADING_ID: &str = "deckcraft_loading";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let dirty = Rc::new(Cell::new(false));
        if let Err(e) = guard_unload(dirty.clone()) {
            log::error!("no unsaved-changes guard: {e}");
        }
        let mut options = eframe::WebOptions::default();
        if query().contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    if let Some(rs) = &cc.wgpu_render_state {
                        log::info!("deckcraft-web {}: wgpu backend {:?}", env!("CARGO_PKG_VERSION"), rs.adapter.get_info().backend);
                    }
                    let inbox: Inbox = Inbox::default();
                    let mut app = SlideApp::new(Session::new(), services(inbox.clone(), cc.egui_ctx.clone()));
                    let q = query();
                    if !q.contains("blank") {
                        if let Err(e) = deckcraft_engine::sample::open_sample(&mut app.session) {
                            log::error!("sample deck: {e}");
                        }
                    } else if let Err(e) = app.run("file.new", serde_json::json!({})) {
                        log::error!("new presentation: {e}");
                    }
                    if q.contains("show") {
                        app.start_show(0, false);
                    }
                    Ok(Box::new(WebShell { app, inbox, dirty }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>DeckCraft failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

/// Wraps the app to read dropped files asynchronously (browsers can't read them synchronously)
/// and feed them through the inbox.
struct WebShell {
    app: SlideApp,
    inbox: Inbox,
    /// Read by the `beforeunload` guard, which can't reach the app.
    dirty: Rc<Cell<bool>>,
}

impl eframe::App for WebShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        for f in dropped {
            let inbox = self.inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped".into());
                match f.bytes_async().await {
                    Ok(bytes) => {
                        inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                        ctx.request_repaint();
                    }
                    Err(e) => log::error!("couldn't read dropped file {name}: {e}"),
                }
            });
        }
        self.app.logic(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        // After `ui`, where typing edits the presentation, so the guard is never a frame behind.
        self.dirty.set(self.app.has_unsaved_changes());
    }
}

/// Web services: no file paths, so `pick_open`/`pick_save`/`read` stay `None` and the UI falls
/// back to `open_async` (browser picker → inbox), `save_file` (Save) and `download` (exports).
fn services(inbox: Inbox, ctx: egui::Context) -> Services {
    let open_inbox = inbox.clone();
    let save_ctx = ctx.clone();
    Services {
        open_async: Some(Box::new(move |purpose: &str| {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            let dialog = match purpose {
                "picture" => rfd::AsyncFileDialog::new().add_filter("Pictures", PICTURE_EXTS),
                "audio" => rfd::AsyncFileDialog::new().add_filter("Audio", AUDIO_EXTS),
                "video" => rfd::AsyncFileDialog::new().add_filter("Video", VIDEO_EXTS),
                _ => rfd::AsyncFileDialog::new().add_filter("Presentations", PRESENTATION_EXTS),
            };
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = dialog.pick_file().await else {
                    return;
                };
                let bytes = file.read().await;
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((file.file_name(), bytes));
                ctx.request_repaint();
            });
        })),
        write: Some(Box::new(|name: &str, bytes: &[u8]| download(name, bytes))),
        download: Some(Box::new(|name: &str, bytes: &[u8]| {
            if let Err(e) = download(name, bytes) {
                log::error!("download of {name} failed: {e}");
            }
        })),
        save_file: Some(Box::new(move |name: &str, bytes: &[u8], done: SaveDone| save_file(&save_ctx, name, bytes, done))),
        inbox: Some(inbox),
        ..Default::default()
    }
}

/// Closing, reloading or leaving the tab with unsaved changes asks first. Browsers show their own
/// confirmation (a page can only ask for it), so there is no Save button: Cancel, then File › Save.
fn guard_unload(dirty: Rc<Cell<bool>>) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let on_unload = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(move |e: web_sys::BeforeUnloadEvent| {
        if dirty.get() {
            e.prevent_default();
            // Older browsers ask only when `returnValue` is set.
            e.set_return_value("unsaved changes");
        }
    });
    window.add_event_listener_with_callback("beforeunload", on_unload.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
    // The listener lives as long as the page.
    on_unload.forget();
    Ok(())
}

/// Save and Save As: the browser's save picker where there is one (Chromium), reporting through
/// `done` once the file is written; elsewhere a download.
fn save_file(ctx: &egui::Context, path: &str, bytes: &[u8], done: SaveDone) {
    let name = file_name(path);
    let picker = web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &wasm_bindgen::JsValue::from_str("showSaveFilePicker")).ok())
        .and_then(|v| v.dyn_into::<js_sys::Function>().ok());
    if let Some(picker) = picker {
        match open_save_picker(&picker, &name) {
            Ok(handle) => {
                let bytes = bytes.to_vec();
                let ctx = ctx.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let outcome = match wasm_bindgen_futures::JsFuture::from(handle).await {
                        Ok(handle) => match write_file_handle(&handle, &bytes).await {
                            Ok(()) => SaveOutcome::Saved,
                            Err(e) => SaveOutcome::Failed(e),
                        },
                        Err(e) if js_error_name(&e) == "AbortError" => SaveOutcome::Cancelled,
                        Err(e) => SaveOutcome::Failed(js_error_message(&e)),
                    };
                    if let SaveOutcome::Failed(e) = &outcome {
                        log::error!("couldn't save {name}: {e}");
                    }
                    done(outcome);
                    ctx.request_repaint();
                });
                return;
            }
            Err(e) => log::warn!("showSaveFilePicker failed: {e}; downloading instead"),
        }
    }
    // Firefox and Safari: the browser never says whether the download finished; without counting
    // it as saved the tab would warn about unsaved changes the user just downloaded, forever.
    done(match download(&name, bytes) {
        Ok(()) => SaveOutcome::Saved,
        Err(e) => SaveOutcome::Failed(e),
    });
}

/// Opens the save picker for `name` (it must be called while the click or key that asked for the
/// save still counts as a user gesture); the promise resolves to the chosen file's handle.
fn open_save_picker(picker: &js_sys::Function, name: &str) -> Result<js_sys::Promise, String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let key = |k: &str| wasm_bindgen::JsValue::from_str(k);
    let ext = name.rsplit('.').next().map(str::to_ascii_lowercase).unwrap_or_default();
    let exts = js_sys::Array::of1(&key(&format!(".{ext}")));
    let accept = js_sys::Object::new();
    js_sys::Reflect::set(&accept, &key(mime_for(name)), &exts).map_err(js)?;
    let description = match ext.as_str() {
        "pptx" => "PPTX presentation",
        "deckcraft" => "DeckCraft presentation",
        _ => "Presentation",
    };
    let file_type = js_sys::Object::new();
    js_sys::Reflect::set(&file_type, &key("description"), &key(description)).map_err(js)?;
    js_sys::Reflect::set(&file_type, &key("accept"), &accept).map_err(js)?;
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &key("suggestedName"), &key(name)).map_err(js)?;
    js_sys::Reflect::set(&options, &key("types"), &js_sys::Array::of1(&file_type)).map_err(js)?;
    let window = web_sys::window().ok_or("no window")?;
    picker.call1(&window, &options).map_err(js)?.dyn_into::<js_sys::Promise>().map_err(|_| "not a promise".to_string())
}

/// Writes `bytes` to a picked file and closes it; only then is the file on disk.
async fn write_file_handle(handle: &wasm_bindgen::JsValue, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| js_error_message(&e);
    let method = |target: &wasm_bindgen::JsValue, name: &str| -> Result<js_sys::Function, String> {
        js_sys::Reflect::get(target, &wasm_bindgen::JsValue::from_str(name)).map_err(js)?.dyn_into().map_err(|_| format!("{name} is not a function"))
    };
    let promise = |v: wasm_bindgen::JsValue| v.dyn_into::<js_sys::Promise>().map_err(|_| "not a promise".to_string());
    let stream = method(handle, "createWritable")?.call0(handle).map_err(js)?;
    let stream = wasm_bindgen_futures::JsFuture::from(promise(stream)?).await.map_err(js)?;
    let written = method(&stream, "write")?.call1(&stream, &js_sys::Uint8Array::from(bytes)).map_err(js)?;
    wasm_bindgen_futures::JsFuture::from(promise(written)?).await.map_err(js)?;
    let closed = method(&stream, "close")?.call0(&stream).map_err(js)?;
    wasm_bindgen_futures::JsFuture::from(promise(closed)?).await.map_err(js)?;
    Ok(())
}

/// A DOMException's `name` (`AbortError` when the user cancels the picker).
fn js_error_name(e: &wasm_bindgen::JsValue) -> String {
    js_sys::Reflect::get(e, &wasm_bindgen::JsValue::from_str("name")).ok().and_then(|v| v.as_string()).unwrap_or_default()
}

fn js_error_message(e: &wasm_bindgen::JsValue) -> String {
    js_sys::Reflect::get(e, &wasm_bindgen::JsValue::from_str("message"))
        .ok()
        .and_then(|v| v.as_string())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("{e:?}"))
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "deckcraft".into())
}

/// Trigger a browser download of `bytes` named after the last component of `path`.
fn download(path: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "deckcraft".into());
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime_for(&name));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(&name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    // Revoke after the click has been dispatched; the download keeps its own reference.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        web_sys::Url::revoke_object_url(&url).ok();
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
    Ok(())
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("deckcraft") => "application/zip",
        _ => "application/octet-stream",
    }
}
