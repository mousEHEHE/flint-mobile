use gpui::prelude::*;
use gpui::{
    div, px, rgb, AnyElement, Context, Entity, EventEmitter, FontWeight, SharedString,
    Subscription, Window,
};
use gpui_component::{h_flex, v_flex, ActiveTheme as _};

use crate::insets;
use crate::phone_theme;
use crate::preferences::{
    Preferences, TerminalScheme, ThemePreference, MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE,
};
use crate::store::Store;
use crate::theme as flint_theme;
use crate::ui::terminal_palette::TerminalColors;
use crate::widgets::{
    ago, back_button, card_inset, chevron, danger_button, icon, initial_badge, nav_bar,
    quiet_button, row, row_subtitle, section_title, sheet, status_dot, title, GUTTER,
};

/// The stepper moves in whole sizes; pinching the terminal can set any size in between.
const FONT_STEP: f32 = 1.;

pub enum SettingsEvent {
    Back,
}

enum SettingsSheet {
    SignOut,
    Colors,
}

pub struct SettingsView {
    store: Entity<Store>,
    sheet: Option<SettingsSheet>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = Store::global(cx);
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe_global::<Preferences>(|_, cx| cx.notify()),
        ];
        Self {
            store,
            sheet: None,
            _subscriptions: subscriptions,
        }
    }

    fn set_theme(
        &mut self,
        preference: ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        Preferences::update(cx, |prefs| prefs.theme = preference);
        phone_theme::apply_preference(preference, Some(window), cx);
        cx.notify();
    }

    fn step_font_size(&mut self, step: f32, cx: &mut Context<Self>) {
        let size = (Preferences::terminal_font_size(cx) + step).round();
        Preferences::set_terminal_font_size(size, cx);
        cx.notify();
    }

    fn render_account(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let login = store.github_login().unwrap_or("unknown").to_string();
        let is_session_ended = store.is_session_ended();
        let (status, color) = if is_session_ended {
            ("Signed out of sync".to_string(), flint_theme::warning())
        } else if store.is_syncing {
            ("Syncing…".to_string(), flint_theme::accent())
        } else if let Some(error) = &store.sync_error {
            (format!("Sync failed: {error}"), flint_theme::red_error())
        } else if let Some(at) = store.last_synced {
            (
                format!("Synced {}", ago(at.elapsed().as_secs())),
                flint_theme::status_online(),
            )
        } else {
            ("Not synced yet".to_string(), flint_theme::text_dim())
        };
        let host_count = store
            .vault()
            .and_then(|vault| vault.list_entries().ok())
            .map_or(0, |hosts| hosts.len());
        let is_syncing = store.is_syncing;

        let account = h_flex()
            .gap(px(14.))
            .px(px(16.))
            .py(px(14.))
            .child(initial_badge(&login, 46.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(flint_theme::text())
                            .truncate()
                            .child(format!("@{login}")),
                    )
                    .child(
                        h_flex()
                            .gap(px(7.))
                            .child(status_dot(color, is_syncing))
                            .child(row_subtitle(status)),
                    ),
            )
            .into_any_element();
        let hosts = setting_row("Hosts on this phone")
            .child(value_text(host_count.to_string()))
            .into_any_element();
        let sync_now = h_flex()
            .id("sync-now")
            .px(px(16.))
            .min_h(px(50.))
            .active(|row| row.bg(phone_theme::fill()))
            .child(
                div()
                    .text_size(px(16.))
                    .text_color(flint_theme::accent())
                    .child(if is_session_ended {
                        "Sign in with GitHub"
                    } else {
                        "Sync now"
                    }),
            )
            .on_click(cx.listener(|view, _, _, cx| {
                view.store.update(cx, |store, cx| {
                    if store.is_session_ended() {
                        store.start_sign_in(cx);
                    } else {
                        store.refresh_all(cx);
                    }
                });
            }))
            .into_any_element();
        v_flex()
            .child(section_title("Account"))
            .child(card_inset([account, hosts, sync_now], 16.))
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = cx.global::<Preferences>().theme;
        let tiles = [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ]
        .into_iter()
        .map(|preference| {
            let is_chosen = preference == current;
            v_flex()
                .id(SharedString::from(format!("theme-{}", preference.label())))
                .flex_1()
                .items_center()
                .gap(px(10.))
                .py(px(14.))
                .active(|tile| tile.opacity(0.7))
                .child(theme_preview(preference, is_chosen))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(if is_chosen {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if is_chosen {
                            flint_theme::text()
                        } else {
                            flint_theme::text_dim()
                        })
                        .child(preference.label()),
                )
                .on_click(
                    cx.listener(move |view, _, window, cx| view.set_theme(preference, window, cx)),
                )
        });
        v_flex().child(section_title("Appearance")).child(
            h_flex()
                .px(px(8.))
                .rounded(px(16.))
                .bg(phone_theme::plate())
                .children(tiles),
        )
    }

    fn render_terminal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let size = Preferences::terminal_font_size(cx);
        let scheme = Preferences::terminal_scheme(cx);
        let font = Preferences::terminal_font(cx).family();
        let colors = TerminalColors::for_scheme(scheme);
        let (_, dots) = TerminalColors::swatch(scheme);

        let preview = v_flex()
            .m(px(8.))
            .p(px(14.))
            .gap(px(2.))
            .rounded(px(10.))
            .bg(colors.bg)
            .font_family(font)
            .text_size(px(size))
            .line_height(px((size * 1.4).round()))
            .child(
                h_flex()
                    .child(div().text_color(dots[1]).child("deploy@web-1"))
                    .child(div().text_color(colors.fg).child(":"))
                    .child(div().text_color(dots[3]).child("~"))
                    .child(div().text_color(colors.fg).child("$ uptime")),
            )
            .child(
                div()
                    .text_color(colors.fg)
                    .child(" 14:02 up 41 days, load 0.08"),
            );

        let stepper_button = |id: &'static str, name: &str, is_enabled: bool| {
            div()
                .id(id)
                .size(px(34.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(phone_theme::fill())
                .when(is_enabled, |button| {
                    button.active(|button| button.bg(phone_theme::fill_pressed()))
                })
                .when(!is_enabled, |button| button.opacity(0.4))
                .child(icon(name, 18., flint_theme::text()))
        };
        let font_size = setting_row("Font size")
            .gap(px(10.))
            .child(
                stepper_button("smaller", "minus", size > MIN_TERMINAL_FONT_SIZE)
                    .on_click(cx.listener(|view, _, _, cx| view.step_font_size(-FONT_STEP, cx))),
            )
            .child(
                div()
                    .w(px(28.))
                    .text_center()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(flint_theme::text())
                    .child(format!("{size:.0}")),
            )
            .child(
                stepper_button("bigger", "plus", size < MAX_TERMINAL_FONT_SIZE)
                    .on_click(cx.listener(|view, _, _, cx| view.step_font_size(FONT_STEP, cx))),
            )
            .into_any_element();
        let colors_row = row("colors")
            .min_h(px(50.))
            .px(px(16.))
            .child(
                div()
                    .flex_1()
                    .text_size(px(16.))
                    .text_color(flint_theme::text())
                    .child("Colors"),
            )
            .child(value_text(scheme.label()))
            .child(chevron())
            .on_click(cx.listener(|view, _, _, cx| {
                view.sheet = Some(SettingsSheet::Colors);
                cx.notify();
            }))
            .into_any_element();

        v_flex().child(section_title("Terminal")).child(card_inset(
            [preview.into_any_element(), font_size, colors_row],
            16.,
        ))
    }

    fn render_colors_sheet(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = Preferences::terminal_scheme(cx);
        let rows = TerminalScheme::ALL.into_iter().map(|scheme| {
            let (bg, dots) = TerminalColors::swatch(scheme);
            let is_chosen = scheme == current;
            row(SharedString::from(format!("scheme-{}", scheme.label())))
                .min_h(px(52.))
                .child(
                    h_flex()
                        .flex_none()
                        .gap(px(3.))
                        .p(px(6.))
                        .rounded(px(7.))
                        .bg(bg)
                        .border_1()
                        .border_color(flint_theme::border())
                        .children(
                            dots.into_iter()
                                .map(|dot| div().size(px(8.)).rounded_full().bg(dot)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(16.))
                        .text_color(flint_theme::text())
                        .child(scheme.label()),
                )
                .when(is_chosen, |row| {
                    row.child(icon("check", 20., flint_theme::accent()))
                })
                .on_click(cx.listener(move |view, _, _, cx| {
                    Preferences::update(cx, |prefs| prefs.terminal_scheme = scheme);
                    view.sheet = None;
                    cx.notify();
                }))
                .into_any_element()
        });
        v_flex()
            .gap(px(16.))
            .child(div().px(px(4.)).child(title("Terminal colors")))
            .child(card_inset(rows, 14.))
            .into_any_element()
    }

    fn render_sign_out_sheet(&self, cx: &mut Context<Self>) -> AnyElement {
        let has_unsynced = self
            .store
            .read(cx)
            .vault()
            .is_some_and(|vault| vault.has_unsynced_changes());
        v_flex()
            .gap(px(8.))
            .child(div().px(px(4.)).child(title("Sign out of this phone?")))
            .child(
                div()
                    .px(px(4.))
                    .text_size(px(15.))
                    .line_height(px(21.))
                    .text_color(flint_theme::text_dim())
                    .child(if has_unsynced {
                        "Some changes made here have not reached your account yet and will be \
                         lost. Your other hosts stay in the account."
                    } else {
                        "The phone forgets your hosts and keys. They stay in your account and \
                         come back when you sign in again."
                    }),
            )
            .child(div().h(px(10.)))
            .child(
                danger_button("confirm-sign-out", "Sign out").on_click(cx.listener(
                    |view, _, _, cx| {
                        view.sheet = None;
                        view.store.update(cx, |store, cx| store.sign_out(cx));
                    },
                )),
            )
            .child(
                quiet_button("keep-signed-in", "Cancel").on_click(cx.listener(|view, _, _, cx| {
                    view.sheet = None;
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    fn render_sheet(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let content = match self.sheet.as_ref()? {
            SettingsSheet::SignOut => self.render_sign_out_sheet(cx),
            SettingsSheet::Colors => self.render_colors_sheet(cx),
        };
        let this = cx.entity().downgrade();
        let dismiss = move |_: &mut Window, cx: &mut gpui::App| {
            this.update(cx, |view, cx| {
                view.sheet = None;
                cx.notify();
            })
            .ok();
        };
        Some(sheet("settings-sheet", content, dismiss))
    }
}

fn setting_row(label: &'static str) -> gpui::Div {
    h_flex().px(px(16.)).min_h(px(50.)).gap(px(12.)).child(
        div()
            .flex_1()
            .text_size(px(16.))
            .text_color(flint_theme::text())
            .child(label),
    )
}

fn value_text(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(px(16.))
        .text_color(flint_theme::text_dim())
        .child(text.into())
}

fn theme_preview(preference: ThemePreference, is_chosen: bool) -> AnyElement {
    const WIDTH: f32 = 58.;
    const HEIGHT: f32 = 86.;
    let screen = |is_light: bool| {
        let (ground, card, line) = if is_light {
            (rgb(0xF1F3F7), rgb(0xFFFFFF), rgb(0xD5D9DF))
        } else {
            (rgb(0x1B1D23), rgb(0x23262E), rgb(0x3A3E48))
        };
        v_flex()
            .w(px(WIDTH))
            .h(px(HEIGHT))
            .gap(px(5.))
            .pt(px(11.))
            .px(px(6.))
            .bg(ground)
            .child(div().h(px(5.)).w(px(22.)).rounded_full().bg(line))
            .child(
                v_flex()
                    .gap(px(3.))
                    .p(px(5.))
                    .rounded(px(5.))
                    .bg(card)
                    .child(div().h(px(4.)).rounded_full().bg(flint_theme::accent()))
                    .child(div().h(px(4.)).w(px(26.)).rounded_full().bg(line)),
            )
            .child(
                v_flex()
                    .gap(px(3.))
                    .p(px(5.))
                    .rounded(px(5.))
                    .bg(card)
                    .child(div().h(px(4.)).w(px(32.)).rounded_full().bg(line))
                    .child(div().h(px(4.)).w(px(18.)).rounded_full().bg(line)),
            )
    };
    let frame = div()
        .relative()
        .w(px(WIDTH))
        .h(px(HEIGHT))
        .rounded(px(10.))
        .overflow_hidden()
        .border_2()
        .border_color(if is_chosen {
            flint_theme::accent()
        } else {
            flint_theme::border()
        });
    match preference {
        ThemePreference::Light => frame.child(screen(true)),
        ThemePreference::Dark => frame.child(screen(false)),
        // The right half of the dark screen, laid over the light one.
        ThemePreference::System => frame.child(screen(true)).child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .w(px(WIDTH / 2.))
                .h_full()
                .overflow_hidden()
                .child(div().absolute().top_0().right_0().child(screen(false))),
        ),
    }
    .into_any_element()
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let insets = insets::current();
        let back =
            back_button("back").on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Back)));
        let sign_out = div()
            .id("sign-out")
            .h(px(50.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(16.))
            .bg(phone_theme::plate())
            .active(|button| button.bg(phone_theme::fill_pressed()))
            .text_size(px(16.))
            .text_color(flint_theme::red_error())
            .child("Sign out of this phone")
            .on_click(cx.listener(|view, _, _, cx| {
                view.sheet = Some(SettingsSheet::SignOut);
                cx.notify();
            }));
        div()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .size_full()
                    .child(nav_bar(insets, back, "Settings", None))
                    .child(
                        div()
                            .id("settings")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap(px(28.))
                                    .pt(px(8.))
                                    .pb(px(insets.bottom + 32.))
                                    .pl(px(insets.left + GUTTER))
                                    .pr(px(insets.right + GUTTER))
                                    .child(self.render_account(cx))
                                    .child(self.render_appearance(cx))
                                    .child(self.render_terminal(cx))
                                    .child(v_flex().gap(px(10.)).child(sign_out).child(footnote(
                                        "Hosts and keys are encrypted on the phone \
                                                 before they go to the server.",
                                    )))
                                    .child(footnote(format!(
                                        "flint {}",
                                        env!("CARGO_PKG_VERSION")
                                    ))),
                            ),
                    ),
            )
            .children(self.render_sheet(cx))
    }
}

fn footnote(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .px(px(4.))
        .text_center()
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(flint_theme::text_placeholder())
        .child(text.into())
}
