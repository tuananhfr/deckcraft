//! Native adapter for the PI ID account (`erpcons-account`). Installed only in builds made with
//! `ERPCONS_PI_ISSUER` (and optionally `ERPCONS_PI_API_BASE`); other builds get no hook, no
//! thread, never touch the credential store or the shared PI folder, and are not gated.
//!
//! One worker thread owns the blocking [`Account`] and runs the UI's commands in order; the UI
//! reads the last published snapshot, and the edit gate reads the account's cached state. Only
//! account state crosses to the library: never a presentation, path or file name.

mod config;
mod gate;

use std::sync::{
    Arc, Mutex, MutexGuard, OnceLock, PoisonError,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, Sender},
};
use std::time::{Duration, Instant};

use deckcraft_ui_egui::account::{
    AccountError, AccountHook, AccountSnapshot, AccountView, EditLock, LicenceView, OrgView, SignedInView, WarningView, is_pro_feature,
};
use deckcraft_ui_egui::i18n;
use erpcons_account::{
    Account, ApiCode, Config, Error, LicenceState, MembershipStatus, OsSecretStore, RevocationReason, SecretStore, Status, WarningStage,
};

use gate::{Resync, ResyncTiming};

/// Read at build time only: a release names its issuer, and nothing at run time can point the app
/// at another one.
const ISSUER: Option<&str> = option_env!("ERPCONS_PI_ISSUER");
const API_BASE: Option<&str> = option_env!("ERPCONS_PI_API_BASE");

/// Network sync while the app runs (products.md §4: entitlements, lease refresh).
const REFRESH_EVERY: Duration = Duration::from_secs(3600);
/// Re-reads the shared session from disk, so signing in or out in another ERPCons app shows here
/// (ADR 0010).
const RELOAD_EVERY: Duration = Duration::from_secs(60);
/// A background refresh that could not reach PI ID tries again after this, or after the server's
/// `Retry-After` (at least [`MIN_RETRY`]).
const RETRY_OFFLINE: Duration = Duration::from_secs(600);
const MIN_RETRY: Duration = Duration::from_secs(60);

type Opener = Box<dyn Fn(&str) -> Result<(), String> + Send>;

/// The account hook, when this build names a PI issuer. Call after the interface language is set:
/// the browser's sign-in pages are written in it. Without `credential_store` (runs without
/// preferences) the account stays closed and editing stays locked: an environment variable must
/// not lift the licence gate.
pub fn install(credential_store: bool) -> Option<DesktopAccount> {
    let issuer = ISSUER.map(str::trim).filter(|issuer| !issuer.is_empty())?;
    if !credential_store {
        return Some(DesktopAccount::unavailable());
    }
    let api_base = API_BASE.map(str::trim).filter(|base| !base.is_empty());
    let config = config::config_for(issuer, api_base, env!("CARGO_PKG_VERSION"));
    Some(DesktopAccount::new(config, Arc::new(OsSecretStore::new()), Box::new(open_browser)))
}

fn open_browser(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("the sign-in address is not a web address".into());
    }
    open::that_detached(url).map_err(|error| error.to_string())
}

enum Command {
    SignIn,
    SignOut,
    SelectOrg(Option<String>),
    RetryActivation,
    WarningShown(WarningStage),
    Refresh,
    /// Wakes the worker for a refused edit; `Shared::resync_wanted` says it is still wanted.
    Resync,
}

/// What the worker and the UI thread share.
#[derive(Default)]
struct Shared {
    snapshot: Mutex<AccountSnapshot>,
    ctx: OnceLock<egui::Context>,
    /// Set by the worker once the account opened; `cancel_sign_in` must reach it while `sign_in` blocks.
    account: OnceLock<Arc<Account>>,
    /// User commands queued or running (the page shows them as busy).
    pending: AtomicUsize,
    /// From the click until `sign_in` or `select_org` returns (both wait for the browser), so a
    /// publish in between never shows the page the wait started from.
    in_browser: AtomicBool,
    /// Cancel pressed before the library started waiting; each browser wait resets the library's own flag.
    cancelled: AtomicBool,
    /// The last user command's error; a new command clears it at once.
    error: Mutex<Option<AccountError>>,
    refresh_queued: AtomicBool,
    /// The account could not be opened or its worker is gone: editing stays locked.
    unavailable: AtomicBool,
    /// An edit was refused for a reason a sync may lift; the worker syncs when its schedule allows.
    resync_wanted: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn repaint(&self) {
        if let Some(ctx) = self.ctx.get() {
            ctx.request_repaint();
        }
    }

    /// Every later command gets the same answer, instead of leaving the page waiting.
    fn unavailable(&self) {
        self.unavailable.store(true, Ordering::SeqCst);
        self.pending.store(0, Ordering::SeqCst);
        self.in_browser.store(false, Ordering::SeqCst);
        *lock(&self.snapshot) = AccountSnapshot { error: Some(AccountError::Other), ..AccountSnapshot::default() };
        self.repaint();
    }
}

pub struct DesktopAccount {
    shared: Arc<Shared>,
    commands: Sender<Command>,
}

impl DesktopAccount {
    /// `store` is the OS credential store in the app; tests pass a `MemorySecretStore`.
    fn new(config: Config, store: Arc<dyn SecretStore>, opener: Opener) -> Self {
        Self::with_timing(config, store, opener, ResyncTiming::default())
    }

    fn with_timing(mut config: Config, store: Arc<dyn SecretStore>, opener: Opener, timing: ResyncTiming) -> Self {
        let vietnamese = i18n::current().code() == "vi";
        config.ui_locale = Some(if vietnamese { "vi" } else { "en" }.into());
        config.callback_success_html = callback_page("Signed in to PI ID. You can close this tab and go back to the app.");
        config.callback_failure_html = callback_page("Sign-in did not finish. Close this tab and try again in the app.");
        let shared = Arc::new(Shared::default());
        let (commands, rx) = mpsc::channel();
        let worker = shared.clone();
        let spawned = std::thread::Builder::new().name("pi-account".into()).spawn(move || Worker::run(worker, config, store, opener, timing, rx));
        if let Err(error) = spawned {
            // Without the worker nothing can grant a lease; the page says PI ID is unavailable.
            log::warn!("PI account worker not started: {error}");
            shared.unavailable();
        }
        Self { shared, commands }
    }

    /// A hook whose account never opens: File › Account says so and editing stays locked.
    fn unavailable() -> Self {
        let shared = Arc::new(Shared::default());
        shared.unavailable();
        let (commands, _) = mpsc::channel();
        Self { shared, commands }
    }

    /// Queues a user command and shows it as running at once.
    fn send(&self, command: Command) {
        let shared = &self.shared;
        shared.pending.fetch_add(1, Ordering::SeqCst);
        *lock(&shared.error) = None;
        {
            let mut snapshot = lock(&shared.snapshot);
            snapshot.busy = true;
            snapshot.error = None;
            if shared.in_browser.load(Ordering::SeqCst) {
                snapshot.view = AccountView::SigningIn;
            }
        }
        if self.commands.send(command).is_err() {
            shared.unavailable();
        }
    }

    /// Queues a command that waits for the browser, shown as the sign-in wait with its Cancel.
    fn start_in_browser(&self, command: Command) {
        self.shared.cancelled.store(false, Ordering::SeqCst);
        self.shared.in_browser.store(true, Ordering::SeqCst);
        self.send(command);
    }
}

impl AccountHook for DesktopAccount {
    fn snapshot(&self) -> AccountSnapshot {
        lock(&self.shared.snapshot).clone()
    }

    fn sign_in(&self) {
        self.start_in_browser(Command::SignIn);
    }

    fn cancel_sign_in(&self) {
        // Not queued: the worker is blocked inside `sign_in` or `select_org` until this unblocks it.
        self.shared.cancelled.store(true, Ordering::SeqCst);
        if let Some(account) = self.shared.account.get() {
            account.cancel_sign_in();
        }
    }

    fn sign_out(&self) {
        self.send(Command::SignOut);
    }

    fn select_org(&self, org_id: Option<String>) {
        // PI switches organization only through a new browser authorization (ADR 0011).
        self.start_in_browser(Command::SelectOrg(org_id));
    }

    fn retry_activation(&self) {
        self.send(Command::RetryActivation);
    }

    fn allowed(&self, feature: &str) -> bool {
        // Until the account has opened, answer as for a guest.
        let Some(account) = self.shared.account.get() else { return !is_pro_feature(feature) };
        let allowed = account.allowed(feature);
        if account.take_refresh_request() && !self.shared.refresh_queued.swap(true, Ordering::SeqCst) {
            let _ = self.commands.send(Command::Refresh);
        }
        allowed
    }

    fn warning_shown(&self, days: u32) {
        let stage = match days {
            7 => WarningStage::Days7,
            3 => WarningStage::Days3,
            1 => WarningStage::Days1,
            _ => return,
        };
        let _ = self.commands.send(Command::WarningShown(stage));
    }

    fn attach(&self, ctx: &egui::Context) {
        let _ = self.shared.ctx.set(ctx.clone());
        ctx.request_repaint();
    }

    fn edit_lock(&self) -> Option<EditLock> {
        if self.shared.unavailable.load(Ordering::SeqCst) {
            return Some(EditLock::Unavailable);
        }
        let Some(account) = self.shared.account.get() else { return Some(EditLock::Checking) };
        gate::lock_of(&account.status(), |feature| self.allowed(feature))
    }

    fn edit_refused(&self) {
        // The worker rate-limits these syncs; only the first refusal sends, so they never pile up.
        if self.edit_lock().is_some_and(EditLock::sync_may_lift) && !self.shared.resync_wanted.swap(true, Ordering::SeqCst) {
            let _ = self.commands.send(Command::Resync);
        }
    }
}

/// The page the browser shows after the sign-in callback, in the interface language.
fn callback_page(key: &str) -> String {
    let text = i18n::t(key).replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!("<!doctype html><meta charset=\"utf-8\"><title>PI ID</title><p style=\"font:16px sans-serif;margin:3em\">{text}</p>")
}

struct Worker {
    shared: Arc<Shared>,
    account: Arc<Account>,
    opener: Opener,
    next_refresh: Instant,
    next_reload: Instant,
    resync: Resync,
}

impl Worker {
    fn run(shared: Arc<Shared>, config: Config, store: Arc<dyn SecretStore>, opener: Opener, timing: ResyncTiming, rx: Receiver<Command>) {
        let account = match Account::new(config, store) {
            Ok(account) => Arc::new(account),
            Err(error) => {
                log::warn!("PI account unavailable: {error}");
                shared.unavailable();
                while rx.recv().is_ok() {
                    shared.unavailable();
                }
                return;
            }
        };
        let _ = shared.account.set(account.clone());
        if let Err(error) = account.reload() {
            log::warn!("PI session not read: {error}");
        }
        let now = Instant::now();
        // The first refresh runs at once (when signed in), then hourly.
        let resync = Resync::new(timing, now);
        let mut worker = Worker { shared, account, opener, next_refresh: now, next_reload: now + RELOAD_EVERY, resync };
        worker.publish();
        loop {
            let due = worker.next_refresh.min(worker.next_reload);
            let due = if worker.shared.resync_wanted.load(Ordering::SeqCst) { due.min(worker.resync.not_before) } else { due };
            match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(Command::Refresh) => {
                    worker.shared.refresh_queued.store(false, Ordering::SeqCst);
                    worker.refresh();
                }
                Ok(Command::WarningShown(stage)) => {
                    if let Err(error) = worker.account.mark_warning_shown(stage) {
                        log::warn!("PI expiry warning not recorded: {error}");
                    }
                }
                Ok(Command::Resync) => {}
                Ok(command) => {
                    worker.handle(command);
                    worker.shared.pending.fetch_sub(1, Ordering::SeqCst);
                }
                Err(RecvTimeoutError::Timeout) => worker.tick(),
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if worker.shared.resync_wanted.load(Ordering::SeqCst) && Instant::now() >= worker.resync.not_before {
                worker.resync();
            }
            worker.publish();
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if now >= self.next_refresh {
            self.refresh();
        } else if now >= self.next_reload {
            self.next_reload = now + RELOAD_EVERY;
            if let Err(error) = self.account.reload() {
                log::warn!("PI session not read: {error}");
            }
        }
    }

    /// Start-up, hourly and on-demand sync. Its failures stay silent and are retried: the page
    /// already says when the app is offline, and only the user's own commands show errors.
    fn refresh(&mut self) {
        let _ = self.sync();
    }

    /// [`refresh`](Self::refresh), returning how it went. A guest has nothing to sync.
    fn sync(&mut self) -> Result<(), Error> {
        let now = Instant::now();
        self.next_reload = now + RELOAD_EVERY;
        if !matches!(self.account.status(), Status::SignedIn(_)) {
            if let Err(error) = self.account.reload() {
                log::warn!("PI session not read: {error}");
            }
            self.next_refresh = now + REFRESH_EVERY;
            return Ok(());
        }
        let result = self.account.refresh();
        let wait = match &result {
            Ok(()) => REFRESH_EVERY,
            Err(error) if error.is_transient() => {
                log::info!("PI account refresh postponed: {error}");
                error.retry_after().map_or(RETRY_OFFLINE, |after| after.max(MIN_RETRY))
            }
            Err(error) => {
                log::warn!("PI account refresh failed: {error}");
                REFRESH_EVERY
            }
        };
        self.next_refresh = Instant::now() + wait;
        result
    }

    /// The sync a refused edit asked for, then the next one's slot.
    fn resync(&mut self) {
        // Cleared first: an edit refused while this runs asks again for the next slot.
        self.shared.resync_wanted.store(false, Ordering::SeqCst);
        let result = self.sync();
        let still_refused = gate::lock_of(&self.account.status(), |feature| self.account.allowed(feature)).is_some_and(EditLock::sync_may_lift);
        self.resync.after(Instant::now(), &result, still_refused);
    }

    /// The user's Retry: sync now, and activate again when a device cap or a missing plan
    /// blocked activation (the library keeps that block until asked).
    fn retry(&mut self) -> Result<(), Error> {
        self.resync.failures = 0;
        let synced = self.sync();
        let blocked = matches!(self.account.status(), Status::SignedIn(s)
            if matches!(s.licence, LicenceState::DeviceCapReached | LicenceState::NotEntitled));
        if blocked { self.account.retry_activation() } else { synced }
    }

    fn handle(&mut self, command: Command) {
        let result = match command {
            Command::SignIn => self.through_browser(|account, open| account.sign_in(open)),
            Command::SignOut => self.account.sign_out(),
            Command::SelectOrg(org) => self.through_browser(|account, open| account.select_org(org.as_deref(), open)),
            Command::RetryActivation => self.retry(),
            Command::WarningShown(_) | Command::Refresh | Command::Resync => Ok(()),
        };
        match result {
            Ok(()) => self.next_refresh = Instant::now() + REFRESH_EVERY,
            Err(error) => {
                log::warn!("PI account: {error}");
                *lock(&self.shared.error) = error_view(&error);
            }
        }
    }

    fn through_browser(&self, run: impl FnOnce(&Account, &dyn Fn(&str) -> Result<(), String>) -> Result<(), Error>) -> Result<(), Error> {
        let (shared, account, opener) = (&self.shared, &self.account, &self.opener);
        let open = |url: &str| {
            if shared.cancelled.load(Ordering::SeqCst) {
                account.cancel_sign_in();
                return Ok(());
            }
            opener(url)
        };
        let result = run(account, &open);
        shared.in_browser.store(false, Ordering::SeqCst);
        match result {
            // A browser that failed to open after Cancel is still a cancel, not an error to show.
            Err(Error::Browser(_)) if shared.cancelled.load(Ordering::SeqCst) => Err(Error::Cancelled),
            other => other,
        }
    }

    fn publish(&self) {
        let view = view_of(&self.account.status());
        let shared = &self.shared;
        {
            // The flags are read under the lock, so a click's optimistic state is never replaced by an older view.
            let mut snapshot = lock(&shared.snapshot);
            let view = if shared.in_browser.load(Ordering::SeqCst) { AccountView::SigningIn } else { view };
            *snapshot = AccountSnapshot { view, busy: shared.pending.load(Ordering::SeqCst) > 0, error: *lock(&shared.error) };
        }
        shared.repaint();
    }
}

fn view_of(status: &Status) -> AccountView {
    let s = match status {
        Status::Guest => return AccountView::Guest,
        Status::SigningIn => return AccountView::SigningIn,
        Status::SignedIn(s) => s,
    };
    let (name, email) = s.user.as_ref().map(|u| (u.display_name.clone(), u.email.clone())).unwrap_or_default();
    AccountView::SignedIn(SignedInView {
        name,
        email,
        orgs: s
            .orgs
            .iter()
            .filter(|m| m.status == MembershipStatus::Active)
            .map(|m| OrgView { id: m.org_id.clone(), name: m.name.clone(), role: Some(m.role.clone()).filter(|r| !r.is_empty()) })
            .collect(),
        active_org: s.active_org.clone(),
        licence: licence_of(&s.licence),
        warning: s.warning.map(|w| WarningView { days: w.stage.days(), expires_at: w.expires_at }),
        online: s.online,
        session_only: s.session_only,
        reauth_required: s.reauth_required,
    })
}

fn licence_of(licence: &LicenceState) -> LicenceView {
    match licence {
        LicenceState::NotActivated => LicenceView::Free,
        LicenceState::Active { features, expires_at, .. } => {
            LicenceView::Active { expires_at: *expires_at, pro: features.iter().any(|f| is_pro_feature(f)) }
        }
        LicenceState::Inactive { expires_at, .. } => LicenceView::Inactive { expires_at: *expires_at },
        LicenceState::DeviceRevoked => LicenceView::DeviceRevoked,
        LicenceState::DeviceCapReached => LicenceView::DeviceCapReached,
        // The organization has no paid plan for this product: free features are all there is.
        LicenceState::NotEntitled => LicenceView::Free,
        LicenceState::SessionOnly => LicenceView::SessionOnly,
    }
}

/// The error line for a failed command; `None` when the licence line already says it (activation
/// blocked, no credential store) or there is nothing to tell.
fn error_view(error: &Error) -> Option<AccountError> {
    Some(match error {
        Error::SignInInProgress | Error::ActivationBlocked | Error::CredentialStoreUnavailable => return None,
        Error::Cancelled => AccountError::Cancelled,
        Error::Network(_) => AccountError::Network,
        Error::Timeout | Error::SignInTimedOut => AccountError::Timeout,
        Error::AuthorizationDenied(_) => AccountError::Denied,
        Error::InvalidGrant | Error::NotSignedIn => AccountError::ReauthRequired,
        Error::UnknownOrg | Error::OrgUnavailable => AccountError::OrgUnavailable,
        Error::Revoked(RevocationReason::DeviceRevoked) => AccountError::DeviceRevoked,
        Error::Revoked(RevocationReason::MembershipRevoked) => AccountError::OrgUnavailable,
        Error::Revoked(RevocationReason::UserSuspended) => AccountError::Suspended,
        Error::Api(api) => match &api.code {
            ApiCode::RateLimited => AccountError::RateLimited { retry_after_secs: api.retry_after.map(|after| after.as_secs()) },
            ApiCode::DependencyUnavailable | ApiCode::IdempotencyInProgress => AccountError::ServiceUnavailable,
            ApiCode::DeviceCapReached => AccountError::DeviceCap,
            ApiCode::DeviceRevoked | ApiCode::DeviceUnknown => AccountError::DeviceRevoked,
            ApiCode::UserSuspended => AccountError::Suspended,
            ApiCode::MembershipInactive => AccountError::OrgUnavailable,
            ApiCode::Unauthenticated | ApiCode::InsufficientScope => AccountError::ReauthRequired,
            ApiCode::ClientVersionUnsupported => AccountError::ClientTooOld,
            // The site shell treats an exhausted quota like a refusal, not as something to wait out.
            ApiCode::Forbidden | ApiCode::QuotaExceeded => AccountError::Denied,
            _ => AccountError::Other,
        },
        _ => AccountError::Other,
    })
}

#[cfg(test)]
mod licence_tests;
#[cfg(test)]
mod tests;
