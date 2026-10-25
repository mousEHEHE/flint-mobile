//! On Android the status and navigation bars also take the background colour.

use gpui::{App, Hsla, Rgba};
use gpui_component::theme::Theme;
use gpui_component::ActiveTheme as _;
use gpui_mobile::{StatusBarContentStyle, SystemChromeStyle};

pub fn follow_theme(cx: &mut App) {
    match_theme(cx);
    cx.observe_global::<Theme>(match_theme).detach();
}

// gpui-mobile's view controller defaults to dark content, and iOS then ignores
// UIStatusBarStyle in Info.plist, so the style has to be set from code.
fn match_theme(cx: &mut App) {
    let content = if crate::theme::is_light() {
        StatusBarContentStyle::Dark
    } else {
        StatusBarContentStyle::Light
    };
    let background = Some(to_rgb(cx.theme().background));
    gpui_mobile::set_system_chrome(&SystemChromeStyle {
        status_bar_color: background,
        status_bar_style: content,
        navigation_bar_color: background,
    });
}

/// 0xRRGGBB, the form gpui-mobile takes colours in.
fn to_rgb(color: Hsla) -> u32 {
    let color = Rgba::from(color);
    let channel = |value: f32| (value * 255.0).round() as u32;
    (channel(color.r) << 16) | (channel(color.g) << 8) | channel(color.b)
}
