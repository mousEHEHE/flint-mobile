//! Edits only the address and sign-in; the rest of a host is kept and edited on the computer.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Context, Entity, EventEmitter, FontWeight, SharedString, Subscription,
    Window,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{h_flex, v_flex, ActiveTheme as _};
use gpui_mobile::KeyboardType;
use uuid::Uuid;

use flint_core::vault::{AuthMethod, ProxyChoice, ServerEntry, SshKeySummary};

use crate::insets;
use crate::phone_theme;
use crate::store::Store;
use crate::theme as flint_theme;
use crate::toast;
use crate::widgets::{
    card_inset, error_text, field, field_input, icon, nav_action, nav_bar, note, row, row_code,
    row_title, section_title, GUTTER,
};

pub enum HostFormEvent {
    Done,
}

#[derive(Clone, Copy, PartialEq)]
enum AuthKind {
    Password,
    Key,
}

pub struct HostFormView {
    store: Entity<Store>,
    original: Option<ServerEntry>,
    label_input: Entity<InputState>,
    host_input: Entity<InputState>,
    port_input: Entity<InputState>,
    user_input: Entity<InputState>,
    password_input: Entity<InputState>,
    auth: AuthKind,
    keys: Vec<SshKeySummary>,
    key_id: Option<Uuid>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<HostFormEvent> for HostFormView {}

impl HostFormView {
    pub fn new(id: Option<Uuid>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = Store::global(cx);
        let (original, keys) = {
            let vault = store.read(cx).vault();
            (
                id.and_then(|id| vault.and_then(|vault| vault.get_entry(id).ok())),
                vault
                    .and_then(|vault| vault.list_keys().ok())
                    .unwrap_or_default(),
            )
        };
        let text = |value: Option<String>,
                    placeholder: &'static str,
                    window: &mut Window,
                    cx: &mut Context<Self>| {
            cx.new(|cx| {
                let input = InputState::new(window, cx).placeholder(placeholder);
                match value {
                    Some(value) => input.default_value(value),
                    None => input,
                }
            })
        };
        let entry = original.as_ref();
        let label_input = text(
            entry.map(|e| e.label.clone()),
            "Name, e.g. web server",
            window,
            cx,
        );
        let host_input = text(
            entry.map(|e| e.host.clone()),
            "server.example.com",
            window,
            cx,
        );
        let port_input = text(
            Some(entry.map_or(22, |e| e.port).to_string()),
            "22",
            window,
            cx,
        );
        let user_input = text(entry.map(|e| e.username.clone()), "root", window, cx);
        let has_password =
            matches!(entry.map(|e| &e.auth), Some(AuthMethod::Password(p)) if !p.is_empty());
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(if has_password {
                    "Unchanged"
                } else {
                    "Password"
                })
        });
        let (auth, key_id) = match entry.map(|e| &e.auth) {
            Some(AuthMethod::PrivateKey { key_id }) => (AuthKind::Key, Some(*key_id)),
            _ => (AuthKind::Password, None),
        };

        let mut subscriptions = Vec::new();
        for (input, keyboard) in [
            (&label_input, KeyboardType::Default),
            (&host_input, KeyboardType::EmailAddress),
            (&port_input, KeyboardType::NumberPad),
            (&user_input, KeyboardType::Default),
            (&password_input, KeyboardType::Password),
        ] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |view, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Focus => gpui_mobile::show_keyboard_with_type(keyboard),
                    InputEvent::Blur => gpui_mobile::hide_keyboard(),
                    InputEvent::PressEnter { .. } => view.save(window, cx),
                    InputEvent::Change => {}
                },
            ));
        }

        Self {
            store,
            original,
            label_input,
            host_input,
            port_input,
            user_input,
            password_input,
            auth,
            keys,
            key_id,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn value(input: &Entity<InputState>, cx: &gpui::App) -> String {
        input.read(cx).value().trim().to_string()
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.build(cx) {
            Ok(entry) => {
                let is_new = self.original.is_none();
                let saved = self.store.update(cx, |store, cx| {
                    store.edit(cx, |vault| {
                        if is_new {
                            vault.add_entry(entry)
                        } else {
                            vault.update_entry(entry)
                        }
                    })
                });
                match saved {
                    Ok(()) => {
                        gpui_mobile::hide_keyboard();
                        window.blur(cx);
                        toast::show(
                            if is_new { "Host added" } else { "Host saved" },
                            toast::Kind::Done,
                            cx,
                        );
                        cx.emit(HostFormEvent::Done);
                    }
                    Err(e) => self.error = Some(e.into()),
                }
            }
            Err(error) => self.error = Some(error.into()),
        }
        cx.notify();
    }

    fn build(&self, cx: &gpui::App) -> Result<ServerEntry, &'static str> {
        let host_field = Self::value(&self.host_input, cx);
        // `user@host:port` in the host field wins over the user and port fields.
        let typed = flint_core::ssh_target::parse_host_field(&host_field);
        let host = typed
            .as_ref()
            .map_or(host_field, |target| target.host.clone());
        if host.is_empty() {
            return Err("Enter the host's address.");
        }
        let port = match typed.as_ref().and_then(|target| target.port) {
            Some(port) => port,
            None => Self::value(&self.port_input, cx)
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .ok_or("The port is a number from 1 to 65535.")?,
        };
        let username = typed
            .and_then(|target| target.user)
            .unwrap_or_else(|| Self::value(&self.user_input, cx));
        if username.is_empty() {
            return Err("Enter the user name.");
        }
        let auth = match self.auth {
            AuthKind::Password => {
                let typed = self.password_input.read(cx).value().to_string();
                match (&self.original, typed.is_empty()) {
                    // Left empty while editing: the saved password stays.
                    (
                        Some(ServerEntry {
                            auth: AuthMethod::Password(old),
                            ..
                        }),
                        true,
                    ) => AuthMethod::Password(old.clone()),
                    _ => AuthMethod::Password(typed),
                }
            }
            AuthKind::Key => AuthMethod::PrivateKey {
                key_id: self.key_id.ok_or("Choose the key to sign in with.")?,
            },
        };
        let label = Self::value(&self.label_input, cx);
        let now = unix_now();
        let entry = match &self.original {
            Some(original) => {
                let mut entry = original.clone();
                // A new address may be another machine, so the old host key no longer applies.
                if entry.host != host || entry.port != port {
                    entry.host_key_fingerprint = None;
                }
                entry.label = label;
                entry.host = host;
                entry.port = port;
                entry.username = username;
                entry.auth = auth;
                entry.updated_at = now;
                entry
            }
            None => ServerEntry {
                id: Uuid::new_v4(),
                label,
                host,
                port,
                username,
                auth,
                notes: None,
                os: None,
                last_opened_at: None,
                host_key_fingerprint: None,
                last_remote_dir: None,
                last_local_dir: None,
                group: None,
                tags: Vec::new(),
                proxy: ProxyChoice::Default,
                jump_host: None,
                forwards: Vec::new(),
                startup_snippet: None,
                snippets: Vec::new(),
                created_at: now,
                updated_at: now,
            },
        };
        Ok(entry)
    }

    fn render_auth_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let segment =
            |id: &'static str, label: &'static str, kind: AuthKind, cx: &mut Context<Self>| {
                let is_on = self.auth == kind;
                div()
                    .id(id)
                    .flex_1()
                    .h(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(9.))
                    .text_size(px(15.))
                    .font_weight(if is_on {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(if is_on {
                        flint_theme::text()
                    } else {
                        flint_theme::text_dim()
                    })
                    .when(is_on, |segment| {
                        segment.bg(phone_theme::selected()).shadow_sm()
                    })
                    .child(label)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.auth = kind;
                        if kind == AuthKind::Key && view.key_id.is_none() {
                            view.key_id = view
                                .keys
                                .iter()
                                .max_by_key(|key| key.created_at)
                                .map(|key| key.id);
                        }
                        cx.notify();
                    }))
            };
        h_flex()
            .p(px(3.))
            .gap(px(2.))
            .rounded(px(12.))
            .bg(phone_theme::fill())
            .child(segment("auth-password", "Password", AuthKind::Password, cx))
            .child(segment("auth-key", "SSH key", AuthKind::Key, cx))
    }

    fn render_keys(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.keys.is_empty() {
            return note(
                "No keys in your vault yet. Add one in flint on your computer; it comes here \
                 with the next sync.",
                false,
            )
            .into_any_element();
        }
        let rows = self.keys.iter().enumerate().map(|(index, key)| {
            let id = key.id;
            let is_chosen = self.key_id == Some(id);
            row(("key", index))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(3.))
                        .child(row_title(key.name.clone()))
                        .child(row_code(key.key_type.clone())),
                )
                .when(is_chosen, |row| {
                    row.child(icon("check", 20., flint_theme::accent()))
                })
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.key_id = Some(id);
                    cx.notify();
                }))
                .into_any_element()
        });
        card_inset(rows, 14.).into_any_element()
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

impl Render for HostFormView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let insets = insets::current();
        let is_new = self.original.is_none();
        let is_agent = matches!(
            self.original.as_ref().map(|e| &e.auth),
            Some(AuthMethod::Agent)
        );
        let cancel =
            nav_action("cancel", "Cancel", false).on_click(cx.listener(|_, _, window, cx| {
                gpui_mobile::hide_keyboard();
                window.blur(cx);
                cx.emit(HostFormEvent::Done);
            }));
        let save = nav_action("save", "Save", true)
            .on_click(cx.listener(|view, _, window, cx| view.save(window, cx)))
            .into_any_element();

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(nav_bar(
                insets,
                cancel,
                if is_new { "New host" } else { "Edit host" },
                Some(save),
            ))
            .child(
                div()
                    .id("host-form")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .w_full()
                            .gap(px(18.))
                            .pt(px(8.))
                            .pb(px(insets.bottom + 32.))
                            .pl(px(insets.left + GUTTER))
                            .pr(px(insets.right + GUTTER))
                            .child(field("Name", field_input(&self.label_input)))
                            .child(field("Address", field_input(&self.host_input)))
                            .child(
                                h_flex()
                                    .gap(px(12.))
                                    .child(field("User", field_input(&self.user_input)).flex_1())
                                    .child(field("Port", field_input(&self.port_input)).w(px(96.))),
                            )
                            .child(div().h(px(4.)))
                            .child(section_title("Sign in with"))
                            .when(is_agent, |form| {
                                form.child(note(
                                    "This host uses the SSH agent of your computer, which the \
                                     phone cannot reach. Pick a password or a key; the computer \
                                     can use them too.",
                                    true,
                                ))
                            })
                            .child(self.render_auth_switch(cx))
                            .child(match self.auth {
                                AuthKind::Password => {
                                    field_input(&self.password_input).into_any_element()
                                }
                                AuthKind::Key => self.render_keys(cx),
                            })
                            .children(self.error.clone().map(error_text)),
                    ),
            )
    }
}
