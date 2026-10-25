use gpui::prelude::*;
use gpui::{
    div, linear_color_stop, linear_gradient, px, AnyElement, App, Entity, FontWeight, Hsla, Window,
};

use crate::phone_theme;
use crate::theme as flint_theme;
use crate::ui::terminal_view::{Modifier, StickyKey, TerminalView};
use crate::widgets::icon;

pub const KEYS_BAR_HEIGHT: f32 = 54.;

const KEY_HEIGHT: f32 = 38.;

#[derive(Clone, Copy)]
enum Key {
    Press(&'static str, Label),
    Text(&'static str),
    Modifier(Modifier, &'static str),
    Paste,
}

#[derive(Clone, Copy)]
enum Label {
    Text(&'static str),
    Icon(&'static str),
}

const COMMAND_KEYS: [Key; 4] = [
    Key::Press("escape", Label::Text("esc")),
    Key::Modifier(Modifier::Ctrl, "ctrl"),
    Key::Modifier(Modifier::Alt, "alt"),
    Key::Press("tab", Label::Text("tab")),
];

const ARROW_KEYS: [Key; 4] = [
    Key::Press("left", Label::Icon("arrow-left")),
    Key::Press("up", Label::Icon("arrow-up")),
    Key::Press("down", Label::Icon("arrow-down")),
    Key::Press("right", Label::Icon("arrow-right")),
];

const SYMBOL_KEYS: [Key; 4] = [
    Key::Text("|"),
    Key::Text("/"),
    Key::Text("-"),
    Key::Text("~"),
];

const PAGE_KEYS: [Key; 5] = [
    Key::Press("home", Label::Text("home")),
    Key::Press("end", Label::Text("end")),
    Key::Press("pageup", Label::Text("pg up")),
    Key::Press("pagedown", Label::Text("pg dn")),
    Key::Paste,
];

type SnippetsHandler = Box<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct KeysBar {
    terminal: Entity<TerminalView>,
    is_keyboard_visible: bool,
    on_snippets: Option<SnippetsHandler>,
}

impl KeysBar {
    pub fn new(terminal: Entity<TerminalView>, is_keyboard_visible: bool) -> Self {
        Self {
            terminal,
            is_keyboard_visible,
            on_snippets: None,
        }
    }

    /// Puts a snippets key first. Only saved hosts have snippets.
    pub fn on_snippets(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_snippets = Some(Box::new(handler));
        self
    }

    fn key(&self, group: &str, index: usize, key: Key, cx: &App) -> gpui::Stateful<gpui::Div> {
        let state = {
            let terminal = self.terminal.read(cx);
            match key {
                Key::Modifier(Modifier::Ctrl, _) => terminal.ctrl,
                Key::Modifier(Modifier::Alt, _) => terminal.alt,
                _ => StickyKey::Off,
            }
        };
        let terminal = self.terminal.clone();
        key_cap(
            gpui::ElementId::from((gpui::SharedString::from(group.to_string()), index)),
            label(key),
            state,
        )
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            terminal.update(cx, |terminal, cx| press(terminal, key, cx));
        })
    }

    fn group(&self, name: &str, keys: &[Key], cx: &App) -> AnyElement {
        gpui_component::h_flex()
            .flex_none()
            .gap(px(5.))
            .children(
                keys.iter()
                    .enumerate()
                    .map(|(index, key)| self.key(name, index, *key, cx)),
            )
            .into_any_element()
    }

    fn arrows(&self) -> AnyElement {
        let mut block = gpui_component::h_flex()
            .flex_none()
            .h(px(KEY_HEIGHT))
            .rounded(px(9.))
            .bg(phone_theme::key_face())
            .border_b_2()
            .border_color(phone_theme::key_edge())
            .overflow_hidden();
        for (index, key) in ARROW_KEYS.iter().enumerate() {
            if index > 0 {
                block = block.child(div().w(px(1.)).h(px(18.)).bg(flint_theme::border()));
            }
            let terminal = self.terminal.clone();
            let key = *key;
            let Label::Icon(name) = label(key) else {
                continue;
            };
            block = block.child(
                div()
                    .id(("arrow", index))
                    .w(px(36.))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .active(|segment| segment.bg(phone_theme::fill_pressed()))
                    .child(icon(name, 17., flint_theme::text()))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        terminal.update(cx, |terminal, cx| press(terminal, key, cx));
                    }),
            );
        }
        block.into_any_element()
    }

    fn keyboard_toggle(&self) -> AnyElement {
        let terminal = self.terminal.clone();
        let is_keyboard_visible = self.is_keyboard_visible;
        let ground = phone_theme::keys_ground();
        div()
            .relative()
            .flex()
            .items_center()
            .h_full()
            .pl(px(4.))
            .pr(px(8.))
            .bg(ground)
            // Fades the keys out under the toggle, to hint that the row scrolls.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(-24.))
                    .w(px(24.))
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(ground.opacity(0.), 0.),
                        linear_color_stop(ground, 1.),
                    )),
            )
            .child(
                key_cap(
                    "keyboard-toggle",
                    Label::Icon(if is_keyboard_visible {
                        "keyboard-off"
                    } else {
                        "keyboard"
                    }),
                    StickyKey::Off,
                )
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    if is_keyboard_visible {
                        gpui_mobile::hide_keyboard();
                    } else {
                        terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
                    }
                }),
            )
            .into_any_element()
    }
}

impl RenderOnce for KeysBar {
    fn render(mut self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let snippets = self.on_snippets.take().map(|on_snippets| {
            key_cap("snippets", Label::Icon("braces"), StickyKey::Off).on_click(
                move |_, window, cx| {
                    cx.stop_propagation();
                    on_snippets(window, cx);
                },
            )
        });
        let keys = div()
            .id("keys")
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(10.))
            .pl(px(8.))
            .pr(px(28.))
            .overflow_x_scroll()
            .children(snippets)
            .child(self.group("command", &COMMAND_KEYS, cx))
            .child(self.arrows())
            .child(self.group("symbol", &SYMBOL_KEYS, cx))
            .child(self.group("page", &PAGE_KEYS, cx));
        div()
            .flex()
            .items_center()
            .h(px(KEYS_BAR_HEIGHT))
            .w_full()
            .bg(phone_theme::keys_ground())
            .child(keys)
            .child(self.keyboard_toggle())
    }
}

fn label(key: Key) -> Label {
    match key {
        Key::Press(_, label) => label,
        Key::Modifier(_, text) | Key::Text(text) => Label::Text(text),
        Key::Paste => Label::Icon("clipboard-paste"),
    }
}

fn press(terminal: &mut TerminalView, key: Key, cx: &mut gpui::Context<TerminalView>) {
    match key {
        Key::Press(name, _) => terminal.press_key(name, cx),
        Key::Text(text) => terminal.commit_text(text, cx),
        Key::Modifier(modifier, _) => terminal.tap_modifier(modifier, cx),
        Key::Paste => terminal.paste(cx),
    }
}

fn key_cap(
    id: impl Into<gpui::ElementId>,
    label: Label,
    state: StickyKey,
) -> gpui::Stateful<gpui::Div> {
    let (face, edge, text): (Hsla, Hsla, Hsla) = match state {
        StickyKey::Off => (
            phone_theme::key_face(),
            phone_theme::key_edge(),
            flint_theme::text(),
        ),
        StickyKey::Once => (
            flint_theme::accent_soft_press(),
            flint_theme::accent().opacity(0.5),
            flint_theme::accent(),
        ),
        StickyKey::Locked => (
            flint_theme::accent(),
            flint_theme::accent().opacity(0.6),
            flint_theme::accent_ink(),
        ),
    };
    let content: AnyElement = match label {
        Label::Text(text_label) => div()
            .font_family(phone_theme::MONO_FONT)
            .text_size(px(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(text)
            .child(text_label)
            .into_any_element(),
        Label::Icon(name) => icon(name, 17., text).into_any_element(),
    };
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .h(px(KEY_HEIGHT))
        .min_w(px(38.))
        .px(px(9.))
        .rounded(px(9.))
        .bg(face)
        .border_b_2()
        .border_color(edge)
        .active(|key| key.bg(flint_theme::element_active()))
        .child(content)
}
