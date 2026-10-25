// Links gpui-mobile's FFI and JNI symbols into the library the host loads.
extern crate gpui_mobile;

mod account_view;
mod app;
mod assets;
mod connect_view;
mod file_editor;
mod file_preview;
mod files_view;
mod host_form;
mod hosts_view;
mod insets;
#[cfg(target_os = "ios")]
mod ios_log;
mod keychain;
mod keys_bar;
mod phone_theme;
// preferences and theme are copies of flint-gpui's, with helpers a phone does not use.
#[allow(dead_code)]
mod preferences;
mod recent;
mod remote_files;
mod session_view;
mod settings_view;
mod share;
mod status_bar;
mod store;
#[allow(dead_code)]
mod theme;
mod toast;
mod ui;
mod web_auth;
mod widgets;

// The terminal files shared with flint-gpui use it as `crate::shell`.
use flint_core::shell;

use gpui::{App, AppContext as _, WindowOptions};

pub use assets::Assets;
use preferences::Preferences;

pub fn start(cx: &mut App) {
    flint_core::app::set_version(env!("CARGO_PKG_VERSION"));
    set_data_dir();

    if let Err(e) = cx.text_system().add_fonts(phone_theme::fonts()) {
        log::error!("could not load the fonts: {e:#}");
    }
    gpui_component::init(cx);

    let prefs = Preferences::load_or_default();
    let theme_preference = prefs.theme;
    cx.set_global(prefs);
    phone_theme::apply_preference(theme_preference, None, cx);
    status_bar::follow_theme(cx);
    toast::install(cx);
    share::clear_scratch();
    store::Store::install(cx);

    let opened = cx.open_window(
        WindowOptions {
            window_bounds: preview_bounds(cx),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| app::AppView::new(window, cx));
            cx.new(|cx| gpui_component::Root::new(view, window, cx))
        },
    );
    if let Err(e) = opened {
        log::error!("could not open the window: {e:#}");
        return;
    }
    cx.activate(true);
}

/// A phone fills its screen; the preview on a computer gets an iPhone's size.
#[cfg(any(target_os = "ios", target_os = "android"))]
fn preview_bounds(_cx: &App) -> Option<gpui::WindowBounds> {
    None
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn preview_bounds(cx: &App) -> Option<gpui::WindowBounds> {
    let size = gpui::size(gpui::px(393.), gpui::px(852.));
    Some(gpui::WindowBounds::Windowed(gpui::Bounds::centered(
        None, size, cx,
    )))
}

fn set_data_dir() {
    match data_dir() {
        Ok(dir) => {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                log::error!("could not create {}: {e}", dir.display());
            }
            flint_core::app::set_data_dir(dir);
        }
        Err(e) => log::error!("the platform gave no data folder: {e}"),
    }
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn data_dir() -> Result<std::path::PathBuf, String> {
    gpui_mobile::packages::path_provider::support_directory()
}

/// The preview keeps its own folder, so it never touches the desktop app's vault.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn data_dir() -> Result<std::path::PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(std::path::PathBuf::from(home).join(".flint-mobile-preview"))
}

/// Called by `android-activity` on its own thread.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: android_activity::AndroidApp) {
    use gpui_mobile::android::jni;

    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("flint"),
    );
    jni::install_panic_hook();

    let _platform = jni::init_platform(&app);
    let Some(platform) = jni::shared_platform() else {
        log::error!("android_main: gpui-mobile has no platform");
        return;
    };
    // `start` runs only once the native window exists.
    gpui::Application::with_platform(platform.into_rc())
        .with_assets(Assets)
        .run(start);
}

/// Called from `main.m` before `gpui_ios_run_demo`.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_register_app() {
    ios_log::install();
    gpui_mobile::ios::ffi::set_assets(Assets);
    gpui_mobile::ios::ffi::set_app_callback(Box::new(start));
}
