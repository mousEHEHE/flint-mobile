use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Context, Entity, EventEmitter, FontWeight, Subscription, Task, Window,
};
use gpui_component::input::{Editor, EditorState, InputEvent};
use gpui_component::{h_flex, v_flex};
use uuid::Uuid;

use flint_core::ssh::{FileEntry, FileHead, SftpHandle};

use crate::phone_theme;
use crate::remote_files::EDIT_MAX_BYTES;
use crate::share;
use crate::theme as flint_theme;
use crate::toast;
use crate::widgets::{
    body_text, danger_button, dialog, dialog_title, empty_state, nav_action, plain_icon_button,
    primary_button, quiet_button, sentence, spinner,
};

pub enum EditorEvent {
    Close,
    Saved,
}

enum Phase {
    Loading,
    /// Waits for a render, which has the `Window` the editor needs.
    Read(String),
    Ready(Entity<EditorState>),
    Failed(String),
}

enum Question {
    Discard,
    Overwrite,
}

pub struct FileEditorView {
    entry: FileEntry,
    remote: String,
    sftp: SftpHandle,
    phase: Phase,
    read_mtime: Option<u32>,
    is_dirty: bool,
    is_saving: bool,
    question: Option<Question>,
    _task: Option<Task<()>>,
    _subscription: Option<Subscription>,
}

impl EventEmitter<EditorEvent> for FileEditorView {}

impl FileEditorView {
    pub fn new(entry: FileEntry, remote: String, sftp: SftpHandle, cx: &mut Context<Self>) -> Self {
        let read = {
            let sftp = sftp.clone();
            let remote = remote.clone();
            cx.spawn(async move |this, cx| {
                let head = sftp
                    .read_head(remote.clone(), EDIT_MAX_BYTES as usize + 1)
                    .await;
                let mtime = sftp.stat_file(remote).await.ok().flatten();
                this.update(cx, |view, cx| view.on_read(head, mtime, cx))
                    .ok();
            })
        };
        Self {
            entry,
            remote,
            sftp,
            phase: Phase::Loading,
            read_mtime: None,
            is_dirty: false,
            is_saving: false,
            question: None,
            _task: Some(read),
            _subscription: None,
        }
    }

    fn on_read(
        &mut self,
        head: anyhow::Result<FileHead>,
        mtime: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        self.read_mtime = mtime;
        self.phase = match head.map(|head| editable_text(head.bytes, head.size)) {
            Ok(Ok(text)) => Phase::Read(text),
            Ok(Err(reason)) => Phase::Failed(reason.to_string()),
            Err(e) => {
                tracing::warn!(?e, remote = %self.remote, "file to edit could not be read");
                Phase::Failed(format!(
                    "The file could not be read: {}",
                    sentence(&e.to_string())
                ))
            }
        };
        cx.notify();
    }

    fn ensure_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Phase::Read(text) = &mut self.phase else {
            return;
        };
        let text = std::mem::take(text);
        let editor = cx.new(|cx| {
            // A phone is too narrow for line numbers and for scrolling sideways.
            let mut state = EditorState::new(window, cx)
                .soft_wrap(true)
                .line_number(false);
            state.set_value(text, window, cx);
            state
        });
        self._subscription =
            Some(
                cx.subscribe(&editor, |view, _, event: &InputEvent, cx| match event {
                    InputEvent::Focus => gpui_mobile::show_keyboard(),
                    InputEvent::Change if !view.is_dirty => {
                        view.is_dirty = true;
                        cx.notify();
                    }
                    _ => {}
                }),
            );
        self.phase = Phase::Ready(editor);
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.is_dirty {
            gpui_mobile::hide_keyboard();
            self.question = Some(Question::Discard);
            cx.notify();
            return;
        }
        gpui_mobile::hide_keyboard();
        cx.emit(EditorEvent::Close);
    }

    fn save(&mut self, is_forced: bool, cx: &mut Context<Self>) {
        let Phase::Ready(editor) = &self.phase else {
            return;
        };
        if self.is_saving || !self.is_dirty {
            return;
        }
        self.question = None;
        self.is_saving = true;
        cx.notify();
        let text = editor.read(cx).value().to_string();
        let sftp = self.sftp.clone();
        let remote = self.remote.clone();
        let name = self.entry.name.clone();
        let read_mtime = self.read_mtime;
        self._task = Some(cx.spawn(async move |this, cx| {
            if !is_forced {
                let mtime = sftp.stat_file(remote.clone()).await.ok().flatten();
                if mtime.is_some() && mtime != read_mtime {
                    this.update(cx, |view, cx| {
                        view.is_saving = false;
                        view.question = Some(Question::Overwrite);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            }
            let written = write_back(&sftp, &remote, &name, text).await;
            let mtime = sftp.stat_file(remote.clone()).await.ok().flatten();
            this.update(cx, |view, cx| view.on_saved(written, mtime, cx))
                .ok();
        }));
    }

    fn on_saved(
        &mut self,
        written: anyhow::Result<()>,
        mtime: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        self.is_saving = false;
        match written {
            Ok(()) => {
                tracing::info!(remote = %self.remote, "file saved from the phone");
                self.is_dirty = false;
                self.read_mtime = mtime;
                toast::show(format!("{} saved", self.entry.name), toast::Kind::Done, cx);
                cx.emit(EditorEvent::Saved);
            }
            Err(e) => {
                tracing::error!(?e, remote = %self.remote, "saving the file failed");
                toast::show(
                    format!("Not saved: {}", sentence(&e.to_string())),
                    toast::Kind::Problem,
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn render_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let save: AnyElement = if self.is_saving {
            div()
                .flex_none()
                .w(px(72.))
                .flex()
                .justify_center()
                .child(spinner(20., flint_theme::accent()))
                .into_any_element()
        } else {
            nav_action("save-file", "Save", self.is_dirty)
                .when(!self.is_dirty, |action| action.opacity(0.5))
                .on_click(cx.listener(|view, _, _, cx| view.save(false, cx)))
                .into_any_element()
        };
        let is_ready = matches!(self.phase, Phase::Ready(_));
        h_flex()
            .w_full()
            .h(px(52.))
            .px(px(8.))
            .gap(px(6.))
            .border_b_1()
            .border_color(flint_theme::border())
            .child(
                plain_icon_button("close-editor", "chevron-left")
                    .on_click(cx.listener(|view, _, _, cx| view.close(cx))),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(flint_theme::text())
                            .truncate()
                            .child(self.entry.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(if self.is_dirty {
                                flint_theme::warning()
                            } else {
                                flint_theme::text_dim()
                            })
                            .truncate()
                            .child(if self.is_dirty {
                                "Edited, not saved"
                            } else {
                                "Saved on the server"
                            }),
                    ),
            )
            .when(is_ready, |bar| bar.child(save))
    }

    fn render_body(&self) -> AnyElement {
        match &self.phase {
            Phase::Loading | Phase::Read(_) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(spinner(28., flint_theme::text_dim()))
                .into_any_element(),
            Phase::Ready(editor) => div()
                .size_full()
                .px(px(6.))
                .pt(px(6.))
                .child(
                    Editor::new(editor)
                        .bordered(false)
                        .h_full()
                        .text_size(px(13.)),
                )
                .into_any_element(),
            Phase::Failed(reason) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(empty_state("Cannot edit this file", reason.clone()))
                .into_any_element(),
        }
    }

    fn render_question(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let name = &self.entry.name;
        let content = match self.question.as_ref()? {
            Question::Discard => v_flex()
                .gap(px(14.))
                .child(dialog_title("Discard your changes?", false))
                .child(body_text(format!("{name} stays as it is on the server.")))
                .child(danger_button("discard", "Discard").on_click(cx.listener(
                    |view, _, _, cx| {
                        view.is_dirty = false;
                        view.question = None;
                        cx.emit(EditorEvent::Close);
                    },
                )))
                .child(
                    quiet_button("keep-editing", "Keep editing").on_click(cx.listener(
                        |view, _, _, cx| {
                            view.question = None;
                            cx.notify();
                        },
                    )),
                ),
            Question::Overwrite => v_flex()
                .gap(px(14.))
                .child(dialog_title(format!("{name} changed on the server"), false))
                .child(body_text(
                    "It was changed after you opened it here. Saving puts your version in its \
                     place.",
                ))
                .child(
                    primary_button("overwrite", "Save anyway")
                        .on_click(cx.listener(|view, _, _, cx| view.save(true, cx))),
                )
                .child(
                    quiet_button("dont-overwrite", "Cancel").on_click(cx.listener(
                        |view, _, _, cx| {
                            view.question = None;
                            cx.notify();
                        },
                    )),
                ),
        };
        Some(dialog(content))
    }
}

fn editable_text(bytes: Vec<u8>, size: u64) -> Result<String, &'static str> {
    if size > EDIT_MAX_BYTES || bytes.len() as u64 > EDIT_MAX_BYTES {
        return Err("Files over 1 MB are too big to edit on the phone.");
    }
    if bytes.contains(&0) {
        return Err("This file is not text.");
    }
    String::from_utf8(bytes)
        .map_err(|_| "This file is not UTF-8 text, so it could not be saved as it is.")
}

async fn write_back(
    sftp: &SftpHandle,
    remote: &str,
    name: &str,
    text: String,
) -> anyhow::Result<()> {
    let folder = share::scratch_dir().join(format!("edit-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&folder)?;
    let local = folder.join(name);
    std::fs::write(&local, text)?;
    let sent = sftp.upload_file(local, remote.to_string()).await;
    let _ = std::fs::remove_dir_all(&folder);
    sent
}

impl Render for FileEditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_editor(window, cx);
        div()
            .relative()
            .size_full()
            .bg(phone_theme::ground())
            .child(
                v_flex()
                    .size_full()
                    .child(self.render_bar(cx))
                    .child(div().flex_1().min_h_0().child(self.render_body())),
            )
            .children(self.render_question(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_utf8_text_within_the_limit_is_editable() {
        assert_eq!(
            editable_text(b"listen 80;\n".to_vec(), 11).as_deref(),
            Ok("listen 80;\n")
        );
        assert!(editable_text(vec![b'a', 0, b'b'], 3).is_err());
        assert!(editable_text(vec![0xFF, 0xFE], 2).is_err());
        assert!(editable_text(b"a".to_vec(), EDIT_MAX_BYTES + 1).is_err());
    }
}
