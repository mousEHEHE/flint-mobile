use std::time::Duration;

use gpui::prelude::*;
use gpui::{div, px, AnyElement, App, Entity, FontWeight, Global, SharedString};
use gpui_component::{h_flex, ActiveTheme as _};

use crate::insets;
use crate::theme as flint_theme;
use crate::widgets::icon;

const SHOWN_FOR: Duration = Duration::from_millis(2200);

/// Clears the hosts' search bar and the terminal's keys bar.
const BOTTOM_OFFSET: f32 = 84.;

pub struct Toast {
    message: Option<(SharedString, Kind)>,
    /// Bumped per toast, so the timer of an old toast does not hide a newer one.
    shown: u64,
}

#[derive(Clone, Copy)]
pub enum Kind {
    Done,
    Problem,
}

struct GlobalToast(Entity<Toast>);

impl Global for GlobalToast {}

pub fn install(cx: &mut App) {
    let toast = cx.new(|_| Toast {
        message: None,
        shown: 0,
    });
    cx.set_global(GlobalToast(toast));
}

pub fn entity(cx: &App) -> Entity<Toast> {
    cx.global::<GlobalToast>().0.clone()
}

pub fn show(message: impl Into<SharedString>, kind: Kind, cx: &mut App) {
    let message = message.into();
    entity(cx).update(cx, |toast, cx| {
        toast.shown += 1;
        toast.message = Some((message, kind));
        cx.notify();
        let shown = toast.shown;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SHOWN_FOR).await;
            this.update(cx, |toast, cx| {
                if toast.shown == shown {
                    toast.message = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    });
}

pub fn render(cx: &App) -> Option<AnyElement> {
    let (message, kind) = entity(cx).read(cx).message.clone()?;
    let insets = insets::current();
    let (name, color) = match kind {
        Kind::Done => ("circle-check", flint_theme::status_online()),
        Kind::Problem => ("circle-alert", flint_theme::red_error()),
    };
    Some(
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(insets.bottom + BOTTOM_OFFSET))
            .flex()
            .justify_center()
            .px(px(24.))
            .child(
                // `min_w_0` here and on the text lets a long message wrap, not run off the screen.
                h_flex()
                    .min_w_0()
                    .max_w(px(420.))
                    .gap(px(10.))
                    .pl(px(14.))
                    .pr(px(18.))
                    .py(px(12.))
                    .rounded(px(14.))
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(flint_theme::float_border())
                    .shadow_md()
                    .child(div().flex_none().child(icon(name, 18., color)))
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(flint_theme::text())
                            .child(message),
                    ),
            )
            .into_any_element(),
    )
}
