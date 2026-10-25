//! Quick connect to a host that is not saved to the vault.

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, App, Context, Entity, EventEmitter, SharedString, Subscription, Window,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{h_flex, v_flex, ActiveTheme as _};
use gpui_mobile::KeyboardType;
use zeroize::Zeroizing;

use crate::insets;
use crate::phone_theme;
use crate::recent::{self, RecentHost};
use crate::theme as flint_theme;
use crate::widgets::{
    back_button, card_inset, error_text, field, field_input, icon, nav_bar, note, primary_button,
    row, row_code, row_title, section_header, GUTTER,
};

#[derive(Clone)]
pub struct ConnectTarget {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Zeroizing<String>,
    /// Trusted on an earlier connect.
    pub host_key: Option<String>,
}

impl ConnectTarget {
    pub fn recent(&self) -> RecentHost {
        RecentHost {
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            host_key: self.host_key.clone(),
        }
    }
}

pub enum ConnectEvent {
    Connect(ConnectTarget),
    Back,
}

pub struct ConnectView {
    host_input: Entity<InputState>,
    port_input: Entity<InputState>,
    user_input: Entity<InputState>,
    password_input: Entity<InputState>,
    recent: Vec<RecentHost>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConnectEvent> for ConnectView {}

impl ConnectView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let host_input = cx.new(|cx| InputState::new(window, cx).placeholder("server.example.com"));
        let port_input = cx.new(|cx| InputState::new(window, cx).default_value("22"));
        let user_input = cx.new(|cx| InputState::new(window, cx).placeholder("root"));
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Password")
        });

        let mut subscriptions = Vec::new();
        for (input, keyboard) in [
            (&host_input, KeyboardType::EmailAddress),
            (&port_input, KeyboardType::NumberPad),
            (&user_input, KeyboardType::Default),
            (&password_input, KeyboardType::Password),
        ] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Focus => gpui_mobile::show_keyboard_with_type(keyboard),
                    InputEvent::Blur => gpui_mobile::hide_keyboard(),
                    InputEvent::PressEnter { .. } => this.submit(window, cx),
                    InputEvent::Change => {}
                },
            ));
        }

        let mut view = Self {
            host_input,
            port_input,
            user_input,
            password_input,
            recent: recent::load(),
            error: None,
            _subscriptions: subscriptions,
        };
        if let Some(last) = view.recent.first().cloned() {
            view.fill(&last, window, cx);
        }
        view
    }

    pub fn show_refusal(
        &mut self,
        host: &RecentHost,
        error: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.fill(host, window, cx);
        self.error = Some(error);
        self.password_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn fill(&mut self, host: &RecentHost, window: &mut Window, cx: &mut Context<Self>) {
        self.host_input.update(cx, |input, cx| {
            input.set_value(host.host.clone(), window, cx)
        });
        self.port_input.update(cx, |input, cx| {
            input.set_value(host.port.to_string(), window, cx)
        });
        self.user_input.update(cx, |input, cx| {
            input.set_value(host.username.clone(), window, cx)
        });
        self.password_input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.read_target(cx) {
            Ok(target) => {
                self.error = None;
                gpui_mobile::hide_keyboard();
                window.blur(cx);
                cx.emit(ConnectEvent::Connect(target));
            }
            Err(error) => self.error = Some(error.into()),
        }
        cx.notify();
    }

    fn read_target(&self, cx: &App) -> Result<ConnectTarget, &'static str> {
        let host_field = self.host_input.read(cx).value().trim().to_string();
        // `user@host:port` in the host field wins over the user and port fields.
        let typed = flint_core::ssh_target::parse_host_field(&host_field);
        let host = typed
            .as_ref()
            .map_or(host_field, |target| target.host.clone());
        if host.is_empty() {
            return Err("Enter the host to connect to.");
        }
        let port = match typed.as_ref().and_then(|target| target.port) {
            Some(port) => port,
            None => self
                .port_input
                .read(cx)
                .value()
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .ok_or("The port is a number from 1 to 65535.")?,
        };
        let username = typed
            .and_then(|target| target.user)
            .unwrap_or_else(|| self.user_input.read(cx).value().trim().to_string());
        if username.is_empty() {
            return Err("Enter the user name.");
        }
        let password = Zeroizing::new(self.password_input.read(cx).value().to_string());
        let host_key = self
            .recent
            .iter()
            .find(|known| known.host == host && known.port == port && known.username == username)
            .and_then(|known| known.host_key.clone());
        Ok(ConnectTarget {
            host,
            port,
            username,
            password,
            host_key,
        })
    }

    fn pick_recent(&mut self, host: RecentHost, window: &mut Window, cx: &mut Context<Self>) {
        self.fill(&host, window, cx);
        self.error = None;
        self.password_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn forget_recent(&mut self, host: &RecentHost, cx: &mut Context<Self>) {
        recent::forget(host);
        self.recent = recent::load();
        cx.notify();
    }

    fn render_form(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap(px(16.))
            .child(field("Host", field_input(&self.host_input)))
            .child(
                h_flex()
                    .gap(px(12.))
                    .child(field("User", field_input(&self.user_input)).flex_1())
                    .child(field("Port", field_input(&self.port_input)).w(px(96.))),
            )
            .child(field("Password", field_input(&self.password_input)))
            .children(self.error.clone().map(error_text))
            .child(
                primary_button("connect", "Connect")
                    .on_click(cx.listener(|view, _, window, cx| view.submit(window, cx))),
            )
            .child(note(
                "The host is not saved and the password is not kept.",
                false,
            ))
    }

    fn render_recent(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.recent.is_empty() {
            return None;
        }
        let count = self.recent.len();
        let rows = self.recent.iter().enumerate().map(|(index, host)| {
            let picked = host.clone();
            let forgotten = host.clone();
            row(("recent", index))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(3.))
                        .child(row_title(host.host.clone()))
                        .child(row_code(host.address())),
                )
                .child(
                    div()
                        .id(("forget", index))
                        .flex_none()
                        .size(px(36.))
                        .mr(px(-6.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .active(|button| button.bg(phone_theme::fill_pressed()))
                        .child(icon("x", 18., flint_theme::text_dim()))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            cx.stop_propagation();
                            view.forget_recent(&forgotten, cx);
                        })),
                )
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.pick_recent(picked.clone(), window, cx);
                }))
                .into_any_element()
        });
        Some(
            v_flex()
                .child(section_header("Recent", count))
                .child(card_inset(rows, 14.))
                .into_any_element(),
        )
    }
}

impl Render for ConnectView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let insets = insets::current();
        let back = back_button("back").on_click(cx.listener(|_, _, _, cx| {
            gpui_mobile::hide_keyboard();
            cx.emit(ConnectEvent::Back);
        }));
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(nav_bar(insets, back, "Quick connect", None))
            .child(
                div()
                    .id("connect")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .w_full()
                            .gap(px(32.))
                            .pl(px(insets.left + GUTTER))
                            .pr(px(insets.right + GUTTER))
                            .pt(px(12.))
                            .pb(px(insets.bottom + 24.))
                            .child(self.render_form(cx))
                            .children(self.render_recent(cx)),
                    ),
            )
    }
}
