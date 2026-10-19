//! The interface language on the ERPCons office site: Vietnamese until the visitor picks another
//! one, remembered in one localStorage key every editor of the site reads, so a choice made in
//! one editor carries over to the others.

use deckcraft_ui_egui::i18n::Lang;

const KEY: &str = "erpcons-ui-language";
const DEFAULT: &str = "vi";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The remembered language code, if DeckCraft has that language; other editors' languages
/// DeckCraft lacks fall back to the default.
pub fn initial() -> String {
    let stored = storage().and_then(|s| s.get_item(KEY).ok().flatten());
    let wanted = match stored.as_deref() {
        // The other editors save "auto" for "follow the browser".
        Some("auto") => Some(browser()),
        Some(code) => Lang::from_code(code),
        None => None,
    };
    wanted.map_or(DEFAULT, Lang::code).to_string()
}

/// Vietnamese for a Vietnamese browser, English otherwise (the only two catalogs).
fn browser() -> Lang {
    let tag = web_sys::window().and_then(|w| w.navigator().language()).unwrap_or_default();
    let primary = tag.split(['-', '_']).next().unwrap_or_default();
    Lang::from_code(primary).unwrap_or(Lang::EN)
}

pub fn remember(code: &str) {
    if let Some(s) = storage()
        && let Err(e) = s.set_item(KEY, code)
    {
        log::warn!("couldn't remember the interface language: {e:?}");
    }
}
