//! Must keep the fields and methods that the shared `TerminalElement` reads.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    canvas, div, px, App, Bounds, ClickEvent, Context, DispatchPhase, EventEmitter, FocusHandle,
    Focusable, KeyDownEvent, Keystroke, LongPressEvent, Modifiers, MouseMoveEvent, MouseUpEvent,
    PinchEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent, Subscription, Task, TouchPhase,
    Window,
};

use crate::preferences::Preferences;
use crate::shell::{
    GridPoint, GridSelectionRange, SelectionSide, SelectionType, ShellEvent, ShellHandle,
    TermSnapshot,
};
use crate::theme as flint_theme;
use crate::ui::mouse_report::{encode_mouse_event, MouseButton as ReportButton, MouseEventKind};
use crate::ui::terminal_element::{
    clamp_to_grid, pixel_to_grid, CachedGrid, ShapedLines, TerminalElement, LINE_HEIGHT_FACTOR,
};
use crate::ui::terminal_keymap;
use crate::ui::terminal_link::TerminalLink;
use crate::ui::terminal_palette::TerminalColors;

/// Half of flint-gpui's padding, to save columns on a narrow screen.
pub(crate) const SIDE_PADDING: f32 = 6.;

const COPIED_HINT_DURATION: Duration = Duration::from_millis(1500);

const LOCK_TAP_WINDOW: Duration = Duration::from_millis(400);

#[derive(Clone, Copy)]
pub struct SelectionState {
    pub range: Option<GridSelectionRange>,
}

/// Set by the shared element but never read: a phone scrolls by panning.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(crate) struct ScrollbarGeom {
    pub hit_x0: Pixels,
    pub hit_x1: Pixels,
    pub hit_y0: Pixels,
    pub hit_y1: Pixels,
    pub track_y: Pixels,
    pub track_h: Pixels,
    pub thumb_y: Pixels,
    pub thumb_h: Pixels,
    pub history_size: usize,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum StickyKey {
    #[default]
    Off,
    Once,
    Locked,
}

impl StickyKey {
    pub fn is_on(self) -> bool {
        self != StickyKey::Off
    }

    fn tapped(self, is_quick: bool) -> Self {
        match self {
            StickyKey::Off => StickyKey::Once,
            StickyKey::Once if is_quick => StickyKey::Locked,
            StickyKey::Once | StickyKey::Locked => StickyKey::Off,
        }
    }

    fn used(self) -> Self {
        match self {
            StickyKey::Once => StickyKey::Off,
            other => other,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Modifier {
    Ctrl,
    Alt,
}

pub enum TerminalEvent {
    Exited,
    Disconnected,
}

pub struct TerminalView {
    _event_task: Task<()>,
    pub(crate) shell: ShellHandle,
    pub last_snapshot: Option<TermSnapshot>,
    pub(crate) snapshot_version: u64,
    pub(crate) cached_grid: Option<(u64, u32, Rc<CachedGrid>)>,
    pub(crate) shaped_lines: ShapedLines,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) last_size: Option<(u16, u16)>,
    pub(crate) marked_text: Option<String>,
    pub(crate) selection: Option<SelectionState>,
    pub(crate) is_selecting: bool,
    pub(crate) last_element_bounds: Option<Bounds<Pixels>>,
    pub(crate) last_cell_size: Option<(Pixels, Pixels)>,
    pub(crate) scrollbar_geom: Option<ScrollbarGeom>,
    /// Always false: links open on Cmd+click, which a touch screen cannot do.
    pub(crate) is_link_click: bool,
    scroll_accumulator: f32,
    pub(crate) ctrl: StickyKey,
    pub(crate) alt: StickyKey,
    last_modifier_tap: Option<(Modifier, Instant)>,
    is_copied_hint_shown: bool,
    copied_hint_task: Option<Task<()>>,
    _preferences_subscription: Subscription,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TerminalView {
    pub fn new(shell: ShellHandle, cx: &mut Context<Self>) -> Self {
        let event_rx = shell.subscribe();
        let event_task = cx.spawn(async move |this, cx| {
            while let Ok(event) = event_rx.recv().await {
                // Only the newest of a burst of snapshots is worth drawing.
                let mut batch = vec![event];
                while let Ok(next) = event_rx.try_recv() {
                    if matches!(next, ShellEvent::Snapshot(_))
                        && matches!(batch.last(), Some(ShellEvent::Snapshot(_)))
                    {
                        *batch.last_mut().expect("batch starts with one event") = next;
                    } else {
                        batch.push(next);
                    }
                }
                let is_gone = this
                    .update(cx, |view, cx| {
                        for event in batch {
                            view.handle_event(event, cx);
                        }
                    })
                    .is_err();
                if is_gone {
                    break;
                }
            }
        });

        let preferences_subscription = cx.observe_global::<Preferences>(|view, cx| {
            view.cached_grid = None;
            cx.notify();
        });

        Self {
            _event_task: event_task,
            shell,
            last_snapshot: None,
            snapshot_version: 0,
            cached_grid: None,
            shaped_lines: ShapedLines::default(),
            focus_handle: cx.focus_handle(),
            last_size: None,
            marked_text: None,
            selection: None,
            is_selecting: false,
            last_element_bounds: None,
            last_cell_size: None,
            scrollbar_geom: None,
            is_link_click: false,
            scroll_accumulator: 0.0,
            ctrl: StickyKey::Off,
            alt: StickyKey::Off,
            last_modifier_tap: None,
            is_copied_hint_shown: false,
            copied_hint_task: None,
            _preferences_subscription: preferences_subscription,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
        gpui_mobile::show_keyboard();
    }

    fn handle_event(&mut self, event: ShellEvent, cx: &mut Context<Self>) {
        match event {
            ShellEvent::Snapshot(snap) => {
                // Keep our range if the snapshot has none: it may predate the selection command.
                if let (Some(sel), Some(range)) = (self.selection.as_mut(), snap.selection) {
                    sel.range = Some(range);
                }
                self.last_snapshot = Some(snap);
                self.snapshot_version = self.snapshot_version.wrapping_add(1);
                cx.notify();
            }
            ShellEvent::Exited => cx.emit(TerminalEvent::Exited),
            ShellEvent::Disconnected => cx.emit(TerminalEvent::Disconnected),
            ShellEvent::Error(e) => tracing::warn!(%e, "terminal error"),
        }
    }

    pub(crate) fn send_input(&self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if bytes.is_empty() {
            return;
        }
        let shell = self.shell.clone();
        cx.spawn(async move |_, _| shell.send_input(bytes).await)
            .detach();
    }

    fn is_app_cursor(&self) -> bool {
        self.last_snapshot.as_ref().is_some_and(|s| s.is_app_cursor)
    }

    fn sticky_modifiers(&self) -> Modifiers {
        Modifiers {
            control: self.ctrl.is_on(),
            alt: self.alt.is_on(),
            ..Default::default()
        }
    }

    fn use_sticky_modifiers(&mut self, cx: &mut Context<Self>) {
        let (ctrl, alt) = (self.ctrl.used(), self.alt.used());
        if (ctrl, alt) != (self.ctrl, self.alt) {
            self.ctrl = ctrl;
            self.alt = alt;
            cx.notify();
        }
    }

    pub fn tap_modifier(&mut self, modifier: Modifier, cx: &mut Context<Self>) {
        let now = Instant::now();
        let is_quick = self
            .last_modifier_tap
            .is_some_and(|(last, at)| last == modifier && now - at < LOCK_TAP_WINDOW);
        self.last_modifier_tap = Some((modifier, now));
        let key = match modifier {
            Modifier::Ctrl => &mut self.ctrl,
            Modifier::Alt => &mut self.alt,
        };
        *key = key.tapped(is_quick);
        cx.notify();
    }

    /// `key` is a gpui key name, such as "escape" or "up".
    pub fn press_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let keystroke = Keystroke {
            modifiers: self.sticky_modifiers(),
            key: key.to_string(),
            key_char: None,
        };
        let Some(bytes) =
            terminal_keymap::keystroke_to_bytes(&keystroke, false, self.is_app_cursor())
        else {
            return;
        };
        self.use_sticky_modifiers(cx);
        self.selection = None;
        self.send_input(bytes, cx);
    }

    pub fn set_marked_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.marked_text = Some(text);
        cx.notify();
    }

    pub fn clear_marked_text(&mut self, cx: &mut Context<Self>) {
        if self.marked_text.is_some() {
            self.marked_text = None;
            cx.notify();
        }
    }

    pub fn commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.selection = None;
        cx.notify();
        let modifiers = self.sticky_modifiers();
        let bytes = typed_bytes(text, modifiers, self.is_app_cursor());
        if modifiers.control || modifiers.alt {
            self.use_sticky_modifiers(cx);
        }
        self.send_input(bytes, cx);
    }

    pub fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let is_bracketed = self
            .last_snapshot
            .as_ref()
            .is_some_and(|snap| snap.bracketed_paste);
        self.selection = None;
        self.send_input(paste_bytes(&text, is_bracketed), cx);
    }

    pub fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_none() {
            return;
        }
        let shell = self.shell.clone();
        cx.spawn(async move |this, cx| {
            let Some(text) = shell.extract_selection_text().await else {
                return;
            };
            this.update(cx, |view, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                view.selection = None;
                view.show_copied_hint(cx);
            })
            .ok();
        })
        .detach();
    }

    fn show_copied_hint(&mut self, cx: &mut Context<Self>) {
        self.is_copied_hint_shown = true;
        self.copied_hint_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_HINT_DURATION).await;
            this.update(cx, |view, cx| {
                view.is_copied_hint_shown = false;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    // Called by the shared element. Most are pointer features a phone lacks.

    pub(crate) fn hovered_link(
        &self,
        _position: Point<Pixels>,
        _modifiers: Modifiers,
    ) -> Option<TerminalLink> {
        None
    }

    pub(crate) fn is_scrollbar_dragging(&self) -> bool {
        false
    }

    pub(crate) fn scrollbar_drag_move(
        &mut self,
        _position: Point<Pixels>,
        _cx: &mut Context<Self>,
    ) {
    }

    pub(crate) fn end_scrollbar_drag(&mut self) -> bool {
        false
    }

    pub(crate) fn report_mouse_drag(
        &mut self,
        _event: &MouseMoveEvent,
        _cx: &mut Context<Self>,
    ) -> bool {
        false
    }

    pub(crate) fn report_mouse_release(
        &mut self,
        _event: &MouseUpEvent,
        _cx: &mut Context<Self>,
    ) -> bool {
        false
    }

    pub(crate) fn set_drag_position(&mut self, _position: Point<Pixels>) {}

    pub(crate) fn end_selection(&mut self) {
        self.is_selecting = false;
    }

    pub(crate) fn update_selection_head(
        &mut self,
        point: GridPoint,
        side: SelectionSide,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_none() {
            return;
        }
        let shell = self.shell.clone();
        cx.spawn(async move |this, cx| {
            let range = shell.update_selection_head(point, side).await;
            this.update(cx, |view, cx| {
                if let Some(sel) = view.selection.as_mut() {
                    sel.range = range;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn select_word(&mut self, point: GridPoint, side: SelectionSide, cx: &mut Context<Self>) {
        self.selection = Some(SelectionState { range: None });
        let shell = self.shell.clone();
        cx.spawn(async move |this, cx| {
            let range = shell
                .set_selection(SelectionType::Semantic, point, side, point, side)
                .await;
            this.update(cx, |view, cx| {
                if let Some(sel) = view.selection.as_mut() {
                    sel.range = range;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Counted from the top of the scrollback, as selections are.
    fn grid_point(&self, position: Point<Pixels>) -> Option<(GridPoint, SelectionSide)> {
        let bounds = self.last_element_bounds?;
        let (cell_width, line_height) = self.last_cell_size?;
        let snapshot = self.last_snapshot.as_ref()?;
        let (point, side) = pixel_to_grid(
            position,
            bounds,
            cell_width,
            line_height,
            snapshot.display_offset,
        );
        let column = point.column.min((snapshot.cols as usize).saturating_sub(1));
        Some((GridPoint { column, ..point }, side))
    }

    fn on_long_press(&mut self, event: &LongPressEvent, cx: &mut Context<Self>) {
        match event.phase {
            TouchPhase::Started => {
                let is_inside = self
                    .last_element_bounds
                    .is_some_and(|bounds| bounds.contains(&event.start_position));
                if !is_inside {
                    return;
                }
                let Some((point, side)) = self.grid_point(event.position) else {
                    return;
                };
                self.is_selecting = true;
                self.select_word(point, side, cx);
            }
            TouchPhase::Moved if self.is_selecting => {
                if let Some((point, side)) = self.grid_point(event.position) {
                    self.update_selection_head(point, side, cx);
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled if self.is_selecting => {
                self.end_selection();
                cx.notify();
            }
            _ => {}
        }
    }

    fn on_tap(&mut self, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection.take().is_some() {
            cx.notify();
            return;
        }
        let mouse_mode = self
            .last_snapshot
            .as_ref()
            .map(|s| s.mouse_mode)
            .unwrap_or_default();
        if mouse_mode.any_active() {
            self.send_tap_report(event.position(), cx);
        }
        self.focus(window, cx);
    }

    fn send_tap_report(&self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(snapshot) = self.last_snapshot.as_ref() else {
            return;
        };
        let Some(bounds) = self.last_element_bounds else {
            return;
        };
        let Some((cell_width, line_height)) = self.last_cell_size else {
            return;
        };
        let (point, _) = pixel_to_grid(position, bounds, cell_width, line_height, 0);
        let point = clamp_to_grid(point, snapshot.rows, snapshot.cols);
        let mut bytes = Vec::new();
        for kind in [MouseEventKind::Press, MouseEventKind::Release] {
            let report = encode_mouse_event(
                snapshot.mouse_mode,
                kind,
                ReportButton::Left,
                point,
                false,
                false,
                false,
            );
            bytes.extend(report.unwrap_or_default());
        }
        self.send_input(bytes, cx);
    }

    fn on_pinch(&mut self, event: &PinchEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.delta == 0.0 {
            return;
        }
        let size = Preferences::terminal_font_size(cx) * (1.0 + event.delta);
        Preferences::set_terminal_font_size(size, cx);
    }

    /// Called for a pan, which arrives as a scroll wheel event.
    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line_height = self
            .last_cell_size
            .map(|(_, line_height)| line_height.as_f32())
            .unwrap_or_else(|| Preferences::terminal_font_size(cx) * LINE_HEIGHT_FACTOR);
        let delta_lines = match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => pixels.y.as_f32() / line_height,
        };
        let lines = take_whole_lines(&mut self.scroll_accumulator, delta_lines);
        cx.stop_propagation();
        if lines == 0 {
            return;
        }
        let Some(snapshot) = self.last_snapshot.as_ref() else {
            return;
        };

        if snapshot.mouse_mode.any_active() {
            let bytes = self.wheel_report_bytes(event.position, lines);
            self.send_input(bytes, cx);
            return;
        }
        if snapshot.alt_screen {
            if snapshot.is_alternate_scroll {
                let bytes = terminal_keymap::wheel_arrow_bytes(lines, snapshot.is_app_cursor);
                self.send_input(bytes, cx);
            }
            return;
        }
        let shell = self.shell.clone();
        cx.background_executor()
            .spawn(async move { shell.scroll_delta(lines).await })
            .detach();
    }

    fn wheel_report_bytes(&self, position: Point<Pixels>, lines: i32) -> Vec<u8> {
        let Some(snapshot) = self.last_snapshot.as_ref() else {
            return Vec::new();
        };
        let (Some(bounds), Some((cell_width, line_height))) =
            (self.last_element_bounds, self.last_cell_size)
        else {
            return Vec::new();
        };
        let (point, _) = pixel_to_grid(position, bounds, cell_width, line_height, 0);
        let point = clamp_to_grid(point, snapshot.rows, snapshot.cols);
        let button = if lines > 0 {
            ReportButton::WheelUp
        } else {
            ReportButton::WheelDown
        };
        encode_mouse_event(
            snapshot.mouse_mode,
            MouseEventKind::Press,
            button,
            point,
            false,
            false,
            false,
        )
        .map(|report| report.repeat(lines.unsigned_abs() as usize))
        .unwrap_or_default()
    }

    /// Gets Backspace from the on-screen keyboard and every key from a hardware one.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        let mut keystroke = event.keystroke.clone();
        let sticky = self.sticky_modifiers();
        keystroke.modifiers.control |= sticky.control;
        keystroke.modifiers.alt |= sticky.alt;
        let Some(bytes) =
            terminal_keymap::keystroke_to_bytes(&keystroke, false, self.is_app_cursor())
        else {
            // Printable keys arrive through the text input, as `commit_text`.
            return;
        };
        self.use_sticky_modifiers(cx);
        self.selection = None;
        cx.stop_propagation();
        self.send_input(bytes, cx);
    }

    fn render_selection_bar(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.selection.is_none() || self.is_selecting {
            return None;
        }
        Some(
            div()
                .absolute()
                .top(px(10.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("terminal-copy")
                        .px(px(18.))
                        .py(px(9.))
                        .rounded(px(18.))
                        .bg(flint_theme::pop_surface())
                        .border_1()
                        .border_color(flint_theme::float_border())
                        .text_size(px(14.))
                        .text_color(flint_theme::text())
                        .child("Copy")
                        .on_click(cx.listener(|view, _, _, cx| {
                            cx.stop_propagation();
                            view.copy_selection(cx);
                        })),
                ),
        )
    }

    fn render_copied_hint(&self) -> Option<impl IntoElement + use<>> {
        if !self.is_copied_hint_shown {
            return None;
        }
        Some(
            div()
                .absolute()
                .top(px(10.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(14.))
                        .py(px(7.))
                        .rounded(px(16.))
                        .bg(flint_theme::pop_surface())
                        .text_size(px(13.))
                        .text_color(flint_theme::muted_gray())
                        .child("Copied"),
                ),
        )
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = TerminalColors::current(cx).bg;
        let view = cx.entity().downgrade();
        // A long press goes to every listener in the window; `on_long_press` checks the bounds.
        let long_press = canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                window.on_mouse_event(move |event: &LongPressEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    if let Some(view) = view.upgrade() {
                        view.update(cx, |view, cx| view.on_long_press(event, cx));
                    }
                });
            },
        )
        .absolute()
        .size_0();

        div()
            .id("terminal-view")
            .track_focus(&self.focus_handle)
            .key_context("Terminal")
            .size_full()
            .px(px(SIDE_PADDING))
            .bg(background)
            .relative()
            .on_key_down(cx.listener(Self::on_key_down))
            .on_click(cx.listener(Self::on_tap))
            .on_pinch(cx.listener(Self::on_pinch))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(TerminalElement::new(cx.entity()))
            .child(long_press)
            .children(self.render_selection_bar(cx))
            .children(self.render_copied_hint())
    }
}

/// Phones type Return as a newline, which a terminal wants as a carriage return.
fn typed_bytes(text: &str, modifiers: Modifiers, is_app_cursor: bool) -> Vec<u8> {
    let mut chars = text.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if modifiers.control || modifiers.alt {
            let keystroke = Keystroke {
                modifiers,
                key: c.to_lowercase().to_string(),
                key_char: Some(c.to_string()),
            };
            if let Some(bytes) =
                terminal_keymap::keystroke_to_bytes(&keystroke, false, is_app_cursor)
            {
                return bytes;
            }
        }
    }
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

/// Keeps the fraction for the next event: a pan sends deltas well under a line.
fn take_whole_lines(accumulator: &mut f32, delta_lines: f32) -> i32 {
    if *accumulator * delta_lines < 0.0 {
        *accumulator = 0.0;
    }
    *accumulator += delta_lines;
    let lines = accumulator.trunc();
    *accumulator -= lines;
    lines as i32
}

/// Keep in sync with flint-gpui's copy, which explains why ESC and Ctrl-C are dropped.
fn paste_bytes(text: &str, is_bracketed: bool) -> Vec<u8> {
    if !is_bracketed {
        return text.replace("\r\n", "\r").replace('\n', "\r").into_bytes();
    }
    let filtered = text.replace(['\x1b', '\x03'], "");
    let mut bytes = Vec::with_capacity(filtered.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(filtered.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> Modifiers {
        Modifiers {
            control: true,
            ..Default::default()
        }
    }

    #[test]
    fn return_from_a_phone_keyboard_is_a_carriage_return() {
        assert_eq!(typed_bytes("\n", Modifiers::default(), false), b"\r");
        assert_eq!(typed_bytes("ls\n", Modifiers::default(), false), b"ls\r");
    }

    #[test]
    fn ctrl_from_the_keys_bar_makes_a_control_code() {
        assert_eq!(typed_bytes("c", ctrl(), false), [0x03]);
        // The Russian layout's "с" is the same key as "c".
        assert_eq!(typed_bytes("с", ctrl(), false), [0x03]);
    }

    #[test]
    fn alt_from_the_keys_bar_prefixes_escape() {
        let alt = Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(typed_bytes("b", alt, false), b"\x1bb");
    }

    #[test]
    fn a_sticky_key_is_armed_locked_and_released_by_taps() {
        assert_eq!(StickyKey::Off.tapped(false), StickyKey::Once);
        assert_eq!(StickyKey::Once.tapped(true), StickyKey::Locked);
        assert_eq!(StickyKey::Once.tapped(false), StickyKey::Off);
        assert_eq!(StickyKey::Locked.tapped(true), StickyKey::Off);
        assert_eq!(StickyKey::Once.used(), StickyKey::Off);
        assert_eq!(StickyKey::Locked.used(), StickyKey::Locked);
    }

    #[test]
    fn small_pan_deltas_add_up_to_whole_lines() {
        let mut accumulator = 0.0;
        assert_eq!(take_whole_lines(&mut accumulator, 0.4), 0);
        assert_eq!(take_whole_lines(&mut accumulator, 0.4), 0);
        assert_eq!(take_whole_lines(&mut accumulator, 0.4), 1);
        assert_eq!(take_whole_lines(&mut accumulator, -2.5), -2);
    }

    #[test]
    fn bracketed_paste_wraps_the_text() {
        assert_eq!(
            paste_bytes("a\nb", true),
            b"\x1b[200~a\nb\x1b[201~".to_vec()
        );
        assert_eq!(paste_bytes("a\nb", false), b"a\rb".to_vec());
    }
}
