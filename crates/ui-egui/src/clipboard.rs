//! Host clipboard services and the document paste path.

use serde_json::{Value, json};

use crate::SlideApp;

pub type SetClipboardHtmlFn = Box<dyn FnMut(&str, &str) -> bool>;

#[derive(Clone, Copy)]
pub enum ClipboardNotice {
    ReadDenied,
    WriteDenied,
    TooLarge,
}

impl ClipboardNotice {
    fn message(self) -> &'static str {
        match self {
            Self::ReadDenied => crate::tl!("The browser cannot read the clipboard — use Ctrl+V"),
            Self::WriteDenied => crate::tl!("Could not write to the clipboard"),
            Self::TooLarge => crate::tl!("Some clipboard content was too large and was skipped."),
        }
    }
}

impl SlideApp {
    pub(crate) fn request_clipboard_for_command(&mut self, id: &str, params: &Value) -> bool {
        if id == "edit.paste"
            && params.get("text").is_none()
            && params.get("html").is_none()
            && let Some(read) = self.services.request_clipboard.as_mut()
        {
            read();
            return true;
        }
        false
    }

    pub(crate) fn copy_command_result(&mut self, id: &str, result: &Result<Value, String>) {
        if matches!(id, "edit.copy" | "edit.cut")
            && self.services.set_clipboard_html.is_some()
            && let Ok(result) = result
        {
            self.write_clipboard(result);
        }
    }

    pub fn note_clipboard(&mut self, notice: ClipboardNotice) {
        let text = notice.message().to_owned();
        self.set_status(text.clone());
        self.toast = Some(crate::toast::Toast { text, at: crate::now_ms() });
    }

    pub(crate) fn write_clipboard(&mut self, result: &Value) {
        if let Some(text) = copy_text(result) {
            let html = result.get("html").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| plain_html(text));
            self.clipboard_fallback = None;
            if !self.write_rich_clipboard(&html, text) && !text.is_empty() {
                self.clipboard_fallback = Some(text.to_owned());
            }
        }
    }

    fn write_rich_clipboard(&mut self, html: &str, text: &str) -> bool {
        self.services.set_clipboard_html.as_mut().is_some_and(|write| write(html, text))
    }

    pub(crate) fn flush_clipboard(&mut self, ctx: &egui::Context) {
        if let Some(text) = self.clipboard_fallback.take() {
            ctx.copy_text(text);
        }
    }

    pub(crate) fn paste_clipboard(&mut self, text: String) {
        let html = self.services.clipboard_html.as_mut().and_then(|read| read());
        let text = self.services.clipboard_text.as_mut().and_then(|read| read()).unwrap_or(text);
        let image = if text.is_empty() { self.services.clipboard_image.as_mut().and_then(|read| read()) } else { None };
        let result = match image {
            Some(bytes) => {
                self.session.execute("insert.picture", &json!({"name": "Pasted image.png", "data": deckcraft_engine::cmd::base64_encode(&bytes)}))
            }
            None => self.session.execute("edit.paste", &json!({"text": text, "html": html})),
        };
        if let Err(e) = result {
            self.set_status(e.to_string());
        }
    }

    /// Image-only web pastes have no text event for egui's dialog text fields.
    pub fn paste_clipboard_image(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some())
            || self.dialog.as_ref().is_some_and(|d| d.modal())
            || self.palette.is_some()
            || self.backstage.is_some()
            || self.show.is_some()
        {
            return;
        }
        self.paste_clipboard(String::new());
    }
}

pub(crate) fn copy_text(result: &Value) -> Option<&str> {
    let text = result.get("text")?.as_str()?;
    let has_content = !text.is_empty() || result.get("html").and_then(Value::as_str).is_some_and(|html| !html.is_empty());
    if has_content && result.get("copied").and_then(Value::as_bool).unwrap_or(true) { Some(text) } else { None }
}

fn plain_html(text: &str) -> String {
    let mut html = String::from("<meta charset=\"utf-8\">");
    for line in text.split('\n') {
        html.push_str("<p>");
        for c in line.trim_end_matches('\r').chars() {
            match c {
                '&' => html.push_str("&amp;"),
                '<' => html.push_str("&lt;"),
                '>' => html.push_str("&gt;"),
                '"' => html.push_str("&quot;"),
                '\'' => html.push_str("&#39;"),
                _ => html.push(c),
            }
        }
        html.push_str("</p>");
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_text_is_escaped_in_separate_paragraphs() {
        assert_eq!(plain_html("<&\"'>\r\nsecond"), "<meta charset=\"utf-8\"><p>&lt;&amp;&quot;&#39;&gt;</p><p>second</p>");
    }

    #[test]
    fn clipboard_notices_have_vietnamese_and_english_text() {
        let vi = crate::i18n::Lang::from_pref("vi");
        for (notice, english, vietnamese) in [
            (
                ClipboardNotice::ReadDenied,
                "The browser cannot read the clipboard — use Ctrl+V",
                "Trình duyệt không cho đọc bảng tạm — hãy dùng Ctrl+V",
            ),
            (ClipboardNotice::WriteDenied, "Could not write to the clipboard", "Không ghi được vào bảng tạm"),
            (
                ClipboardNotice::TooLarge,
                "Some clipboard content was too large and was skipped.",
                "Một phần nội dung bảng tạm quá lớn nên đã được bỏ qua.",
            ),
        ] {
            crate::i18n::set_current(crate::i18n::Lang::EN);
            assert_eq!(notice.message(), english);
            crate::i18n::set_current(vi);
            assert_eq!(notice.message(), vietnamese);
        }
        crate::i18n::set_current(crate::i18n::Lang::EN);
    }
}
