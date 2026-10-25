use std::pin::pin;
use std::time::{Duration, Instant};

use futures::future::{select, Either};
use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, AsyncApp, Context, Entity, EventEmitter, FontWeight, Hsla, SharedString,
    Subscription, Task, WeakEntity, Window,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{h_flex, v_flex};
use gpui_mobile::KeyboardType;
use uuid::Uuid;
use zeroize::Zeroizing;

use flint_core::route::{self, CredentialIssue};
use flint_core::shell::ShellHandle;
use flint_core::ssh::{
    self, AuthCredential, AuthError, AuthPrompt, HostKeyPrompt, KeyIssue, SshConnection,
    SshConnectionEvent, SshError, SshHop, SshRoute,
};
use flint_core::vault::{AuthMethod, SnippetBody};

use crate::connect_view::ConnectTarget;
use crate::files_view::{FilesEvent, FilesView};
use crate::insets::{self, Insets};
use crate::keys_bar::{KeysBar, KEYS_BAR_HEIGHT};
use crate::phone_theme;
use crate::recent::{self, RecentHost};
use crate::store::Store;
use crate::theme as flint_theme;
use crate::ui::host_icon::host_icon;
use crate::ui::terminal_palette::TerminalColors;
use crate::ui::terminal_view::{TerminalEvent, TerminalView};
use crate::widgets::{
    body_text, card_inset, danger_button, dialog, dialog_title, empty_state, error_text,
    field_input, icon, icon_button, primary_button, quiet_button, row, row_code, row_subtitle,
    row_title, secondary_button, sentence, sheet, sheet_action, sheet_heading, spinner, status_dot,
    text_button, title,
};

/// After this long in the background, the connection may have died without notice.
const STALE_AFTER: Duration = Duration::from_secs(15);

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

const OS_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// A drop noticed this soon after returning from the background reconnects by itself.
const AUTO_RECONNECT_WINDOW: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub enum SessionTarget {
    /// Only the id: the route is rebuilt on every connect, so host edits apply.
    Saved { id: Uuid },
    Quick(ConnectTarget),
}

pub enum SessionEvent {
    /// Back to the hosts list; the session keeps running.
    Hide,
    Closed,
    ShowSessions,
    EditHost(Uuid),
    LoginRefused(RecentHost, SharedString),
}

enum Phase {
    Connecting,
    Connected,
    /// Had a terminal, which stays on screen to read.
    Lost,
    Failed(SharedString),
}

struct AuthQuestion {
    prompt: AuthPrompt,
    inputs: Vec<Entity<InputState>>,
}

struct PassphraseQuestion {
    key_name: String,
    pem: Zeroizing<String>,
    input: Entity<InputState>,
    error: Option<SharedString>,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Terminal,
    Files,
}

struct OutputSearch {
    input: Entity<InputState>,
    query: String,
    /// 1-based index of the current match; 0 before the first step.
    position: usize,
    /// Matches in the whole buffer; `None` until counted.
    total: Option<usize>,
    _subscription: Subscription,
}

enum Sheet {
    Menu,
    Snippets,
}

pub struct SessionView {
    target: SessionTarget,
    pub title: SharedString,
    pub address: SharedString,
    pub os: Option<String>,
    phase: Phase,
    connection: Option<SshConnection>,
    terminal: Option<Entity<TerminalView>>,
    /// Kept so reconnects do not ask for the key's passphrase again.
    passphrase: Option<Zeroizing<String>>,
    host_key_question: Option<HostKeyPrompt>,
    pending_auth_prompt: Option<AuthPrompt>,
    auth_question: Option<AuthQuestion>,
    pending_passphrase: Option<(String, Zeroizing<String>, KeyIssue)>,
    passphrase_question: Option<PassphraseQuestion>,
    sheet: Option<Sheet>,
    search: Option<OutputSearch>,
    mode: Mode,
    files: Option<(Entity<FilesView>, Subscription)>,
    pub session_count: usize,
    is_focus_pending: bool,
    inactive_since: Option<Instant>,
    returned_at: Option<Instant>,
    _drive_task: Option<Task<()>>,
    _probe_task: Option<Task<()>>,
    _os_task: Option<Task<()>>,
    _terminal_subscriptions: Vec<Subscription>,
}

impl EventEmitter<SessionEvent> for SessionView {}

impl SessionView {
    pub fn new(target: SessionTarget, cx: &mut Context<Self>) -> Self {
        let (title, address, os) = Self::names(&target, cx);
        let mut view = Self {
            target,
            title,
            address,
            os,
            phase: Phase::Connecting,
            connection: None,
            terminal: None,
            passphrase: None,
            host_key_question: None,
            pending_auth_prompt: None,
            auth_question: None,
            pending_passphrase: None,
            passphrase_question: None,
            sheet: None,
            search: None,
            mode: Mode::Terminal,
            files: None,
            session_count: 1,
            is_focus_pending: false,
            inactive_since: None,
            returned_at: None,
            _drive_task: None,
            _probe_task: None,
            _os_task: None,
            _terminal_subscriptions: Vec::new(),
        };
        if let SessionTarget::Saved { id } = view.target {
            Store::global(cx).update(cx, |store, _| {
                store.note(|vault| vault.touch_entry_opened(id));
            });
        }
        view.connect(cx);
        view
    }

    pub fn host_id(&self) -> Option<Uuid> {
        match self.target {
            SessionTarget::Saved { id } => Some(id),
            SessionTarget::Quick(_) => None,
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.phase, Phase::Connected)
    }

    pub fn status(&self) -> (&'static str, Hsla, bool) {
        match &self.phase {
            Phase::Connecting => ("Connecting…", flint_theme::warning(), true),
            Phase::Connected => ("Connected", flint_theme::status_online(), false),
            Phase::Lost => ("Connection lost", flint_theme::red_error(), false),
            Phase::Failed(_) => ("Not connected", flint_theme::red_error(), false),
        }
    }

    pub fn last_lines(&self, count: usize, cx: &gpui::App) -> Vec<String> {
        let Some(terminal) = &self.terminal else {
            return Vec::new();
        };
        let Some(snapshot) = &terminal.read(cx).last_snapshot else {
            return Vec::new();
        };
        let cursor_row = usize::from(snapshot.cursor.0);
        let mut lines: Vec<String> = snapshot
            .cells
            .iter()
            .take(cursor_row + 1)
            .map(|row| {
                let line: String = row
                    .iter()
                    .filter(|cell| !cell.wide_char_spacer)
                    .map(|cell| cell.c)
                    .collect();
                line.trim_end().to_string()
            })
            .collect();
        while lines.first().is_some_and(String::is_empty) {
            lines.remove(0);
        }
        let start = lines.len().saturating_sub(count);
        lines.split_off(start)
    }

    fn names(
        target: &SessionTarget,
        cx: &gpui::App,
    ) -> (SharedString, SharedString, Option<String>) {
        match target {
            SessionTarget::Quick(target) => {
                let address = target.recent().address();
                (target.host.clone().into(), address.into(), None)
            }
            SessionTarget::Saved { id } => {
                let entry = Store::global(cx)
                    .read(cx)
                    .vault()
                    .and_then(|vault| vault.get_entry(*id).ok());
                match entry {
                    Some(entry) => {
                        let title = if entry.label.is_empty() {
                            entry.host.clone()
                        } else {
                            entry.label.clone()
                        };
                        let address = RecentHost {
                            host: entry.host,
                            port: entry.port,
                            username: entry.username,
                            host_key: None,
                        }
                        .address();
                        (title.into(), address.into(), entry.os)
                    }
                    None => ("Host".into(), SharedString::default(), None),
                }
            }
        }
    }

    /// An empty `Err` means the key needs a passphrase first; see `pending_passphrase`.
    fn route(&mut self, cx: &mut Context<Self>) -> Result<SshRoute, String> {
        let id = match &self.target {
            SessionTarget::Quick(target) => {
                return Ok(SshRoute {
                    jumps: Vec::new(),
                    target: SshHop {
                        entry_id: Uuid::new_v4().to_string(),
                        label: target.host.clone(),
                        host: target.host.clone(),
                        port: target.port,
                        username: target.username.clone(),
                        credential: AuthCredential::Password(target.password.clone()),
                        expected_host_key: target.host_key.clone(),
                    },
                    proxy: None,
                });
            }
            SessionTarget::Saved { id } => *id,
        };
        let store = Store::global(cx);
        let store = store.read(cx);
        let vault = store.vault().ok_or("The vault is not open.")?;
        let entry = vault
            .get_entry(id)
            .map_err(|_| "This host is no longer in your vault.".to_string())?;
        if matches!(entry.auth, AuthMethod::Agent) {
            return Err(
                "This host signs in with the SSH agent of your computer. Give it a password \
                 or a key to use it here."
                    .into(),
            );
        }
        let credential = match route::target_credential(vault, &entry) {
            Ok(credential) => credential,
            Err(CredentialIssue::KeyMissing) => {
                return Err("The SSH key of this host is not in your vault.".into());
            }
            Err(CredentialIssue::Passphrase {
                key_name,
                pem,
                issue,
            }) => match &self.passphrase {
                Some(passphrase) => AuthCredential::PrivateKey {
                    pem,
                    passphrase: Some(passphrase.clone()),
                },
                None => {
                    self.pending_passphrase = Some((key_name, pem, issue));
                    return Err(String::new());
                }
            },
        };
        self.title = if entry.label.is_empty() {
            entry.host.clone().into()
        } else {
            entry.label.clone().into()
        };
        route::route_for(vault, &entry, credential)
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        self.phase = Phase::Connecting;
        self.host_key_question = None;
        self.pending_auth_prompt = None;
        self.auth_question = None;
        let route = match self.route(cx) {
            Ok(route) => route,
            Err(reason) if reason.is_empty() => {
                cx.notify();
                return;
            }
            Err(reason) => {
                self.on_failed(reason, cx);
                return;
            }
        };
        let connection = SshConnection::spawn(route);
        self.connection = Some(connection.clone());
        // Dropping the old task keeps the old connection's late events out.
        self._drive_task = Some(cx.spawn(async move |this, cx| {
            drive_connection(connection, this, cx).await;
        }));
        cx.notify();
    }

    pub fn set_app_active(&mut self, is_active: bool, cx: &mut Context<Self>) {
        if !is_active {
            self.inactive_since.get_or_insert_with(Instant::now);
            return;
        }
        let away = self.inactive_since.take().map(|since| since.elapsed());
        if away.is_some() {
            self.returned_at = Some(Instant::now());
        }
        match self.phase {
            Phase::Lost => self.connect(cx),
            Phase::Connected if away.is_some_and(|away| away >= STALE_AFTER) => {
                self.check_connection(cx)
            }
            _ => {}
        }
    }

    pub fn focus_terminal(&mut self, cx: &mut Context<Self>) {
        self.is_focus_pending = self.terminal.is_some() && self.mode == Mode::Terminal;
        cx.notify();
    }

    pub fn show_files(&mut self, cx: &mut Context<Self>) {
        self.close_search(cx);
        gpui_mobile::hide_keyboard();
        self.mode = Mode::Files;
        self.is_focus_pending = false;
        self.ensure_files(cx);
        cx.notify();
    }

    fn show_terminal(&mut self, cx: &mut Context<Self>) {
        self.mode = Mode::Terminal;
        self.is_focus_pending = self.terminal.is_some();
        cx.notify();
    }

    fn ensure_files(&mut self, cx: &mut Context<Self>) {
        if self.files.is_some() || !matches!(self.phase, Phase::Connected) {
            return;
        }
        let Some(connection) = self.connection.clone() else {
            return;
        };
        let host_id = self.host_id();
        let files = cx.new(|cx| FilesView::new(connection, host_id, cx));
        let subscription = cx.subscribe(&files, |view, _, event: &FilesEvent, cx| match event {
            FilesEvent::OpenInTerminal(path) => {
                if let Some(terminal) = view.terminal.clone() {
                    let command = format!("cd -- {}\r", shell_quote(path));
                    terminal.update(cx, |terminal, cx| {
                        terminal.send_input(command.into_bytes(), cx)
                    });
                }
                view.show_terminal(cx);
            }
            FilesEvent::Reconnect => view.connect(cx),
        });
        self.files = Some((files, subscription));
    }

    /// Keepalive takes minutes to notice a dead connection; a round trip notices at once.
    fn check_connection(&mut self, cx: &mut Context<Self>) {
        let Some(connection) = self.connection.clone() else {
            return;
        };
        let timer = cx.background_executor().timer(PROBE_TIMEOUT);
        self._probe_task = Some(cx.spawn(async move |this, cx| {
            let probe = connection.open_exec("true".to_string());
            let is_alive = match select(pin!(probe), pin!(timer)).await {
                Either::Left((result, _)) => result.is_ok(),
                Either::Right(_) => false,
            };
            if is_alive {
                return;
            }
            tracing::info!("connection did not survive the background, reconnecting");
            this.update(cx, |view, cx| view.connect(cx)).ok();
        }));
    }

    fn on_connected(&mut self, shell: Result<ShellHandle, SshError>, cx: &mut Context<Self>) {
        let shell = match shell {
            Ok(shell) => shell,
            Err(e) => {
                self.on_failed(format!("Could not open a shell: {e}"), cx);
                return;
            }
        };
        let terminal = cx.new(|cx| TerminalView::new(shell, cx));
        self._terminal_subscriptions = vec![
            cx.subscribe(
                &terminal,
                |view, _, event: &TerminalEvent, cx| match event {
                    TerminalEvent::Exited => view.close(cx),
                    TerminalEvent::Disconnected => view.on_lost(cx),
                },
            ),
            // The hosts list shows the last lines of an open session.
            cx.observe(&terminal, |_, _, cx| cx.notify()),
        ];
        self.terminal = Some(terminal);
        self.phase = Phase::Connected;
        self.is_focus_pending = self.mode == Mode::Terminal;
        // Files opened before keep their folder on the new connection.
        match (&self.files, self.connection.clone()) {
            (Some((files, _)), Some(connection)) => {
                files.update(cx, |files, cx| files.attach(connection, cx));
            }
            _ if self.mode == Mode::Files => self.ensure_files(cx),
            _ => {}
        }
        match &self.target {
            SessionTarget::Quick(target) => recent::remember(target.recent()),
            SessionTarget::Saved { id } => {
                let id = *id;
                self.run_startup_snippet(id, cx);
                self.detect_os(id, cx);
            }
        }
        cx.notify();
    }

    fn detect_os(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(connection) = self.connection.clone() else {
            return;
        };
        let limit = cx.background_executor().timer(OS_PROBE_TIMEOUT);
        // Separate task, so a host whose exec never closes cannot block the session.
        self._os_task = Some(cx.spawn(async move |this, cx| {
            let os = match select(pin!(detect_os(&connection)), pin!(limit)).await {
                Either::Left((Some(os), _)) => os,
                _ => return,
            };
            this.update(cx, |view, cx| {
                if view.os.as_deref() == Some(os.as_str()) {
                    return;
                }
                view.os = Some(os.clone());
                Store::global(cx).update(cx, |store, cx| {
                    store.note(|vault| vault.set_entry_os(id, os));
                    cx.notify();
                });
                cx.notify();
            })
            .ok();
        }));
    }

    /// Runs on every reconnect too, since each one gets a fresh shell.
    fn run_startup_snippet(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let startup = Store::global(cx).read(cx).vault().and_then(|vault| {
            let snippet_id = vault.get_entry(id).ok()?.startup_snippet?;
            Some((snippet_id, vault.script_for_host(id, snippet_id)?))
        });
        if let Some((snippet_id, content)) = startup {
            self.send_snippet(snippet_id, SnippetBody::Script { content }, cx);
        }
    }

    fn send_snippet(&mut self, snippet_id: Uuid, body: SnippetBody, cx: &mut Context<Self>) {
        let Some(terminal) = self.terminal.clone() else {
            return;
        };
        let SnippetBody::Script { content } = body;
        let mut bytes = content.into_bytes();
        bytes.push(b'\r');
        terminal.update(cx, |terminal, cx| terminal.send_input(bytes, cx));
        if let Some(server_id) = self.host_id() {
            Store::global(cx).update(cx, |store, _| {
                // A host's own snippet keeps no run history.
                store.note(|vault| match vault.get_snippet(snippet_id) {
                    Ok(_) => vault.touch_snippet_used(snippet_id, server_id),
                    Err(_) => Ok(()),
                });
            });
        }
    }

    fn on_lost(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Connected) {
            return;
        }
        self.phase = Phase::Lost;
        self.connection = None;
        if let Some((files, _)) = &self.files {
            files.update(cx, |files, cx| files.on_lost(cx));
        }
        // Taken, so a server that keeps dropping the connection cannot cause a reconnect loop.
        let returned_at = self.returned_at.take();
        let is_just_back = returned_at.is_some_and(|at| at.elapsed() < AUTO_RECONNECT_WINDOW);
        if is_just_back && self.inactive_since.is_none() {
            self.connect(cx);
            return;
        }
        cx.notify();
    }

    fn on_failed(&mut self, reason: String, cx: &mut Context<Self>) {
        self.connection = None;
        self.host_key_question = None;
        self.auth_question = None;
        self.phase = match self.terminal {
            Some(_) => Phase::Lost,
            None => Phase::Failed(explain(&reason).into()),
        };
        cx.notify();
    }

    fn on_auth_failed(&mut self, error: AuthError, cx: &mut Context<Self>) {
        let is_login_refused = matches!(
            error,
            AuthError::WrongPassword
                | AuthError::KeyRejected(_)
                | AuthError::KbdInteractiveRejected
        );
        match (&self.target, error) {
            (_, AuthError::HostKeyRejected | AuthError::Canceled) => self.close(cx),
            // A refused reconnect stays here, so the old terminal can still be read.
            (SessionTarget::Quick(target), error)
                if is_login_refused && self.terminal.is_none() =>
            {
                self.connection = None;
                let message = sentence(&error.to_string());
                cx.emit(SessionEvent::LoginRefused(target.recent(), message.into()));
            }
            (_, error) => self.on_failed(sentence(&error.to_string()), cx),
        }
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.connection = None;
        self._drive_task = None;
        gpui_mobile::hide_keyboard();
        cx.emit(SessionEvent::Closed);
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        gpui_mobile::hide_keyboard();
        cx.emit(SessionEvent::Hide);
    }

    fn ask_host_key(&mut self, prompt: HostKeyPrompt, cx: &mut Context<Self>) {
        gpui_mobile::hide_keyboard();
        self.host_key_question = Some(prompt);
        cx.notify();
    }

    fn answer_host_key(&mut self, is_trusted: bool, cx: &mut Context<Self>) {
        let Some(prompt) = self.host_key_question.take() else {
            return;
        };
        if is_trusted {
            self.remember_host_key(prompt.fingerprint.clone(), cx);
        }
        let _ = prompt.reply.send(is_trusted);
        cx.notify();
    }

    fn remember_host_key(&mut self, fingerprint: String, cx: &mut Context<Self>) {
        match &mut self.target {
            SessionTarget::Quick(target) => target.host_key = Some(fingerprint),
            SessionTarget::Saved { id } => {
                let id = *id;
                Store::global(cx).update(cx, |store, cx| {
                    if let Err(e) =
                        store.edit(cx, |vault| vault.set_entry_host_key(id, fingerprint))
                    {
                        tracing::warn!(error = %e, "could not save the host key");
                    }
                });
            }
        }
    }

    fn answer_auth(&mut self, is_sent: bool, cx: &mut Context<Self>) {
        let Some(question) = self.auth_question.take() else {
            return;
        };
        let answers = is_sent.then(|| {
            question
                .inputs
                .iter()
                .map(|input| Zeroizing::new(input.read(cx).value().to_string()))
                .collect()
        });
        let _ = question.prompt.reply.send(answers);
        gpui_mobile::hide_keyboard();
        cx.notify();
    }

    fn answer_passphrase(&mut self, is_sent: bool, cx: &mut Context<Self>) {
        if !is_sent {
            self.passphrase_question = None;
            gpui_mobile::hide_keyboard();
            self.close(cx);
            return;
        }
        let Some(question) = &mut self.passphrase_question else {
            return;
        };
        let answer = Zeroizing::new(question.input.read(cx).value().to_string());
        if ssh::check_key_passphrase(&question.pem, Some(answer.as_str())).is_some() {
            question.error = Some("Wrong passphrase.".into());
            cx.notify();
            return;
        }
        self.passphrase_question = None;
        self.passphrase = Some(answer);
        gpui_mobile::hide_keyboard();
        self.connect(cx);
    }

    /// Input fields need a `Window`, so prompts that arrived without one get them here.
    fn take_pending_prompts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(prompt) = self.pending_auth_prompt.take() {
            let inputs = prompt
                .fields
                .iter()
                .map(|field| cx.new(|cx| InputState::new(window, cx).masked(!field.echo)))
                .collect::<Vec<_>>();
            if let (Some(first), Some(field)) = (inputs.first(), prompt.fields.first()) {
                first.update(cx, |input, cx| input.focus(window, cx));
                gpui_mobile::show_keyboard_with_type(if field.echo {
                    KeyboardType::Default
                } else {
                    KeyboardType::Password
                });
            }
            self.auth_question = Some(AuthQuestion { prompt, inputs });
        }
        if let Some((key_name, pem, issue)) = self.pending_passphrase.take() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Passphrase")
            });
            input.update(cx, |input, cx| input.focus(window, cx));
            gpui_mobile::show_keyboard_with_type(KeyboardType::Password);
            let error = matches!(issue, KeyIssue::WrongPassphrase)
                .then(|| "The saved passphrase no longer opens this key.".into());
            self.passphrase_question = Some(PassphraseQuestion {
                key_name,
                pem,
                input,
                error,
            });
        }
    }

    fn open_sheet(&mut self, sheet: Sheet, cx: &mut Context<Self>) {
        gpui_mobile::hide_keyboard();
        self.sheet = Some(sheet);
        cx.notify();
    }

    fn tile(&self, size: f32, ring: Hsla) -> AnyElement {
        host_icon(self.os.as_deref(), false, ring, size)
    }

    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet = None;
        if self.terminal.is_none() {
            return;
        }
        if let Some(search) = &self.search {
            search.input.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in output"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |view, input, event: &InputEvent, _, cx| match event {
                InputEvent::Focus => gpui_mobile::show_keyboard(),
                InputEvent::Change => {
                    let query = input.read(cx).value().to_string();
                    view.set_search_query(query, cx);
                }
                InputEvent::PressEnter { .. } => view.step_search(true, cx),
                InputEvent::Blur => {}
            },
        );
        input.update(cx, |input, cx| input.focus(window, cx));
        self.search = Some(OutputSearch {
            input,
            query: String::new(),
            position: 0,
            total: None,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn set_search_query(&mut self, query: String, cx: &mut Context<Self>) {
        let (Some(search), Some(terminal)) = (&mut self.search, &self.terminal) else {
            return;
        };
        search.query = query.clone();
        search.position = 0;
        if query.is_empty() {
            search.total = None;
        }
        cx.notify();
        let shell = terminal.read(cx).shell.clone();
        cx.spawn(async move |this, cx| {
            let total = shell.set_search(query.clone()).await;
            this.update(cx, |view, cx| {
                // The query may have changed while the worker counted; drop a stale reply.
                let Some(search) = &mut view.search else {
                    return;
                };
                if search.query == query {
                    search.total = total;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn step_search(&mut self, is_forward: bool, cx: &mut Context<Self>) {
        let (Some(search), Some(terminal)) = (&self.search, &self.terminal) else {
            return;
        };
        if !matches!(search.total, Some(total) if total > 0) {
            return;
        }
        let query = search.query.clone();
        let shell = terminal.read(cx).shell.clone();
        cx.spawn(async move |this, cx| {
            let progress = shell.step_search(is_forward).await;
            this.update(cx, |view, cx| {
                let Some(search) = &mut view.search else {
                    return;
                };
                let Some((position, total)) = progress else {
                    return;
                };
                if search.query == query {
                    search.position = position;
                    search.total = Some(total);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn close_search(&mut self, cx: &mut Context<Self>) {
        if self.search.take().is_none() {
            return;
        }
        if let Some(terminal) = &self.terminal {
            let shell = terminal.read(cx).shell.clone();
            cx.spawn(async move |_, _| {
                shell.set_search(String::new()).await;
            })
            .detach();
        }
        gpui_mobile::hide_keyboard();
        self.is_focus_pending = true;
        cx.notify();
    }

    fn render_search_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let search = self.search.as_ref()?;
        let status = match (search.query.is_empty(), search.position, search.total) {
            (true, _, _) | (_, _, None) => String::new(),
            (_, _, Some(0)) => "No matches".to_string(),
            (_, 0, Some(total)) => format!("{total} found"),
            (_, position, Some(total)) => format!("{position} of {total}"),
        };
        let has_matches = matches!(search.total, Some(total) if total > 0);
        let step = |id: &'static str, name: &'static str| {
            div()
                .id(id)
                .flex_none()
                .size(px(38.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .when(has_matches, |button| {
                    button.active(|button| button.bg(phone_theme::fill_pressed()))
                })
                .child(icon(
                    name,
                    20.,
                    if has_matches {
                        flint_theme::text()
                    } else {
                        flint_theme::text_placeholder()
                    },
                ))
        };
        Some(
            h_flex()
                .h(px(KEYS_BAR_HEIGHT))
                .w_full()
                .px(px(8.))
                .gap(px(4.))
                .bg(phone_theme::keys_ground())
                .child(
                    div().flex_1().min_w_0().child(
                        Styled::h(
                            Input::new(&search.input).prefix(icon(
                                "search",
                                16.,
                                flint_theme::text_dim(),
                            )),
                            px(38.),
                        )
                        .rounded(px(10.))
                        .bg(phone_theme::key_face())
                        .border_color(gpui::transparent_black())
                        .text_size(px(15.)),
                    ),
                )
                .child(
                    div()
                        .flex_none()
                        .min_w(px(36.))
                        .text_center()
                        .text_size(px(13.))
                        .text_color(flint_theme::text_dim())
                        .child(status),
                )
                .child(
                    step("search-previous", "chevron-up")
                        .on_click(cx.listener(|view, _, _, cx| view.step_search(false, cx))),
                )
                .child(
                    step("search-next", "chevron-down")
                        .on_click(cx.listener(|view, _, _, cx| view.step_search(true, cx))),
                )
                .child(
                    text_button("search-done", "Done")
                        .px(px(8.))
                        .on_click(cx.listener(|view, _, _, cx| view.close_search(cx))),
                )
                .into_any_element(),
        )
    }

    fn render_top_bar(&self, insets: Insets, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .pt(px(insets.top))
            .bg(phone_theme::ground())
            .child(
                h_flex()
                    .w_full()
                    .h(px(58.))
                    .px(px(12.))
                    .gap(px(10.))
                    .child(
                        icon_button("back", "chevron-left")
                            .on_click(cx.listener(|view, _, _, cx| view.hide(cx))),
                    )
                    .child(self.tile(32., phone_theme::ground()))
                    .child(self.render_title(cx))
                    .child(self.render_mode_switch(cx))
                    .child(
                        icon_button("menu", "ellipsis").on_click(
                            cx.listener(|view, _, _, cx| view.open_sheet(Sheet::Menu, cx)),
                        ),
                    ),
            )
    }

    fn render_title(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (status, color, is_busy) = self.status();
        let has_others = self.session_count > 1;
        v_flex()
            .id("title")
            .flex_1()
            .min_w_0()
            .gap(px(2.))
            .child(
                h_flex()
                    .gap(px(4.))
                    .min_w_0()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(flint_theme::text())
                            .child(self.title.clone()),
                    )
                    .when(has_others, |line| {
                        line.child(icon("chevron-down", 15., flint_theme::text_dim()))
                    }),
            )
            .child(
                h_flex()
                    .gap(px(6.))
                    .child(status_dot(color, is_busy))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(flint_theme::text_dim())
                            .child(status),
                    ),
            )
            .when(has_others, |title| {
                title
                    .active(|title| title.opacity(0.6))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SessionEvent::ShowSessions)))
            })
    }

    fn render_mode_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let segment = |id: &'static str, name: &'static str, mode: Mode| {
            let is_current = self.mode == mode;
            div()
                .id(id)
                .size(px(34.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .when(is_current, |segment| {
                    segment.bg(phone_theme::selected()).shadow_sm()
                })
                .child(icon(
                    name,
                    18.,
                    if is_current {
                        flint_theme::text()
                    } else {
                        flint_theme::text_dim()
                    },
                ))
        };
        h_flex()
            .flex_none()
            .p(px(3.))
            .gap(px(2.))
            .rounded_full()
            .bg(phone_theme::fill())
            .child(
                segment("show-terminal", "square-terminal", Mode::Terminal)
                    .on_click(cx.listener(|view, _, _, cx| view.show_terminal(cx))),
            )
            .child(
                segment("show-files", "folder", Mode::Files)
                    .on_click(cx.listener(|view, _, _, cx| view.show_files(cx))),
            )
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        if let (Mode::Files, Some((files, _))) = (self.mode, &self.files) {
            return div()
                .flex_1()
                .min_h_0()
                .child(files.clone())
                .into_any_element();
        }
        if let Some(terminal) = &self.terminal {
            return v_flex()
                .flex_1()
                .min_h_0()
                .children(matches!(self.phase, Phase::Lost).then(|| self.render_lost_bar(cx)))
                .child(div().flex_1().min_h_0().pt(px(6.)).child(terminal.clone()))
                .into_any_element();
        }
        let content = match &self.phase {
            Phase::Failed(reason) => v_flex()
                .w_full()
                .max_w(px(360.))
                .gap(px(8.))
                .child(title("Could not connect").px(px(4.)))
                .child(body_text(reason.clone()).px(px(4.)))
                .child(div().h(px(16.)))
                .child(
                    primary_button("retry", "Try again")
                        .on_click(cx.listener(|view, _, _, cx| view.connect(cx))),
                )
                .children(self.host_id().map(|id| {
                    secondary_button("edit-host", "Edit host").on_click(
                        cx.listener(move |_, _, _, cx| cx.emit(SessionEvent::EditHost(id))),
                    )
                }))
                .child(
                    quiet_button("close", "Close")
                        .on_click(cx.listener(|view, _, _, cx| view.close(cx))),
                ),
            _ => v_flex()
                .items_center()
                .gap(px(14.))
                .child(spinner(28., flint_theme::text_dim()))
                .child(
                    v_flex()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_size(px(15.))
                                .text_color(flint_theme::text_dim())
                                .child("Connecting to"),
                        )
                        .child(
                            div()
                                .font_family(phone_theme::MONO_FONT)
                                .text_size(px(14.))
                                .text_color(flint_theme::text())
                                .child(self.address.clone()),
                        ),
                )
                .child(
                    text_button("cancel-connect", "Cancel")
                        .mt(px(8.))
                        .on_click(cx.listener(|view, _, _, cx| view.close(cx))),
                ),
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .px(px(24.))
            .pb(px(40.))
            .bg(phone_theme::ground())
            .child(content)
            .into_any_element()
    }

    fn render_lost_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("lost-bar")
            // A tap on it must not place the terminal's cursor or open the keyboard.
            .occlude()
            .w_full()
            .px(px(16.))
            .py(px(6.))
            .gap(px(12.))
            .bg(phone_theme::ground())
            .border_b_1()
            .border_color(flint_theme::border())
            .child(status_dot(flint_theme::red_error(), false))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(15.))
                    .text_color(flint_theme::text())
                    .child("Connection lost"),
            )
            .child(
                text_button("reconnect", "Reconnect")
                    .on_click(cx.listener(|view, _, _, cx| view.connect(cx))),
            )
    }

    fn render_host_key_question(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompt = self.host_key_question.as_ref()?;
        let is_changed = prompt.stored_fingerprint.is_some();
        let host = &self.title;
        let (title, text) = if is_changed {
            (
                "The server key has changed",
                format!(
                    "{host} shows a key other than the one you trusted. Someone may be \
                     intercepting the connection. Connect only if you know the server was \
                     reinstalled."
                ),
            )
        } else {
            (
                "New server",
                format!(
                    "This is the first connection to {host}. If you can, check that the \
                     fingerprint matches the server's."
                ),
            )
        };
        let trust = if is_changed {
            danger_button("trust", "Connect anyway")
        } else {
            primary_button("trust", "Trust and connect")
        }
        .on_click(cx.listener(|view, _, _, cx| view.answer_host_key(true, cx)));
        Some(dialog(
            v_flex()
                .gap(px(14.))
                .child(dialog_title(title, is_changed))
                .child(body_text(text))
                .child(
                    v_flex()
                        .p(px(14.))
                        .gap(px(6.))
                        .rounded(px(14.))
                        .bg(phone_theme::plate())
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(flint_theme::text_dim())
                                .child(prompt.key_type.clone()),
                        )
                        .child(
                            div()
                                .font_family(phone_theme::MONO_FONT)
                                .text_size(px(13.))
                                .line_height(px(19.))
                                .text_color(flint_theme::text())
                                .child(prompt.fingerprint.clone()),
                        ),
                )
                .child(trust)
                .child(
                    quiet_button("distrust", "Cancel")
                        .on_click(cx.listener(|view, _, _, cx| view.answer_host_key(false, cx))),
                ),
        ))
    }

    fn render_auth_question(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let question = self.auth_question.as_ref()?;
        let prompt = &question.prompt;
        let title = if prompt.name.trim().is_empty() {
            "The server asks".to_string()
        } else {
            prompt.name.clone()
        };
        let fields = prompt
            .fields
            .iter()
            .zip(&question.inputs)
            .map(|(field, input)| {
                v_flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .px(px(4.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(flint_theme::text_dim())
                            .child(field.text.trim().to_string()),
                    )
                    .child(field_input(input))
            });
        Some(dialog(
            v_flex()
                .gap(px(14.))
                .child(dialog_title(title, false))
                .when(!prompt.instructions.trim().is_empty(), |column| {
                    column.child(body_text(prompt.instructions.trim().to_string()))
                })
                .children(fields)
                .child(
                    primary_button("auth-send", "Continue")
                        .on_click(cx.listener(|view, _, _, cx| view.answer_auth(true, cx))),
                )
                .child(
                    quiet_button("auth-cancel", "Cancel")
                        .on_click(cx.listener(|view, _, _, cx| view.answer_auth(false, cx))),
                ),
        ))
    }

    fn render_passphrase_question(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let question = self.passphrase_question.as_ref()?;
        Some(dialog(
            v_flex()
                .gap(px(14.))
                .child(dialog_title("Key passphrase", false))
                .child(body_text(format!(
                    "The key \"{}\" is encrypted. Its passphrase is kept for this session only.",
                    question.key_name
                )))
                .child(field_input(&question.input))
                .children(question.error.clone().map(error_text))
                .child(
                    primary_button("passphrase-send", "Connect")
                        .on_click(cx.listener(|view, _, _, cx| view.answer_passphrase(true, cx))),
                )
                .child(
                    quiet_button("passphrase-cancel", "Cancel")
                        .on_click(cx.listener(|view, _, _, cx| view.answer_passphrase(false, cx))),
                ),
        ))
    }

    fn render_sheet(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet_kind = self.sheet.as_ref()?;
        let this = cx.entity().downgrade();
        let dismiss = move |_: &mut Window, cx: &mut gpui::App| {
            this.update(cx, |view, cx| {
                view.sheet = None;
                cx.notify();
            })
            .ok();
        };
        let content = match sheet_kind {
            Sheet::Menu => self.render_menu(cx),
            Sheet::Snippets => self.render_snippets(cx),
        };
        Some(sheet("session-sheet", content, dismiss))
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut actions = Vec::new();
        if self.terminal.is_some() {
            actions.push(
                sheet_action("menu-find", "search", "Find in output", false)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.mode = Mode::Terminal;
                        view.open_search(window, cx);
                    }))
                    .into_any_element(),
            );
        }
        if self.terminal.is_some() && self.host_id().is_some() {
            actions.push(
                sheet_action("menu-snippets", "braces", "Snippets", false)
                    .on_click(cx.listener(|view, _, _, cx| view.open_sheet(Sheet::Snippets, cx)))
                    .into_any_element(),
            );
        }
        if self.terminal.is_some() {
            actions.push(
                sheet_action("menu-paste", "clipboard-paste", "Paste", false)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.sheet = None;
                        if let Some(terminal) = view.terminal.clone() {
                            terminal.update(cx, |terminal, cx| terminal.paste(cx));
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        if matches!(self.phase, Phase::Lost) {
            actions.push(
                sheet_action("menu-reconnect", "refresh-cw", "Reconnect", false)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.sheet = None;
                        view.connect(cx);
                    }))
                    .into_any_element(),
            );
        }
        if let Some(id) = self.host_id() {
            actions.push(
                sheet_action("menu-edit", "pen-line", "Edit host", false)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.sheet = None;
                        cx.emit(SessionEvent::EditHost(id));
                    }))
                    .into_any_element(),
            );
        }
        v_flex()
            .gap(px(16.))
            .child(
                h_flex()
                    .gap(px(14.))
                    .px(px(4.))
                    .child(self.tile(44., phone_theme::ground()))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(3.))
                            .child(title(self.title.clone()).truncate())
                            .child(row_code(self.address.clone())),
                    ),
            )
            .when(!actions.is_empty(), |menu| {
                menu.child(card_inset(actions, 50.))
            })
            .child(card_inset(
                [
                    sheet_action("menu-disconnect", "unplug", "Disconnect", true)
                        .on_click(cx.listener(|view, _, _, cx| view.close(cx)))
                        .into_any_element(),
                ],
                50.,
            ))
            .into_any_element()
    }

    fn render_snippets(&self, cx: &mut Context<Self>) -> AnyElement {
        let server_id = self.host_id();
        let store = Store::global(cx);
        let mut snippets = store
            .read(cx)
            .vault()
            .and_then(|vault| vault.list_snippets().ok())
            .unwrap_or_default();
        let used_here = |snippet: &flint_core::vault::Snippet| {
            snippet
                .usages
                .iter()
                .filter(|usage| Some(usage.server_id) == server_id)
                .map(|usage| usage.at)
                .max()
                .unwrap_or(0)
        };
        snippets.sort_by(|a, b| {
            used_here(b)
                .cmp(&used_here(a))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        let header = sheet_heading("Snippets", row_subtitle(format!("Runs on {}", self.title)));
        if snippets.is_empty() {
            return v_flex()
                .child(header)
                .child(empty_state(
                    "No snippets yet",
                    "Make them in flint on your computer; they come here with the next sync.",
                ))
                .into_any_element();
        }
        let rows = snippets.into_iter().enumerate().map(|(index, snippet)| {
            let SnippetBody::Script { content } = &snippet.body;
            let preview = content.lines().next().unwrap_or_default().to_string();
            let id = snippet.id;
            let body = snippet.body.clone();
            row(("snippet", index))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(3.))
                        .child(row_title(snippet.name.clone()))
                        .child(row_code(preview)),
                )
                .child(icon(
                    "corner-down-left",
                    18.,
                    flint_theme::text_placeholder(),
                ))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.sheet = None;
                    view.send_snippet(id, body.clone(), cx);
                    view.is_focus_pending = true;
                    cx.notify();
                }))
                .into_any_element()
        });
        v_flex()
            .gap(px(16.))
            .child(header)
            .child(card_inset(rows, 14.))
            .into_any_element()
    }
}

impl Render for SessionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.take_pending_prompts(window, cx);
        if self.is_focus_pending && self.sheet.is_none() && self.search.is_none() {
            self.is_focus_pending = false;
            if let Some(terminal) = self.terminal.clone() {
                cx.on_next_frame(window, move |_, window, cx| {
                    terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
                });
            }
        }

        let insets = insets::current();
        let background = TerminalColors::current(cx).bg;
        let this = cx.entity().downgrade();
        let has_snippets = self.host_id().is_some();
        let is_terminal_shown = self.terminal.is_some()
            && !matches!(self.phase, Phase::Failed(_))
            && self.mode == Mode::Terminal;
        let search_bar = self.render_search_bar(cx).filter(|_| is_terminal_shown);
        let keys_bar = self
            .terminal
            .clone()
            .filter(|_| is_terminal_shown && search_bar.is_none())
            .map(|terminal| {
                let keys_bar = KeysBar::new(terminal, insets.is_keyboard_visible);
                if !has_snippets {
                    return keys_bar;
                }
                keys_bar.on_snippets(move |_, cx| {
                    this.update(cx, |view, cx| view.open_sheet(Sheet::Snippets, cx))
                        .ok();
                })
            });
        // The bottom inset strip matches what is above it: the keys bar or the file list.
        let bottom_strip = if self.mode == Mode::Files && self.files.is_some() {
            phone_theme::ground()
        } else {
            phone_theme::keys_ground()
        };

        div()
            .relative()
            .size_full()
            .bg(background)
            .child(
                v_flex()
                    .size_full()
                    .pl(px(insets.left))
                    .pr(px(insets.right))
                    .child(self.render_top_bar(insets, cx))
                    .child(self.render_body(cx))
                    .children(search_bar)
                    .children(keys_bar)
                    .child(div().h(px(insets.bottom)).w_full().bg(bottom_strip)),
            )
            .children(self.render_host_key_question(cx))
            .children(self.render_auth_question(cx))
            .children(self.render_passphrase_question(cx))
            .children(self.render_sheet(cx))
    }
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Plain words for common network errors; the system's texts and codes say little.
fn explain(reason: &str) -> String {
    const KNOWN: [(&[&str], &str); 5] = [
        (
            &["connection refused"],
            "Nothing answers on that port. Check that SSH runs on the server and that the \
             port is right.",
        ),
        (
            &["timed out", "timeout"],
            "The server did not answer in time. Check the address, and that this network \
             can reach it.",
        ),
        (
            &[
                "failed to lookup address",
                "name or service not known",
                "nodename nor servname",
                "no such host",
                "name resolution",
            ],
            "This address could not be found. Check it for typos.",
        ),
        (
            &["network is unreachable", "no route to host", "host is down"],
            "The server cannot be reached from this network. It may need a VPN.",
        ),
        (
            &["connection reset"],
            "The server closed the connection. It may let SSH in from some addresses only.",
        ),
    ];
    let lower = reason.to_lowercase();
    if let Some((_, text)) = KNOWN
        .iter()
        .find(|(needles, _)| needles.iter().any(|needle| lower.contains(needle)))
    {
        return (*text).to_string();
    }
    // The title already says "Could not connect", and the os error code means nothing here.
    let text = reason.trim();
    let text = text
        .strip_prefix("Could not connect: ")
        .or_else(|| text.strip_prefix("could not connect: "))
        .unwrap_or(text);
    let text = text.find(" (os error").map_or(text, |end| &text[..end]);
    sentence(text)
}

/// A copy of the desktop's probe in `server_session.rs`. Gives e.g. "Ubuntu 22.04".
async fn detect_os(connection: &SshConnection) -> Option<String> {
    let command = ". /etc/os-release 2>/dev/null && \
        printf '%s %s' \"$NAME\" \"$VERSION_ID\" || uname -sr"
        .to_string();
    let handle = connection.open_exec(command).await.ok()?;
    let output = handle.subscribe();
    let mut bytes: Vec<u8> = Vec::new();
    // The exec channel closes when the command exits.
    while let Ok(chunk) = output.recv().await {
        bytes.extend_from_slice(&chunk);
    }
    let label = String::from_utf8_lossy(&bytes).trim().to_string();
    (!label.is_empty()).then_some(label)
}

async fn drive_connection(
    connection: SshConnection,
    this: WeakEntity<SessionView>,
    cx: &mut AsyncApp,
) {
    let events = connection.subscribe_events();
    while let Ok(event) = events.recv().await {
        let result = match event {
            SshConnectionEvent::Connected => {
                let shell = connection.open_shell(24, 80).await;
                this.update(cx, |view, cx| view.on_connected(shell, cx))
            }
            SshConnectionEvent::AuthFailed { error, .. } => {
                this.update(cx, |view, cx| view.on_auth_failed(error, cx))
            }
            SshConnectionEvent::HostKeyPrompt(prompt) => {
                this.update(cx, |view, cx| view.ask_host_key(prompt, cx))
            }
            SshConnectionEvent::AuthPrompt(prompt) => this.update(cx, |view, cx| {
                view.pending_auth_prompt = Some(prompt);
                cx.notify();
            }),
            SshConnectionEvent::HostKeyTrusted { fingerprint, .. } => {
                this.update(cx, |view, cx| view.remember_host_key(fingerprint, cx))
            }
            SshConnectionEvent::Disconnected => this.update(cx, |view, cx| view.on_lost(cx)),
            SshConnectionEvent::ForwardChanged { .. } => Ok(()),
        };
        if result.is_err() {
            return;
        }
    }
    // The connection thread ended without telling us why.
    this.update(cx, |view, cx| match view.phase {
        Phase::Connecting => view.on_failed("The connection closed.".to_string(), cx),
        Phase::Connected => view.on_lost(cx),
        Phase::Lost | Phase::Failed(_) => {}
    })
    .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_network_errors_are_said_in_words() {
        assert!(
            explain("Could not connect: Connection refused (os error 111).")
                .starts_with("Nothing answers on that port")
        );
        assert!(explain(
            "Could not connect: failed to lookup address information: Name or \
                         service not known."
        )
        .starts_with("This address could not be found"));
        assert!(explain("Could not connect: connection timed out.")
            .starts_with("The server did not answer"));
    }

    #[test]
    fn a_path_is_one_word_for_the_shell() {
        assert_eq!(shell_quote("/var/log"), "'/var/log'");
        assert_eq!(shell_quote("/srv/it's here"), "'/srv/it'\\''s here'");
    }

    #[test]
    fn other_errors_lose_the_repeated_title_and_the_code() {
        assert_eq!(
            explain("Could not connect: Permission denied (os error 13)."),
            "Permission denied."
        );
        assert_eq!(explain("Wrong password."), "Wrong password.");
    }
}
