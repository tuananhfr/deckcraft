//! Optional PI ID account (ERPCons): the PI ID section of File › Account, the title bar's account
//! button and the paid licence expiry notice. The host installs an [`AccountHook`] (desktop:
//! `erpcons-account` on a worker thread; web: the office shell's `window.erpAccount`). Without one
//! nothing here shows, File › Account is unchanged and every feature is allowed.

mod page;

pub use page::{date_text, error_text};
pub(crate) use page::{section, title_button, warning_notice};

use crate::SlideApp;

/// Implemented by the native and web adapters. Every method returns at once; work runs on the
/// adapter's own worker (thread on desktop, promise on web) and the result shows up in the next
/// `snapshot()`.
pub trait AccountHook {
    fn snapshot(&self) -> AccountSnapshot;
    fn sign_in(&self);
    fn cancel_sign_in(&self);
    fn sign_out(&self);
    fn select_org(&self, org_id: Option<String>);
    fn retry_activation(&self);
    fn allowed(&self, feature: &str) -> bool;
    fn warning_shown(&self, days: u32);
    /// Called once with the egui context so the adapter can request a repaint when its snapshot
    /// changes.
    fn attach(&self, ctx: &egui::Context);
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountSnapshot {
    pub view: AccountView,
    /// A command the user started is still running.
    pub busy: bool,
    pub error: Option<AccountError>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AccountView {
    #[default]
    Guest,
    SigningIn,
    SignedIn(SignedInView),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedInView {
    pub name: String,
    pub email: Option<String>,
    pub orgs: Vec<OrgView>,
    /// `None` is the personal context.
    pub active_org: Option<String>,
    pub licence: LicenceView,
    pub warning: Option<WarningView>,
    pub online: bool,
    pub session_only: bool,
    pub reauth_required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrgView {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LicenceView {
    /// No non-free entitlement (normal today).
    Free,
    /// Desktop lease.
    Active {
        expires_at: i64,
        pro: bool,
    },
    /// Web: no lease, entitlements read online.
    Online {
        pro: bool,
    },
    Inactive {
        expires_at: i64,
    },
    DeviceRevoked,
    DeviceCapReached,
    SessionOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WarningView {
    pub days: u32,
    pub expires_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountError {
    Cancelled,
    Network,
    Timeout,
    Denied,
    RateLimited { retry_after_secs: Option<u64> },
    DeviceCap,
    DeviceRevoked,
    Suspended,
    OrgUnavailable,
    ReauthRequired,
    ClientTooOld,
    ServiceUnavailable,
    Other,
}

/// Whether holding `feature` means a paid plan. `.view` and `.export` are pinned free and the
/// built-in catalog lists `<product>.use` as free (products.md §2, §3); anything else needs an
/// entitlement.
pub fn is_pro_feature(feature: &str) -> bool {
    let builtin_free = feature.split_once('.').is_some_and(|(_, name)| name == "use");
    !(feature.ends_with(".view") || feature.ends_with(".export") || builtin_free)
}

/// What the app keeps between frames about the account.
#[derive(Default)]
pub(crate) struct AccountState {
    attached: bool,
    /// The hook's snapshot, read once per frame.
    snapshot: AccountSnapshot,
    /// The expiry warning closed in this run; hidden even before the adapter's snapshot drops it.
    dismissed: Option<WarningView>,
}

impl SlideApp {
    /// Whether `feature` may be used. True without an account hook; nothing is gated on it yet.
    pub fn allowed(&self, feature: &str) -> bool {
        self.account_hook.as_ref().is_none_or(|hook| hook.allowed(feature))
    }
}

/// Attaches the hook on the first frame and reads its snapshot for this one.
pub(crate) fn poll(app: &mut SlideApp, ctx: &egui::Context) {
    let Some(hook) = &app.account_hook else { return };
    if !app.account.attached {
        app.account.attached = true;
        hook.attach(ctx);
    }
    app.account.snapshot = hook.snapshot();
}

#[cfg(test)]
mod tests {
    use super::is_pro_feature;

    #[test]
    fn only_features_outside_the_free_built_ins_are_pro() {
        for free in ["trinhchieu.view", "trinhchieu.export", "trinhchieu.use", "pdf.view", "vanban.export"] {
            assert!(!is_pro_feature(free), "{free}");
        }
        for pro in ["trinhchieu.advanced", "trinhchieu.media.record", "trinhchieu.use.extra", "trinhchieu.viewer"] {
            assert!(is_pro_feature(pro), "{pro}");
        }
    }
}
