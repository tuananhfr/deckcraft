//! Browser clipboard transport. Rich content stays out of the page DOM.

const RICH_LIMIT: usize = 8 * 1024 * 1024;
const IMAGE_LIMIT: usize = 20 * 1024 * 1024;
const IMAGE_COUNT: usize = 20;

#[derive(Default, Debug)]
pub struct PendingPaste {
    pub text: String,
    pub html: Option<String>,
    pub rtf: Option<String>,
    pub images: Vec<(String, Vec<u8>)>,
}

impl PendingPaste {
    fn take_image(&mut self) -> Option<Vec<u8>> {
        std::mem::take(&mut self.images).into_iter().next().map(|(_, bytes)| bytes)
    }

    fn over_cap(&self) -> bool {
        self.html.as_ref().is_some_and(|s| s.len() > RICH_LIMIT)
            || self.rtf.as_ref().is_some_and(|s| s.len() > RICH_LIMIT)
            || self.images.len() > IMAGE_COUNT
            || self.images.iter().any(|(_, bytes)| bytes.len() > IMAGE_LIMIT)
    }

    fn bounded(mut self) -> Self {
        self.html = self.html.filter(|s| s.len() <= RICH_LIMIT);
        self.rtf = self.rtf.filter(|s| s.len() <= RICH_LIMIT);
        self.images.truncate(IMAGE_COUNT);
        self.images.retain(|(mime, bytes)| mime.starts_with("image/") && bytes.len() <= IMAGE_LIMIT);
        if self.text.is_empty() {
            self.text = self.html.as_deref().map(html_to_plain).unwrap_or_default();
        }
        self
    }
}

pub fn paste_events(pending: &PendingPaste) -> Vec<egui::Event> {
    // A textless internal shape still needs the engine paste path; image-only data uses pictures.
    if pending.text.is_empty() && (!pending.images.is_empty() || pending.html.is_none()) {
        Vec::new()
    } else {
        vec![egui::Event::Paste(pending.text.clone())]
    }
}

fn advance_frame(inbox: &mut Option<PendingPaste>, current: &mut Option<PendingPaste>) -> Vec<egui::Event> {
    *current = inbox.take();
    current.as_ref().map(paste_events).unwrap_or_default()
}

fn native_html_paste(tag: &str, content_editable: bool, egui_text_agent: bool) -> bool {
    !egui_text_agent && (matches!(tag, "INPUT" | "TEXTAREA") || content_editable)
}

/// A bounded, non-rendering fallback; script, style and document metadata are not text.
fn html_to_plain(html: &str) -> String {
    if html.len() > RICH_LIMIT {
        return String::new();
    }
    let mut out = String::new();
    let mut chars = html.chars().peekable();
    let mut hidden: Option<String> = None;
    while let Some(c) = chars.next() {
        if c == '<' {
            if hidden.is_some() && chars.peek() != Some(&'/') {
                continue;
            }
            if chars.clone().take(3).eq("!--".chars()) {
                for _ in 0..3 {
                    chars.next();
                }
                let mut dashes = 0_u8;
                for c in chars.by_ref() {
                    if c == '>' && dashes >= 2 {
                        break;
                    }
                    dashes = if c == '-' { dashes.saturating_add(1) } else { 0 };
                }
                continue;
            }
            let mut tag = String::new();
            let mut quote = None;
            for c in chars.by_ref() {
                if let Some(q) = quote {
                    if c == q {
                        quote = None;
                    }
                } else if c == '\'' || c == '"' {
                    quote = Some(c);
                } else if c == '>' {
                    break;
                }
                tag.push(c);
            }
            let tag = tag.trim().to_ascii_lowercase();
            let name = tag.split(|c: char| c.is_whitespace() || c == '/').find(|s| !s.is_empty()).unwrap_or("");
            let closing = tag.starts_with('/');
            if let Some(ref h) = hidden {
                if closing && name == h.as_str() {
                    hidden = None;
                }
                continue;
            }
            if !closing && matches!(name, "script" | "style" | "head" | "title") {
                hidden = Some(name.to_owned());
            } else if matches!(name, "p" | "div" | "br" | "li" | "tr" | "h1" | "h2" | "h3" | "blockquote") && !out.is_empty() && !out.ends_with('\n')
            {
                out.push('\n');
            } else if closing && matches!(name, "td" | "th") && !out.ends_with(['\n', '\t']) {
                out.push('\t');
            }
        } else if hidden.is_none() && c == '&' {
            let mut entity = String::new();
            while entity.len() < 32 && chars.peek().is_some_and(|c| c.is_ascii_alphanumeric() || *c == '#') {
                if let Some(c) = chars.next() {
                    entity.push(c);
                }
            }
            let terminated = chars.peek() == Some(&';');
            if terminated {
                chars.next();
            }
            let decoded = match entity.as_str() {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                "ndash" => Some('\u{2013}'),
                "mdash" => Some('\u{2014}'),
                "hellip" => Some('\u{2026}'),
                value => value
                    .strip_prefix("#x")
                    .or_else(|| value.strip_prefix("#X"))
                    .and_then(|n| u32::from_str_radix(n, 16).ok())
                    .or_else(|| value.strip_prefix('#').and_then(|n| n.parse().ok()))
                    .and_then(char::from_u32),
            };
            if let Some(c) = decoded.filter(|_| terminated) {
                out.push(c);
            } else {
                out.push('&');
                out.push_str(&entity);
                if terminated {
                    out.push(';');
                }
            }
        } else if hidden.is_none() {
            out.push(c);
        }
    }
    out.trim().to_owned()
}

#[cfg(target_arch = "wasm32")]
pub use browser::ClipboardBridge;

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    use super::*;

    #[derive(Clone)]
    pub struct ClipboardBridge {
        inbox: Rc<RefCell<Option<PendingPaste>>>,
        current: Rc<RefCell<Option<PendingPaste>>>,
        notice: Rc<RefCell<Option<deckcraft_ui_egui::clipboard::ClipboardNotice>>>,
        generation: Rc<Cell<u64>>,
        ctx: egui::Context,
    }

    impl ClipboardBridge {
        pub fn install(ctx: egui::Context) -> Result<Self, String> {
            let bridge = Self { inbox: Rc::default(), current: Rc::default(), notice: Rc::default(), generation: Rc::default(), ctx };
            let window = web_sys::window().ok_or("no window")?;
            let handler_bridge = bridge.clone();
            let handler = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(move |event: web_sys::ClipboardEvent| {
                if focused_html_editor() {
                    return;
                }
                let Some(data) = event.clipboard_data() else { return };
                let pending = PendingPaste {
                    text: data.get_data("text/plain").unwrap_or_default(),
                    html: data.get_data("text/html").ok().filter(|s| !s.is_empty()),
                    rtf: data.get_data("text/rtf").ok().filter(|s| !s.is_empty()),
                    images: Vec::new(),
                };
                let mut over_cap = pending.over_cap();
                let pending = pending.bounded();
                let mut files = Vec::new();
                if let Some(list) = data.files() {
                    for i in 0..list.length() {
                        if files.len() == IMAGE_COUNT {
                            over_cap = true;
                            break;
                        }
                        if let Some(file) = list.get(i)
                            && file.type_().starts_with("image/")
                        {
                            if file.size() <= IMAGE_LIMIT as f64 {
                                files.push(file);
                            } else {
                                over_cap = true;
                            }
                        }
                    }
                }
                event.prevent_default();
                event.stop_immediate_propagation();
                let bridge = handler_bridge.clone();
                if over_cap {
                    bridge.failed(deckcraft_ui_egui::clipboard::ClipboardNotice::TooLarge);
                }
                let generation = bridge.next_generation();
                if files.is_empty() {
                    bridge.deliver(generation, pending);
                } else {
                    spawn_local(async move {
                        let mut pending = pending;
                        for file in files {
                            if let Ok(bytes) = read_blob(file.as_ref(), IMAGE_LIMIT).await {
                                pending.images.push((file.type_(), bytes));
                            }
                        }
                        bridge.deliver(generation, pending);
                    });
                }
            });
            window
                .add_event_listener_with_callback_and_bool("paste", handler.as_ref().unchecked_ref(), true)
                .map_err(|_| "clipboard listener failed")?;
            handler.forget();
            Ok(bridge)
        }

        fn next_generation(&self) -> u64 {
            let next = self.generation.get().wrapping_add(1);
            self.generation.set(next);
            // A newer user gesture supersedes an unfinished asynchronous read.
            self.inbox.borrow_mut().take();
            next
        }

        fn deliver(&self, generation: u64, pending: PendingPaste) {
            if self.generation.get() == generation {
                *self.inbox.borrow_mut() = Some(pending.bounded());
                self.ctx.request_repaint();
            }
        }

        pub fn take(&self) -> Option<PendingPaste> {
            self.inbox.borrow_mut().take()
        }

        pub fn begin_frame(&self, raw: &mut egui::RawInput) {
            let mut pending = self.take();
            raw.events.extend(advance_frame(&mut pending, &mut self.current.borrow_mut()));
        }

        pub fn peek_html(&self) -> Option<String> {
            self.current.borrow().as_ref().and_then(|p| p.html.clone())
        }

        pub fn peek_text(&self) -> Option<String> {
            self.current.borrow().as_ref().map(|p| p.text.clone())
        }

        pub fn take_image(&self) -> Option<Vec<u8>> {
            self.current.borrow_mut().as_mut().and_then(PendingPaste::take_image)
        }

        pub fn image_only(&self) -> bool {
            self.current.borrow().as_ref().is_some_and(|p| p.text.is_empty() && !p.images.is_empty())
        }

        pub fn take_notice(&self) -> Option<deckcraft_ui_egui::clipboard::ClipboardNotice> {
            self.notice.borrow_mut().take()
        }

        fn failed(&self, notice: deckcraft_ui_egui::clipboard::ClipboardNotice) {
            *self.notice.borrow_mut() = Some(notice);
            self.ctx.request_repaint();
        }

        pub fn write(&self, html: &str, text: &str) -> bool {
            use deckcraft_ui_egui::clipboard::ClipboardNotice;
            if html.len() > RICH_LIMIT {
                self.failed(ClipboardNotice::TooLarge);
                return false;
            }
            let Some(window) = web_sys::window() else { return false };
            let Ok(constructor) = method(window.as_ref(), "ClipboardItem") else { return false };
            let result = (|| {
                let parts = js_sys::Object::new();
                for (mime, content) in [("text/html", html), ("text/plain", text)] {
                    let options = web_sys::BlobPropertyBag::new();
                    options.set_type(mime);
                    let blob = web_sys::Blob::new_with_str_sequence_and_options(&js_sys::Array::of1(&JsValue::from_str(content)), &options)?;
                    js_sys::Reflect::set(&parts, &JsValue::from_str(mime), &blob)?;
                }
                let item = js_sys::Reflect::construct(&constructor, &js_sys::Array::of1(&parts))?;
                let clipboard = clipboard()?;
                method(&clipboard, "write")?.call1(&clipboard, &js_sys::Array::of1(&item))?.dyn_into::<js_sys::Promise>()
            })();
            let bridge = self.clone();
            match result {
                Ok(promise) => spawn_local(async move {
                    if JsFuture::from(promise).await.is_err() {
                        bridge.failed(ClipboardNotice::WriteDenied);
                    }
                }),
                Err(_) => self.failed(ClipboardNotice::WriteDenied),
            }
            // Once rich writing is attempted, eframe must not overwrite it with writeText.
            true
        }

        pub fn request_read(&self) {
            let bridge = self.clone();
            let generation = self.next_generation();
            let promise = clipboard().and_then(|c| method(&c, "read")?.call0(&c)?.dyn_into::<js_sys::Promise>());
            spawn_local(async move {
                let result = match promise {
                    Ok(promise) => read_items(promise).await,
                    Err(e) => Err(e),
                };
                match result {
                    Ok((pending, over_cap)) => {
                        if bridge.generation.get() != generation {
                            return;
                        }
                        if over_cap {
                            bridge.failed(deckcraft_ui_egui::clipboard::ClipboardNotice::TooLarge);
                        }
                        bridge.deliver(generation, pending);
                    }
                    Err(_) if bridge.generation.get() == generation => bridge.failed(deckcraft_ui_egui::clipboard::ClipboardNotice::ReadDenied),
                    Err(_) => {}
                }
            });
        }
    }

    fn focused_html_editor() -> bool {
        let Some(element) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.active_element()) else { return false };
        // eframe 0.36's IME input has no id; it is inserted immediately after its canvas.
        let agent = element.tag_name() == "INPUT"
            && element.previous_element_sibling().is_some_and(|s| s.id() == "deckcraft_canvas" && s.tag_name() == "CANVAS")
            && element.get_attribute("autocapitalize").as_deref() == Some("off")
            && element.dyn_ref::<web_sys::HtmlElement>().is_some_and(|e| {
                e.style().get_property_value("position").ok().as_deref() == Some("absolute")
                    && e.style().get_property_value("width").ok().as_deref() == Some("1px")
                    && e.style().get_property_value("height").ok().as_deref() == Some("1px")
            });
        native_html_paste(&element.tag_name(), element.dyn_ref::<web_sys::HtmlElement>().is_some_and(|e| e.is_content_editable()), agent)
    }

    fn method(target: &JsValue, name: &str) -> Result<js_sys::Function, JsValue> {
        js_sys::Reflect::get(target, &JsValue::from_str(name))?.dyn_into()
    }

    fn clipboard() -> Result<JsValue, JsValue> {
        let window = web_sys::window().ok_or(JsValue::NULL)?;
        js_sys::Reflect::get(window.navigator().as_ref(), &JsValue::from_str("clipboard"))
    }

    async fn read_blob(value: &JsValue, limit: usize) -> Result<Vec<u8>, JsValue> {
        let blob = value.dyn_ref::<web_sys::Blob>().ok_or(JsValue::NULL)?;
        if blob.size() > limit as f64 {
            return Err(JsValue::NULL);
        }
        let buffer = JsFuture::from(blob.array_buffer()).await?;
        let array = js_sys::Uint8Array::new(&buffer);
        if array.length() as usize > limit {
            return Err(JsValue::NULL);
        }
        Ok(array.to_vec())
    }

    async fn read_items(promise: js_sys::Promise) -> Result<(PendingPaste, bool), JsValue> {
        let items = js_sys::Array::from(&JsFuture::from(promise).await?);
        let mut pending = PendingPaste::default();
        let mut over_cap = items.length() as usize > IMAGE_COUNT;
        for item in items.iter().take(IMAGE_COUNT) {
            let types = js_sys::Reflect::get(&item, &JsValue::from_str("types"))?;
            for mime in js_sys::Array::from(&types).iter().filter_map(|v| v.as_string()).take(64) {
                if mime.starts_with("image/") && pending.images.len() == IMAGE_COUNT {
                    over_cap = true;
                    continue;
                }
                let wanted = match mime.as_str() {
                    "text/plain" => pending.text.is_empty(),
                    "text/html" => pending.html.is_none(),
                    "text/rtf" => pending.rtf.is_none(),
                    _ => mime.starts_with("image/") && pending.images.len() < IMAGE_COUNT,
                };
                if !wanted {
                    continue;
                }
                let value = method(&item, "getType")?.call1(&item, &JsValue::from_str(&mime))?.dyn_into::<js_sys::Promise>()?;
                let blob = JsFuture::from(value).await?;
                let limit = if mime.starts_with("image/") { IMAGE_LIMIT } else { RICH_LIMIT };
                if blob.dyn_ref::<web_sys::Blob>().is_some_and(|b| b.size() > limit as f64) {
                    over_cap = true;
                    continue;
                }
                let Ok(bytes) = read_blob(&blob, limit).await else { continue };
                match mime.as_str() {
                    "text/plain" => pending.text = String::from_utf8_lossy(&bytes).into_owned(),
                    "text/html" => pending.html = String::from_utf8(bytes).ok(),
                    "text/rtf" => pending.rtf = String::from_utf8(bytes).ok(),
                    _ => pending.images.push((mime, bytes)),
                }
            }
        }
        Ok((pending.bounded(), over_cap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_html_editors_keep_native_paste_but_egui_ime_does_not() {
        assert!(native_html_paste("INPUT", false, false));
        assert!(native_html_paste("TEXTAREA", false, false));
        assert!(native_html_paste("DIV", true, false));
        assert!(!native_html_paste("INPUT", false, true));
        assert!(!native_html_paste("CANVAS", false, false));
    }

    #[test]
    fn html_fallback_strips_markup_and_decodes_entities_without_rendering() {
        assert_eq!(html_to_plain("<head><style>bad</style></head><p title='>'>A &amp; B &#x1ec7;</p><p>&lt;C&gt;&#7871;</p>"), "A & B ệ\n<C>ế");
        assert_eq!(html_to_plain("<script>bad()</script><p>safe</p><style>bad</style>"), "safe");
        assert_eq!(html_to_plain("<!-- > hidden <p>comment</p> --><script>if (a < b) bad()</script><p>safe</p>"), "safe");
        assert_eq!(html_to_plain("<meta charset='utf-8'><img src='https://invalid.test'>"), "");
        assert_eq!(html_to_plain("&unknown; &#x110000; &amp"), "&unknown; &#x110000; &amp");
    }

    #[test]
    fn caps_drop_parts_and_keep_plain_fallback() {
        let pending = PendingPaste {
            text: "fallback".into(),
            html: Some("x".repeat(RICH_LIMIT + 1)),
            rtf: Some("x".repeat(RICH_LIMIT + 1)),
            images: vec![("image/png".into(), vec![0; IMAGE_LIMIT + 1])],
        };
        assert!(pending.over_cap());
        let p = pending.bounded();
        assert_eq!(p.text, "fallback");
        assert!(p.html.is_none() && p.rtf.is_none() && p.images.is_empty());
        assert!(!p.over_cap());
        let p = PendingPaste { images: vec![("image/png".into(), vec![1]); IMAGE_COUNT + 1], ..Default::default() }.bounded();
        assert_eq!(p.images.len(), IMAGE_COUNT);
        let p = PendingPaste { html: Some("<p>only HTML</p>".into()), ..Default::default() }.bounded();
        assert_eq!(p.text, "only HTML");
    }

    #[test]
    fn one_paste_event_and_no_image_only_event() {
        let p = PendingPaste { text: "once".into(), html: Some("<p>once</p>".into()), ..Default::default() };
        assert_eq!(paste_events(&p), vec![egui::Event::Paste("once".into())]);
        let p = PendingPaste { images: vec![("image/png".into(), vec![1])], ..Default::default() };
        assert!(paste_events(&p).is_empty());
        assert!(paste_events(&PendingPaste::default()).is_empty());
        let p = PendingPaste { html: Some("<meta charset='utf-8'><p></p>".into()), ..Default::default() };
        assert_eq!(paste_events(&p), vec![egui::Event::Paste(String::new())]);
    }

    #[test]
    fn rich_slot_expires_at_the_next_frame() {
        let mut inbox = Some(PendingPaste { text: "once".into(), html: Some("<p>once</p>".into()), ..Default::default() });
        let mut current = None;
        assert_eq!(advance_frame(&mut inbox, &mut current), vec![egui::Event::Paste("once".into())]);
        assert!(current.as_ref().and_then(|p| p.html.as_ref()).is_some());
        assert!(advance_frame(&mut inbox, &mut current).is_empty());
        assert!(current.is_none() && inbox.is_none());
    }

    #[test]
    fn only_the_first_image_is_consumed_once() {
        let mut p = PendingPaste { images: vec![("image/png".into(), vec![1]), ("image/jpeg".into(), vec![2])], ..Default::default() };
        assert_eq!(p.take_image(), Some(vec![1]));
        assert_eq!(p.take_image(), None);
    }
}
