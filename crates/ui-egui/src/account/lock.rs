//! The desktop's licence lock (ADR 0013 of `erpcons-account`): while the account hook names a
//! reason, every presentation is read-only the way the office site's view mode is, but Save,
//! Save As, Export and Print stay, and refusals say the reason. Nothing is closed or reloaded:
//! a new lease gives editing back to the presentations as they are.

use deckcraft_engine::Access;

use crate::{SlideApp, tl};

/// Why the account refuses edits now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditLock {
    /// The account has not been read yet.
    Checking,
    SignIn,
    /// PI licenses organizations only; the personal context has no lease.
    ChooseOrg,
    Revoked,
    DeviceCap,
    NotActivated,
    /// No OS credential store, so this device can keep no lease.
    SessionOnly,
    RenewOffline,
    /// Online, yet the lease ran out: renewal failed or has not run yet, which is not "no plan".
    RenewFailed,
    NoPlan,
    /// The account could not be opened; failing open would let anyone unlock editing.
    Unavailable,
}

impl EditLock {
    /// The notice, in the interface language.
    pub fn text(self) -> &'static str {
        match self {
            EditLock::Checking => tl!("Checking your PI ID license…"),
            EditLock::SignIn => tl!("Sign in with PI ID to edit"),
            EditLock::ChooseOrg => tl!("Choose an organization to edit"),
            EditLock::Revoked => tl!("This device was removed from your PI ID account. Sign in again to edit"),
            EditLock::DeviceCap => tl!("Your PI ID plan has reached its device limit. Remove a device in PI ID, then press Retry"),
            EditLock::NotActivated => tl!("This device is not activated for editing yet"),
            EditLock::SessionOnly => tl!("Editing needs a credential store on this computer"),
            EditLock::RenewOffline => tl!("Your license has expired. Connect to the internet to renew it"),
            EditLock::RenewFailed => tl!("Could not renew your license. The app keeps trying"),
            EditLock::NoPlan => tl!("This organization has no ERPCons Office plan"),
            EditLock::Unavailable => tl!("PI ID licensing is unavailable on this computer, so editing is off"),
        }
    }

    /// A plain sync with PI may lift it: activation retried, lease renewed, a plan bought or an
    /// organization joined since the last sync.
    pub fn sync_may_lift(self) -> bool {
        matches!(self, EditLock::ChooseOrg | EditLock::NotActivated | EditLock::RenewOffline | EditLock::RenewFailed | EditLock::NoPlan)
    }

    /// File › Account offers Retry; a device-cap block lifts only on the user's retry (lease.md §6).
    pub fn retry_offered(self) -> bool {
        self.sync_may_lift() || self == EditLock::DeviceCap
    }
}

/// Applies the hook's lock of this frame, before any input or command of the frame runs.
pub(crate) fn apply(app: &mut SlideApp, lock: Option<EditLock>) {
    let previous = std::mem::replace(&mut app.account.lock, lock);
    if previous != lock {
        app.session.set_new_document_access(if lock.is_some() { Access::edits_locked() } else { Access::default() });
        announce(app, previous, lock);
    }
    // Every frame, so presentations opened since are covered; the host's own view modes
    // (`Access::read_only`) are left alone.
    let (from, to) = if lock.is_some() { (Access::default(), Access::edits_locked()) } else { (Access::edits_locked(), Access::default()) };
    let uids: Vec<u64> = app.session.documents().iter().filter(|d| d.access == from).map(|d| d.uid).collect();
    for uid in uids {
        app.set_access(uid, to);
    }
}

/// Says a new reason once in the status bar, and takes back one that no longer holds.
fn announce(app: &mut SlideApp, previous: Option<EditLock>, lock: Option<EditLock>) {
    if let Some(lock) = lock.filter(|l| *l != EditLock::Checking) {
        app.set_status(lock.text());
    } else if let Some(previous) = previous
        && app.status.as_ref().is_some_and(|(text, _)| text == previous.text())
    {
        app.status = None;
    }
}

impl SlideApp {
    /// The licence lock's reason when it is what refuses a change to the active presentation.
    pub(crate) fn licence_refusal(&self) -> Option<EditLock> {
        let locked = self.session.active().is_some_and(|d| d.access == Access::edits_locked());
        self.account.lock.filter(|_| locked)
    }
}

#[cfg(test)]
mod tests;
