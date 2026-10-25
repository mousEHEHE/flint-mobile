//! Recent quick connect hosts. Passwords are never stored here.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 6;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentHost {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// "SHA256:...", checked silently on the next connect.
    #[serde(default)]
    pub host_key: Option<String>,
}

impl RecentHost {
    pub fn address(&self) -> String {
        if self.port == 22 {
            format!("{}@{}", self.username, self.host)
        } else {
            format!("{}@{}:{}", self.username, self.host, self.port)
        }
    }

    fn is_same_host(&self, other: &RecentHost) -> bool {
        self.host == other.host && self.port == other.port && self.username == other.username
    }
}

pub fn load() -> Vec<RecentHost> {
    let Some(path) = path() else {
        return Vec::new();
    };
    let Ok(data) = std::fs::read(&path) else {
        return Vec::new();
    };
    serde_json::from_slice(&data).unwrap_or_else(|e| {
        tracing::warn!(?e, "invalid recent hosts file, starting empty");
        Vec::new()
    })
}

/// Puts `host` first, keeping a host key trusted for it earlier.
pub fn remember(host: RecentHost) {
    let mut list = load();
    let known_key = list
        .iter()
        .find(|known| known.is_same_host(&host))
        .and_then(|known| known.host_key.clone());
    list.retain(|known| !known.is_same_host(&host));
    list.insert(
        0,
        RecentHost {
            host_key: host.host_key.or(known_key),
            ..host
        },
    );
    list.truncate(MAX_RECENT);
    save(&list);
}

pub fn forget(host: &RecentHost) {
    let mut list = load();
    list.retain(|known| !known.is_same_host(host));
    save(&list);
}

fn save(list: &[RecentHost]) {
    let Some(path) = path() else {
        return;
    };
    let result = serde_json::to_vec_pretty(list)
        .map_err(std::io::Error::other)
        .and_then(|json| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, json)
        });
    if let Err(e) = result {
        tracing::warn!(?e, path = %path.display(), "could not save recent hosts");
    }
}

fn path() -> Option<PathBuf> {
    Some(flint_core::vault::app_data_dir()?.join("recent_hosts.json"))
}

#[cfg(test)]
mod tests {
    use super::RecentHost;

    #[test]
    fn the_address_names_the_port_only_when_it_is_not_22() {
        let mut host = RecentHost {
            host: "example.com".into(),
            port: 22,
            username: "root".into(),
            host_key: None,
        };
        assert_eq!(host.address(), "root@example.com");
        host.port = 2222;
        assert_eq!(host.address(), "root@example.com:2222");
    }
}
