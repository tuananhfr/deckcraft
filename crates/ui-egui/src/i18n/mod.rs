//! Interface languages. Strings in the code stay English and are the lookup keys; a catalog per
//! language (`*.tsv`, format in `vi.tsv`) maps them to display text when the UI is drawn. Command
//! ids, parameters, presentation text, file names and the CLI never see translated text, so
//! scripts and agents are unaffected; only the status-bar message (`app.status`) is stored as
//! shown. A string without a translation shows in English.
//!
//! The catalog format and lookup are WordCraft's (`wordcraft/crates/ui-egui/src/i18n`), so
//! catalogs and habits carry over between the editors.
//!
//! # Looking strings up
//! - [`tl!`](crate::tl) / [`t`]: a string in the current language. [`tr`]: in a given one.
//! - [`fmt`]: fill `{name}` placeholders after a lookup; translators may reorder them.

mod catalog;

use std::cell::Cell;
use std::sync::OnceLock;

use catalog::Catalog;

/// One supported interface language.
pub struct LangInfo {
    /// BCP 47 code, lowercase. Also the `language` setting's value.
    pub code: &'static str,
    /// The language's name in itself, shown in the language picker.
    pub name: &'static str,
    /// Catalog file contents (empty for the built-in English).
    pub source: &'static str,
    catalog: OnceLock<Catalog>,
}

/// The registry. English first: it is the fallback and the source language.
pub static LANGUAGES: [LangInfo; 2] = [
    LangInfo { code: "en", name: "English", source: "", catalog: OnceLock::new() },
    LangInfo { code: "vi", name: "Tiếng Việt", source: include_str!("vi.tsv"), catalog: OnceLock::new() },
];

impl LangInfo {
    fn catalog(&self) -> &Catalog {
        self.catalog.get_or_init(|| {
            let (catalog, errors) = Catalog::parse(self.source);
            for error in errors {
                log::warn!("{} interface catalog: {error}; that entry shows in English", self.code);
            }
            catalog
        })
    }
}

/// A language the UI can be shown in (a handle into [`LANGUAGES`]).
#[derive(Clone, Copy)]
pub struct Lang(&'static LangInfo);

impl std::fmt::Debug for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lang({})", self.0.code)
    }
}

impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        self.0.code == other.0.code
    }
}

impl Eq for Lang {}

impl Lang {
    pub const EN: Lang = Lang(&LANGUAGES[0]);

    pub fn code(self) -> &'static str {
        self.0.code
    }

    pub fn name(self) -> &'static str {
        self.0.name
    }

    /// A language by its exact code (any case).
    pub fn from_code(code: &str) -> Option<Lang> {
        LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code)).map(Lang)
    }

    /// Resolve the `language` setting; anything unknown (a code from a newer version) is English.
    pub fn from_pref(pref: &str) -> Lang {
        Lang::from_code(pref).unwrap_or(Lang::EN)
    }

    /// Every registered language.
    pub fn all() -> impl Iterator<Item = Lang> {
        LANGUAGES.iter().map(Lang)
    }

    fn catalog(self) -> &'static Catalog {
        self.0.catalog()
    }
}

thread_local! {
    // Per thread: parallel test harnesses must not change each other's language.
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::EN) };
}

/// Set the language the UI is drawn in (the app does this every frame from its setting).
pub fn set_current(lang: Lang) {
    CURRENT.set(lang);
}

/// The language the UI is drawn in.
pub fn current() -> Lang {
    CURRENT.get()
}

/// Does `lang` have a catalog entry for this string? (English never does: it is the source.)
pub fn has(lang: Lang, s: &str) -> bool {
    lang.catalog().plain(s).is_some()
}

/// `s` in the current language ([`tr`] with [`current`]).
pub fn t(s: &str) -> &str {
    tr(current(), s)
}

/// `s` in `lang`; strings without a translation come back unchanged.
pub fn tr(lang: Lang, s: &str) -> &str {
    lang.catalog().plain(s).unwrap_or(s)
}

/// A label with an action after a fixed English prefix, e.g. `Undo Typing`: both parts translated
/// where the catalog has them.
pub fn prefixed(prefix: &str, rest: &str) -> String {
    if rest.is_empty() {
        return t(prefix).to_string();
    }
    fmt(t(&format!("{prefix} {{action}}")), &[("action", t(rest))])
}

/// Fill `{name}` placeholders in one pass. Unknown placeholders stay as written, and inserted
/// values are never read as templates again, so a file name containing `{n}` stays intact.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some((before, after_open)) = rest.split_once('{') {
        out.push_str(before);
        let Some((name, after_close)) = after_open.split_once('}') else {
            out.push('{');
            out.push_str(after_open);
            return out;
        };
        match args.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = after_close;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests;
