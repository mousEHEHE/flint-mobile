//! The icons are built into the binary: a phone has no folder to read them from.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((
            concat!("icons/", $name, ".svg"),
            include_bytes!(concat!("../assets/icons/", $name, ".svg")),
        )),*]
    };
}

/// Lucide icons (`assets/icons/LUCIDE-LICENSE.txt`) and the distro marks of flint-gpui.
const ICONS: &[(&str, &[u8])] = icons![
    "arrow-up-down",
    "at-sign",
    "braces",
    "chevrons-up-down",
    "clipboard-copy",
    "clipboard-paste",
    "clock",
    "cloud",
    "cloud-off",
    "corner-down-left",
    "corner-left-up",
    "download",
    "file-archive",
    "file-code",
    "file-image",
    "file-key",
    "file-music",
    "file-terminal",
    "fingerprint",
    "folder-input",
    "folder-plus",
    "hash",
    "house",
    "key-round",
    "keyboard",
    "keyboard-off",
    "layers",
    "link",
    "lock",
    "log-in",
    "log-out",
    "monitor",
    "pen-line",
    "route",
    "server",
    "share",
    "shield-alert",
    "shield-check",
    "smartphone",
    "sun-moon",
    "terminal",
    "trash",
    "type",
    "unlink",
    "unplug",
    "upload",
    "users",
    "waypoints",
    "wifi-off",
    "x",
    "distro/almalinux",
    "distro/alpinelinux",
    "distro/apple",
    "distro/archlinux",
    "distro/centos",
    "distro/debian",
    "distro/fedora",
    "distro/freebsd",
    "distro/gentoo",
    "distro/kalilinux",
    "distro/linux",
    "distro/linuxmint",
    "distro/manjaro",
    "distro/nixos",
    "distro/opensuse",
    "distro/oracle",
    "distro/raspberrypi",
    "distro/redhat",
    "distro/rockylinux",
    "distro/ubuntu",
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = ICONS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths: Vec<SharedString> = ICONS
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| name.starts_with(path))
            .map(Into::into)
            .collect();
        paths.extend(gpui_kit_assets::Assets.list(path)?);
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apps_icons_and_the_components_load() {
        assert!(Assets.load("icons/server.svg").unwrap().is_some());
        assert!(Assets.load("icons/distro/ubuntu.svg").unwrap().is_some());
        // gpui-component's search field draws this one.
        assert!(Assets.load("icons/circle-x.svg").unwrap().is_some());
        // gpui-kit-assets answers an unknown path with an error.
        assert!(Assets
            .load("icons/no-such-icon.svg")
            .ok()
            .flatten()
            .is_none());
    }

    #[test]
    fn every_icon_file_is_built_in() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let mut files = Vec::new();
        for dir in ["icons", "icons/distro"] {
            for entry in std::fs::read_dir(root.join(dir)).unwrap() {
                let name = entry.unwrap().file_name().into_string().unwrap();
                if name.ends_with(".svg") {
                    files.push(format!("{dir}/{name}"));
                }
            }
        }
        for file in files {
            assert!(
                ICONS.iter().any(|(name, _)| *name == file),
                "{file} is not in ICONS"
            );
        }
    }
}
