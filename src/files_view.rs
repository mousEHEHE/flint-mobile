use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, AnyElement, ClipboardItem, Context, Entity, EventEmitter, FontWeight,
    Hsla, PathPromptOptions, SharedString, Subscription, Task, Window,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{h_flex, v_flex, ActiveTheme as _};
use gpui_mobile::KeyboardType;
use uuid::Uuid;

use flint_core::ssh::{
    CancelFlag, FileEntry, SftpEvent, SftpHandle, SshConnection, SshError, TransferOutcome,
};

use crate::file_editor::{EditorEvent, FileEditorView};
use crate::file_preview::{preview_plan, PreviewContent, PreviewEvent, PreviewPlan, PreviewView};
use crate::phone_theme;
use crate::remote_files::{self, fmt_size, Kind};
use crate::share;
use crate::store::Store;
use crate::theme as flint_theme;
use crate::toast;
use crate::widgets::{
    ago_since, body_text, card_inset, danger_button, dialog, dialog_title, empty_state, error_text,
    field_input, icon, info_row, plain_icon_button, primary_button, quiet_button, row_code,
    row_subtitle, row_title, secondary_button, sentence, sheet, sheet_action, spinner, text_button,
    title, GUTTER,
};

/// `uniform_list` needs every row to have the same height.
const ROW_HEIGHT: f32 = 60.;

pub enum FilesEvent {
    OpenInTerminal(String),
    Reconnect,
}

enum Phase {
    Opening,
    Ready,
    Failed(SharedString),
    Lost,
}

enum FilesSheet {
    Entry(String),
    Folder,
    Add,
    ConfirmDelete(String),
}

enum DialogKind {
    NewFolder,
    Rename(String),
}

struct NameDialog {
    kind: DialogKind,
    /// `None` until the next render, since `InputState` needs the window.
    input: Option<Entity<InputState>>,
    error: Option<SharedString>,
    _subscription: Option<Subscription>,
}

struct MoveRequest {
    name: String,
    from: String,
}

struct PermissionsDialog {
    name: String,
    path: String,
    mode: u32,
}

#[derive(Clone, Copy, PartialEq)]
enum Direction {
    Up,
    Down,
}

struct Transfer {
    id: u64,
    name: String,
    direction: Direction,
    done: u64,
    total: u64,
    cancel: CancelFlag,
    /// Download: where the file is saved. Upload: the source file.
    local: PathBuf,
    folder: String,
    /// The original name, when an upload was renamed so it does not replace a file.
    renamed_from: Option<String>,
}

pub struct FilesView {
    host_id: Option<Uuid>,
    /// Kept to retry when SFTP fails to open.
    connection: Option<SshConnection>,
    sftp: Option<SftpHandle>,
    home: String,
    /// The folder last opened on this host.
    start: Option<String>,
    path: String,
    loading: Option<String>,
    entries: Vec<FileEntry>,
    phase: Phase,
    show_hidden: bool,
    transfers: Vec<Transfer>,
    next_transfer: u64,
    sheet: Option<FilesSheet>,
    dialog: Option<NameDialog>,
    moving: Option<MoveRequest>,
    permissions: Option<PermissionsDialog>,
    preview: Option<(Entity<PreviewView>, Subscription)>,
    editor: Option<(Entity<FileEditorView>, Subscription)>,
    _open: Option<Task<()>>,
    _events: Option<Task<()>>,
}

impl EventEmitter<FilesEvent> for FilesView {}

impl FilesView {
    pub fn new(connection: SshConnection, host_id: Option<Uuid>, cx: &mut Context<Self>) -> Self {
        let start = host_id.and_then(|id| {
            Store::global(cx)
                .read(cx)
                .vault()?
                .get_entry(id)
                .ok()?
                .last_remote_dir
        });
        let mut view = Self {
            host_id,
            connection: None,
            sftp: None,
            home: String::new(),
            start,
            path: String::new(),
            loading: None,
            entries: Vec::new(),
            phase: Phase::Opening,
            show_hidden: false,
            transfers: Vec::new(),
            next_transfer: 1,
            sheet: None,
            dialog: None,
            moving: None,
            permissions: None,
            preview: None,
            editor: None,
            _open: None,
            _events: None,
        };
        view.attach(connection, cx);
        view
    }

    pub fn attach(&mut self, connection: SshConnection, cx: &mut Context<Self>) {
        self.connection = Some(connection.clone());
        self.sftp = None;
        self._events = None;
        if !matches!(self.phase, Phase::Ready) {
            self.phase = Phase::Opening;
        }
        self._open = Some(cx.spawn(async move |this, cx| {
            let opened = connection.open_sftp().await;
            this.update(cx, |view, cx| view.on_opened(opened, cx)).ok();
        }));
        cx.notify();
    }

    pub fn on_lost(&mut self, cx: &mut Context<Self>) {
        self.sftp = None;
        self._events = None;
        self.loading = None;
        // A transfer cannot outlive its channel.
        for transfer in self.transfers.drain(..) {
            if transfer.direction == Direction::Down {
                remove_download(&transfer.local);
            }
        }
        self.phase = Phase::Lost;
        self.sync_preview(cx);
        cx.notify();
    }

    fn on_opened(
        &mut self,
        opened: Result<(SftpHandle, String), SshError>,
        cx: &mut Context<Self>,
    ) {
        let (sftp, home) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                tracing::warn!(error = %e, "SFTP did not open");
                let reason = match e {
                    SshError::SftpInit(_) => {
                        "The server did not start file access (SFTP). It may be turned off \
                         there."
                            .to_string()
                    }
                    e => sentence(&e.to_string()),
                };
                self.phase = Phase::Failed(reason.into());
                cx.notify();
                return;
            }
        };
        self.home = home.clone();
        let events = sftp.subscribe();
        self._events = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |view, cx| view.on_event(event, cx))
                    .is_err()
                {
                    return;
                }
            }
            this.update(cx, |view, cx| view.on_lost(cx)).ok();
        }));
        self.sftp = Some(sftp);
        let folder = if self.path.is_empty() {
            self.start.clone().unwrap_or(home)
        } else {
            self.path.clone()
        };
        self.go(folder, cx);
    }

    fn go(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(sftp) = &self.sftp else { return };
        sftp.list_dir(path.clone());
        self.loading = Some(path);
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.path.is_empty() && self.loading.is_none() {
            self.go(self.path.clone(), cx);
        }
    }

    fn on_event(&mut self, event: SftpEvent, cx: &mut Context<Self>) {
        match event {
            SftpEvent::DirListed { path, mut entries } => {
                let is_wanted = match &self.loading {
                    Some(loading) => *loading == path,
                    None => path == self.path,
                };
                if !is_wanted {
                    return;
                }
                self.loading = None;
                let is_new_folder = path != self.path;
                remote_files::sort(&mut entries);
                self.entries = entries;
                self.path = path;
                self.phase = Phase::Ready;
                if is_new_folder {
                    self.remember_folder(cx);
                }
                cx.notify();
            }
            SftpEvent::ListFailed {
                path,
                error,
                is_background,
            } => {
                if is_background || self.loading.as_deref() != Some(path.as_str()) {
                    return;
                }
                self.loading = None;
                tracing::warn!(%path, %error, "listing failed");
                let name = remote_files::name_of(&path).to_string();
                if matches!(self.phase, Phase::Ready) {
                    toast::show(
                        format!("{name} could not be opened"),
                        toast::Kind::Problem,
                        cx,
                    );
                } else if path != self.home && !self.home.is_empty() {
                    // The folder from last time may be gone; fall back to home.
                    self.go(self.home.clone(), cx);
                } else {
                    self.phase = Phase::Failed(sentence(&error).into());
                }
                cx.notify();
            }
            SftpEvent::TransferProgress {
                id: Some(id),
                done,
                total,
                ..
            } => {
                if let Some(transfer) = self.transfers.iter_mut().find(|t| t.id == id) {
                    transfer.done = done;
                    if total > 0 {
                        transfer.total = total;
                    }
                    self.sync_preview(cx);
                    cx.notify();
                }
            }
            SftpEvent::TransferFinished { id, outcome, .. } => self.finish(id, outcome, cx),
            SftpEvent::Error(message) => {
                tracing::warn!(%message, "SFTP error");
                toast::show(sentence(&message), toast::Kind::Problem, cx);
            }
            SftpEvent::Disconnected => self.on_lost(cx),
            _ => {}
        }
    }

    fn remember_folder(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.host_id else { return };
        let path = self.path.clone();
        Store::global(cx).update(cx, |store, _| {
            store.note(|vault| vault.set_entry_last_remote_dir(id, path));
        });
    }

    fn entry(&self, name: &str) -> Option<&FileEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    fn visible_entries(&self) -> Vec<FileEntry> {
        self.entries
            .iter()
            .filter(|entry| self.show_hidden || !remote_files::is_hidden(entry))
            .cloned()
            .collect()
    }

    fn open_sheet(&mut self, sheet: FilesSheet, cx: &mut Context<Self>) {
        gpui_mobile::hide_keyboard();
        self.sheet = Some(sheet);
        cx.notify();
    }

    fn close_sheet(&mut self, cx: &mut Context<Self>) {
        self.sheet = None;
        cx.notify();
    }

    fn open_entry(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(entry) = self.entry(name).cloned() else {
            return;
        };
        if entry.opens_as_folder() {
            self.go(remote_files::join(&self.path, &entry.name), cx);
        } else {
            self.open_preview(entry, cx);
        }
    }

    fn open_preview(&mut self, entry: FileEntry, cx: &mut Context<Self>) {
        let remote = remote_files::join(&self.path, &entry.name);
        let (content, limit) = match preview_plan(&entry) {
            PreviewPlan::Ready(content) => (content, None),
            PreviewPlan::Read { limit } => (PreviewContent::Loading, Some(limit)),
        };
        let preview = cx.new(|cx| PreviewView::new(entry, content, cx));
        if let (Some(limit), Some(sftp)) = (limit, self.sftp.clone()) {
            let weak = preview.downgrade();
            let task = cx.spawn(async move |_, cx| {
                let result = sftp.read_head(remote, limit).await;
                weak.update(cx, |preview, cx| preview.show(result, cx)).ok();
            });
            preview.update(cx, |preview, _| preview.set_load(task));
        }
        let subscription = cx.subscribe(&preview, |view, preview, event: &PreviewEvent, cx| {
            match event {
                PreviewEvent::Close => view.preview = None,
                PreviewEvent::Share => {
                    let name = preview.read(cx).entry().name.clone();
                    view.share(&name, cx);
                }
                PreviewEvent::Edit => {
                    let name = preview.read(cx).entry().name.clone();
                    view.preview = None;
                    view.open_editor(&name, cx);
                }
            }
            cx.notify();
        });
        self.preview = Some((preview, subscription));
        self.sync_preview(cx);
        cx.notify();
    }

    fn sync_preview(&self, cx: &mut Context<Self>) {
        let Some((preview, _)) = &self.preview else {
            return;
        };
        let name = preview.read(cx).entry().name.clone();
        let sharing = self
            .download_of(&name)
            .map(|transfer| match transfer.total {
                0 => 0.,
                total => transfer.done as f32 / total as f32,
            });
        preview.update(cx, |preview, cx| preview.set_sharing(sharing, cx));
    }

    fn download_of(&self, name: &str) -> Option<&Transfer> {
        self.transfers.iter().find(|transfer| {
            transfer.direction == Direction::Down
                && transfer.folder == self.path
                && transfer.name == name
        })
    }

    fn open_editor(&mut self, name: &str, cx: &mut Context<Self>) {
        self.sheet = None;
        let (Some(sftp), Some(entry)) = (self.sftp.clone(), self.entry(name).cloned()) else {
            return;
        };
        let remote = remote_files::join(&self.path, &entry.name);
        let editor = cx.new(|cx| FileEditorView::new(entry, remote, sftp, cx));
        let subscription = cx.subscribe(&editor, |view, _, event: &EditorEvent, cx| {
            match event {
                EditorEvent::Close => view.editor = None,
                EditorEvent::Saved => view.refresh(cx),
            }
            cx.notify();
        });
        self.editor = Some((editor, subscription));
        cx.notify();
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_transfer;
        self.next_transfer += 1;
        id
    }

    fn share(&mut self, name: &str, cx: &mut Context<Self>) {
        // Check first, so we do not download a file we cannot hand over.
        if !share::can_hand_over() {
            toast::show(
                "Saving files on Android is not ready yet",
                toast::Kind::Problem,
                cx,
            );
            return;
        }
        let Some(sftp) = self.sftp.clone() else {
            return;
        };
        let Some(entry) = self.entry(name).cloned() else {
            return;
        };
        // A second tap while it downloads would fetch it twice.
        if entry.opens_as_folder() || self.download_of(&entry.name).is_some() {
            return;
        }
        let id = self.next_id();
        let folder = share::scratch_dir().join(id.to_string());
        if let Err(e) = std::fs::create_dir_all(&folder) {
            tracing::error!(?e, "no room for the download");
            toast::show("The phone has no room for it", toast::Kind::Problem, cx);
            return;
        }
        let local = folder.join(&entry.name);
        let cancel: CancelFlag = Arc::new(AtomicBool::new(false));
        sftp.download(
            remote_files::join(&self.path, &entry.name),
            local.clone(),
            id,
            cancel.clone(),
        );
        self.transfers.push(Transfer {
            id,
            name: entry.name,
            direction: Direction::Down,
            done: 0,
            total: entry.size,
            cancel,
            local,
            folder: self.path.clone(),
            renamed_from: None,
        });
        self.sync_preview(cx);
        cx.notify();
    }

    fn pick_uploads(&mut self, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Upload".into()),
        });
        cx.spawn(async move |this, cx| match chosen.await {
            Ok(Ok(Some(paths))) => {
                this.update(cx, |view, cx| view.upload(paths, cx)).ok();
            }
            Ok(Ok(None)) => {}
            Ok(Err(e)) => {
                tracing::warn!(?e, "no files picked");
                cx.update(|cx| toast::show("Files could not be picked", toast::Kind::Problem, cx));
            }
            Err(_) => {}
        })
        .detach();
    }

    fn upload(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(sftp) = self.sftp.clone() else {
            return;
        };
        for local in paths {
            let Some(name) = local
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                continue;
            };
            let total = std::fs::metadata(&local).map_or(0, |meta| meta.len());
            let folder = self.path.clone();
            let free = remote_files::free_name(&name, |candidate| {
                self.entry(candidate).is_some()
                    || self.transfers.iter().any(|transfer| {
                        transfer.direction == Direction::Up
                            && transfer.folder == folder
                            && transfer.name == candidate
                    })
            });
            let id = self.next_id();
            let cancel: CancelFlag = Arc::new(AtomicBool::new(false));
            sftp.upload(
                local.clone(),
                remote_files::join(&folder, &free),
                id,
                cancel.clone(),
            );
            self.transfers.push(Transfer {
                id,
                renamed_from: (free != name).then_some(name),
                name: free,
                direction: Direction::Up,
                done: 0,
                total,
                cancel,
                local,
                folder,
            });
        }
        cx.notify();
    }

    fn cancel(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(transfer) = self.transfers.iter().find(|t| t.id == id) {
            transfer.cancel.store(true, Ordering::Relaxed);
        }
        cx.notify();
    }

    fn finish(&mut self, id: u64, outcome: TransferOutcome, cx: &mut Context<Self>) {
        let Some(index) = self.transfers.iter().position(|t| t.id == id) else {
            return;
        };
        let transfer = self.transfers.remove(index);
        self.sync_preview(cx);
        match (outcome, transfer.direction) {
            (TransferOutcome::Done { .. }, Direction::Down) => {
                share::hand_over(transfer.local, cx);
            }
            (TransferOutcome::Done { .. }, Direction::Up) => {
                forget_picked_copy(&transfer.local);
                let message = match &transfer.renamed_from {
                    Some(original) => format!("{original} uploaded as {}", transfer.name),
                    None => format!("{} uploaded", transfer.name),
                };
                toast::show(message, toast::Kind::Done, cx);
                if transfer.folder == self.path {
                    self.refresh(cx);
                }
            }
            (TransferOutcome::Canceled, direction) => {
                if direction == Direction::Down {
                    remove_download(&transfer.local);
                }
            }
            (TransferOutcome::Failed(error), direction) => {
                tracing::warn!(%error, name = %transfer.name, "transfer failed");
                if direction == Direction::Down {
                    remove_download(&transfer.local);
                }
                toast::show(
                    format!("{} failed: {}", transfer.name, sentence(&error)),
                    toast::Kind::Problem,
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn change(
        &mut self,
        work: impl Future<Output = anyhow::Result<()>> + 'static,
        done: String,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(()) => toast::show(done, toast::Kind::Done, cx),
                    Err(e) => {
                        tracing::warn!(?e, "folder change failed");
                        toast::show(sentence(&e.to_string()), toast::Kind::Problem, cx);
                    }
                }
                view.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    fn delete(&mut self, name: &str, cx: &mut Context<Self>) {
        self.sheet = None;
        let (Some(sftp), Some(entry)) = (self.sftp.clone(), self.entry(name).cloned()) else {
            return;
        };
        let path = remote_files::join(&self.path, name);
        // `is_dir`, not `opens_as_folder`: for a link to a folder, only the link is removed.
        let is_folder = entry.is_dir;
        self.change(
            async move {
                if is_folder {
                    sftp.remove_dir_recursive(path).await
                } else {
                    sftp.remove_file(path).await
                }
            },
            format!("{name} deleted"),
            cx,
        );
    }

    fn open_dialog(&mut self, kind: DialogKind, cx: &mut Context<Self>) {
        self.sheet = None;
        self.dialog = Some(NameDialog {
            kind,
            input: None,
            error: None,
            _subscription: None,
        });
        cx.notify();
    }

    fn submit_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let Some(input) = &dialog.input else { return };
        let name = input.read(cx).value().trim().to_string();
        if !remote_files::is_valid_name(&name) {
            dialog.error = Some("A name cannot be empty or hold a /.".into());
            cx.notify();
            return;
        }
        let is_taken = self.entries.iter().any(|entry| entry.name == name)
            && !matches!(&dialog.kind, DialogKind::Rename(old) if *old == name);
        if is_taken {
            dialog.error = Some("Something here already has that name.".into());
            cx.notify();
            return;
        }
        let Some(sftp) = self.sftp.clone() else {
            return;
        };
        let target = remote_files::join(&self.path, &name);
        let kind = self.dialog.take().map(|dialog| dialog.kind);
        gpui_mobile::hide_keyboard();
        match kind {
            Some(DialogKind::NewFolder) => self.change(
                async move { sftp.make_dir(target).await },
                format!("{name} created"),
                cx,
            ),
            Some(DialogKind::Rename(old)) if old != name => {
                let from = remote_files::join(&self.path, &old);
                self.change(
                    async move { sftp.rename(from, target).await },
                    format!("Renamed to {name}"),
                    cx,
                );
            }
            _ => {}
        }
        cx.notify();
    }

    fn start_move(&mut self, name: &str, cx: &mut Context<Self>) {
        self.sheet = None;
        self.moving = Some(MoveRequest {
            name: name.to_string(),
            from: remote_files::join(&self.path, name),
        });
        cx.notify();
    }

    fn move_problem(&self, request: &MoveRequest) -> Option<&'static str> {
        if !remote_files::can_move_into(&request.from, &self.path) {
            return Some("Open another folder to move it there.");
        }
        if self.entry(&request.name).is_some() {
            return Some("Something here already has that name.");
        }
        None
    }

    fn finish_move(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.moving.take() else {
            return;
        };
        let (Some(sftp), None) = (self.sftp.clone(), self.move_problem(&request)) else {
            self.moving = Some(request);
            return;
        };
        let target = remote_files::join(&self.path, &request.name);
        let done = format!("{} moved to {}", request.name, self.folder_title());
        self.change(
            async move { sftp.rename(request.from, target).await },
            done,
            cx,
        );
        cx.notify();
    }

    fn open_permissions(&mut self, name: &str, cx: &mut Context<Self>) {
        self.sheet = None;
        let Some(mode) = self.entry(name).and_then(|entry| entry.permissions) else {
            return;
        };
        self.permissions = Some(PermissionsDialog {
            name: name.to_string(),
            path: remote_files::join(&self.path, name),
            mode,
        });
        cx.notify();
    }

    fn save_permissions(&mut self, cx: &mut Context<Self>) {
        let (Some(sftp), Some(dialog)) = (self.sftp.clone(), self.permissions.take()) else {
            return;
        };
        let done = format!(
            "{} is now {}",
            dialog.name,
            remote_files::mode_string(dialog.mode)
        );
        let PermissionsDialog { path, mode, .. } = dialog;
        self.change(
            async move { sftp.set_permissions(path, mode).await },
            done,
            cx,
        );
        cx.notify();
    }

    fn close_dialog(&mut self, cx: &mut Context<Self>) {
        self.dialog = None;
        gpui_mobile::hide_keyboard();
        cx.notify();
    }

    fn copy_path(&mut self, path: String, cx: &mut Context<Self>) {
        self.sheet = None;
        cx.write_to_clipboard(ClipboardItem::new_string(path));
        toast::show("Path copied", toast::Kind::Done, cx);
        cx.notify();
    }

    fn make_dialog_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        if dialog.input.is_some() {
            return;
        }
        let value = match &dialog.kind {
            DialogKind::NewFolder => String::new(),
            DialogKind::Rename(name) => name.clone(),
        };
        // Select only the stem, so typing keeps the extension, like the phone's Files app.
        let selected = 0..remote_files::stem_len(&value);
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Name")
                .default_value(value)
        });
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.set_selected_range(selected, cx);
        });
        gpui_mobile::show_keyboard_with_type(KeyboardType::Default);
        let subscription = cx.subscribe_in(&input, window, |view, _, event: &InputEvent, _, cx| {
            match event {
                InputEvent::PressEnter { .. } => view.submit_dialog(cx),
                InputEvent::Change => {
                    if let Some(dialog) = &mut view.dialog {
                        if dialog.error.take().is_some() {
                            cx.notify();
                        }
                    }
                }
                _ => {}
            }
        });
        dialog.input = Some(input);
        dialog._subscription = Some(subscription);
    }

    fn folder_title(&self) -> String {
        if self.path.is_empty() {
            "Files".to_string()
        } else if self.path == self.home {
            "Home".to_string()
        } else {
            remote_files::name_of(&self.path).to_string()
        }
    }

    fn render_folder_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let parent = remote_files::parent(&self.path).filter(|_| self.sftp.is_some());
        let title = self.folder_title();
        // The title already says Home, so show the real path here instead of `~`.
        let shown_path = if self.path == self.home {
            self.home.clone()
        } else {
            remote_files::display_path(&self.path, &self.home)
        };
        let is_ready = matches!(self.phase, Phase::Ready);
        h_flex()
            .w_full()
            .h(px(52.))
            .px(px(8.))
            .gap(px(6.))
            .border_b_1()
            .border_color(flint_theme::border())
            .child(
                plain_icon_button("up", "corner-left-up")
                    .when(parent.is_none(), |button| button.opacity(0.35))
                    .when_some(parent, |button, parent| {
                        button.on_click(
                            cx.listener(move |view, _, _, cx| view.go(parent.clone(), cx)),
                        )
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                div()
                                    .min_w_0()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(flint_theme::text())
                                    .truncate()
                                    .child(title),
                            )
                            .when(self.loading.is_some(), |line| {
                                line.child(spinner(14., flint_theme::text_dim()))
                            }),
                    )
                    .child(row_code(shown_path).text_size(px(11.5))),
            )
            .when(is_ready && self.moving.is_none(), |bar| {
                bar.child(
                    plain_icon_button("files-add", "plus").on_click(
                        cx.listener(|view, _, _, cx| view.open_sheet(FilesSheet::Add, cx)),
                    ),
                )
            })
            .when(is_ready, |bar| {
                bar.child(plain_icon_button("folder-menu", "ellipsis").on_click(
                    cx.listener(|view, _, _, cx| view.open_sheet(FilesSheet::Folder, cx)),
                ))
            })
    }

    fn render_transfers(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.transfers.is_empty() {
            return None;
        }
        let rows = self.transfers.iter().map(|transfer| {
            let id = transfer.id;
            let fraction = if transfer.total > 0 {
                (transfer.done as f32 / transfer.total as f32).clamp(0., 1.)
            } else {
                0.
            };
            let verb = match transfer.direction {
                Direction::Up => "Uploading",
                Direction::Down => "Downloading",
            };
            let detail = if transfer.cancel.load(Ordering::Relaxed) {
                "Stopping…".to_string()
            } else if transfer.total > 0 && transfer.done > 0 {
                format!(
                    "{verb}, {} of {}",
                    fmt_size(transfer.done),
                    fmt_size(transfer.total)
                )
            } else {
                format!("{verb}…")
            };
            h_flex()
                .pl(px(16.))
                .pr(px(8.))
                .py(px(10.))
                .gap(px(12.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(6.))
                        .child(row_title(transfer.name.clone()).text_size(px(15.)))
                        .child(
                            div()
                                .h(px(3.))
                                .w_full()
                                .rounded_full()
                                .bg(phone_theme::fill())
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(fraction))
                                        .rounded_full()
                                        .bg(flint_theme::accent()),
                                ),
                        )
                        .child(row_subtitle(detail).text_size(px(12.))),
                )
                .child(
                    div()
                        .id(("cancel-transfer", id as usize))
                        .flex_none()
                        .size(px(36.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .active(|button| button.bg(phone_theme::fill_pressed()))
                        .child(icon("x", 18., flint_theme::text_dim()))
                        .on_click(cx.listener(move |view, _, _, cx| view.cancel(id, cx))),
                )
                .into_any_element()
        });
        Some(
            div()
                .px(px(GUTTER))
                .pt(px(12.))
                .child(card_inset(rows, 16.))
                .into_any_element(),
        )
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let message = |content: gpui::Div| {
            v_flex()
                .flex_1()
                .justify_center()
                .px(px(24.))
                .pb(px(40.))
                .gap(px(8.))
                .child(content)
                .into_any_element()
        };
        match &self.phase {
            Phase::Opening => {
                return v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap(px(14.))
                    .child(spinner(28., flint_theme::text_dim()))
                    .child(row_subtitle("Opening the files…"))
                    .into_any_element();
            }
            Phase::Failed(reason) => {
                return message(
                    v_flex()
                        .gap(px(8.))
                        .child(title("The files could not be shown"))
                        .child(body_text(reason.clone()))
                        .child(div().h(px(12.)))
                        .child(
                            primary_button("files-retry", "Try again").on_click(cx.listener(
                                |view, _, _, cx| {
                                    if let Some(connection) = view.connection.clone() {
                                        view.attach(connection, cx);
                                    }
                                },
                            )),
                        ),
                );
            }
            Phase::Lost => {
                return message(
                    v_flex()
                        .gap(px(8.))
                        .child(title("Connection lost"))
                        .child(body_text("The files come back with the connection."))
                        .child(div().h(px(12.)))
                        .child(
                            primary_button("files-reconnect", "Reconnect").on_click(
                                cx.listener(|_, _, _, cx| cx.emit(FilesEvent::Reconnect)),
                            ),
                        ),
                );
            }
            Phase::Ready => {}
        }

        let entries = self.visible_entries();
        if entries.is_empty() {
            let hidden = self.entries.len();
            let mut empty = empty_state(
                "Nothing here",
                if hidden > 0 {
                    format!(
                        "{hidden} hidden {}",
                        if hidden == 1 { "item" } else { "items" }
                    )
                } else {
                    "Upload files with the + button.".to_string()
                },
            );
            if hidden > 0 {
                empty = empty.child(text_button("show-hidden", "Show hidden files").on_click(
                    cx.listener(|view, _, _, cx| {
                        view.show_hidden = true;
                        cx.notify();
                    }),
                ));
            }
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .pb(px(60.))
                .child(empty)
                .into_any_element();
        }

        let count = entries.len();
        let entries = Arc::new(entries);
        uniform_list(
            "files",
            count,
            cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| view.render_row(index, &entries[index], cx))
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .pb(px(if self.moving.is_some() { 160. } else { 24. }))
        .into_any_element()
    }

    fn render_row(&self, index: usize, entry: &FileEntry, cx: &mut Context<Self>) -> AnyElement {
        let kind = Kind::of(entry);
        let name = entry.name.clone();
        let more_name = entry.name.clone();
        let changed = match entry.mtime {
            0 => None,
            mtime => Some(ago_since(i64::from(mtime))),
        };
        let detail = match kind {
            Kind::Folder if entry.kind == flint_core::ssh::EntryKind::Symlink => {
                format!("Link to {}", entry.link_target.clone().unwrap_or_default())
            }
            Kind::Folder => changed.unwrap_or_else(|| "Folder".to_string()),
            Kind::BrokenLink => "Link that points nowhere".to_string(),
            Kind::Link => format!("Link to {}", entry.link_target.clone().unwrap_or_default()),
            _ => match changed {
                Some(changed) => format!("{}, {changed}", fmt_size(entry.size)),
                None => fmt_size(entry.size),
            },
        };
        h_flex()
            .id(("file", index))
            .when(remote_files::is_hidden(entry), |row| row.opacity(0.6))
            .h(px(ROW_HEIGHT))
            .w_full()
            .pl(px(GUTTER + 2.))
            .pr(px(8.))
            .gap(px(14.))
            .active(|row| row.bg(phone_theme::fill()))
            .child(icon(kind.icon(), 22., kind_color(kind)))
            .child(
                // Border on the text only, so the separator starts at the text, as in iOS lists.
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .gap(px(4.))
                    .border_b_1()
                    .border_color(flint_theme::border())
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(row_title(entry.name.clone()).text_size(px(15.)))
                            .child(row_subtitle(detail).text_size(px(12.5))),
                    )
                    .child(self.render_more_button(index, more_name, cx)),
            )
            .on_click(cx.listener(move |view, _, _, cx| view.open_entry(&name, cx)))
            .into_any_element()
    }

    fn render_more_button(
        &self,
        index: usize,
        name: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(("file-more", index))
            .flex_none()
            .size(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .active(|button| button.bg(phone_theme::fill_pressed()))
            .child(icon("ellipsis", 20., flint_theme::text_placeholder()))
            .on_click(cx.listener(move |view, _, _, cx| {
                cx.stop_propagation();
                view.open_sheet(FilesSheet::Entry(name.clone()), cx);
            }))
    }

    fn render_move_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let request = self.moving.as_ref()?;
        let problem = self.move_problem(request);
        let can_move = problem.is_none() && matches!(self.phase, Phase::Ready);
        Some(
            v_flex()
                .id("move-bar")
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .occlude()
                .gap(px(12.))
                .px(px(GUTTER))
                .pt(px(14.))
                .pb(px(14.))
                .bg(phone_theme::ground())
                .border_t_1()
                .border_color(flint_theme::border())
                .child(
                    v_flex()
                        .px(px(4.))
                        .gap(px(2.))
                        .child(row_title(format!("Move {}", request.name)))
                        .child(row_subtitle(match problem {
                            Some(problem) => problem.to_string(),
                            None => format!("Into {}", self.folder_title()),
                        })),
                )
                .child(
                    h_flex()
                        .gap(px(10.))
                        .child(div().flex_1().child(
                            secondary_button("move-cancel", "Cancel").on_click(cx.listener(
                                |view, _, _, cx| {
                                    view.moving = None;
                                    cx.notify();
                                },
                            )),
                        ))
                        .child(
                            div().flex_1().child(
                                primary_button("move-here", "Move here")
                                    .when(!can_move, |button| button.opacity(0.4))
                                    .on_click(cx.listener(|view, _, _, cx| view.finish_move(cx))),
                            ),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_entry_sheet(&self, name: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let entry = self.entry(name)?.clone();
        let path = remote_files::join(&self.path, &entry.name);
        let is_folder = entry.opens_as_folder();

        let mut facts = Vec::new();
        if !is_folder {
            facts.push(("Size", fmt_size(entry.size)));
        }
        if entry.mtime > 0 {
            facts.push(("Changed", ago_since(i64::from(entry.mtime))));
        }
        if let Some(mode) = entry.permissions {
            facts.push(("Permissions", remote_files::mode_string(mode)));
        }
        if let Some(target) = &entry.link_target {
            facts.push(("Points to", target.clone()));
        }

        let header = v_flex()
            .px(px(4.))
            .gap(px(4.))
            .child(title(entry.name.clone()).truncate())
            .child(row_code(remote_files::display_path(&path, &self.home)));

        let open_name = entry.name.clone();
        let primary = if is_folder {
            primary_button("sheet-open", "Open")
        } else {
            primary_button("sheet-share", "Share or save")
        }
        .on_click(cx.listener(move |view, _, _, cx| {
            view.sheet = None;
            if is_folder {
                view.open_entry(&open_name, cx);
            } else {
                view.share(&open_name, cx);
            }
            cx.notify();
        }));

        let mut actions = Vec::new();
        if !is_folder {
            let preview_name = entry.name.clone();
            actions.push(
                sheet_action("sheet-preview", "eye", "Preview", false)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.sheet = None;
                        view.open_entry(&preview_name, cx);
                    }))
                    .into_any_element(),
            );
        }
        if remote_files::can_edit(&entry) {
            let edit_name = entry.name.clone();
            actions.push(
                sheet_action("sheet-edit", "pen-line", "Edit", false)
                    .on_click(cx.listener(move |view, _, _, cx| view.open_editor(&edit_name, cx)))
                    .into_any_element(),
            );
        }
        if is_folder {
            let folder_path = path.clone();
            actions.push(
                sheet_action("sheet-cd", "square-terminal", "Open in terminal", false)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.sheet = None;
                        cx.emit(FilesEvent::OpenInTerminal(folder_path.clone()));
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        let rename_name = entry.name.clone();
        actions.push(
            sheet_action("sheet-rename", "type", "Rename", false)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.open_dialog(DialogKind::Rename(rename_name.clone()), cx)
                }))
                .into_any_element(),
        );
        let move_name = entry.name.clone();
        actions.push(
            sheet_action("sheet-move", "folder-input", "Move", false)
                .on_click(cx.listener(move |view, _, _, cx| view.start_move(&move_name, cx)))
                .into_any_element(),
        );
        if entry.permissions.is_some() {
            let permissions_name = entry.name.clone();
            actions.push(
                sheet_action("sheet-permissions", "lock", "Permissions", false)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.open_permissions(&permissions_name, cx)
                    }))
                    .into_any_element(),
            );
        }
        let copy_path = path.clone();
        actions.push(
            sheet_action("sheet-copy-path", "copy", "Copy path", false)
                .on_click(cx.listener(move |view, _, _, cx| view.copy_path(copy_path.clone(), cx)))
                .into_any_element(),
        );
        let delete_name = entry.name.clone();
        let delete = sheet_action("sheet-delete", "trash", "Delete", true)
            .on_click(cx.listener(move |view, _, _, cx| {
                view.open_sheet(FilesSheet::ConfirmDelete(delete_name.clone()), cx)
            }))
            .into_any_element();

        Some(
            v_flex()
                .gap(px(16.))
                .child(header)
                .child(primary)
                .when(!facts.is_empty(), |content| {
                    content.child(card_inset(
                        facts
                            .into_iter()
                            .map(|(label, value)| info_row(label, value).into_any_element()),
                        16.,
                    ))
                })
                .child(card_inset(actions, 50.))
                .child(card_inset([delete], 50.))
                .into_any_element(),
        )
    }

    fn render_folder_sheet(&self, cx: &mut Context<Self>) -> AnyElement {
        let path = self.path.clone();
        let cd_path = self.path.clone();
        let is_home = self.path == self.home;
        let mut actions = vec![
            sheet_action(
                "folder-hidden",
                if self.show_hidden { "eye-off" } else { "eye" },
                if self.show_hidden {
                    "Hide hidden files"
                } else {
                    "Show hidden files"
                },
                false,
            )
            .on_click(cx.listener(|view, _, _, cx| {
                view.show_hidden = !view.show_hidden;
                view.close_sheet(cx);
            }))
            .into_any_element(),
            sheet_action("folder-refresh", "refresh-cw", "Refresh", false)
                .on_click(cx.listener(|view, _, _, cx| {
                    view.close_sheet(cx);
                    view.refresh(cx);
                }))
                .into_any_element(),
            sheet_action("folder-cd", "square-terminal", "Open in terminal", false)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.close_sheet(cx);
                    cx.emit(FilesEvent::OpenInTerminal(cd_path.clone()));
                }))
                .into_any_element(),
            sheet_action("folder-copy-path", "copy", "Copy path", false)
                .on_click(cx.listener(move |view, _, _, cx| view.copy_path(path.clone(), cx)))
                .into_any_element(),
        ];
        if !is_home && !self.home.is_empty() {
            actions.push(
                sheet_action("folder-home", "house", "Go home", false)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.close_sheet(cx);
                        view.go(view.home.clone(), cx);
                    }))
                    .into_any_element(),
            );
        }
        v_flex()
            .gap(px(16.))
            .child(sheet_heading(self.folder_title(), self.path.clone()))
            .child(card_inset(actions, 50.))
            .into_any_element()
    }

    fn render_add_sheet(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap(px(16.))
            .child(sheet_heading(
                format!("Add to {}", self.folder_title()),
                self.path.clone(),
            ))
            .child(card_inset(
                [
                    sheet_action("add-upload", "upload", "Upload files", false)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.close_sheet(cx);
                            view.pick_uploads(cx);
                        }))
                        .into_any_element(),
                    sheet_action("add-folder", "folder-plus", "New folder", false)
                        .on_click(
                            cx.listener(|view, _, _, cx| {
                                view.open_dialog(DialogKind::NewFolder, cx)
                            }),
                        )
                        .into_any_element(),
                ],
                50.,
            ))
            .into_any_element()
    }

    fn render_delete_sheet(&self, name: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let entry = self.entry(name)?;
        let is_folder = entry.is_dir;
        let name = name.to_string();
        Some(
            v_flex()
                .gap(px(8.))
                .child(div().px(px(4.)).child(title(format!("Delete {name}?"))))
                .child(
                    div()
                        .px(px(4.))
                        .text_size(px(15.))
                        .line_height(px(21.))
                        .text_color(flint_theme::text_dim())
                        .child(if is_folder {
                            "The folder and everything in it go from the server. This cannot \
                             be undone."
                        } else {
                            "The file goes from the server. This cannot be undone."
                        }),
                )
                .child(div().h(px(10.)))
                .child(
                    danger_button("confirm-delete-file", "Delete")
                        .on_click(cx.listener(move |view, _, _, cx| view.delete(&name, cx))),
                )
                .child(
                    quiet_button("keep-file", "Cancel")
                        .on_click(cx.listener(|view, _, _, cx| view.close_sheet(cx))),
                )
                .into_any_element(),
        )
    }

    fn render_sheet(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let content = match self.sheet.as_ref()? {
            FilesSheet::Entry(name) => self.render_entry_sheet(name, cx)?,
            FilesSheet::Folder => self.render_folder_sheet(cx),
            FilesSheet::Add => self.render_add_sheet(cx),
            FilesSheet::ConfirmDelete(name) => self.render_delete_sheet(name, cx)?,
        };
        let this = cx.entity().downgrade();
        let dismiss = move |_: &mut Window, cx: &mut gpui::App| {
            this.update(cx, |view, cx| view.close_sheet(cx)).ok();
        };
        Some(sheet("files-sheet", content, dismiss))
    }

    fn render_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog_state = self.dialog.as_ref()?;
        let input = dialog_state.input.as_ref()?;
        let (title, action) = match &dialog_state.kind {
            DialogKind::NewFolder => ("New folder", "Create"),
            DialogKind::Rename(_) => ("Rename", "Rename"),
        };
        Some(dialog(
            v_flex()
                .gap(px(14.))
                .child(dialog_title(title, false))
                .child(field_input(input))
                .children(dialog_state.error.clone().map(error_text))
                .child(
                    primary_button("dialog-submit", action)
                        .on_click(cx.listener(|view, _, _, cx| view.submit_dialog(cx))),
                )
                .child(
                    quiet_button("dialog-cancel", "Cancel")
                        .on_click(cx.listener(|view, _, _, cx| view.close_dialog(cx))),
                ),
        ))
    }

    fn render_permissions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let permissions = self.permissions.as_ref()?;
        let mode = permissions.mode;
        let column_title = |text: &'static str| {
            div()
                .w(px(56.))
                .text_center()
                .text_size(px(13.))
                .text_color(flint_theme::text_dim())
                .child(text)
        };
        let rows = [("Owner", 6u32), ("Group", 3), ("Others", 0)].map(|(who, shift)| {
            let boxes = [(4u32, "read"), (2, "write"), (1, "run")].map(|(bit, what)| {
                let bit = bit << shift;
                let is_on = mode & bit != 0;
                div().w(px(56.)).flex().justify_center().child(
                    div()
                        .id(gpui::SharedString::from(format!("mode-{who}-{what}")))
                        .size(px(34.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(9.))
                        .bg(if is_on {
                            flint_theme::accent()
                        } else {
                            phone_theme::fill()
                        })
                        .active(|toggle| toggle.opacity(0.7))
                        .when(is_on, |toggle| {
                            toggle.child(icon("check", 18., flint_theme::accent_ink()))
                        })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if let Some(permissions) = &mut view.permissions {
                                permissions.mode ^= bit;
                                cx.notify();
                            }
                        })),
                )
            });
            h_flex()
                .h(px(44.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(15.))
                        .text_color(flint_theme::text())
                        .child(who),
                )
                .children(boxes)
        });
        Some(dialog(
            v_flex()
                .gap(px(14.))
                .child(
                    v_flex()
                        .gap(px(4.))
                        .child(dialog_title("Permissions", false))
                        .child(row_code(permissions.path.clone())),
                )
                .child(
                    v_flex()
                        .child(
                            h_flex()
                                .child(div().flex_1())
                                .child(column_title("Read"))
                                .child(column_title("Write"))
                                .child(column_title("Run")),
                        )
                        .children(rows),
                )
                .child(
                    h_flex()
                        .px(px(14.))
                        .py(px(10.))
                        .gap(px(14.))
                        .rounded(px(12.))
                        .bg(phone_theme::plate())
                        .font_family(phone_theme::MONO_FONT)
                        .text_size(px(14.))
                        .child(
                            div()
                                .flex_1()
                                .text_color(flint_theme::text())
                                .child(remote_files::mode_string(mode)),
                        )
                        .child(
                            div()
                                .text_color(flint_theme::text_dim())
                                .child(format!("{:03o}", mode & 0o777)),
                        ),
                )
                .child(
                    primary_button("permissions-save", "Save")
                        .on_click(cx.listener(|view, _, _, cx| view.save_permissions(cx))),
                )
                .child(
                    quiet_button("permissions-cancel", "Cancel").on_click(cx.listener(
                        |view, _, _, cx| {
                            view.permissions = None;
                            cx.notify();
                        },
                    )),
                ),
        ))
    }
}

fn sheet_heading(title_text: impl Into<SharedString>, subtitle: String) -> impl IntoElement {
    v_flex()
        .px(px(4.))
        .gap(px(4.))
        .child(title(title_text).truncate())
        .child(row_code(subtitle))
}

fn kind_color(kind: Kind) -> Hsla {
    match kind {
        Kind::Folder => flint_theme::accent(),
        Kind::BrokenLink => flint_theme::red_error(),
        _ => flint_theme::text_dim(),
    }
}

/// Each download has its own scratch folder, so the whole folder goes.
fn remove_download(local: &Path) {
    if let Some(folder) = local.parent() {
        let _ = std::fs::remove_dir_all(folder);
    }
}

/// iOS gives the app a copy of each picked file in its temp folder; delete the copy once it is
/// uploaded. A file anywhere else belongs to the user and stays.
fn forget_picked_copy(local: &Path) {
    let Ok(temporary) = gpui_mobile::packages::path_provider::temporary_directory() else {
        return;
    };
    if local.starts_with(&temporary) && !local.starts_with(share::scratch_dir()) {
        let _ = std::fs::remove_file(local);
    }
}

impl Render for FilesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.make_dialog_input(window, cx);
        if let Some((editor, _)) = &self.editor {
            return div().size_full().child(editor.clone()).into_any_element();
        }
        if let Some((preview, _)) = &self.preview {
            return div().size_full().child(preview.clone()).into_any_element();
        }
        div()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .size_full()
                    .child(self.render_folder_bar(cx))
                    .children(self.render_transfers(cx))
                    .child(self.render_list(cx)),
            )
            .children(self.render_move_bar(cx))
            .children(self.render_sheet(cx))
            .children(self.render_dialog(cx))
            .children(self.render_permissions(cx))
            .into_any_element()
    }
}
