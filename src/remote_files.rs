//! Server paths are always `/`-separated, whatever the phone uses.

use flint_core::ssh::{EntryKind, FileEntry};

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn parent(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return None;
    }
    match path.rfind('/') {
        Some(0) => Some("/".into()),
        Some(end) => Some(path[..end].to_string()),
        // A relative path from the server: its parent is unknown.
        None => None,
    }
}

pub fn name_of(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/";
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// Writes the home folder as `~`, unless home is `/`.
pub fn display_path(path: &str, home: &str) -> String {
    if home.len() > 1 && path == home {
        return "~".into();
    }
    match path.strip_prefix(home) {
        Some(rest) if home.len() > 1 && rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && name != "." && name != ".."
}

pub fn free_name(name: &str, is_taken: impl Fn(&str) -> bool) -> String {
    if !is_taken(name) {
        return name.to_string();
    }
    let (stem, extension) = name.split_at(stem_len(name));
    (2..)
        .map(|n| format!("{stem} ({n}){extension}"))
        .find(|candidate| !is_taken(candidate))
        .expect("some number is free")
}

/// A leading dot does not start an extension: the stem of `.bashrc` is all of it.
pub fn stem_len(name: &str) -> usize {
    match name.rfind('.') {
        Some(dot) if dot > 0 => dot,
        _ => name.len(),
    }
}

pub fn fmt_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{} KB", bytes / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn mode_string(mode: u32) -> String {
    let mut text = String::with_capacity(9);
    for shift in [6u32, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        text.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        text.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        text.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
    text
}

pub const EDIT_MAX_BYTES: u64 = 1024 * 1024;

/// Only a guess: whether the file is really text is known once it is read.
pub fn can_edit(entry: &FileEntry) -> bool {
    let kind = Kind::of(entry);
    !matches!(
        kind,
        Kind::Folder | Kind::BrokenLink | Kind::Image | Kind::Archive | Kind::Media
    ) && entry.kind != EntryKind::Other
        && entry.size <= EDIT_MAX_BYTES
}

pub fn can_move_into(from: &str, folder: &str) -> bool {
    let from = from.trim_end_matches('/');
    let folder = folder.trim_end_matches('/');
    let is_inside_itself = folder == from || folder.starts_with(&format!("{from}/"));
    let is_already_there =
        parent(from).is_some_and(|parent| parent.trim_end_matches('/') == folder);
    !is_inside_itself && !is_already_there
}

pub fn sort(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        b.opens_as_folder()
            .cmp(&a.opens_as_folder())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

pub fn is_hidden(entry: &FileEntry) -> bool {
    entry.name.starts_with('.')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Link,
    BrokenLink,
    Image,
    Archive,
    Code,
    Text,
    Media,
    Program,
    Key,
    Other,
}

impl Kind {
    pub fn of(entry: &FileEntry) -> Kind {
        if entry.is_link_broken {
            return Kind::BrokenLink;
        }
        if entry.opens_as_folder() {
            return Kind::Folder;
        }
        if entry.kind == EntryKind::Symlink {
            return Kind::Link;
        }
        let name = entry.name.to_lowercase();
        let extension = name.rsplit_once('.').map_or("", |(_, extension)| extension);
        if matches!(
            name.as_str(),
            "dockerfile"
                | "makefile"
                | "justfile"
                | "vagrantfile"
                | ".bashrc"
                | ".zshrc"
                | ".profile"
                | ".bash_profile"
                | ".gitconfig"
                | ".vimrc"
                | "crontab"
        ) {
            return Kind::Code;
        }
        if name.starts_with("id_") || name == "authorized_keys" || name == "known_hosts" {
            return Kind::Key;
        }
        match extension {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg" | "ico"
            | "heic" => Kind::Image,
            "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "deb" | "rpm" => {
                Kind::Archive
            }
            "rs" | "py" | "js" | "ts" | "go" | "c" | "h" | "cpp" | "hpp" | "sh" | "bash"
            | "zsh" | "fish" | "json" | "yaml" | "yml" | "toml" | "ini" | "conf" | "cfg"
            | "xml" | "html" | "css" | "sql" | "lua" | "rb" | "php" | "java" | "kt" | "swift"
            | "env" | "service" | "nginx" | "tf" => Kind::Code,
            "txt" | "md" | "log" | "csv" | "tsv" | "rst" | "out" => Kind::Text,
            "mp3" | "flac" | "wav" | "ogg" | "m4a" | "mp4" | "mkv" | "mov" | "avi" | "webm" => {
                Kind::Media
            }
            "pem" | "key" | "pub" | "crt" | "cer" | "p12" => Kind::Key,
            _ if entry.permissions.is_some_and(|mode| mode & 0o111 != 0) => Kind::Program,
            _ => Kind::Other,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Folder => "folder",
            Kind::Link => "link",
            Kind::BrokenLink => "unlink",
            Kind::Image => "file-image",
            Kind::Archive => "file-archive",
            Kind::Code => "file-code",
            Kind::Text => "file-text",
            Kind::Media => "file-music",
            Kind::Program => "file-terminal",
            Kind::Key => "file-key",
            Kind::Other => "file",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: name.into(),
            is_dir,
            size: 0,
            mtime: 0,
            permissions: Some(0o644),
            kind: if is_dir {
                EntryKind::Folder
            } else {
                EntryKind::File
            },
            link_target: None,
            is_link_to_dir: false,
            is_link_broken: false,
        }
    }

    #[test]
    fn paths_join_and_go_up() {
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(join("/home/me", "notes.txt"), "/home/me/notes.txt");
        assert_eq!(parent("/home/me/notes.txt").as_deref(), Some("/home/me"));
        assert_eq!(parent("/etc").as_deref(), Some("/"));
        assert_eq!(parent("/home/me/").as_deref(), Some("/home"));
        assert_eq!(parent("/"), None);
        assert_eq!(name_of("/var/log"), "log");
        assert_eq!(name_of("/"), "/");
    }

    #[test]
    fn the_home_folder_reads_as_a_tilde() {
        assert_eq!(display_path("/home/me", "/home/me"), "~");
        assert_eq!(display_path("/home/me/app", "/home/me"), "~/app");
        assert_eq!(display_path("/home/meg", "/home/me"), "/home/meg");
        assert_eq!(display_path("/etc", "/"), "/etc");
    }

    #[test]
    fn an_upload_never_takes_a_name_that_is_there() {
        let taken = ["report.pdf", "report (2).pdf", ".bashrc", "notes"];
        let is_taken = |name: &str| taken.contains(&name);
        assert_eq!(free_name("photo.jpg", is_taken), "photo.jpg");
        assert_eq!(free_name("report.pdf", is_taken), "report (3).pdf");
        assert_eq!(free_name(".bashrc", is_taken), ".bashrc (2)");
        assert_eq!(free_name("notes", is_taken), "notes (2)");
    }

    #[test]
    fn the_extension_is_not_part_of_the_stem() {
        assert_eq!(stem_len("notes.md"), 5);
        assert_eq!(stem_len("backup.tar.gz"), 10);
        assert_eq!(stem_len(".bashrc"), 7);
        assert_eq!(stem_len("Makefile"), 8);
        assert_eq!(stem_len(""), 0);
    }

    #[test]
    fn names_that_would_leave_the_folder_are_refused() {
        assert!(is_valid_name("app.log"));
        for name in ["", ".", "..", "a/b"] {
            assert!(!is_valid_name(name), "{name:?}");
        }
    }

    #[test]
    fn sizes_and_permissions_read_like_ls() {
        assert_eq!(fmt_size(512), "512 B");
        assert_eq!(fmt_size(4096), "4 KB");
        assert_eq!(fmt_size(5 * 1024 * 1024 + 1), "5.0 MB");
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o600), "rw-------");
    }

    #[test]
    fn text_and_unknown_files_can_be_edited_but_not_images_or_big_files() {
        assert!(can_edit(&entry("nginx.conf", false)));
        assert!(can_edit(&entry("hosts", false)));
        assert!(!can_edit(&entry("logs", true)));
        assert!(!can_edit(&entry("photo.jpg", false)));
        assert!(!can_edit(&entry("backup.tar.gz", false)));
        let mut big = entry("huge.log", false);
        big.size = EDIT_MAX_BYTES + 1;
        assert!(!can_edit(&big));
    }

    #[test]
    fn a_move_needs_another_folder_outside_the_thing_moved() {
        assert!(can_move_into("/home/me/notes.txt", "/tmp"));
        assert!(can_move_into("/home/me/app", "/home/me/backup"));
        assert!(can_move_into("/home/me/app", "/"));
        assert!(!can_move_into("/home/me/notes.txt", "/home/me"));
        assert!(!can_move_into("/home/me/app", "/home/me/app"));
        assert!(!can_move_into("/home/me/app", "/home/me/app/logs"));
        assert!(can_move_into("/home/me/app", "/home/me/apples"));
        assert!(!can_move_into("/etc", "/"));
    }

    #[test]
    fn folders_come_first_then_names_without_case() {
        let mut entries = vec![
            entry("b.txt", false),
            entry("Zeta", true),
            entry("a.txt", false),
            entry("alpha", true),
        ];
        sort(&mut entries);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Zeta", "a.txt", "b.txt"]);
    }

    #[test]
    fn files_get_the_icon_of_what_they_are() {
        assert_eq!(Kind::of(&entry("logs", true)), Kind::Folder);
        assert_eq!(Kind::of(&entry("photo.JPG", false)), Kind::Image);
        assert_eq!(Kind::of(&entry("nginx.conf", false)), Kind::Code);
        assert_eq!(Kind::of(&entry("Dockerfile", false)), Kind::Code);
        assert_eq!(Kind::of(&entry("app.log", false)), Kind::Text);
        assert_eq!(Kind::of(&entry("backup.tar.gz", false)), Kind::Archive);
        assert_eq!(Kind::of(&entry("id_ed25519", false)), Kind::Key);
        let mut program = entry("deploy", false);
        program.permissions = Some(0o755);
        assert_eq!(Kind::of(&program), Kind::Program);
        assert_eq!(Kind::of(&entry("data.bin", false)), Kind::Other);
    }
}
