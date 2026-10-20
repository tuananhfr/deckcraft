use deckcraft_ui_egui::{SaveOutcome, tl};

pub(crate) fn validate_file_name(name: &str) -> Result<(), SaveOutcome> {
    if deckcraft_engine::legacy_format::detect(name, &[]).is_some() {
        Err(SaveOutcome::Failed(tl!("Choose a .pptx file to preserve the legacy original.").into()))
    } else {
        Ok(())
    }
}

pub(crate) fn outcome(name: &str) -> SaveOutcome {
    if name == "AbortError" {
        SaveOutcome::Cancelled
    } else {
        SaveOutcome::Failed(message(name))
    }
}

pub(crate) fn message(name: &str) -> String {
    match name {
        "NotAllowedError" | "SecurityError" => tl!("The browser denied permission to save the file. Please allow access and try again."),
        _ => tl!("The browser couldn't save the file. Please try again."),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckcraft_ui_egui::i18n::{self, Lang};

    #[test]
    fn browser_save_errors_are_translated_without_exception_details() {
        i18n::set_current(Lang::from_code("vi").unwrap_or(Lang::EN));
        assert_eq!(outcome("AbortError"), SaveOutcome::Cancelled);
        for name in ["NotAllowedError", "SecurityError"] {
            assert_eq!(outcome(name), SaveOutcome::Failed("Trình duyệt không cho phép lưu tệp. Hãy cấp quyền truy cập và thử lại.".into()));
        }
        assert_eq!(outcome("JsValue(private path)"), SaveOutcome::Failed("Trình duyệt không lưu được tệp. Vui lòng thử lại.".into()));
        i18n::set_current(Lang::EN);
    }

    #[test]
    fn legacy_file_handles_are_rejected_before_writing() {
        i18n::set_current(Lang::from_code("vi").unwrap_or(Lang::EN));
        for name in ["slides.ppt", "slides.ODP", "slides.PPTM"] {
            assert_eq!(validate_file_name(name), Err(SaveOutcome::Failed("Hãy chọn tệp .pptx để giữ nguyên tệp gốc ở định dạng cũ.".into())));
        }
        assert_eq!(validate_file_name("slides.pptx"), Ok(()));
        i18n::set_current(Lang::EN);
    }
}
