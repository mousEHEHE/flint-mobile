//! Safe-area insets; `bottom` also covers the keyboard while it is up.

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Insets {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
    pub is_keyboard_visible: bool,
}

#[cfg(target_os = "ios")]
pub fn current() -> Insets {
    let (top, bottom, left, right) = gpui_mobile::safe_area_insets();
    let keyboard = gpui_mobile::keyboard_height();
    Insets {
        top,
        bottom: bottom.max(keyboard),
        left,
        right,
        is_keyboard_visible: keyboard > 0.0,
    }
}

#[cfg(target_os = "android")]
pub fn current() -> Insets {
    /// The navigation bar alone is far below this, so a larger bottom inset is the keyboard.
    const KEYBOARD_MIN_HEIGHT: f32 = 120.0;

    let window = gpui_mobile::android::jni::platform().and_then(|p| p.primary_window());
    let Some(window) = window else {
        return Insets::default();
    };
    // On Android the bottom inset already includes the keyboard.
    let insets = window.safe_area_insets_logical();
    Insets {
        top: insets.top,
        bottom: insets.bottom,
        left: insets.left,
        right: insets.right,
        is_keyboard_visible: insets.bottom > KEYBOARD_MIN_HEIGHT,
    }
}

/// The desktop preview uses an iPhone's insets, with the keyboard hidden.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn current() -> Insets {
    Insets {
        top: 59.0,
        bottom: 34.0,
        ..Insets::default()
    }
}
