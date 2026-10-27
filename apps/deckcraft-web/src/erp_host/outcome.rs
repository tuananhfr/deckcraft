//! The shell's `SaveResult`, read field by field so a malformed answer can't be taken for a save.

use super::autosave::Outcome;

/// `{status, version}`, `{status: 'failed', retryable, message}`, …; anything else is retried by
/// the shell, so the document just stays dirty.
pub(crate) fn outcome(status: Option<&str>, version: Option<f64>, retryable: Option<bool>, message: Option<String>) -> Outcome {
    match status {
        Some("saved") => Outcome::Saved { version: version.filter(|v| v.is_finite() && *v >= 0.0).map(|v| v as u64) },
        Some("conflict") => Outcome::Conflict,
        Some("failed") if retryable != Some(true) => {
            Outcome::Failed { message: message.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| failed_message().into()) }
        }
        _ => Outcome::Retry,
    }
}

/// Shown when the shell's failure has no message, or the presentation couldn't be encoded.
pub(crate) fn failed_message() -> &'static str {
    deckcraft_ui_egui::tl!("The presentation couldn't be saved to the server.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckcraft_ui_egui::i18n::{self, Lang};

    #[test]
    fn save_results_map_to_outcomes() {
        assert_eq!(outcome(Some("saved"), Some(4.0), None, None), Outcome::Saved { version: Some(4) });
        assert_eq!(outcome(Some("saved"), None, None, None), Outcome::Saved { version: None });
        assert_eq!(outcome(Some("conflict"), Some(9.0), None, None), Outcome::Conflict);
        assert_eq!(outcome(Some("offline"), None, None, None), Outcome::Retry);
        assert_eq!(outcome(Some("failed"), None, Some(true), Some("Máy chủ bận".into())), Outcome::Retry);
        assert_eq!(outcome(Some("failed"), None, Some(false), Some("Tệp quá lớn".into())), Outcome::Failed { message: "Tệp quá lớn".into() });
        assert_eq!(outcome(None, None, None, None), Outcome::Retry, "a malformed answer is never a save");
        assert_eq!(outcome(Some("SAVED"), Some(1.0), None, None), Outcome::Retry);
    }

    #[test]
    fn a_failure_without_a_message_says_so_in_vietnamese() {
        i18n::set_current(Lang::from_code("vi").unwrap_or(Lang::EN));
        assert_eq!(
            outcome(Some("failed"), None, None, Some(" ".into())),
            Outcome::Failed { message: "Không lưu được bản trình chiếu lên máy chủ.".into() }
        );
        i18n::set_current(Lang::EN);
    }
}
