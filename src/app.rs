use std::time::Duration;

use gpui::prelude::*;
use gpui::{div, px, AnyElement, AnyView, Context, Entity, Subscription, Task, Window};
use gpui_component::{h_flex, v_flex, ActiveTheme as _};
use uuid::Uuid;

use crate::account_view::{AccountEvent, AccountView};
use crate::connect_view::{ConnectEvent, ConnectView};
use crate::host_form::{HostFormEvent, HostFormView};
use crate::hosts_view::{HostsEvent, HostsView};
use crate::insets::{self, Insets};
use crate::phone_theme;
use crate::preferences::{Preferences, ThemePreference};
use crate::session_view::{SessionEvent, SessionTarget, SessionView};
use crate::settings_view::{SettingsEvent, SettingsView};
use crate::shell;
use crate::store::{SignIn, Store, VaultState};
use crate::theme as flint_theme;
use crate::toast;
use crate::ui::host_icon::host_icon;
use crate::widgets::{
    card, chevron, icon, row, row_code, row_subtitle, row_title, secondary_button, sheet,
    status_dot, title,
};

/// gpui gets no event when the keyboard moves, so the insets are polled for the keys bar.
const INSETS_POLL_INTERVAL: Duration = Duration::from_millis(150);

enum Overlay {
    QuickConnect(Entity<ConnectView>),
    HostForm(Entity<HostFormView>),
    Settings(Entity<SettingsView>),
}

pub struct AppView {
    store: Entity<Store>,
    account: Entity<AccountView>,
    hosts: Entity<HostsView>,
    overlay: Option<(Overlay, Subscription)>,
    sessions: Vec<(Entity<SessionView>, Subscription)>,
    active: Option<Entity<SessionView>>,
    is_switcher_open: bool,
    insets: Insets,
    _subscriptions: Vec<Subscription>,
    _insets_task: Task<()>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = Store::global(cx);
        let account = cx.new(|cx| AccountView::new(window, cx));
        let hosts = cx.new(|cx| HostsView::new(window, cx));

        let subscriptions =
            vec![
                cx.observe_window_activation(window, |view, window, cx| {
                    let is_active = window.is_window_active();
                    shell::set_window_visible(is_active);
                    for (session, _) in &view.sessions {
                        session.update(cx, |session, cx| session.set_app_active(is_active, cx));
                    }
                    view.store.update(cx, |store, cx| {
                        if is_active {
                            store.refresh_if_stale(cx);
                        } else {
                            store.save_now();
                        }
                    });
                }),
                cx.observe(&store, |view, store, cx| {
                    // Signed out: sessions of saved hosts close along with the vault.
                    if !matches!(store.read(cx).vault, VaultState::Open(_)) {
                        let saved = view
                            .sessions
                            .iter()
                            .filter(|(session, _)| session.read(cx).host_id().is_some())
                            .map(|(session, _)| session.clone())
                            .collect::<Vec<_>>();
                        for session in saved {
                            session.update(cx, |session, cx| session.close(cx));
                        }
                        if matches!(
                            view.overlay,
                            Some((Overlay::HostForm(_) | Overlay::Settings(_), _))
                        ) {
                            view.overlay = None;
                        }
                    }
                    cx.notify();
                }),
                cx.observe(&toast::entity(cx), |_, _, cx| cx.notify()),
                cx.observe_window_appearance(window, |_, window, cx| {
                    if cx.global::<Preferences>().theme == ThemePreference::System {
                        phone_theme::apply_preference(ThemePreference::System, Some(window), cx);
                    }
                }),
                cx.subscribe_in(
                    &account,
                    window,
                    |view, _, event: &AccountEvent, window, cx| match event {
                        AccountEvent::QuickConnect => {
                            view.show_quick_connect(window, cx);
                        }
                    },
                ),
                cx.subscribe_in(&hosts, window, |view, _, event: &HostsEvent, window, cx| {
                    match event {
                        HostsEvent::Connect(id) => {
                            view.connect_host(*id, window, cx);
                        }
                        HostsEvent::OpenFiles(id) => {
                            let session = view.connect_host(*id, window, cx);
                            session.update(cx, |session, cx| session.show_files(cx));
                        }
                        HostsEvent::OpenSession(session) => view.show_session(session.clone(), cx),
                        HostsEvent::EditHost(id) => view.show_host_form(*id, window, cx),
                        HostsEvent::QuickConnect => {
                            view.show_quick_connect(window, cx);
                        }
                        HostsEvent::Settings => view.show_settings(cx),
                    }
                }),
            ];

        let insets_task = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(INSETS_POLL_INTERVAL).await;
            let current = insets::current();
            let is_gone = this
                .update(cx, |view, cx| {
                    if view.insets != current {
                        view.insets = current;
                        view.notify_screens(cx);
                    }
                })
                .is_err();
            if is_gone {
                break;
            }
        });

        Self {
            store,
            account,
            hosts,
            overlay: None,
            sessions: Vec::new(),
            active: None,
            is_switcher_open: false,
            insets: insets::current(),
            _subscriptions: subscriptions,
            _insets_task: insets_task,
        }
    }

    /// The screens read the insets as they render.
    fn notify_screens(&self, cx: &mut Context<Self>) {
        cx.notify();
        self.account.update(cx, |_, cx| cx.notify());
        self.hosts.update(cx, |_, cx| cx.notify());
        if let Some(session) = &self.active {
            session.update(cx, |_, cx| cx.notify());
        }
        match &self.overlay {
            Some((Overlay::QuickConnect(view), _)) => view.update(cx, |_, cx| cx.notify()),
            Some((Overlay::HostForm(view), _)) => view.update(cx, |_, cx| cx.notify()),
            Some((Overlay::Settings(view), _)) => view.update(cx, |_, cx| cx.notify()),
            None => {}
        }
    }

    fn show_quick_connect(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ConnectView> {
        let view = cx.new(|cx| ConnectView::new(window, cx));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &ConnectEvent, window, cx| match event {
                ConnectEvent::Connect(target) => {
                    this.overlay = None;
                    this.open_session(SessionTarget::Quick(target.clone()), window, cx);
                }
                ConnectEvent::Back => {
                    this.overlay = None;
                    cx.notify();
                }
            },
        );
        self.overlay = Some((Overlay::QuickConnect(view.clone()), subscription));
        cx.notify();
        view
    }

    fn show_host_form(&mut self, id: Option<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.new(|cx| HostFormView::new(id, window, cx));
        let subscription = cx.subscribe(&view, |this, _, event: &HostFormEvent, cx| match event {
            HostFormEvent::Done => {
                this.overlay = None;
                cx.notify();
            }
        });
        self.overlay = Some((Overlay::HostForm(view), subscription));
        cx.notify();
    }

    fn show_settings(&mut self, cx: &mut Context<Self>) {
        let view = cx.new(SettingsView::new);
        let subscription = cx.subscribe(&view, |this, _, event: &SettingsEvent, cx| match event {
            SettingsEvent::Back => {
                this.overlay = None;
                cx.notify();
            }
        });
        self.overlay = Some((Overlay::Settings(view), subscription));
        cx.notify();
    }

    /// A host opens once: tapping it again goes back to its session.
    fn connect_host(
        &mut self,
        id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SessionView> {
        let open = self
            .sessions
            .iter()
            .map(|(session, _)| session)
            .find(|session| session.read(cx).host_id() == Some(id))
            .cloned();
        match open {
            Some(session) => {
                self.show_session(session.clone(), cx);
                session
            }
            None => self.open_session(SessionTarget::Saved { id }, window, cx),
        }
    }

    fn open_session(
        &mut self,
        target: SessionTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SessionView> {
        let session = cx.new(|cx| SessionView::new(target, cx));
        let subscription = cx.subscribe_in(
            &session,
            window,
            |this, session, event: &SessionEvent, window, cx| {
                this.on_session_event(session.clone(), event, window, cx)
            },
        );
        self.sessions.push((session.clone(), subscription));
        self.show_session(session.clone(), cx);
        session
    }

    /// The session shown last goes first in the switcher and on the hosts screen.
    fn show_session(&mut self, session: Entity<SessionView>, cx: &mut Context<Self>) {
        if let Some(position) = self
            .sessions
            .iter()
            .position(|(other, _)| *other == session)
        {
            let entry = self.sessions.remove(position);
            self.sessions.insert(0, entry);
            self.sessions_changed(cx);
        }
        self.is_switcher_open = false;
        session.update(cx, |session, cx| session.focus_terminal(cx));
        self.active = Some(session);
        cx.notify();
    }

    fn sessions_changed(&mut self, cx: &mut Context<Self>) {
        let count = self.sessions.len();
        for (session, _) in &self.sessions {
            session.update(cx, |session, cx| {
                session.session_count = count;
                cx.notify();
            });
        }
        let sessions = self
            .sessions
            .iter()
            .map(|(session, _)| session.clone())
            .collect();
        self.hosts
            .update(cx, |hosts, cx| hosts.set_sessions(sessions, cx));
    }

    fn on_session_event(
        &mut self,
        session: Entity<SessionView>,
        event: &SessionEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Without a vault there is no hosts list to come back to, so close it.
            SessionEvent::Hide if !matches!(self.store.read(cx).vault, VaultState::Open(_)) => {
                session.update(cx, |session, cx| session.close(cx));
            }
            SessionEvent::Hide => {
                self.active = None;
            }
            SessionEvent::Closed => self.remove_session(&session, cx),
            SessionEvent::ShowSessions => self.is_switcher_open = true,
            SessionEvent::EditHost(id) => {
                self.active = None;
                self.show_host_form(Some(*id), window, cx);
            }
            SessionEvent::LoginRefused(host, message) => {
                self.remove_session(&session, cx);
                let connect = self.show_quick_connect(window, cx);
                connect.update(cx, |connect, cx| {
                    connect.show_refusal(host, message.clone(), window, cx)
                });
            }
        }
        cx.notify();
    }

    fn remove_session(&mut self, session: &Entity<SessionView>, cx: &mut Context<Self>) {
        self.sessions.retain(|(other, _)| other != session);
        if self.active.as_ref() == Some(session) {
            self.active = None;
        }
        self.sessions_changed(cx);
    }

    fn render_switcher(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.is_switcher_open || self.active.is_none() {
            return None;
        }
        let rows = self
            .sessions
            .iter()
            .enumerate()
            .map(|(index, (session, _))| {
                let view = session.read(cx);
                let (status, color, is_busy) = view.status();
                let is_front = self.active.as_ref() == Some(session);
                let session = session.clone();
                row(("switch", index))
                    .child(host_icon(
                        view.os.as_deref(),
                        false,
                        phone_theme::plate(),
                        40.,
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(3.))
                            .child(row_title(view.title.clone()))
                            .child(
                                h_flex()
                                    .gap(px(6.))
                                    .child(status_dot(color, is_busy))
                                    .child(row_subtitle(status))
                                    .child(row_code(view.address.clone())),
                            ),
                    )
                    .child(if is_front {
                        icon("check", 20., flint_theme::accent()).into_any_element()
                    } else {
                        chevron().into_any_element()
                    })
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.show_session(session.clone(), cx)),
                    )
                    .into_any_element()
            });
        let content = v_flex()
            .gap(px(16.))
            .child(div().px(px(4.)).child(title("Sessions")))
            .child(card(rows))
            .child(
                secondary_button("switch-hosts", "All hosts").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.is_switcher_open = false;
                        this.active = None;
                        gpui_mobile::hide_keyboard();
                        cx.notify();
                    },
                )),
            );
        let this = cx.entity().downgrade();
        Some(sheet("switcher", content, move |_, cx| {
            this.update(cx, |this, cx| {
                this.is_switcher_open = false;
                cx.notify();
            })
            .ok();
        }))
    }

    fn screen(&self, cx: &mut Context<Self>) -> AnyView {
        let store = self.store.read(cx);
        if matches!(store.sign_in, SignIn::Password(_) | SignIn::Working(_)) {
            return self.account.clone().into();
        }
        if let Some(session) = &self.active {
            return session.clone().into();
        }
        let is_open = matches!(store.vault, VaultState::Open(_));
        match &self.overlay {
            Some((Overlay::QuickConnect(view), _)) => return view.clone().into(),
            Some((Overlay::HostForm(view), _)) if is_open => return view.clone().into(),
            Some((Overlay::Settings(view), _)) if is_open => return view.clone().into(),
            _ => {}
        }
        if is_open {
            self.hosts.clone().into()
        } else {
            self.account.clone().into()
        }
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = self.screen(cx);
        div()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .child(screen)
            .children(self.render_switcher(cx))
            .children(toast::render(cx))
    }
}
