//! The phone UI in a phone-sized window on the computer: `cargo run --example preview`.

fn main() {
    env_logger::init();
    let platform = gpui_platform::current_platform(false);
    gpui::Application::with_platform(platform)
        .with_assets(flint_mobile::Assets)
        .run(flint_mobile::start);
}
