//! What log records may say about the user's files: a document by its extension and size, never
//! its name, folder or URL; a settings or log path with the home folder written `~`.

/// A document as log records name it: `.pptx (12345 bytes)`, or `file` without a usable extension.
pub fn log_name(name: &str, size: Option<u64>) -> String {
    let kind = extension(name).map_or_else(|| "file".to_string(), |ext| format!(".{ext}"));
    match size {
        Some(n) => format!("{kind} ({n} bytes)"),
        None => kind,
    }
}

/// `message` (an error that may quote the document it failed on) with `name` written as its
/// extension.
pub fn redact(message: &str, name: &str) -> String {
    let mut out = message.to_string();
    let file = last_component(name);
    for quoted in [name, file] {
        if !quoted.is_empty() {
            out = out.replace(quoted, &log_name(name, None));
        }
    }
    out
}

/// `text` with the user's home folder written `~`, since it carries the account name.
pub fn tilde_home(text: &str) -> String {
    tilde_home_in(text, std::env::home_dir().as_deref().and_then(std::path::Path::to_str))
}

fn tilde_home_in(text: &str, home: Option<&str>) -> String {
    let home = home.map(|h| h.trim_end_matches(['/', '\\']));
    // A drive or filesystem root alone would rewrite every path.
    let Some(home) = home.filter(|h| h.contains(['/', '\\'])) else { return text.to_string() };
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(home) {
        let after = &rest[at + home.len()..];
        out.push_str(&rest[..at]);
        // `/home/ann` must not turn `/home/anna` into `~a`.
        out.push_str(if after.is_empty() || after.starts_with(['/', '\\']) { "~" } else { home });
        rest = after;
    }
    out.push_str(rest);
    out
}

fn last_component(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

fn extension(name: &str) -> Option<String> {
    let (stem, ext) = last_component(name).rsplit_once('.')?;
    // Anything longer or odder than an extension may be part of the name (or a URL's query).
    let plain = !stem.is_empty() && (1..=10).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric());
    plain.then(|| ext.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_are_named_by_extension_and_size_only() {
        assert_eq!(log_name("Báo cáo quý 3.PPTX", Some(12345)), ".pptx (12345 bytes)");
        assert_eq!(log_name("/home/ann/Talks/Secret plan.deckcraft", None), ".deckcraft");
        assert_eq!(log_name(r"C:\Users\Ann\Desktop\notes.txt", Some(0)), ".txt (0 bytes)");
        assert_eq!(log_name("https://example.com/x.pptx?token=abc", Some(9)), "file (9 bytes)");
        for odd in ["", "README", ".pptx", "name.with spaces", "a.verylongextension", "Talk."] {
            assert_eq!(log_name(odd, None), "file", "{odd:?}");
        }
    }

    #[test]
    fn errors_lose_the_path_and_file_name_they_quote() {
        let path = "/home/ann/Talks/Secret plan.pptx";
        assert_eq!(redact(&format!("{path}: No such file or directory"), path), ".pptx: No such file or directory");
        assert_eq!(redact("Secret plan.pptx: not a presentation", path), ".pptx: not a presentation");
        assert_eq!(redact("part ppt/slides/slide1.xml is too large", path), "part ppt/slides/slide1.xml is too large");
        assert_eq!(redact("unchanged", ""), "unchanged");
    }

    #[test]
    fn the_home_folder_is_written_as_a_tilde() {
        let windows = r"C:\Users\Ann\AppData\Roaming\ERPCons/TrinhChieu\logs\deckcraft.log";
        assert_eq!(tilde_home_in(windows, Some(r"C:\Users\Ann")), r"~\AppData\Roaming\ERPCons/TrinhChieu\logs\deckcraft.log");
        assert_eq!(tilde_home_in("/home/ann/.config/x: denied", Some("/home/ann/")), "~/.config/x: denied");
        assert_eq!(tilde_home_in("/home/anna/x", Some("/home/ann")), "/home/anna/x");
        assert_eq!(tilde_home_in("/home/ann", Some("/home/ann")), "~");
        for root in [None, Some("/"), Some(r"C:\"), Some("")] {
            assert_eq!(tilde_home_in("/home/ann/x", root), "/home/ann/x");
        }
    }
}
