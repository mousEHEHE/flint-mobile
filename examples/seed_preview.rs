//! Replaces the preview's vault with sample hosts, so `preview` opens on the hosts list.
//! Usage: `cargo run --example seed_preview [-- demo-password [key.pem]]`. The demo password
//! is for `demo@127.0.0.1:2222`, a local sshd for trying sessions out.

use secrecy::SecretString;
use uuid::Uuid;

use flint_core::vault::{AuthMethod, ProxyChoice, ServerEntry, SnippetBody, VaultManager};

fn main() {
    let home = std::env::var_os("HOME").expect("HOME is set");
    let dir = std::path::PathBuf::from(home).join(".flint-mobile-preview");
    std::fs::create_dir_all(&dir).expect("the preview folder can be made");
    flint_core::app::set_data_dir(dir.clone());

    let path = dir.join("vault.bin");
    flint_core::vault::delete_vault_files(&path).expect("the old vault goes");
    let (mut vault, _) = VaultManager::create(&path, &SecretString::new("preview".into()))
        .expect("the vault is made");

    let mut args = std::env::args().skip(1);
    let demo_password = args.next().unwrap_or_else(|| "flint-demo".into());
    let key_id = args.next().map(|key_path| {
        let pem = std::fs::read_to_string(key_path).expect("the key file reads");
        vault
            .add_key("demo key".into(), pem, None)
            .expect("the key goes in")
    });

    let berlin = vault.find_or_create_group("Berlin", None).unwrap();
    let home_lab = vault.find_or_create_group("Home lab", None).unwrap();
    let prod = vault.find_or_create_tag("prod").unwrap();
    let vpn = vault.find_or_create_tag("vpn").unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs() as i64;
    // Name, address, port, user, folder, last opened, system.
    let hosts = [
        ("demo", "127.0.0.1", 2222, "demo", None, Some(now), None),
        (
            "web-1",
            "10.0.1.11",
            22,
            "deploy",
            Some(berlin),
            Some(now - 3600),
            Some("Ubuntu 24.04"),
        ),
        (
            "web-2",
            "10.0.1.12",
            22,
            "deploy",
            Some(berlin),
            None,
            Some("Ubuntu 22.04"),
        ),
        (
            "db",
            "10.0.1.20",
            5022,
            "postgres",
            Some(berlin),
            Some(now - 86_400),
            Some("Debian GNU/Linux 12"),
        ),
        (
            "nas",
            "192.168.1.5",
            22,
            "admin",
            Some(home_lab),
            None,
            Some("FreeBSD 14.1-RELEASE"),
        ),
        (
            "pi-hole",
            "192.168.1.8",
            22,
            "pi",
            Some(home_lab),
            Some(now - 7200),
            Some("Raspbian GNU/Linux 11"),
        ),
        (
            "",
            "build.example.com",
            22,
            "ci",
            None,
            None,
            Some("Alpine Linux 3.20.3"),
        ),
    ];
    for (label, host, port, username, group, opened, os) in hosts {
        let tags = match label {
            "web-1" | "web-2" | "db" => vec![prod],
            "pi-hole" => vec![vpn],
            _ => Vec::new(),
        };
        let auth = match (label, key_id) {
            ("demo", _) => AuthMethod::Password(demo_password.clone()),
            (_, Some(key_id)) => AuthMethod::PrivateKey { key_id },
            _ => AuthMethod::Password("secret".into()),
        };
        vault
            .add_entry(ServerEntry {
                id: Uuid::new_v4(),
                label: label.into(),
                host: host.into(),
                port,
                username: username.into(),
                auth,
                notes: None,
                os: os.map(Into::into),
                last_opened_at: opened,
                host_key_fingerprint: None,
                last_remote_dir: None,
                last_local_dir: None,
                group,
                tags,
                proxy: ProxyChoice::Default,
                jump_host: None,
                forwards: Vec::new(),
                startup_snippet: None,
                snippets: Vec::new(),
                created_at: now,
                updated_at: now,
            })
            .expect("the host goes in");
    }

    for (name, content) in [
        ("Disk space", "df -h"),
        ("Who is logged in", "w"),
        ("Restart nginx", "sudo systemctl restart nginx"),
        ("Last errors", "journalctl -p err -n 30 --no-pager"),
    ] {
        vault
            .add_snippet(
                name.into(),
                None,
                SnippetBody::Script {
                    content: content.into(),
                },
            )
            .expect("the snippet goes in");
    }

    let key = vault.device_key();
    std::fs::write(dir.join("device.key"), key.as_slice()).expect("the device key is written");
    println!("seeded {}", path.display());
}
