//! Vault, sign-in and sync. The phone can only join an account made on a computer.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::channel::oneshot;
use gpui::{App, AppContext as _, Context, Entity, Global, SharedString};
use secrecy::SecretString;

use flint_core::account::flow::{self, Created, PreVaultError, Round, RoundWork};
use flint_core::account::{self, AccountError, Api, GitHubStep, Landing, OAuthError};
use flint_core::vault::sync::AdoptedAccount;
use flint_core::vault::{self, VaultError, VaultManager};

use crate::keychain;
use crate::web_auth;
use crate::widgets::sentence;

/// How often to check for local edits to send.
const TICK: Duration = Duration::from_secs(20);
const PULL_INTERVAL: Duration = Duration::from_secs(120);
const RETRY_DELAY: Duration = Duration::from_secs(60);
/// Stops two devices editing the same host from syncing back and forth forever.
const MAX_ROUNDS_IN_A_ROW: u8 = 3;
/// Returning to the app refreshes, but not more often than this.
const REFRESH_AFTER: Duration = Duration::from_secs(30);

pub enum VaultState {
    Missing,
    /// No keychain key opens it, so it needs the master password.
    Locked,
    Open(VaultManager),
}

pub enum SignIn {
    Idle,
    Browser,
    Password(Arc<GitHubStep>),
    Working(Arc<GitHubStep>),
}

pub struct Store {
    pub vault: VaultState,
    pub sign_in: SignIn,
    pub sign_in_error: Option<SharedString>,
    pub is_unlocking: bool,
    pub unlock_error: Option<SharedString>,
    is_cancelled: Arc<AtomicBool>,
    /// Bumped per sign-in attempt, so a late answer to a cancelled one is ignored.
    attempt: u64,
    /// Bumped on sign-out, so a late result for one account is not applied to the next.
    generation: u64,
    pub is_syncing: bool,
    pub sync_error: Option<SharedString>,
    pub last_synced: Option<Instant>,
    /// The first rounds after opening are still running, so hosts may still be arriving.
    pub is_first_series: bool,
    teams_checked: Option<Instant>,
    retry_after: Option<Instant>,
    rounds_left: u8,
}

struct GlobalStore(Entity<Store>);

impl Global for GlobalStore {}

fn in_background<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> oneshot::Receiver<T> {
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx
}

impl Store {
    pub fn install(cx: &mut App) {
        let store = cx.new(|cx: &mut Context<Self>| {
            cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            })
            .detach();
            let mut store = Self {
                vault: VaultState::Missing,
                sign_in: SignIn::Idle,
                sign_in_error: None,
                is_unlocking: false,
                unlock_error: None,
                is_cancelled: Arc::new(AtomicBool::new(false)),
                attempt: 0,
                generation: 0,
                is_syncing: false,
                sync_error: None,
                last_synced: None,
                is_first_series: false,
                teams_checked: None,
                retry_after: None,
                rounds_left: 0,
            };
            store.open_at_start(cx);
            store
        });
        cx.set_global(GlobalStore(store));
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalStore>().0.clone()
    }

    fn path() -> Option<PathBuf> {
        vault::default_vault_path().ok()
    }

    fn open_at_start(&mut self, cx: &mut Context<Self>) {
        let Some(path) = Self::path().filter(|path| path.exists()) else {
            self.vault = VaultState::Missing;
            return;
        };
        let opened = keychain::load().and_then(|key| {
            VaultManager::unlock_with_device_key(&path, &key)
                .inspect_err(|e| tracing::warn!(?e, "the device key did not open the vault"))
                .ok()
        });
        match opened {
            Some(manager) => self.set_open(manager, false, cx),
            None => self.vault = VaultState::Locked,
        }
    }

    fn set_open(&mut self, manager: VaultManager, is_new_key: bool, cx: &mut Context<Self>) {
        if is_new_key {
            keychain::save(&manager.device_key());
        }
        self.vault = VaultState::Open(manager);
        // The next round then pulls the personal vault first, see `flow::is_first_pull`.
        self.last_synced = None;
        self.is_first_series = true;
        self.refresh_all(cx);
        cx.notify();
    }

    pub fn vault(&self) -> Option<&VaultManager> {
        match &self.vault {
            VaultState::Open(manager) => Some(manager),
            _ => None,
        }
    }

    pub fn edit<T>(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut VaultManager) -> Result<T, VaultError>,
    ) -> Result<T, String> {
        let VaultState::Open(manager) = &mut self.vault else {
            return Err("The vault is not open.".into());
        };
        let result = change(manager).map_err(|e| sentence(&e.to_string()))?;
        cx.notify();
        self.sync_now(cx);
        Ok(result)
    }

    /// A small change that does not need to reach the server at once.
    pub fn note(&mut self, change: impl FnOnce(&mut VaultManager) -> Result<(), VaultError>) {
        if let VaultState::Open(manager) = &mut self.vault {
            if let Err(e) = change(manager) {
                tracing::warn!(?e, "could not note a change in the vault");
            }
        }
    }

    pub fn github_login(&self) -> Option<&str> {
        Some(self.vault()?.sync_state()?.github_login.as_str())
    }

    pub fn is_session_ended(&self) -> bool {
        self.vault()
            .and_then(VaultManager::sync_state)
            .is_some_and(|state| state.session_token.is_none())
    }

    /// Small edits wait in memory, and the app may be killed in the background.
    pub fn save_now(&mut self) {
        if let VaultState::Open(manager) = &mut self.vault {
            if let Err(e) = manager.save_pending() {
                tracing::error!(?e, "could not save the vault");
            }
        }
    }

    pub fn unlock(&mut self, password: SecretString, cx: &mut Context<Self>) {
        if !matches!(self.vault, VaultState::Locked) || self.is_unlocking {
            return;
        }
        let Some(path) = Self::path() else { return };
        self.is_unlocking = true;
        self.unlock_error = None;
        cx.notify();
        let opened = in_background(move || VaultManager::unlock_with_password(&path, &password));
        cx.spawn(async move |this, cx| {
            let Ok(opened) = opened.await else { return };
            this.update(cx, |this, cx| {
                this.is_unlocking = false;
                match opened {
                    Ok(manager) => this.set_open(manager, true, cx),
                    Err(VaultError::InvalidPassword) => {
                        this.unlock_error = Some("Wrong master password.".into());
                    }
                    Err(e) => this.unlock_error = Some(sentence(&e.to_string()).into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn start_sign_in(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.sign_in, SignIn::Idle) {
            return;
        }
        self.sign_in = SignIn::Browser;
        self.sign_in_error = None;
        // An earlier attempt's page may still be waiting on its port.
        self.is_cancelled.store(true, Ordering::Relaxed);
        self.is_cancelled = Arc::new(AtomicBool::new(false));
        self.attempt += 1;
        let attempt = self.attempt;
        cx.notify();

        let (url_tx, url_rx) = oneshot::channel::<String>();
        let is_cancelled = self.is_cancelled.clone();
        let step = in_background(move || {
            let open_browser = |url| {
                let _ = url_tx.send(url);
            };
            account::github_sign_in(
                &Api::shared(),
                open_browser,
                &is_cancelled,
                Landing::Redirect(web_auth::LANDING),
            )
        });
        let is_cancelled = self.is_cancelled.clone();
        cx.spawn(async move |this, cx| {
            if let Ok(url) = url_rx.await {
                let is_current = this
                    .update(cx, |this, _| this.attempt == attempt)
                    .unwrap_or(false);
                if is_current {
                    let opened = cx.update(|cx| web_auth::open(&url, is_cancelled.clone(), cx));
                    if let Err(e) = opened {
                        tracing::error!(error = e, "could not show GitHub's sign-in page");
                        // Stop the port; `finish_github` gets Cancelled and keeps this error.
                        is_cancelled.store(true, Ordering::Relaxed);
                        this.update(cx, |this, cx| {
                            this.sign_in_error =
                                Some("Could not open GitHub's sign-in page.".into());
                            cx.notify();
                        })
                        .ok();
                    }
                }
            }
            let Ok(step) = step.await else { return };
            this.update(cx, |this, cx| {
                if this.attempt == attempt {
                    this.finish_github(step, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn finish_github(&mut self, step: Result<GitHubStep, AccountError>, cx: &mut Context<Self>) {
        // After a failure or timeout the sheet would stay on a page that waits forever.
        web_auth::close();
        self.sign_in = SignIn::Idle;
        match step {
            Ok(step) if step.is_new => {
                self.sign_in_error = Some(
                    format!(
                        "@{} has no flint account yet. Create it in flint on your computer, \
                         then sign in here.",
                        step.github_login
                    )
                    .into(),
                );
            }
            Ok(step) => self.sign_in = SignIn::Password(Arc::new(step)),
            Err(AccountError::OAuth(OAuthError::Cancelled)) => {}
            Err(e) => {
                tracing::warn!(error = %e, "GitHub sign-in failed");
                self.sign_in_error = Some(sentence(&e.to_string()).into());
            }
        }
        cx.notify();
    }

    pub fn cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        self.is_cancelled.store(true, Ordering::Relaxed);
        web_auth::close();
        self.attempt += 1;
        self.sign_in = SignIn::Idle;
        self.sign_in_error = None;
        cx.notify();
    }

    pub fn submit_password(&mut self, password: SecretString, cx: &mut Context<Self>) {
        let SignIn::Password(step) = &self.sign_in else {
            return;
        };
        let step = step.clone();
        self.sign_in = SignIn::Working(step.clone());
        self.sign_in_error = None;
        cx.notify();

        let generation = self.generation;
        if matches!(self.vault, VaultState::Open(_)) {
            let adopted = in_background({
                let step = step.clone();
                move || account::sign_in(&Api::shared(), &step, &password)
            });
            cx.spawn(async move |this, cx| {
                let Ok(adopted) = adopted.await else { return };
                this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.finish_sign_in_again(&step, adopted, cx);
                    }
                })
                .ok();
            })
            .detach();
            return;
        }

        let created = in_background({
            let step = step.clone();
            move || {
                let path = vault::default_vault_path().map_err(AccountError::Vault)?;
                flow::create_vault_for(&path, &step, &password)
            }
        });
        cx.spawn(async move |this, cx| {
            let Ok(created) = created.await else { return };
            this.update(cx, |this, cx| this.finish_new_vault(&step, created, cx))
                .ok();
        })
        .detach();
    }

    fn finish_new_vault(
        &mut self,
        step: &Arc<GitHubStep>,
        created: Result<Created, PreVaultError>,
        cx: &mut Context<Self>,
    ) {
        match created {
            Ok(Created::Opened(manager) | Created::WithPhrase(manager, _)) => {
                tracing::info!(login = %step.github_login, "vault made from the account");
                self.sign_in = SignIn::Idle;
                self.set_open(manager, true, cx);
            }
            Err(PreVaultError::Account(e)) => self.show_sign_in_error(step, &e),
            Err(PreVaultError::VaultExists) => {
                self.sign_in = SignIn::Idle;
                self.open_at_start(cx);
            }
            Err(PreVaultError::NotSaved(e) | PreVaultError::RegisteredNotSaved(e, _)) => {
                tracing::error!(?e, "signed in, but the vault was not saved");
                self.sign_in = SignIn::Idle;
                self.sign_in_error =
                    Some(format!("Signed in, but the vault could not be saved: {e}.").into());
            }
        }
        cx.notify();
    }

    fn finish_sign_in_again(
        &mut self,
        step: &Arc<GitHubStep>,
        adopted: Result<AdoptedAccount, AccountError>,
        cx: &mut Context<Self>,
    ) {
        let VaultState::Open(manager) = &mut self.vault else {
            self.sign_in = SignIn::Idle;
            return;
        };
        let installed =
            adopted.and_then(|account| manager.adopt_account(account).map_err(AccountError::Vault));
        match installed {
            Ok(()) => {
                keychain::save(&manager.device_key());
                self.sign_in = SignIn::Idle;
                self.sync_error = None;
                self.retry_after = None;
                self.refresh_all(cx);
            }
            Err(e) => self.show_sign_in_error(step, &e),
        }
        cx.notify();
    }

    fn show_sign_in_error(&mut self, step: &Arc<GitHubStep>, error: &AccountError) {
        tracing::warn!(error = %error, "sign-in failed");
        // Another password will not fix these; the GitHub step must be done again.
        let is_stale = matches!(
            error,
            AccountError::Api(api) if matches!(api.code(), Some("ticket_invalid" | "account_exists"))
        );
        self.sign_in = if is_stale {
            SignIn::Idle
        } else {
            SignIn::Password(step.clone())
        };
        self.sign_in_error = Some(flow::sign_in_problem(error, step.is_new).into());
    }

    /// Deletes only the vault on this phone; the hosts stay in the account.
    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        let token = self
            .vault()
            .and_then(VaultManager::sync_state)
            .and_then(|state| state.session_token.clone());
        flow::log_out_quietly(token);
        // Dropping the manager wipes the key from memory, so do it first.
        self.vault = VaultState::Missing;
        keychain::delete();
        if let Some(path) = Self::path() {
            if let Err(e) = vault::delete_vault_files(&path) {
                tracing::error!(?e, "could not delete the vault on this phone");
            }
        }
        self.cancel_sign_in(cx);
        self.generation += 1;
        self.is_syncing = false;
        self.sync_error = None;
        self.last_synced = None;
        self.teams_checked = None;
        self.retry_after = None;
        self.rounds_left = 0;
        cx.notify();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.retry_after.is_some_and(|at| Instant::now() < at) {
            return;
        }
        let is_pull_due = self
            .last_synced
            .is_none_or(|at| at.elapsed() >= PULL_INTERVAL);
        let has_edits = self.vault().is_some_and(VaultManager::has_unsynced_changes);
        let is_refresh_owed = self.teams_checked.is_none();
        if is_pull_due || has_edits || is_refresh_owed {
            self.sync_now(cx);
        }
    }

    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        self.rounds_left = MAX_ROUNDS_IN_A_ROW;
        self.run_round(cx);
    }

    /// Like `sync_now`, but also checks the teams.
    pub fn refresh_all(&mut self, cx: &mut Context<Self>) {
        self.teams_checked = None;
        self.sync_now(cx);
    }

    pub fn refresh_if_stale(&mut self, cx: &mut Context<Self>) {
        if self.retry_after.is_some_and(|at| Instant::now() < at) {
            return;
        }
        if self
            .teams_checked
            .is_some_and(|at| at.elapsed() < REFRESH_AFTER)
        {
            return;
        }
        self.refresh_all(cx);
    }

    fn run_round(&mut self, cx: &mut Context<Self>) {
        if self.is_syncing || self.rounds_left == 0 {
            return;
        }
        let Some(vault) = self.vault() else { return };
        let is_teams_due = self
            .teams_checked
            .is_none_or(|at| at.elapsed() >= PULL_INTERVAL);
        let Some(work) = RoundWork::plan(vault, is_teams_due, self.last_synced.is_some()) else {
            self.is_first_series = false;
            return;
        };
        if work.is_team_check() {
            self.teams_checked = Some(Instant::now());
        }
        self.rounds_left -= 1;
        self.is_syncing = true;
        cx.notify();

        let round = in_background(move || work.run(&Api::shared()));
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let round = round.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match round {
                    Ok(round) => this.finish_round(round, cx),
                    Err(_) => {
                        this.is_syncing = false;
                        this.is_first_series = false;
                        this.sync_error = Some("Sync stopped unexpectedly.".into());
                        this.retry_after = Some(Instant::now() + RETRY_DELAY);
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn finish_round(&mut self, round: Round, cx: &mut Context<Self>) {
        self.is_syncing = false;
        let was_team_check_ok = matches!(round, Round::Teams(_, Ok(Some(_))));
        let outcome = match &mut self.vault {
            VaultState::Open(manager) => flow::take_in(manager, round),
            // Signed out while it ran.
            _ => Ok((false, false)),
        };
        match outcome {
            Ok((has_more, _)) => {
                self.sync_error = None;
                self.retry_after = None;
                self.last_synced = Some(Instant::now());
                // The team check may have found a new team vault, which needs another round.
                if was_team_check_ok {
                    self.rounds_left = self.rounds_left.max(1);
                }
                let is_refresh_owed = self.teams_checked.is_none();
                if has_more || is_refresh_owed {
                    self.run_round(cx);
                }
            }
            Err(message) => {
                tracing::warn!(error = %message, "sync round failed");
                self.sync_error = Some(sentence(&message).into());
                self.retry_after = Some(Instant::now() + RETRY_DELAY);
            }
        }
        if !self.is_syncing {
            self.is_first_series = false;
        }
        cx.notify();
    }
}
