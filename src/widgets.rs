use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, hsla, percentage, pulsating_between, px, svg, Animation, AnimationExt as _, AnyElement,
    App, Div, ElementId, Entity, FontWeight, Hsla, Image, ImageFormat, SharedString, Stateful, Svg,
    Transformation,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::{h_flex, v_flex, Sizable as _};

use crate::insets::{self, Insets};
use crate::phone_theme;
use crate::theme as flint_theme;

pub const CONTROL_HEIGHT: f32 = 48.;

pub const BUTTON_HEIGHT: f32 = 50.;

pub const GUTTER: f32 = 16.;

pub fn app_icon() -> Arc<Image> {
    static ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../assets/icon.png").to_vec(),
        ))
    });
    ICON.clone()
}

/// An icon from `assets/icons` or gpui-component's set, by its file name.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .flex_none()
        .size(px(size))
        .text_color(color)
}

pub fn spinner(size: f32, color: Hsla) -> AnyElement {
    icon("loader-circle", size, color)
        .with_animation(
            "spinner",
            Animation::new(Duration::from_millis(900)).repeat(),
            |icon, delta| icon.with_transformation(Transformation::rotate(percentage(delta))),
        )
        .into_any_element()
}

pub fn large_title(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(32.))
        .line_height(px(38.))
        .font_weight(FontWeight::BOLD)
        .text_color(flint_theme::text())
        .child(text.into())
}

pub fn title(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(20.))
        .line_height(px(26.))
        .font_weight(FontWeight::BOLD)
        .text_color(flint_theme::text())
        .child(text.into())
}

pub fn body_text(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(15.))
        .line_height(px(22.))
        .text_color(flint_theme::text_dim())
        .child(text.into())
}

/// Button labels follow the button's size; this one is a size up, for a thumb.
pub fn button_text(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(px(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

pub fn primary_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id)
        .primary()
        .w_full()
        .h(px(BUTTON_HEIGHT))
        .rounded(px(14.))
        .child(button_text(label))
}

pub fn busy_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    is_busy: bool,
) -> Button {
    use gpui_component::Disableable as _;
    if !is_busy {
        return primary_button(id, label);
    }
    Button::new(id)
        .primary()
        .w_full()
        .h(px(BUTTON_HEIGHT))
        .rounded(px(14.))
        .disabled(true)
        .child(
            h_flex()
                .gap(px(10.))
                .child(spinner(18., flint_theme::accent_ink()))
                .child(button_text(label)),
        )
}

pub fn secondary_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .h(px(BUTTON_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .rounded(px(14.))
        .bg(phone_theme::fill())
        .text_color(flint_theme::text())
        .active(|button| button.bg(phone_theme::fill_pressed()))
        .child(button_text(label))
}

pub fn quiet_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .h(px(CONTROL_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(14.))
        .text_size(px(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(flint_theme::text_dim())
        .active(|button| button.bg(phone_theme::fill()))
        .child(label.into())
}

pub fn danger_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id)
        .danger()
        .w_full()
        .h(px(BUTTON_HEIGHT))
        .rounded(px(14.))
        .child(button_text(label))
}

pub fn icon_button(id: impl Into<ElementId>, name: &str) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex_none()
        .size(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(phone_theme::fill())
        .active(|button| button.bg(phone_theme::fill_pressed()))
        .child(icon(name, 20., flint_theme::text()))
}

pub fn plain_icon_button(id: impl Into<ElementId>, name: &str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(36.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .active(|button| button.bg(phone_theme::fill_pressed()))
        .child(icon(name, 20., flint_theme::text()))
}

pub fn text_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .py(px(8.))
        .text_size(px(15.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(flint_theme::accent())
        .active(|button| button.opacity(0.6))
        .child(label.into())
}

pub fn chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    is_chosen: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(34.))
        .px(px(14.))
        .flex()
        .items_center()
        .rounded_full()
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .when(is_chosen, |chip| {
            chip.bg(flint_theme::text())
                .text_color(phone_theme::ground())
        })
        .when(!is_chosen, |chip| {
            chip.bg(phone_theme::fill())
                .text_color(flint_theme::text())
                .active(|chip| chip.bg(phone_theme::fill_pressed()))
        })
        .child(label.into())
}

pub fn field(label: impl Into<SharedString>, input: impl IntoElement) -> Div {
    v_flex()
        .gap(px(8.))
        .child(
            div()
                .px(px(4.))
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(flint_theme::text_dim())
                .child(label.into()),
        )
        .child(input)
}

pub fn field_input(state: &Entity<InputState>) -> Input {
    // `Input::h` sizes only a multi-line input; the style's height fits any.
    Styled::h(Input::new(state).large(), px(CONTROL_HEIGHT))
        .text_size(px(16.))
        .rounded(px(12.))
        .bg(phone_theme::fill())
        .border_color(gpui::transparent_black())
}

pub fn section_title(text: impl Into<SharedString>) -> Div {
    div()
        .px(px(4.))
        .pb(px(8.))
        .text_size(px(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(flint_theme::text_dim())
        .child(text.into())
}

pub fn section_header(text: impl Into<SharedString>, count: usize) -> Div {
    h_flex()
        .px(px(4.))
        .pb(px(8.))
        .gap(px(6.))
        .text_size(px(14.))
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(flint_theme::text_dim())
                .child(text.into()),
        )
        .child(
            div()
                .text_color(flint_theme::text_placeholder())
                .child(count.to_string()),
        )
}

/// For rows that start with a 40px tile: the hairlines start where the rows' text does.
pub fn card(rows: impl IntoIterator<Item = AnyElement>) -> Div {
    card_inset(rows, 66.)
}

pub fn card_inset(rows: impl IntoIterator<Item = AnyElement>, inset: f32) -> Div {
    let mut column = v_flex()
        .w_full()
        .rounded(px(16.))
        .bg(phone_theme::plate())
        .overflow_hidden();
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            column = column.child(div().h(px(1.)).ml(px(inset)).bg(flint_theme::border()));
        }
        column = column.child(row);
    }
    column
}

pub fn row(id: impl Into<ElementId>) -> Stateful<Div> {
    h_flex()
        .id(id)
        .w_full()
        .min_h(px(62.))
        .px(px(14.))
        .py(px(10.))
        .gap(px(12.))
        .active(|row| row.bg(phone_theme::fill()))
}

pub fn row_title(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(16.))
        .line_height(px(21.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(flint_theme::text())
        .truncate()
        .child(text.into())
}

pub fn row_subtitle(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(13.))
        .line_height(px(18.))
        .text_color(flint_theme::text_dim())
        .truncate()
        .child(text.into())
}

pub fn row_code(text: impl Into<SharedString>) -> Div {
    row_subtitle(text)
        .font_family(phone_theme::MONO_FONT)
        .text_size(px(12.5))
}

pub fn chevron() -> Svg {
    icon("chevron-right", 18., flint_theme::text_placeholder())
}

pub fn initial_badge(name: &str, size: f32) -> Div {
    let letter = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string());
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(phone_theme::fill())
        .text_color(flint_theme::text())
        .text_size(px((size * 0.42).round()))
        .font_weight(FontWeight::SEMIBOLD)
        .map(|badge| match letter {
            Some(letter) => badge.child(letter),
            None => badge.child(icon("user", (size * 0.5).round(), flint_theme::text_dim())),
        })
}

pub fn nav_bar(
    insets: Insets,
    leading: impl IntoElement,
    title: impl Into<SharedString>,
    trailing: Option<AnyElement>,
) -> Div {
    div().w_full().pt(px(insets.top)).child(
        h_flex()
            .relative()
            .w_full()
            .h(px(56.))
            .px(px(12.))
            .justify_between()
            .child(leading)
            .children(trailing)
            .child(
                div()
                    .absolute()
                    .left(px(96.))
                    .right(px(96.))
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(flint_theme::text())
                            .truncate()
                            .child(title.into()),
                    ),
            ),
    )
}

pub fn back_button(id: impl Into<ElementId>) -> Stateful<Div> {
    icon_button(id, "chevron-left")
}

pub fn nav_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    is_strong: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(40.))
        .px(px(16.))
        .flex()
        .items_center()
        .rounded_full()
        .text_size(px(16.))
        .when(is_strong, |action| {
            action
                .font_weight(FontWeight::SEMIBOLD)
                .bg(flint_theme::accent())
                .text_color(flint_theme::accent_ink())
        })
        .when(!is_strong, |action| {
            action
                .font_weight(FontWeight::MEDIUM)
                .bg(phone_theme::fill())
                .text_color(flint_theme::text())
        })
        .active(|action| action.opacity(0.7))
        .child(label.into())
}

pub fn sheet(
    id: impl Into<ElementId>,
    content: impl IntoElement,
    on_dismiss: impl Fn(&mut gpui::Window, &mut App) + 'static,
) -> AnyElement {
    let insets = insets::current();
    div()
        .id(id)
        .absolute()
        .inset_0()
        // Taps must not reach the screen under the dimmed part.
        .occlude()
        .flex()
        .flex_col()
        .justify_end()
        .bg(hsla(0., 0., 0., 0.5))
        .on_click(move |_, window, cx| on_dismiss(window, cx))
        .child(
            div()
                .id("sheet-body")
                .w_full()
                .max_h(gpui::relative(0.88))
                .overflow_y_scroll()
                .pt(px(8.))
                .pb(px(insets.bottom.max(12.) + 8.))
                .pl(px(insets.left + GUTTER))
                .pr(px(insets.right + GUTTER))
                .rounded_t(px(28.))
                .bg(phone_theme::ground())
                .border_t_1()
                .border_color(flint_theme::float_border())
                // Otherwise a tap inside the sheet would also dismiss it.
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .w(px(36.))
                        .h(px(5.))
                        .mx_auto()
                        .mb(px(16.))
                        .rounded_full()
                        .bg(flint_theme::border_focused().opacity(0.6)),
                )
                .child(content),
        )
        .into_any_element()
}

pub fn sheet_heading(title_text: impl Into<SharedString>, subtitle: impl IntoElement) -> Div {
    v_flex()
        .px(px(4.))
        .gap(px(3.))
        .child(title(title_text).truncate())
        .child(subtitle)
}

pub fn sheet_action(
    id: impl Into<ElementId>,
    name: &str,
    label: impl Into<SharedString>,
    is_destructive: bool,
) -> Stateful<Div> {
    let color = if is_destructive {
        flint_theme::red_error()
    } else {
        flint_theme::text()
    };
    h_flex()
        .id(id)
        .w_full()
        .h(px(52.))
        .px(px(16.))
        .gap(px(14.))
        .active(|action| action.bg(phone_theme::fill()))
        .child(icon(
            name,
            20.,
            if is_destructive {
                color
            } else {
                flint_theme::text_dim()
            },
        ))
        .child(
            div()
                .flex_1()
                .text_size(px(16.))
                .text_color(color)
                .child(label.into()),
        )
}

pub fn info_row(label: impl Into<SharedString>, value: impl Into<SharedString>) -> Div {
    h_flex()
        .w_full()
        .min_h(px(46.))
        .px(px(16.))
        .py(px(10.))
        .gap(px(14.))
        .child(
            div()
                .flex_none()
                .text_size(px(15.))
                .text_color(flint_theme::text_dim())
                .child(label.into()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_right()
                .truncate()
                .text_size(px(15.))
                .text_color(flint_theme::text())
                .child(value.into()),
        )
}

pub fn note(text: impl Into<SharedString>, is_warning: bool) -> Div {
    div()
        .px(px(4.))
        .text_size(px(14.))
        .line_height(px(20.))
        .text_color(if is_warning {
            flint_theme::warning()
        } else {
            flint_theme::text_dim()
        })
        .child(text.into())
}

/// Sits near the top rather than centered, so the keyboard does not cover it.
pub fn dialog(content: impl IntoElement) -> AnyElement {
    let insets = insets::current();
    div()
        .id("dialog")
        .absolute()
        .inset_0()
        .occlude()
        .bg(hsla(0., 0., 0., 0.55))
        .flex()
        .flex_col()
        .items_center()
        .pt(px(insets.top + 48.))
        .px(px(12.))
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .w_full()
                .max_w(px(440.))
                .p(px(20.))
                .rounded(px(26.))
                .bg(phone_theme::ground())
                .border_1()
                .border_color(flint_theme::float_border())
                .child(content),
        )
        .into_any_element()
}

pub fn dialog_title(text: impl Into<SharedString>, is_warning: bool) -> impl IntoElement {
    title(text).when(is_warning, |title| {
        title.text_color(flint_theme::red_error())
    })
}

pub fn empty_state(title: impl Into<SharedString>, text: impl Into<SharedString>) -> Div {
    v_flex()
        .items_center()
        .gap(px(6.))
        .py(px(48.))
        .px(px(24.))
        .child(
            div()
                .text_size(px(17.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(flint_theme::text())
                .child(title.into()),
        )
        .child(
            div()
                .max_w(px(300.))
                .text_center()
                .text_size(px(15.))
                .line_height(px(21.))
                .text_color(flint_theme::text_dim())
                .child(text.into()),
        )
}

pub fn status_dot(color: Hsla, is_busy: bool) -> AnyElement {
    let dot = div().size(px(8.)).rounded_full().bg(color);
    if !is_busy {
        return dot.into_any_element();
    }
    dot.with_animation(
        "status-pulse",
        Animation::new(Duration::from_millis(1100))
            .repeat()
            .with_easing(pulsating_between(0.3, 1.0)),
        |dot, delta| dot.opacity(delta),
    )
    .into_any_element()
}

pub fn error_text(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(14.))
        .line_height(px(20.))
        .text_color(flint_theme::red_error())
        .child(text.into())
}

/// flint-core's errors start in lower case and have no full stop.
pub fn sentence(text: &str) -> String {
    let text = text.trim();
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let end = if text.ends_with(['.', '!', '?']) {
        ""
    } else {
        "."
    };
    format!("{}{}{end}", first.to_uppercase(), chars.as_str())
}

pub fn ago(seconds: u64) -> String {
    const DAY: u64 = 86_400;
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..DAY => format!("{} h ago", seconds / 3600),
        DAY..172_800 => "yesterday".into(),
        172_800..2_592_000 => format!("{} days ago", seconds / DAY),
        2_592_000..5_184_000 => "a month ago".into(),
        5_184_000..31_536_000 => format!("{} months ago", seconds / 2_592_000),
        _ => "over a year ago".into(),
    }
}

pub fn ago_since(unix_seconds: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    ago(now.saturating_sub(unix_seconds).max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_become_sentences() {
        assert_eq!(sentence("connection refused"), "Connection refused.");
        assert_eq!(sentence("Wrong password."), "Wrong password.");
        assert_eq!(sentence(""), "");
    }

    #[test]
    fn times_read_like_speech() {
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(125), "2 min ago");
        assert_eq!(ago(7300), "2 h ago");
        assert_eq!(ago(90_000), "yesterday");
        assert_eq!(ago(400_000), "4 days ago");
        assert_eq!(ago(3_000_000), "a month ago");
        assert_eq!(ago(16_000_000), "6 months ago");
        assert_eq!(ago(40_000_000), "over a year ago");
    }

    #[test]
    fn a_time_in_the_future_reads_as_now() {
        assert_eq!(ago_since(i64::MAX), "just now");
    }
}
