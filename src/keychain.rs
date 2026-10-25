//! iOS keeps the vault key for this device only: a backup restored to another phone asks for
//! the master password again.

use zeroize::Zeroizing;

pub type DeviceKey = Zeroizing<[u8; 32]>;

#[cfg(target_os = "ios")]
mod platform {
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::passwords::{
        delete_generic_password, generic_password, set_generic_password_options, PasswordOptions,
    };
    use zeroize::Zeroizing;

    const SERVICE: &str = "flint vault";
    const ACCOUNT: &str = "device key";

    pub fn save(key: &[u8; 32]) -> Result<(), String> {
        // Delete first: an update would keep the old item's access settings.
        let _ = delete_generic_password(SERVICE, ACCOUNT);
        let access = SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
            0,
        )
        .map_err(|e| e.to_string())?;
        let mut options = PasswordOptions::new_generic_password(SERVICE, ACCOUNT);
        options.set_access_control(access);
        set_generic_password_options(key, options).map_err(|e| e.to_string())
    }

    pub fn load() -> Option<Zeroizing<Vec<u8>>> {
        generic_password(PasswordOptions::new_generic_password(SERVICE, ACCOUNT))
            .ok()
            .map(Zeroizing::new)
    }

    pub fn delete() {
        let _ = delete_generic_password(SERVICE, ACCOUNT);
    }
}

/// Android and the preview keep the key in a file in the app's folder. TODO: Android Keystore.
#[cfg(not(target_os = "ios"))]
mod platform {
    use std::path::PathBuf;

    use zeroize::Zeroizing;

    fn path() -> Option<PathBuf> {
        Some(flint_core::vault::app_data_dir()?.join("device.key"))
    }

    pub fn save(key: &[u8; 32]) -> Result<(), String> {
        let path = path().ok_or("no data folder")?;
        write_private(&path, key).map_err(|e| e.to_string())
    }

    #[cfg(unix)]
    fn write_private(path: &std::path::Path, key: &[u8]) -> std::io::Result<()> {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(key)?;
        file.sync_all()
    }

    #[cfg(not(unix))]
    fn write_private(path: &std::path::Path, key: &[u8]) -> std::io::Result<()> {
        std::fs::write(path, key)
    }

    pub fn load() -> Option<Zeroizing<Vec<u8>>> {
        std::fs::read(path()?).ok().map(Zeroizing::new)
    }

    pub fn delete() {
        if let Some(path) = path() {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn save(key: &DeviceKey) {
    if let Err(e) = platform::save(key) {
        // The vault still opens with the password next time.
        tracing::error!(error = %e, "could not keep the vault key on this device");
    }
}

pub fn load() -> Option<DeviceKey> {
    let bytes = platform::load()?;
    let key: [u8; 32] = bytes.as_slice().try_into().ok()?;
    Some(Zeroizing::new(key))
}

pub fn delete() {
    platform::delete();
}
