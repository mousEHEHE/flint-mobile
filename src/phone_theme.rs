//! Phone-only theme additions. `theme.rs` stays a copy of the desktop's, so changes go here.

use std::borrow::Cow;

use gpui::{hsla, App, Hsla, Window};
use gpui_component::theme::{Theme, ThemeTokens};

use crate::preferences::ThemePreference;
use crate::theme;

/// Addresses, paths, keys and the terminal use `MONO_FONT` instead.
pub const UI_FONT: &str = "Inter";
pub const MONO_FONT: &str = theme::MONO_FONT;

pub fn fonts() -> Vec<Cow<'static, [u8]>> {
    vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/CascadiaCode.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Bold.ttf").as_slice()),
    ]
}

pub fn apply_preference(preference: ThemePreference, window: Option<&mut Window>, cx: &mut App) {
    theme::apply_preference(preference, window, cx);
    let t = Theme::global_mut(cx);
    t.font_family = UI_FONT.into();
    t.background = ground();
    t.tokens = ThemeTokens::from(&t.colors);
}

pub fn ground() -> Hsla {
    theme::chrome()
}

/// Background of cards and grouped rows, on top of `ground`.
pub fn plate() -> Hsla {
    theme::modal_surface()
}

/// Translucent, so buttons and fields show on both `ground` and `plate`.
pub fn fill() -> Hsla {
    if theme::is_light() {
        hsla(0.62, 0.15, 0.13, 0.07)
    } else {
        hsla(0., 0., 1., 0.08)
    }
}

pub fn fill_pressed() -> Hsla {
    if theme::is_light() {
        hsla(0.62, 0.15, 0.13, 0.13)
    } else {
        hsla(0., 0., 1., 0.15)
    }
}

pub fn selected() -> Hsla {
    if theme::is_light() {
        gpui::white()
    } else {
        theme::element_hover()
    }
}

/// Background of the keys bar above the keyboard, close to the system keyboard's colour.
pub fn keys_ground() -> Hsla {
    if theme::is_light() {
        theme::surface_sunken()
    } else {
        theme::chrome()
    }
}

pub fn key_face() -> Hsla {
    theme::pop_surface()
}

pub fn key_edge() -> Hsla {
    if theme::is_light() {
        gpui::rgb(0xC3C9D4).into()
    } else {
        gpui::rgb(0x111216).into()
    }
}
