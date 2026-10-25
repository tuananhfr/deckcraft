//! The PI ID section of File › Account, the title bar's account button and the paid licence
//! expiry notice.

use egui::{Align2, CornerRadius, Rect, RichText, Sense, Ui, vec2};

use super::{AccountError, AccountView, LicenceView, SignedInView};
use crate::backstage::{Page, dim, heading};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{SlideApp, i18n, tl};

/// The section's widest; the rest of the page keeps its own columns.
const SECTION_W: f32 = 520.0;

enum Action {
    SignIn,
    CancelSignIn,
    SignOut,
    SelectOrg(Option<String>),
    RetryActivation,
}

/// The top of File › Account while an account hook is installed, under PowerPoint's
/// "User Information" heading.
pub(crate) fn section(app: &mut SlideApp, ui: &mut Ui) {
    let Some(hook) = app.account_hook.clone() else { return };
    let snapshot = app.account.snapshot.clone();
    let t = Tokens::get(ui.ctx());
    let mut action = None;
    ui.vertical(|ui| {
        ui.set_max_width(ui.available_width().min(SECTION_W));
        heading(ui, "User Information");
        match &snapshot.view {
            AccountView::Guest => {
                dim(ui, tl!("Signing in is optional: every feature works without an account."));
                ui.add_space(10.0);
                if primary(ui, tl!("Sign in with PI ID"), !snapshot.busy, &t).clicked() {
                    action = Some(Action::SignIn);
                }
            }
            AccountView::SigningIn => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tl!("Continue in your browser…")).font(theme::font(13.0)).color(t.text));
                });
                ui.add_space(8.0);
                if ui.button(tl!("Cancel")).clicked() {
                    action = Some(Action::CancelSignIn);
                }
            }
            AccountView::SignedIn(view) => signed_in(ui, view, snapshot.busy, &t, &mut action),
        }
        if let Some(text) = snapshot.error.filter(|e| !repeats_the_page(*e, &snapshot.view)).and_then(error_text) {
            ui.add_space(10.0);
            ui.add(egui::Label::new(RichText::new(text).font(theme::font(12.5)).color(ui.visuals().error_fg_color)).wrap());
        }
    });
    // The web adapter opens the shell's popup here: still inside the click's frame, as browsers require.
    match action {
        Some(Action::SignIn) => hook.sign_in(),
        Some(Action::CancelSignIn) => hook.cancel_sign_in(),
        Some(Action::SignOut) => hook.sign_out(),
        Some(Action::SelectOrg(org)) => hook.select_org(org),
        Some(Action::RetryActivation) => hook.retry_activation(),
        None => {}
    }
}

fn signed_in(ui: &mut Ui, view: &SignedInView, busy: bool, t: &Tokens, action: &mut Option<Action>) {
    let name = display_name(view);
    // One line, cut with "…" (full text on hover): wrapping would split an address mid-word.
    ui.add(egui::Label::new(RichText::new(&name).font(theme::bold(18.0)).color(t.text)).truncate());
    if let Some(email) = view.email.as_deref().filter(|e| *e != name) {
        ui.add(egui::Label::new(RichText::new(email).font(theme::font(12.5)).color(t.text_dim)).truncate());
    }
    if view.reauth_required {
        ui.add_space(10.0);
        ui.label(RichText::new(tl!("Your session has expired. Sign in again.")).font(theme::font(13.0)).color(t.text));
        ui.add_space(6.0);
        if primary(ui, tl!("Sign in again"), !busy, t).clicked() {
            *action = Some(Action::SignIn);
        }
    }
    ui.add_space(14.0);
    ui.add_enabled_ui(!busy, |ui| {
        subheading(ui, tl!("Organization"), t);
        let personal = tl!("Personal").to_string();
        let selected = match &view.active_org {
            Some(id) => view.orgs.iter().find(|o| &o.id == id).map_or_else(|| id.clone(), |o| o.name.clone()),
            None => personal.clone(),
        };
        egui::ComboBox::from_id_salt("account_org").width(ui.available_width().min(260.0)).truncate().selected_text(selected).show_ui(ui, |ui| {
            let orgs = view.orgs.iter().map(|org| (Some(org.id.clone()), org.name.clone()));
            for (id, label) in std::iter::once((None, personal.clone())).chain(orgs) {
                let current = view.active_org == id;
                if ui.selectable_label(current, label).clicked() && !current {
                    *action = Some(Action::SelectOrg(id));
                }
            }
        });
        ui.add_space(14.0);
        subheading(ui, tl!("License"), t);
        ui.add(egui::Label::new(RichText::new(licence_text(view.licence)).font(theme::font(13.0)).color(t.text)).wrap());
        if view.licence == LicenceView::DeviceCapReached {
            ui.add_space(4.0);
            if ui.button(tl!("Retry")).clicked() {
                *action = Some(Action::RetryActivation);
            }
        }
        if view.session_only && view.licence != LicenceView::SessionOnly {
            dim(ui, tl!("You will need to sign in again the next time you open the app."));
        }
        ui.add_space(8.0);
        dim(ui, if view.online { tl!("Connected to PI ID") } else { tl!("Offline. Your account syncs when you reconnect.") });
        ui.add_space(14.0);
    });
    ui.horizontal(|ui| {
        if ui.add_enabled(!busy, egui::Button::new(tl!("Sign out"))).clicked() {
            *action = Some(Action::SignOut);
        }
        if busy {
            ui.spinner();
        }
    });
}

fn subheading(ui: &mut Ui, text: &str, t: &Tokens) {
    ui.label(RichText::new(text).font(theme::bold(13.0)).color(t.text));
    ui.add_space(4.0);
}

/// An accent button, like the title bar's Present.
fn primary(ui: &mut Ui, label: &str, enabled: bool, t: &Tokens) -> egui::Response {
    let text = RichText::new(label).font(theme::bold(13.0)).color(t.accent_text);
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = vec2(14.0, 5.0);
        ui.add_enabled(enabled, egui::Button::new(text).fill(t.accent).corner_radius(CornerRadius::same(13)).min_size(vec2(84.0, 28.0)))
    })
    .inner
}

/// One message per fact: an error the licence or session line already states is not repeated.
fn repeats_the_page(error: AccountError, view: &AccountView) -> bool {
    let AccountView::SignedIn(s) = view else { return false };
    match error {
        AccountError::DeviceCap => s.licence == LicenceView::DeviceCapReached,
        AccountError::DeviceRevoked => s.licence == LicenceView::DeviceRevoked,
        AccountError::ReauthRequired => s.reauth_required,
        _ => false,
    }
}

fn licence_text(licence: LicenceView) -> String {
    let dated = |key: &str, at: i64| i18n::fmt(i18n::t(key), &[("date", &date_text(at))]);
    match licence {
        LicenceView::Free | LicenceView::Online { pro: false } | LicenceView::Active { pro: false, .. } => tl!("Free").into(),
        LicenceView::Active { expires_at, pro: true } => dated("Paid features are valid until {date}.", expires_at),
        LicenceView::Online { pro: true } => tl!("Paid features are active.").into(),
        LicenceView::Inactive { expires_at } => dated("Your license expired on {date}. Free features keep working.", expires_at),
        LicenceView::DeviceRevoked => tl!("This device was removed from your account. Free features keep working.").into(),
        LicenceView::DeviceCapReached => tl!("Your account has reached its device limit, so this device is not activated.").into(),
        LicenceView::SessionOnly => tl!("Signed in for this session only: this computer has no credential store.").into(),
    }
}

fn display_name(view: &SignedInView) -> String {
    let name = view.name.trim();
    match view.email.as_deref().filter(|e| !e.trim().is_empty()) {
        _ if !name.is_empty() => name.to_string(),
        Some(email) => email.to_string(),
        None => tl!("PI ID account").to_string(),
    }
}

fn initial(view: &SignedInView) -> Option<String> {
    crate::account::avatar_letter(&view.name, view.email.as_deref())
}

/// The title bar's account button: the user's initial when signed in, a person otherwise. It
/// opens File › Account. Not drawn, and takes no room, without an account hook.
pub(crate) fn title_button(app: &mut SlideApp, ui: &mut Ui) {
    if app.account_hook.is_none() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let (tip, letter) = match &app.account.snapshot.view {
        AccountView::SignedIn(view) => (display_name(view), initial(view)),
        AccountView::SigningIn => (tl!("Signing in…").to_string(), None),
        AccountView::Guest => (tl!("Sign in").to_string(), None),
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
    let painter = ui.painter_at(rect);
    if resp.hovered() || app.backstage.as_ref().is_some_and(|b| b.page == Page::Account) {
        painter.rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    match letter {
        Some(letter) => {
            painter.circle_filled(rect.center(), rect.height() / 2.0 - 1.5, t.accent);
            painter.text(rect.center(), Align2::CENTER_CENTER, letter, theme::bold(11.0), t.accent_text);
        }
        None => icons::paint(&painter, Rect::from_center_size(rect.center(), vec2(18.0, 16.0)), Icon::Account, t.text, false),
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &tip));
    if resp.on_hover_text(&tip).clicked() {
        app.open_backstage(Page::Account);
    }
}

/// The lease expiry notice (lease.md §8), only while paid features hang on that lease. It sits
/// where the window's brief notices do, after any of them, so it never covers the chrome.
/// Closing it tells the adapter, which records the stage as shown.
pub(crate) fn warning_notice(app: &mut SlideApp, ctx: &egui::Context) {
    if app.toast.is_some() {
        return;
    }
    let Some(hook) = app.account_hook.clone() else { return };
    let AccountView::SignedIn(view) = &app.account.snapshot.view else { return };
    let (Some(warning), LicenceView::Active { pro: true, .. }) = (view.warning, view.licence) else { return };
    if app.account.dismissed == Some(warning) {
        return;
    }
    let text = i18n::fmt(tl!("Paid features expire on {date}. Connect to the internet to renew them."), &[("date", &date_text(warning.expires_at))]);
    let t = Tokens::get(ctx);
    let (mut close, mut open) = (false, false);
    egui::Area::new(egui::Id::new("account_lease_warning")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -44.0)).order(egui::Order::Foreground).show(
        ctx,
        |ui| {
            crate::toast::frame(&t).show(ui, |ui| {
                ui.set_max_width((ctx.content_rect().width() - 2.0 * 16.0 - 2.0 * 14.0).clamp(120.0, 520.0));
                ui.add(egui::Label::new(RichText::new(text).font(theme::font(13.0)).color(t.text)).wrap());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    open = ui.button(tl!("Account")).clicked();
                    close = ui.button(tl!("Close")).clicked();
                });
            });
        },
    );
    if close {
        app.account.dismissed = Some(warning);
        hook.warning_shown(warning.days);
    }
    if open {
        app.open_backstage(Page::Account);
    }
}

/// The local calendar date of `unix_secs`: `dd/mm/yyyy` in Vietnamese, the short date of the
/// presentations' date fields (M/D/YYYY) otherwise. Empty for a time out of range.
pub fn date_text(unix_secs: i64) -> String {
    let lang = if i18n::current().code() == "vi" { "vi" } else { "en-US" };
    local_date(unix_secs).and_then(|date| deckcraft_text::datetime::field_text("datetime1", Some(lang), date)).unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn local_date(unix_secs: i64) -> Option<deckcraft_text::datetime::DateTime> {
    use chrono::Datelike;
    let local = chrono::DateTime::from_timestamp(unix_secs, 0)?.with_timezone(&chrono::Local);
    Some(deckcraft_text::datetime::DateTime::new(local.year(), local.month(), local.day(), 12, 0, 0))
}

#[cfg(target_arch = "wasm32")]
fn local_date(unix_secs: i64) -> Option<deckcraft_text::datetime::DateTime> {
    let date = js_sys::Date::new_0();
    if date.set_time(unix_secs as f64 * 1000.0).is_nan() {
        return None;
    }
    let year = i32::try_from(date.get_full_year()).ok()?;
    Some(deckcraft_text::datetime::DateTime::new(year, date.get_month() + 1, date.get_date(), 12, 0, 0))
}

/// The error line in the current language; `None` for a cancelled sign-in, which needs no message.
pub fn error_text(error: AccountError) -> Option<String> {
    let text = match error {
        AccountError::Cancelled => return None,
        AccountError::PopupBlocked => tl!("The browser blocked the sign-in window. Allow pop-ups for this site and try again."),
        AccountError::Network => tl!("Could not reach PI ID. Check your connection and try again."),
        AccountError::Timeout => tl!("PI ID took too long to answer. Try again."),
        AccountError::Denied => tl!("PI ID refused the request. Free features keep working."),
        AccountError::RateLimited { retry_after_secs: Some(secs) } if secs > 0 => {
            let minutes = secs.div_ceil(60).to_string();
            return Some(i18n::fmt(tl!("PI ID is busy. Try again in {minutes} min."), &[("minutes", &minutes)]));
        }
        AccountError::RateLimited { .. } => tl!("PI ID is busy. Try again in a few minutes."),
        AccountError::DeviceCap => tl!("Your account has reached its device limit. Remove a device in PI ID, then retry."),
        AccountError::DeviceRevoked => tl!("This device was removed from your account. Free features keep working."),
        AccountError::Suspended => tl!("Your PI ID account is suspended. Free features keep working."),
        AccountError::OrgUnavailable => tl!("This organization is not available to your account."),
        AccountError::ReauthRequired => tl!("Your session has expired. Sign in again."),
        AccountError::ClientTooOld => tl!("This version of the app is too old for PI ID. Update the app, then try again."),
        AccountError::ServiceUnavailable => tl!("PI ID is temporarily unavailable. Try again later."),
        AccountError::Other => tl!("Something went wrong with PI ID. Try again."),
    };
    Some(text.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::i18n::Lang;

    /// 15 March 2027, noon UTC: the same calendar day in every time zone the tests run in.
    const NOON: i64 = 1_805_112_000;

    fn in_lang<R>(code: &str, f: impl FnOnce() -> R) -> R {
        i18n::set_current(Lang::from_pref(code));
        let r = f();
        i18n::set_current(Lang::EN);
        r
    }

    #[test]
    fn dates_follow_the_interface_language() {
        assert_eq!(in_lang("vi", || date_text(NOON)), "15/03/2027");
        assert_eq!(in_lang("en", || date_text(NOON)), "3/15/2027");
        assert_eq!(date_text(i64::MAX), "");
    }

    #[test]
    fn a_cancelled_sign_in_shows_nothing_and_rate_limits_say_when_to_retry() {
        assert_eq!(error_text(AccountError::Cancelled), None);
        assert_eq!(error_text(AccountError::RateLimited { retry_after_secs: Some(90) }).unwrap(), "PI ID is busy. Try again in 2 min.");
        assert_eq!(
            in_lang("vi", || error_text(AccountError::RateLimited { retry_after_secs: Some(1) })).unwrap(),
            "PI ID đang bận. Hãy thử lại sau 1 phút."
        );
        assert_eq!(error_text(AccountError::RateLimited { retry_after_secs: Some(0) }).unwrap(), "PI ID is busy. Try again in a few minutes.");
    }

    #[test]
    fn every_error_and_licence_line_is_translated() {
        let errors = [
            AccountError::PopupBlocked,
            AccountError::Network,
            AccountError::Timeout,
            AccountError::Denied,
            AccountError::RateLimited { retry_after_secs: None },
            AccountError::DeviceCap,
            AccountError::DeviceRevoked,
            AccountError::Suspended,
            AccountError::OrgUnavailable,
            AccountError::ReauthRequired,
            AccountError::ClientTooOld,
            AccountError::ServiceUnavailable,
            AccountError::Other,
        ];
        for error in errors {
            let en = error_text(error).unwrap();
            assert_ne!(in_lang("vi", || error_text(error)).unwrap(), en, "untranslated: {en}");
        }
        let licences = [
            LicenceView::Free,
            LicenceView::Active { expires_at: NOON, pro: true },
            LicenceView::Online { pro: true },
            LicenceView::Inactive { expires_at: NOON },
            LicenceView::DeviceRevoked,
            LicenceView::DeviceCapReached,
            LicenceView::SessionOnly,
        ];
        for licence in licences {
            let en = licence_text(licence);
            assert_ne!(in_lang("vi", || licence_text(licence)), en, "untranslated: {en}");
        }
        assert_eq!(licence_text(LicenceView::Active { expires_at: NOON, pro: false }), "Free", "a free lease is just the free plan");
        assert_eq!(licence_text(LicenceView::Online { pro: false }), "Free");
    }
}
