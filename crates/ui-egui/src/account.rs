//! Optional PI ID account (ERPCons): the PI ID section of File › Account, the title bar's account
//! button, the licence expiry notice and the desktop's licence lock. The host installs an
//! [`AccountHook`] (desktop: `erpcons-account` on a worker thread; web: the office shell's
//! `window.erpAccount`). Without one nothing here shows, File › Account is unchanged and every
//! feature is allowed.

mod lock;
mod page;

pub use lock::EditLock;
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
    /// Why edits are refused now, read once per frame from cached state. Only the desktop gates
    /// editing on a licence; the web stays editable for everyone.
    fn edit_lock(&self) -> Option<EditLock> {
        None
    }
    /// An edit was refused for [`edit_lock`](Self::edit_lock)'s reason; the adapter may sync with
    /// PI sooner than its schedule.
    fn edit_refused(&self) {}
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
    PopupBlocked,
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

/// Whether holding `feature` needs a plan. Only `.view` and `.export` are pinned free (products.md
/// §2); `erpconsoffice.use`, editing, needs the lease since erpcons-account 0.5.0 (ADR 0013).
pub fn is_pro_feature(feature: &str) -> bool {
    !(feature.ends_with(".view") || feature.ends_with(".export"))
}

/// Whether holding `feature` means a paid plan: a pro feature other than `.use`, which the free
/// plan grants.
pub fn is_paid_feature(feature: &str) -> bool {
    is_pro_feature(feature) && !feature.ends_with(".use")
}

/// The avatar letter: the given name's first letter (Vietnamese names put it last, as the office
/// shell does), else the email's.
pub fn avatar_letter(name: &str, email: Option<&str>) -> Option<String> {
    let given = name.split_whitespace().next_back().unwrap_or_default();
    given.chars().chain(email.unwrap_or_default().chars()).find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect())
}

/// What the app keeps between frames about the account.
#[derive(Default)]
pub(crate) struct AccountState {
    attached: bool,
    /// The hook's snapshot, read once per frame.
    snapshot: AccountSnapshot,
    /// The expiry warning closed in this run; hidden even before the adapter's snapshot drops it.
    dismissed: Option<WarningView>,
    /// The hook's licence lock in this frame.
    pub(crate) lock: Option<EditLock>,
}

impl SlideApp {
    /// Whether `feature` may be used. True without an account hook; nothing is gated on it yet
    /// (editing is, through [`AccountHook::edit_lock`]).
    pub fn allowed(&self, feature: &str) -> bool {
        self.account_hook.as_ref().is_none_or(|hook| hook.allowed(feature))
    }
}

/// Attaches the hook on the first frame and reads its snapshot and licence lock for this one.
pub(crate) fn poll(app: &mut SlideApp, ctx: &egui::Context) {
    let Some(hook) = app.account_hook.clone() else { return };
    if !app.account.attached {
        app.account.attached = true;
        hook.attach(ctx);
    }
    app.account.snapshot = hook.snapshot();
    lock::apply(app, hook.edit_lock());
}

#[cfg(test)]
mod tests {
    use super::{is_paid_feature, is_pro_feature};

    #[test]
    fn the_avatar_shows_the_given_name_like_the_office_shell() {
        assert_eq!(super::avatar_letter("Nguyễn Văn An", None).as_deref(), Some("A"));
        assert_eq!(super::avatar_letter(" ", Some("binh@example.com")).as_deref(), Some("B"));
        assert_eq!(super::avatar_letter("", None), None);
    }

    #[test]
    fn only_view_and_export_are_free() {
        // erpcons-account's OFFICE_VIEW, OFFICE_EXPORT and OFFICE_USE; this crate also builds for the web.
        for free in ["erpconsoffice.view", "erpconsoffice.export", "trinhchieu.view", "pdf.view", "vanban.export"] {
            assert!(!is_pro_feature(free), "{free}");
        }
        for pro in ["erpconsoffice.use", "trinhchieu.use", "trinhchieu.advanced", "trinhchieu.media.record", "trinhchieu.viewer"] {
            assert!(is_pro_feature(pro), "{pro}");
        }
    }

    #[test]
    fn editing_is_no_paid_feature() {
        for unpaid in ["erpconsoffice.view", "erpconsoffice.export", "erpconsoffice.use", "trinhchieu.use"] {
            assert!(!is_paid_feature(unpaid), "{unpaid}");
        }
        for paid in ["erpconsoffice.advanced", "trinhchieu.media.record", "erpconsoffice.use.extra"] {
            assert!(is_paid_feature(paid), "{paid}");
        }
    }
}
