use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Context, Entity, EventEmitter, FontWeight, Hsla, SharedString,
    Subscription, Window,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{h_flex, v_flex, Sizable as _};
use uuid::Uuid;

use flint_core::vault::{AuthMethod, ServerEntrySummary, VaultManager};

use crate::insets;
use crate::phone_theme;
use crate::session_view::SessionView;
use crate::store::{SignIn, Store};
use crate::theme as flint_theme;
use crate::toast;
use crate::ui::host_icon::host_icon;
use crate::widgets::{
    ago, ago_since, card, card_inset, chevron, chip, danger_button, empty_state, icon, icon_button,
    info_row, initial_badge, large_title, primary_button, quiet_button, row, row_code,
    row_subtitle, row_title, secondary_button, section_header, sheet, sheet_action, status_dot,
    text_button, title, GUTTER,
};

const RECENT_COUNT: usize = 3;

const TILE: f32 = 40.;

const SESSION_CARD_WIDTH: f32 = 290.;

const SESSION_PREVIEW_LINES: usize = 2;

pub enum HostsEvent {
    Connect(Uuid),
    OpenFiles(Uuid),
    OpenSession(Entity<SessionView>),
    /// `None` adds a host.
    EditHost(Option<Uuid>),
    QuickConnect,
    Settings,
}

enum HostsSheet {
    Add,
    Host(Uuid),
    ConfirmDelete(Uuid),
}

#[derive(Clone, PartialEq, Debug)]
enum HostFilter {
    Folder(String),
    Tag(String),
}

pub struct HostsView {
    store: Entity<Store>,
    sessions: Vec<Entity<SessionView>>,
    search_input: Entity<InputState>,
    filter: Option<HostFilter>,
    sheet: Option<HostsSheet>,
    _subscriptions: Vec<Subscription>,
    _session_subscriptions: Vec<Subscription>,
}

impl EventEmitter<HostsEvent> for HostsView {}

impl HostsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = Store::global(cx);
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &search_input,
                window,
                |_, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Focus => gpui_mobile::show_keyboard(),
                    InputEvent::Blur => gpui_mobile::hide_keyboard(),
                    InputEvent::PressEnter { .. } => gpui_mobile::hide_keyboard(),
                    InputEvent::Change => cx.notify(),
                },
            ),
        ];
        Self {
            store,
            sessions: Vec::new(),
            search_input,
            filter: None,
            sheet: None,
            _subscriptions: subscriptions,
            _session_subscriptions: Vec::new(),
        }
    }

    pub fn set_sessions(&mut self, sessions: Vec<Entity<SessionView>>, cx: &mut Context<Self>) {
        self._session_subscriptions = sessions
            .iter()
            .map(|session| cx.observe(session, |_, _, cx| cx.notify()))
            .collect();
        self.sessions = sessions;
        cx.notify();
    }

    fn open_sheet(&mut self, sheet: HostsSheet, cx: &mut Context<Self>) {
        gpui_mobile::hide_keyboard();
        self.sheet = Some(sheet);
        cx.notify();
    }

    fn close_sheet(&mut self, cx: &mut Context<Self>) {
        self.sheet = None;
        cx.notify();
    }

    fn delete_host(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.sheet = None;
        let deleted = self.store.update(cx, |store, cx| {
            store.edit(cx, |vault| vault.delete_entry(id))
        });
        match deleted {
            Ok(()) => toast::show("Host deleted", toast::Kind::Done, cx),
            Err(e) => toast::show(e, toast::Kind::Problem, cx),
        }
        cx.notify();
    }

    fn is_open(&self, id: Uuid, cx: &Context<Self>) -> bool {
        self.sessions
            .iter()
            .any(|session| session.read(cx).host_id() == Some(id))
    }

    fn is_connected(&self, id: Uuid, cx: &Context<Self>) -> bool {
        self.sessions.iter().any(|session| {
            let session = session.read(cx);
            session.host_id() == Some(id) && session.is_connected()
        })
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let login = store.github_login().unwrap_or_default().to_string();
        let (status, color, is_busy) = sync_status(store);
        h_flex()
            .w_full()
            .items_start()
            .justify_between()
            .child(
                v_flex().gap(px(4.)).child(large_title("Hosts")).child(
                    h_flex()
                        .gap(px(7.))
                        .child(status_dot(color, is_busy))
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(flint_theme::text_dim())
                                .child(status),
                        ),
                ),
            )
            .child(
                h_flex()
                    .mt(px(2.))
                    .gap(px(10.))
                    .child(icon_button("add", "plus").on_click(
                        cx.listener(|view, _, _, cx| view.open_sheet(HostsSheet::Add, cx)),
                    ))
                    .child(
                        div()
                            .id("settings")
                            .rounded_full()
                            .active(|avatar| avatar.opacity(0.6))
                            .child(initial_badge(&login, 40.))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(HostsEvent::Settings))),
                    ),
            )
    }

    fn render_signed_out_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let store = self.store.read(cx);
        if !store.is_session_ended() {
            return None;
        }
        let is_waiting = matches!(store.sign_in, SignIn::Browser);
        let error = store.sign_in_error.clone();
        Some(
            v_flex()
                .gap(px(4.))
                .px(px(16.))
                .pt(px(14.))
                .pb(px(6.))
                .rounded(px(16.))
                .bg(phone_theme::plate())
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(flint_theme::text())
                        .child("Signed out of sync"),
                )
                .child(
                    div()
                        .text_size(px(14.))
                        .line_height(px(20.))
                        .text_color(flint_theme::text_dim())
                        .child("Your hosts are still here. Sign in again to get changes."),
                )
                .children(error.map(|error| {
                    div()
                        .text_size(px(14.))
                        .text_color(flint_theme::red_error())
                        .child(error)
                }))
                .child(
                    text_button(
                        "sign-in-again",
                        if is_waiting {
                            "Waiting for GitHub…"
                        } else {
                            "Sign in with GitHub"
                        },
                    )
                    .self_start()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.store.update(cx, |store, cx| store.start_sign_in(cx));
                    })),
                )
                .into_any_element(),
        )
    }

    fn render_filters(
        &self,
        hosts: &[ServerEntrySummary],
        filter: Option<&HostFilter>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let choices = filter_choices(hosts);
        let has_tag = choices
            .iter()
            .any(|choice| matches!(choice, HostFilter::Tag(_)));
        if !has_tag && choices.len() < 2 {
            return None;
        }
        let insets = insets::current();
        let all =
            chip("filter-all", "All", filter.is_none()).on_click(cx.listener(|view, _, _, cx| {
                view.filter = None;
                cx.notify();
            }));
        let chips = choices.into_iter().enumerate().map(|(index, choice)| {
            let label = match &choice {
                HostFilter::Folder(name) => name.clone(),
                HostFilter::Tag(name) => format!("#{name}"),
            };
            let is_chosen = filter == Some(&choice);
            chip(("filter", index), label, is_chosen).on_click(cx.listener(
                move |view, _, _, cx| {
                    view.filter = if is_chosen {
                        None
                    } else {
                        Some(choice.clone())
                    };
                    cx.notify();
                },
            ))
        });
        Some(
            div()
                .id("filters")
                .flex()
                .gap(px(8.))
                // Runs to the screen's edges, so a chip can scroll out of sight.
                .ml(px(-(insets.left + GUTTER)))
                .mr(px(-(insets.right + GUTTER)))
                .pl(px(insets.left + GUTTER))
                .pr(px(insets.right + GUTTER))
                .overflow_x_scroll()
                .child(all)
                .children(chips)
                .into_any_element(),
        )
    }

    fn render_sessions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.sessions.is_empty() {
            return None;
        }
        let is_alone = self.sessions.len() == 1;
        let cards = self
            .sessions
            .iter()
            .enumerate()
            .map(|(index, session)| self.session_card(index, session, is_alone, cx))
            .collect::<Vec<_>>();
        let count = cards.len();
        let insets = insets::current();
        Some(
            v_flex()
                .child(section_header("Open", count))
                .child(
                    div()
                        .id("sessions-strip")
                        .flex()
                        .gap(px(10.))
                        .ml(px(-(insets.left + GUTTER)))
                        .mr(px(-(insets.right + GUTTER)))
                        .pl(px(insets.left + GUTTER))
                        .pr(px(insets.right + GUTTER))
                        .overflow_x_scroll()
                        .children(cards),
                )
                .into_any_element(),
        )
    }

    fn session_card(
        &self,
        index: usize,
        session: &Entity<SessionView>,
        is_alone: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = session.read(cx);
        let (status, color, is_busy) = view.status();
        let os = view.host_id().and_then(|id| {
            let vault = self.store.read(cx).vault()?;
            vault.get_entry(id).ok()?.os
        });
        let mut lines = view.last_lines(SESSION_PREVIEW_LINES, cx);
        let is_quiet = lines.is_empty();
        if is_quiet {
            lines.push(status.to_string());
        }
        let last = lines.len().saturating_sub(1);
        let glimpse = lines.into_iter().enumerate().map(|(index, line)| {
            div()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .when(index == last && !is_quiet, |line| {
                    line.text_color(flint_theme::text())
                })
                .child(line)
        });
        let session = session.clone();
        v_flex()
            .id(("session", index))
            .flex_none()
            .when(is_alone, |card| card.w_full())
            .when(!is_alone, |card| card.w(px(SESSION_CARD_WIDTH)))
            .p(px(14.))
            .gap(px(12.))
            .rounded(px(16.))
            .bg(phone_theme::plate())
            .active(|card| card.bg(phone_theme::fill_pressed()))
            .child(
                h_flex()
                    .gap(px(12.))
                    .child(host_icon(os.as_deref(), false, phone_theme::plate(), 36.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(1.))
                            .child(row_title(view.title.clone()))
                            .child(row_code(view.address.clone())),
                    )
                    .child(status_dot(color, is_busy)),
            )
            .child(
                v_flex()
                    .h(px(SESSION_PREVIEW_LINES as f32 * 17.))
                    .justify_end()
                    .overflow_hidden()
                    .font_family(phone_theme::MONO_FONT)
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(flint_theme::text_dim())
                    .children(glimpse),
            )
            .on_click(
                cx.listener(move |_, _, _, cx| cx.emit(HostsEvent::OpenSession(session.clone()))),
            )
            .into_any_element()
    }

    fn host_row(
        &self,
        prefix: &str,
        index: usize,
        host: &ServerEntrySummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = host.id;
        let is_connected = self.is_connected(id, cx);
        row(SharedString::from(format!("{prefix}-{index}")))
            .child(host_icon(
                host.os.as_deref(),
                is_connected,
                phone_theme::plate(),
                TILE,
            ))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(row_title(display_name(host)))
                    .child(row_code(address(host))),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{prefix}-more-{index}")))
                    .flex_none()
                    .size(px(40.))
                    .mr(px(-8.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .active(|button| button.bg(phone_theme::fill_pressed()))
                    .child(icon("ellipsis", 20., flint_theme::text_placeholder()))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.open_sheet(HostsSheet::Host(id), cx);
                    })),
            )
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(HostsEvent::Connect(id))))
            .into_any_element()
    }

    fn render_hosts(
        &self,
        mut hosts: Vec<ServerEntrySummary>,
        filter: Option<&HostFilter>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        let is_searching = !query.is_empty();
        if is_searching {
            hosts.retain(|host| matches_query(host, &query));
        }
        if let Some(filter) = filter {
            hosts.retain(|host| matches_filter(host, filter));
        }
        hosts.sort_by_key(|host| display_name(host).to_lowercase());

        if hosts.is_empty() {
            let store = self.store.read(cx);
            let is_first_sync = store.is_first_series && store.is_syncing;
            return vec![self.render_empty(is_searching, is_first_sync, cx)];
        }

        let mut sections = Vec::new();
        if is_searching {
            let rows = hosts
                .iter()
                .enumerate()
                .map(|(index, host)| self.host_row("found", index, host, cx))
                .collect::<Vec<_>>();
            sections.push(
                v_flex()
                    .child(section_header("Found", rows.len()))
                    .child(card(rows))
                    .into_any_element(),
            );
            return sections;
        }

        let mut recent: Vec<_> = hosts
            .iter()
            .filter(|host| host.last_opened_at.is_some())
            .collect();
        recent.sort_by_key(|host| std::cmp::Reverse(host.last_opened_at));
        recent.truncate(RECENT_COUNT);
        // Not worth it when the folders below fit on the screen anyway.
        if filter.is_none() && hosts.len() > 6 && !recent.is_empty() {
            let rows = recent
                .iter()
                .enumerate()
                .map(|(index, host)| self.host_row("recent", index, host, cx))
                .collect::<Vec<_>>();
            sections.push(
                v_flex()
                    .child(section_header("Recent", rows.len()))
                    .child(card(rows))
                    .into_any_element(),
            );
        }

        let groups = group_hosts(&hosts);
        let is_one_group = groups.len() == 1;
        for (group_index, group) in groups.into_iter().enumerate() {
            let title = match (&group.name, filter) {
                (_, Some(HostFilter::Tag(tag))) if is_one_group => format!("#{tag}"),
                (None, _) if is_one_group => "All hosts".to_string(),
                (None, _) => "Other hosts".to_string(),
                (Some(name), _) => name.clone(),
            };
            let prefix = format!("group{group_index}");
            let rows = group
                .hosts
                .iter()
                .enumerate()
                .map(|(index, host)| self.host_row(&prefix, index, host, cx))
                .collect::<Vec<_>>();
            sections.push(
                v_flex()
                    .child(section_header(title, rows.len()))
                    .child(card(rows))
                    .into_any_element(),
            );
        }
        sections
    }

    fn render_empty(
        &self,
        is_searching: bool,
        is_first_sync: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if is_searching {
            return empty_state("Nothing found", "No host matches the search.").into_any_element();
        }
        if is_first_sync {
            return empty_state(
                "Getting your hosts…",
                "They are on their way from your account.",
            )
            .into_any_element();
        }
        empty_state(
            "No hosts yet",
            "Add one here, or in flint on your computer: it shows up on the phone in a moment.",
        )
        .child(
            text_button("add-first", "Add a host")
                .mt(px(6.))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(HostsEvent::EditHost(None)))),
        )
        .into_any_element()
    }

    fn render_bottom_bar(&self) -> impl IntoElement {
        let insets = insets::current();
        div()
            .id("bottom-bar")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            // The list scrolls under it; a tap here must not reach a host.
            .occlude()
            .pt(px(10.))
            .pb(px(insets.bottom.max(12.) + 4.))
            .pl(px(insets.left + GUTTER))
            .pr(px(insets.right + GUTTER))
            .bg(phone_theme::ground())
            .border_t_1()
            .border_color(flint_theme::border())
            .child(
                Styled::h(
                    Input::new(&self.search_input)
                        .large()
                        .prefix(icon("search", 18., flint_theme::text_dim()))
                        .cleanable(true),
                    px(44.),
                )
                .rounded(px(12.))
                .bg(phone_theme::fill())
                .border_color(gpui::transparent_black())
                .text_size(px(16.)),
            )
    }

    fn render_add_sheet(&self, cx: &mut Context<Self>) -> AnyElement {
        let option = |id: &'static str, name: &str, title: &str, text: &str| {
            row(id)
                .child(
                    div()
                        .w(px(24.))
                        .child(icon(name, 22., flint_theme::text_dim())),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(row_title(title.to_string()))
                        .child(row_subtitle(text.to_string())),
                )
                .child(chevron())
        };
        v_flex()
            .gap(px(16.))
            .child(div().px(px(4.)).child(title("Add")))
            .child(card_inset(
                [
                    option(
                        "new-host",
                        "server",
                        "New host",
                        "Saved to your account and every device",
                    )
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.close_sheet(cx);
                        cx.emit(HostsEvent::EditHost(None));
                    }))
                    .into_any_element(),
                    option(
                        "quick",
                        "terminal",
                        "Quick connect",
                        "Connect once without saving",
                    )
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.close_sheet(cx);
                        cx.emit(HostsEvent::QuickConnect);
                    }))
                    .into_any_element(),
                ],
                50.,
            ))
            .into_any_element()
    }

    fn render_host_sheet(&self, id: Uuid, cx: &mut Context<Self>) -> Option<AnyElement> {
        let store = self.store.read(cx);
        let vault = store.vault()?;
        let entry = vault.get_entry(id).ok()?;
        let summary = vault
            .list_entries()
            .ok()?
            .into_iter()
            .find(|host| host.id == id)?;
        let is_open = self.is_open(id, cx);
        let is_connected = self.is_connected(id, cx);
        let facts = host_facts(vault, &summary, &entry.auth, entry.jump_host);

        let header = h_flex()
            .gap(px(14.))
            .px(px(4.))
            .child(host_icon(
                summary.os.as_deref(),
                is_connected,
                phone_theme::ground(),
                52.,
            ))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(title(display_name(&summary)).truncate())
                    .child(row_code(address(&summary))),
            );
        let buttons = h_flex()
            .gap(px(10.))
            .child(
                div().flex_1().child(
                    primary_button("connect", if is_open { "Go to session" } else { "Connect" })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.close_sheet(cx);
                            cx.emit(HostsEvent::Connect(id));
                        })),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .child(
                        secondary_button("browse-files", "Files").on_click(cx.listener(
                            move |view, _, _, cx| {
                                view.close_sheet(cx);
                                cx.emit(HostsEvent::OpenFiles(id));
                            },
                        )),
                    ),
            );
        let facts = card_inset(
            facts
                .into_iter()
                .map(|(label, value)| info_row(label, value).into_any_element()),
            16.,
        );
        let actions = card_inset(
            [
                sheet_action("edit", "pen-line", "Edit host", false)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.close_sheet(cx);
                        cx.emit(HostsEvent::EditHost(Some(id)));
                    }))
                    .into_any_element(),
                sheet_action("delete", "trash", "Delete host", true)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.open_sheet(HostsSheet::ConfirmDelete(id), cx)
                    }))
                    .into_any_element(),
            ],
            50.,
        );
        Some(
            v_flex()
                .gap(px(16.))
                .child(header)
                .child(buttons)
                .child(facts)
                .child(actions)
                .into_any_element(),
        )
    }

    fn render_delete_sheet(&self, id: Uuid, cx: &mut Context<Self>) -> Option<AnyElement> {
        let store = self.store.read(cx);
        let vault = store.vault()?;
        let entry = vault.get_entry(id).ok()?;
        let name = if entry.label.trim().is_empty() {
            entry.host.clone()
        } else {
            entry.label.clone()
        };
        let is_shared = vault.sharing_label(id).is_some();
        Some(
            v_flex()
                .gap(px(8.))
                .child(div().px(px(4.)).child(title(format!("Delete {name}?"))))
                .child(
                    div()
                        .px(px(4.))
                        .text_size(px(15.))
                        .line_height(px(21.))
                        .text_color(flint_theme::text_dim())
                        .child(if is_shared {
                            "It goes for everyone in the team, from every device."
                        } else {
                            "It goes from every device of your account."
                        }),
                )
                .child(div().h(px(10.)))
                .child(
                    danger_button("confirm-delete", "Delete host")
                        .on_click(cx.listener(move |view, _, _, cx| view.delete_host(id, cx))),
                )
                .child(
                    quiet_button("keep", "Cancel")
                        .on_click(cx.listener(|view, _, _, cx| view.close_sheet(cx))),
                )
                .into_any_element(),
        )
    }

    fn render_sheet(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let content = match self.sheet.as_ref()? {
            HostsSheet::Add => self.render_add_sheet(cx),
            HostsSheet::Host(id) => self.render_host_sheet(*id, cx)?,
            HostsSheet::ConfirmDelete(id) => self.render_delete_sheet(*id, cx)?,
        };
        let this = cx.entity().downgrade();
        let dismiss = move |_: &mut Window, cx: &mut gpui::App| {
            this.update(cx, |view, cx| view.close_sheet(cx)).ok();
        };
        Some(sheet("hosts-sheet", content, dismiss))
    }
}

fn host_facts(
    vault: &VaultManager,
    host: &ServerEntrySummary,
    auth: &AuthMethod,
    jump_host: Option<Uuid>,
) -> Vec<(&'static str, String)> {
    let mut facts = vec![
        ("Address", host.host.clone()),
        ("User", host.username.clone()),
    ];
    if host.port != 22 {
        facts.push(("Port", host.port.to_string()));
    }
    let sign_in = match auth {
        AuthMethod::Password(_) => "Password".to_string(),
        AuthMethod::PrivateKey { key_id } => vault
            .list_keys()
            .ok()
            .and_then(|keys| keys.into_iter().find(|key| key.id == *key_id))
            .map_or_else(|| "SSH key".to_string(), |key| key.name),
        AuthMethod::Agent => "SSH agent of your computer".to_string(),
    };
    facts.push(("Sign in", sign_in));
    if let Some(jump) = jump_host.and_then(|id| vault.get_entry(id).ok()) {
        let name = if jump.label.trim().is_empty() {
            jump.host
        } else {
            jump.label
        };
        facts.push(("Through", name));
    }
    if let Some(os) = &host.os {
        facts.push(("System", os.clone()));
    }
    if let Some(group) = &host.group_name {
        facts.push(("Folder", group.clone()));
    }
    if !host.tag_names.is_empty() {
        let tags = host
            .tag_names
            .iter()
            .map(|tag| format!("#{tag}"))
            .collect::<Vec<_>>();
        facts.push(("Tags", tags.join(" ")));
    }
    if let Some(team) = &host.team_name {
        facts.push(("Shared with", team.clone()));
    }
    if let Some(at) = host.last_opened_at {
        facts.push(("Last opened", ago_since(at)));
    }
    facts
}

/// Folders leave out team hosts, which have their own sections already.
fn filter_choices(hosts: &[ServerEntrySummary]) -> Vec<HostFilter> {
    let mut folders: Vec<&String> = hosts
        .iter()
        .filter(|host| host.team_name.is_none())
        .filter_map(|host| host.group_name.as_ref())
        .collect();
    folders.sort_by_key(|name| name.to_lowercase());
    folders.dedup();
    let mut tags: Vec<&String> = hosts.iter().flat_map(|host| &host.tag_names).collect();
    tags.sort_by_key(|name| name.to_lowercase());
    tags.dedup();
    folders
        .into_iter()
        .map(|name| HostFilter::Folder(name.clone()))
        .chain(tags.into_iter().map(|name| HostFilter::Tag(name.clone())))
        .collect()
}

fn matches_filter(host: &ServerEntrySummary, filter: &HostFilter) -> bool {
    match filter {
        HostFilter::Folder(name) => {
            host.team_name.is_none() && host.group_name.as_ref() == Some(name)
        }
        HostFilter::Tag(name) => host.tag_names.contains(name),
    }
}

struct Group<'a> {
    /// `None` for the hosts in no folder and no team.
    name: Option<String>,
    is_team: bool,
    hosts: Vec<&'a ServerEntrySummary>,
}

fn group_hosts(hosts: &[ServerEntrySummary]) -> Vec<Group<'_>> {
    let mut groups: Vec<Group> = Vec::new();
    for host in hosts {
        let (name, is_team) = match (&host.team_name, &host.group_name) {
            (Some(team), _) => (Some(team.clone()), true),
            (None, Some(group)) => (Some(group.clone()), false),
            (None, None) => (None, false),
        };
        match groups
            .iter_mut()
            .find(|group| group.name == name && group.is_team == is_team)
        {
            Some(group) => group.hosts.push(host),
            None => groups.push(Group {
                name,
                is_team,
                hosts: vec![host],
            }),
        }
    }
    groups.sort_by_key(|group| {
        (
            group.is_team,
            group.name.is_none(),
            group.name.as_deref().map(str::to_lowercase),
        )
    });
    groups
}

fn display_name(host: &ServerEntrySummary) -> String {
    if host.label.trim().is_empty() {
        host.host.clone()
    } else {
        host.label.clone()
    }
}

fn address(host: &ServerEntrySummary) -> String {
    if host.port == 22 {
        format!("{}@{}", host.username, host.host)
    } else {
        format!("{}@{}:{}", host.username, host.host, host.port)
    }
}

fn matches_query(host: &ServerEntrySummary, query: &str) -> bool {
    [
        Some(&host.label),
        Some(&host.host),
        Some(&host.username),
        host.group_name.as_ref(),
        host.team_name.as_ref(),
        host.os.as_ref(),
    ]
    .into_iter()
    .flatten()
    .chain(host.tag_names.iter())
    .any(|text| text.to_lowercase().contains(query))
}

fn sync_status(store: &Store) -> (String, Hsla, bool) {
    if store.is_session_ended() {
        return ("Signed out of sync".into(), flint_theme::warning(), false);
    }
    if store.is_syncing {
        return ("Syncing…".into(), flint_theme::accent(), true);
    }
    if let Some(error) = &store.sync_error {
        return (
            format!("Sync failed: {error}"),
            flint_theme::red_error(),
            false,
        );
    }
    match store.last_synced {
        Some(at) => (
            format!("Synced {}", ago(at.elapsed().as_secs())),
            flint_theme::status_online(),
            false,
        ),
        None => ("Not synced yet".into(), flint_theme::text_dim(), false),
    }
}

impl Render for HostsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let insets = insets::current();
        let hosts = self
            .store
            .read(cx)
            .vault()
            .and_then(|vault| vault.list_entries().ok());
        // Ignore a picked folder or tag that has no hosts left, after a delete or a sync.
        let filter = self.filter.clone().filter(|filter| {
            hosts
                .as_ref()
                .is_some_and(|hosts| hosts.iter().any(|host| matches_filter(host, filter)))
        });
        let filters = hosts
            .as_deref()
            .and_then(|hosts| self.render_filters(hosts, filter.as_ref(), cx));
        let sections = hosts
            .map(|hosts| self.render_hosts(hosts, filter.as_ref(), cx))
            .unwrap_or_default();
        div()
            .relative()
            .size_full()
            .bg(phone_theme::ground())
            .child(
                div().id("hosts").size_full().overflow_y_scroll().child(
                    v_flex()
                        .w_full()
                        .gap(px(24.))
                        .pt(px(insets.top + 12.))
                        .pb(px(insets.bottom + 96.))
                        .pl(px(insets.left + GUTTER))
                        .pr(px(insets.right + GUTTER))
                        .child(self.render_header(cx))
                        .children(self.render_signed_out_banner(cx))
                        .children(self.render_sessions(cx))
                        .children(filters)
                        .children(sections),
                ),
            )
            .child(self.render_bottom_bar())
            .children(self.render_sheet(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(label: &str, host: &str, group: Option<&str>) -> ServerEntrySummary {
        ServerEntrySummary {
            id: Uuid::new_v4(),
            label: label.into(),
            host: host.into(),
            port: 22,
            username: "root".into(),
            os: Some("Ubuntu 24.04".into()),
            last_opened_at: None,
            group_name: group.map(Into::into),
            tag_names: vec!["prod".into()],
            proxy: Default::default(),
            team_name: None,
        }
    }

    #[test]
    fn search_looks_at_names_addresses_folders_tags_and_systems() {
        let web = host("web", "10.0.0.1", Some("Berlin"));
        assert!(matches_query(&web, "web"));
        assert!(matches_query(&web, "10.0"));
        assert!(matches_query(&web, "berlin"));
        assert!(matches_query(&web, "prod"));
        assert!(matches_query(&web, "ubuntu"));
        assert!(!matches_query(&web, "db"));
    }

    #[test]
    fn a_host_without_a_name_goes_by_its_address() {
        assert_eq!(display_name(&host(" ", "10.0.0.1", None)), "10.0.0.1");
        let mut other = host("db", "db.local", None);
        other.port = 2222;
        assert_eq!(address(&other), "root@db.local:2222");
    }

    #[test]
    fn folders_come_first_then_loose_hosts_then_teams() {
        let mut shared = host("ci", "ci.local", None);
        shared.team_name = Some("Ops".into());
        let hosts = vec![
            host("b", "b", Some("Zurich")),
            shared,
            host("a", "a", Some("berlin")),
            host("loose", "l", None),
        ];
        let names: Vec<_> = group_hosts(&hosts)
            .into_iter()
            .map(|group| (group.name, group.is_team))
            .collect();
        assert_eq!(
            names,
            vec![
                (Some("berlin".into()), false),
                (Some("Zurich".into()), false),
                (None, false),
                (Some("Ops".into()), true),
            ]
        );
    }

    #[test]
    fn filters_list_each_folder_and_tag_once() {
        let mut vpn = host("vpn", "v", Some("Berlin"));
        vpn.tag_names = vec!["vpn".into(), "prod".into()];
        let mut shared = host("ci", "c", Some("Team folder"));
        shared.team_name = Some("Ops".into());
        let hosts = vec![host("a", "a", Some("Berlin")), vpn, shared];
        assert_eq!(
            filter_choices(&hosts),
            vec![
                HostFilter::Folder("Berlin".into()),
                HostFilter::Tag("prod".into()),
                HostFilter::Tag("vpn".into()),
            ]
        );
    }

    #[test]
    fn a_filter_keeps_the_hosts_of_its_folder_or_tag() {
        let mut vpn = host("vpn", "v", Some("Berlin"));
        vpn.tag_names = vec!["vpn".into()];
        assert!(matches_filter(&vpn, &HostFilter::Folder("Berlin".into())));
        assert!(matches_filter(&vpn, &HostFilter::Tag("vpn".into())));
        assert!(!matches_filter(&vpn, &HostFilter::Tag("prod".into())));
        assert!(!matches_filter(&vpn, &HostFilter::Folder("Zurich".into())));
    }
}
