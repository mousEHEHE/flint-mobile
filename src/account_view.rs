use gpui::prelude::*;
use gpui::{
    div, img, px, AnyElement, Context, Entity, EventEmitter, FontWeight, Subscription, Window,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{v_flex, ActiveTheme as _};
use gpui_mobile::KeyboardType;
use secrecy::SecretString;

use crate::insets;
use crate::store::{SignIn, Store, VaultState};
use crate::theme as flint_theme;
use crate::widgets::{
    app_icon, back_button, body_text, busy_button, error_text, field_input, large_title, nav_bar,
    primary_button, quiet_button, GUTTER,
};

pub enum AccountEvent {
    QuickConnect,
}

pub struct AccountView {
    store: Entity<Store>,
    password_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AccountEvent> for AccountView {}

impl AccountView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = Store::global(cx);
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Master password")
        });
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &password_input,
                window,
                |view, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Focus => {
                        gpui_mobile::show_keyboard_with_type(KeyboardType::Password)
                    }
                    InputEvent::Blur => gpui_mobile::hide_keyboard(),
                    InputEvent::PressEnter { .. } => view.submit(window, cx),
                    InputEvent::Change => {}
                },
            ),
        ];
        Self {
            store,
            password_input,
            _subscriptions: subscriptions,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.password_input.read(cx).value().to_string();
        if password.is_empty() {
            return;
        }
        let password = SecretString::new(password);
        self.password_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        gpui_mobile::hide_keyboard();
        window.blur(cx);
        self.store.update(cx, |store, cx| {
            if matches!(store.sign_in, SignIn::Password(_)) {
                store.submit_password(password, cx);
            } else {
                store.unlock(password, cx);
            }
        });
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let is_waiting = matches!(store.sign_in, SignIn::Browser);
        let error = store.sign_in_error.clone();
        let insets = insets::current();

        let sign_in = if is_waiting {
            v_flex()
                .gap(px(6.))
                .child(busy_button("signing-in", "Waiting for GitHub…", true))
                .child(
                    quiet_button("cancel-sign-in", "Cancel").on_click(cx.listener(
                        |view, _, _, cx| {
                            view.store.update(cx, |store, cx| store.cancel_sign_in(cx));
                        },
                    )),
                )
        } else {
            v_flex()
                .gap(px(6.))
                .child(
                    primary_button("sign-in", "Sign in with GitHub").on_click(cx.listener(
                        |view, _, _, cx| {
                            view.store.update(cx, |store, cx| store.start_sign_in(cx));
                        },
                    )),
                )
                .child(
                    quiet_button("quick-connect", "Connect without an account")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(AccountEvent::QuickConnect))),
                )
        };

        v_flex()
            .size_full()
            .pt(px(insets.top + 32.))
            .pb(px(insets.bottom + 12.))
            .pl(px(insets.left + GUTTER + 4.))
            .pr(px(insets.right + GUTTER + 4.))
            .child(div().flex_1())
            .child(
                v_flex()
                    .gap(px(10.))
                    .child(img(app_icon()).size(px(60.)).mb(px(14.)))
                    .child(
                        div()
                            .text_size(px(40.))
                            .line_height(px(44.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(flint_theme::text())
                            .child("flint"),
                    )
                    .child(
                        div()
                            .max_w(px(320.))
                            .text_size(px(17.))
                            .line_height(px(24.))
                            .text_color(flint_theme::text_dim())
                            .child(
                                "Your servers on the phone: the hosts, keys and snippets you \
                                 keep in flint on your computer.",
                            ),
                    ),
            )
            .child(div().h(px(40.)))
            .child(
                v_flex()
                    .gap(px(12.))
                    .children(error.map(error_text))
                    .child(sign_in)
                    .child(
                        div()
                            .text_center()
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .text_color(flint_theme::text_placeholder())
                            .child("Uses the flint account you made on your computer."),
                    ),
            )
            .into_any_element()
    }

    fn render_password(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let (login, is_working) = match &store.sign_in {
            SignIn::Password(step) => (step.github_login.clone(), false),
            SignIn::Working(step) => (step.github_login.clone(), true),
            _ => (String::new(), false),
        };
        let error = store.sign_in_error.clone();
        let insets = insets::current();
        let cancel = back_button("cancel").on_click(cx.listener(|view, _, _, cx| {
            gpui_mobile::hide_keyboard();
            view.store.update(cx, |store, cx| store.cancel_sign_in(cx));
        }));

        v_flex()
            .size_full()
            .child(nav_bar(insets, cancel, "", None))
            .child(
                v_flex()
                    .flex_1()
                    .gap(px(16.))
                    .pt(px(12.))
                    .pl(px(insets.left + GUTTER))
                    .pr(px(insets.right + GUTTER))
                    .child(
                        v_flex()
                            .px(px(4.))
                            .gap(px(8.))
                            .child(large_title("Master password"))
                            .child(body_text(format!(
                                "Signed in with GitHub as @{login}. Enter the password you \
                                 set in flint on your computer. The phone keeps the vault key \
                                 afterwards, so it asks only once."
                            ))),
                    )
                    .child(div().h(px(4.)))
                    .child(field_input(&self.password_input))
                    .children(error.map(error_text))
                    .child(
                        busy_button(
                            "open",
                            if is_working {
                                "Opening the vault…"
                            } else {
                                "Open my hosts"
                            },
                            is_working,
                        )
                        .on_click(cx.listener(|view, _, window, cx| view.submit(window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_unlock(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let is_working = store.is_unlocking;
        let error = store.unlock_error.clone();
        let insets = insets::current();
        v_flex()
            .size_full()
            .pt(px(insets.top + 56.))
            .pb(px(insets.bottom + 12.))
            .pl(px(insets.left + GUTTER))
            .pr(px(insets.right + GUTTER))
            .gap(px(16.))
            .child(
                v_flex()
                    .px(px(4.))
                    .gap(px(8.))
                    .child(large_title("Unlock flint"))
                    .child(body_text(
                        "This phone no longer has the key to your vault, for example after a \
                         restore from a backup. Enter your master password once.",
                    )),
            )
            .child(div().h(px(4.)))
            .child(field_input(&self.password_input))
            .children(error.map(error_text))
            .child(
                busy_button(
                    "unlock",
                    if is_working { "Unlocking…" } else { "Unlock" },
                    is_working,
                )
                .on_click(cx.listener(|view, _, window, cx| view.submit(window, cx))),
            )
            .child(div().flex_1())
            .child(quiet_button("forget", "Sign out of this phone").on_click(
                cx.listener(|view, _, _, cx| view.store.update(cx, |store, cx| store.sign_out(cx))),
            ))
            .into_any_element()
    }
}

impl Render for AccountView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let content = if matches!(store.sign_in, SignIn::Password(_) | SignIn::Working(_)) {
            self.render_password(cx)
        } else if matches!(store.vault, VaultState::Locked) {
            self.render_unlock(cx)
        } else {
            self.render_welcome(cx)
        };
        div()
            .id("account")
            .size_full()
            .overflow_y_scroll()
            .bg(cx.theme().background)
            .child(content)
    }
}
