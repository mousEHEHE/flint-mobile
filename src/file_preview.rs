//! Reads and tells text from binary the same way as the desktop's `file_preview.rs`.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, img, px, Context, Entity, EventEmitter, FontWeight, Image, ImageFormat, ObjectFit,
    RenderImage, Subscription, Task, Window,
};
use gpui_component::input::{Editor, EditorState};
use gpui_component::{h_flex, v_flex, ActiveTheme as _};

use flint_core::ssh::{EntryKind, FileEntry, FileHead};

use crate::remote_files::{self, fmt_size};
use crate::theme as flint_theme;
use crate::widgets::{ago_since, empty_state, plain_icon_button, spinner};

/// Enough for any config file; a long log shows only its start.
pub const TEXT_PREVIEW_BYTES: usize = 256 * 1024;
/// Images are read whole, and decoded they take many times their file size.
const IMAGE_PREVIEW_MAX_BYTES: u64 = 10 * 1024 * 1024;
/// The editor lays out each line whole, so one huge minified line would stall it.
const MAX_LINE_CHARS: usize = 2000;
const BINARY_PROBE_BYTES: usize = 8 * 1024;
/// gpui decodes all GIF frames to full RGBA canvases at once; a small file can need gigabytes.
const GIF_DECODED_MAX_BYTES: u64 = 256 * 1024 * 1024;

pub enum PreviewContent {
    Loading,
    Text {
        text: String,
        is_head_only: bool,
        has_long_lines_cut: bool,
    },
    Image(Arc<Image>),
    Empty,
    Binary,
    TooLarge(u64),
    NotAFile,
    Failed(String),
}

pub enum PreviewPlan {
    Ready(PreviewContent),
    Read { limit: usize },
}

pub fn preview_plan(entry: &FileEntry) -> PreviewPlan {
    if entry.is_link_broken {
        return PreviewPlan::Ready(PreviewContent::Failed("The link points nowhere.".into()));
    }
    if entry.opens_as_folder() || entry.kind == EntryKind::Other {
        return PreviewPlan::Ready(PreviewContent::NotAFile);
    }
    if entry.size == 0 {
        return PreviewPlan::Ready(PreviewContent::Empty);
    }
    if image_format(&entry.name).is_none() {
        return PreviewPlan::Read {
            limit: TEXT_PREVIEW_BYTES,
        };
    }
    if entry.size > IMAGE_PREVIEW_MAX_BYTES {
        return PreviewPlan::Ready(PreviewContent::TooLarge(entry.size));
    }
    PreviewPlan::Read {
        limit: entry.size as usize,
    }
}

fn image_format(name: &str) -> Option<ImageFormat> {
    let extension = name.rsplit_once('.')?.1.to_lowercase();
    Some(match extension.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "webp" => ImageFormat::Webp,
        "bmp" => ImageFormat::Bmp,
        "tif" | "tiff" => ImageFormat::Tiff,
        "svg" => ImageFormat::Svg,
        "ico" => ImageFormat::Ico,
        _ => return None,
    })
}

pub fn classify(name: &str, head: FileHead) -> PreviewContent {
    if head.bytes.is_empty() {
        return PreviewContent::Empty;
    }
    if let Some(format) = image_format(name) {
        if head.size > IMAGE_PREVIEW_MAX_BYTES {
            return PreviewContent::TooLarge(head.size);
        }
        if matches!(format, ImageFormat::Gif) && !is_gif_small_enough(&head.bytes) {
            return PreviewContent::Failed("This animation is too big to show.".into());
        }
        return PreviewContent::Image(Arc::new(Image::from_bytes(format, head.bytes)));
    }
    let probe = &head.bytes[..head.bytes.len().min(BINARY_PROBE_BYTES)];
    if probe.contains(&0) {
        return PreviewContent::Binary;
    }
    let is_head_only = head.size > head.bytes.len() as u64;
    let mut has_long_lines_cut = false;
    let text = decode_text(head.bytes);
    let mut lines = Vec::new();
    for line in text.split('\n') {
        if line.chars().count() > MAX_LINE_CHARS {
            has_long_lines_cut = true;
            lines.push(line.chars().take(MAX_LINE_CHARS).collect::<String>());
        } else {
            lines.push(line.to_string());
        }
    }
    PreviewContent::Text {
        text: lines.join("\n"),
        is_head_only,
        has_long_lines_cut,
    }
}

/// A GIF whose headers do not parse counts as too big, so gpui never tries it.
fn is_gif_small_enough(bytes: &[u8]) -> bool {
    let mut options = gif::DecodeOptions::new();
    options.skip_frame_decoding(true);
    let Ok(mut decoder) = options.read_info(bytes) else {
        return false;
    };
    let canvas = u64::from(decoder.width()) * u64::from(decoder.height()) * 4;
    let mut decoded = 0u64;
    loop {
        match decoder.next_frame_info() {
            Ok(Some(_)) => decoded += canvas,
            Ok(None) => return true,
            Err(_) => return false,
        }
        if decoded > GIF_DECODED_MAX_BYTES {
            return false;
        }
    }
}

/// Strips a BOM. A character cut by the read limit is dropped, not shown as U+FFFD.
fn decode_text(mut bytes: Vec<u8>) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes.drain(..3);
    }
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) if e.utf8_error().error_len().is_none() => {
            let valid = e.utf8_error().valid_up_to();
            let mut bytes = e.into_bytes();
            bytes.truncate(valid);
            String::from_utf8(bytes).unwrap_or_default()
        }
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

pub enum PreviewEvent {
    Close,
    Share,
    Edit,
}

pub struct PreviewView {
    entry: FileEntry,
    content: PreviewContent,
    /// Kept so its GPU atlas entry can be freed on release.
    rendered: Option<Arc<RenderImage>>,
    /// Created on the first render, because it needs the `Window`.
    editor: Option<Entity<EditorState>>,
    /// Download progress from 0 to 1 while the file is being shared.
    sharing: Option<f32>,
    /// Dropped with the preview, which cancels the read.
    _load: Option<Task<()>>,
    _release: Subscription,
}

impl EventEmitter<PreviewEvent> for PreviewView {}

impl PreviewView {
    pub fn new(entry: FileEntry, content: PreviewContent, cx: &mut Context<Self>) -> Self {
        // gpui keeps decoded images in its asset cache and GPU atlas until told to drop them.
        let release = cx.on_release(|this, cx| {
            if let Some(rendered) = this.rendered.take() {
                cx.drop_image(rendered, None);
            }
            if let PreviewContent::Image(image) = &this.content {
                image.clone().remove_asset(cx);
            }
        });
        Self {
            entry,
            content,
            rendered: None,
            editor: None,
            sharing: None,
            _load: None,
            _release: release,
        }
    }

    pub fn entry(&self) -> &FileEntry {
        &self.entry
    }

    pub fn set_load(&mut self, task: Task<()>) {
        self._load = Some(task);
    }

    pub fn set_sharing(&mut self, sharing: Option<f32>, cx: &mut Context<Self>) {
        if self.sharing != sharing {
            self.sharing = sharing;
            cx.notify();
        }
    }

    pub fn show(&mut self, result: anyhow::Result<FileHead>, cx: &mut Context<Self>) {
        self.content = match result {
            Ok(head) => classify(&self.entry.name, head),
            Err(e) => {
                tracing::warn!(?e, "preview read failed");
                PreviewContent::Failed(format!("The file could not be read: {e}."))
            }
        };
        cx.notify();
    }

    fn ensure_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            return;
        }
        let PreviewContent::Text { text, .. } = &mut self.content else {
            return;
        };
        let text = std::mem::take(text);
        self.editor = Some(cx.new(|cx| {
            // A phone is too narrow to scroll sideways through long lines.
            let mut state = EditorState::new(window, cx).soft_wrap(true);
            state.set_value(text, window, cx);
            state
        }));
    }

    fn details(&self) -> String {
        if let Some(fraction) = self.sharing {
            let done = (fraction.clamp(0., 1.) * self.entry.size as f32) as u64;
            return format!(
                "Downloading, {} of {}",
                fmt_size(done),
                fmt_size(self.entry.size)
            );
        }
        let mut parts = vec![fmt_size(self.entry.size)];
        if self.entry.mtime > 0 {
            parts.push(ago_since(i64::from(self.entry.mtime)));
        }
        if let PreviewContent::Text {
            is_head_only,
            has_long_lines_cut,
            ..
        } = self.content
        {
            if is_head_only {
                parts.push(format!(
                    "first {} shown",
                    fmt_size(TEXT_PREVIEW_BYTES as u64)
                ));
            }
            if has_long_lines_cut {
                parts.push("long lines cut".into());
            }
        }
        parts.join(", ")
    }

    fn render_body(&self) -> gpui::AnyElement {
        let message = |title: &str, text: String| {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(empty_state(title.to_string(), text))
                .into_any_element()
        };
        match &self.content {
            PreviewContent::Loading => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(spinner(32., flint_theme::text_dim()))
                .into_any_element(),
            PreviewContent::Text { .. } => match &self.editor {
                Some(editor) => div()
                    .size_full()
                    .px(px(4.))
                    .child(
                        Editor::new(editor)
                            .readonly(true)
                            .bordered(false)
                            .h_full()
                            .text_size(px(13.)),
                    )
                    .into_any_element(),
                None => div().into_any_element(),
            },
            // Absolute, so the box gets the body's final size for the image to fit into.
            PreviewContent::Image(image) => div()
                .absolute()
                .inset_0()
                .p(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(image.clone())
                        .id("preview-image")
                        .max_w_full()
                        .max_h_full()
                        .rounded(px(10.))
                        .object_fit(ObjectFit::Contain)
                        .with_loading(|| spinner(32., flint_theme::text_dim()))
                        .with_fallback(|| {
                            div()
                                .text_size(px(14.))
                                .text_color(flint_theme::text_dim())
                                .child("The image could not be decoded.")
                                .into_any_element()
                        }),
                )
                .into_any_element(),
            PreviewContent::Empty => message("Empty file", "There is nothing in it.".into()),
            PreviewContent::Binary => message(
                "No preview",
                "This file is not text. Share it to open it in another app.".into(),
            ),
            PreviewContent::TooLarge(size) => message(
                "Too large to show",
                format!(
                    "This image is {}; the phone shows images up to {}. Share it to open it \
                     in another app.",
                    fmt_size(*size),
                    fmt_size(IMAGE_PREVIEW_MAX_BYTES)
                ),
            ),
            PreviewContent::NotAFile => message("No preview", "This is not a regular file.".into()),
            PreviewContent::Failed(error) => message("No preview", error.clone()),
        }
    }
}

impl Render for PreviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_editor(window, cx);
        if let (PreviewContent::Image(image), None) = (&self.content, &self.rendered) {
            self.rendered = image.clone().get_render_image(window, cx);
        }
        let can_share = !matches!(
            self.content,
            PreviewContent::NotAFile | PreviewContent::Failed(_)
        ) && !self.entry.opens_as_folder();
        let can_edit = matches!(
            self.content,
            PreviewContent::Text { .. } | PreviewContent::Empty
        ) && remote_files::can_edit(&self.entry);
        let share_button = match self.sharing {
            // Same size as the button, so the title does not move.
            Some(_) => div()
                .flex_none()
                .size(px(40.))
                .flex()
                .items_center()
                .justify_center()
                .child(spinner(20., flint_theme::accent()))
                .into_any_element(),
            None => plain_icon_button("share-preview", "share")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(PreviewEvent::Share)))
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(
                div()
                    .relative()
                    .w_full()
                    .border_b_1()
                    .border_color(flint_theme::border())
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(52.))
                            .px(px(8.))
                            .gap(px(6.))
                            .child(
                                plain_icon_button("close-preview", "chevron-left").on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(PreviewEvent::Close)),
                                ),
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
                                            .text_color(flint_theme::text_dim())
                                            .truncate()
                                            .child(self.details()),
                                    ),
                            )
                            .when(can_edit, |bar| {
                                bar.child(plain_icon_button("edit-preview", "pen-line").on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(PreviewEvent::Edit)),
                                ))
                            })
                            .when(can_share, |bar| bar.child(share_button)),
                    )
                    .when_some(self.sharing, |header, fraction| {
                        header.child(
                            div()
                                .absolute()
                                .left_0()
                                .bottom(px(-1.))
                                .h(px(3.))
                                .w(gpui::relative(fraction.clamp(0.02, 1.)))
                                .bg(flint_theme::accent()),
                        )
                    }),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.render_body()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> FileEntry {
        FileEntry {
            name: name.into(),
            is_dir: false,
            size,
            mtime: 0,
            permissions: None,
            kind: EntryKind::File,
            link_target: None,
            is_link_to_dir: false,
            is_link_broken: false,
        }
    }

    fn head(bytes: &[u8], size: u64) -> FileHead {
        FileHead {
            bytes: bytes.to_vec(),
            size,
        }
    }

    #[test]
    fn text_files_read_their_start_and_images_read_whole() {
        assert!(matches!(
            preview_plan(&file("app.log", 10_000_000)),
            PreviewPlan::Read {
                limit: TEXT_PREVIEW_BYTES
            }
        ));
        assert!(matches!(
            preview_plan(&file("photo.png", 2048)),
            PreviewPlan::Read { limit: 2048 }
        ));
        assert!(matches!(
            preview_plan(&file("huge.jpg", IMAGE_PREVIEW_MAX_BYTES + 1)),
            PreviewPlan::Ready(PreviewContent::TooLarge(_))
        ));
        assert!(matches!(
            preview_plan(&file("empty", 0)),
            PreviewPlan::Ready(PreviewContent::Empty)
        ));
    }

    #[test]
    fn a_nul_byte_means_no_text_preview() {
        assert!(matches!(
            classify("data", head(b"ELF\0\x01", 5)),
            PreviewContent::Binary
        ));
        match classify("notes.txt", head(b"hello\nworld", 100)) {
            PreviewContent::Text {
                text, is_head_only, ..
            } => {
                assert_eq!(text, "hello\nworld");
                assert!(is_head_only);
            }
            _ => panic!("text expected"),
        }
    }

    #[test]
    fn a_character_cut_by_the_limit_is_dropped() {
        // "é" is two bytes; the read stopped after the first.
        assert_eq!(decode_text(vec![b'a', 0xC3]), "a");
        assert_eq!(decode_text(vec![0xEF, 0xBB, 0xBF, b'x']), "x");
    }
}
