#[cfg(target_os = "windows")]
use std::mem;

use eframe::egui::{self, CursorIcon, Rect, Vec2};

use crate::config::{MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, RESIZE_HANDLE_SIZE};
use crate::logging;

use super::{CodexAgentApp, MonitorKey, TileCell, TiledWindowState, WindowRestoreState};

const BOTTOM_RESIZE_OFFSET: f32 = 20.0;
const TILE_GRID_DIMENSION: i32 = 4;
const TILE_SNAP_TOLERANCE: i32 = 56;
const TILE_RELEASE_TOLERANCE: i32 = 84;
const TILE_RECT_TOLERANCE: i32 = 2;

impl CodexAgentApp {
    pub(super) fn min_inner_size(&self) -> Vec2 {
        self.min_inner_size
            .unwrap_or(Vec2::new(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT))
    }

    fn sync_min_inner_size(&mut self, inner_rect: Option<Rect>) {
        let Some(inner_size) = inner_rect.map(|rect| rect.size()) else {
            return;
        };
        if self.min_inner_size.is_some() {
            return;
        }
        self.min_inner_size = Some(inner_size);
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::MinInnerSize(inner_size));
    }

    pub(super) fn sync_viewport_state(&mut self) -> bool {
        let (minimized, maximized, inner_rect, outer_rect) = self.ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.minimized.unwrap_or(false),
                viewport.maximized.unwrap_or(self.maximized),
                viewport.inner_rect,
                viewport.outer_rect,
            )
        });

        self.sync_min_inner_size(inner_rect);
        self.maximized = maximized;

        if minimized {
            if !self.was_minimized {
                self.prepare_for_minimize(inner_rect, outer_rect);
            }
            self.was_minimized = true;
            return true;
        }

        if self.was_minimized {
            self.was_minimized = false;
            self.restore_from_minimize(inner_rect, outer_rect);
            return false;
        }

        self.capture_window_restore_state(inner_rect, outer_rect);
        false
    }

    pub(super) fn prepare_for_minimize_from_ctx(&mut self) {
        let (inner_rect, outer_rect) = self.ctx.input(|input| {
            let viewport = input.viewport();
            (viewport.inner_rect, viewport.outer_rect)
        });
        self.prepare_for_minimize(inner_rect, outer_rect);
    }

    fn prepare_for_minimize(&mut self, inner_rect: Option<Rect>, outer_rect: Option<Rect>) {
        let state = self.capture_window_restore_state(inner_rect, outer_rect);
        self.minimized_restore_state = state.or(self.minimized_restore_state);
        self.minimized_monitor = self.monitor_key_from_outer_rect(outer_rect);
    }

    fn restore_from_minimize(&mut self, inner_rect: Option<Rect>, outer_rect: Option<Rect>) {
        if self.maximized {
            self.minimized_restore_state = None;
            self.minimized_monitor = None;
            return;
        }

        let state = self
            .monitor_key_from_outer_rect(outer_rect)
            .and_then(|key| self.window_restore_states.get(&key).copied())
            .or_else(|| {
                self.minimized_monitor
                    .and_then(|key| self.window_restore_states.get(&key).copied())
            })
            .or(self.minimized_restore_state);

        self.minimized_restore_state = None;
        self.minimized_monitor = None;

        let Some(state) = state else {
            return;
        };

        self.user_height_override = state.user_height_override;
        self.last_inner_size = Some(state.inner_size);
        self.last_outer_size = Some(state.outer_size);
        if !Self::same_size(inner_rect.map(|rect| rect.size()), state.inner_size) {
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(state.inner_size));
        }
        self.invalidate_text_layout();
    }

    fn capture_window_restore_state(
        &mut self,
        inner_rect: Option<Rect>,
        outer_rect: Option<Rect>,
    ) -> Option<WindowRestoreState> {
        if self.maximized {
            return None;
        }

        let inner_size = inner_rect?.size();
        let outer_size = outer_rect.map(|rect| rect.size()).unwrap_or(inner_size);
        let state = WindowRestoreState {
            inner_size,
            outer_size,
            user_height_override: self.user_height_override,
        };

        self.last_inner_size = Some(inner_size);
        self.last_outer_size = Some(outer_size);
        if let Some(key) = self.monitor_key_from_outer_rect(outer_rect) {
            self.window_restore_states.insert(key, state);
        }

        Some(state)
    }

    #[cfg(target_os = "windows")]
    fn outer_rect_to_native_rect(outer_rect: Rect) -> windows_sys::Win32::Foundation::RECT {
        windows_sys::Win32::Foundation::RECT {
            left: outer_rect.min.x.round() as i32,
            top: outer_rect.min.y.round() as i32,
            right: outer_rect.max.x.round() as i32,
            bottom: outer_rect.max.y.round() as i32,
        }
    }

    #[cfg(target_os = "windows")]
    fn current_monitor(
        &self,
        outer_rect: Option<Rect>,
    ) -> Option<windows_sys::Win32::Graphics::Gdi::HMONITOR> {
        use windows_sys::Win32::Graphics::Gdi::{
            MONITOR_DEFAULTTONEAREST, MonitorFromRect, MonitorFromWindow,
        };

        unsafe {
            let monitor = if self.hwnd.is_null() {
                let outer_rect = outer_rect?;
                let rect = Self::outer_rect_to_native_rect(outer_rect);
                MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST)
            } else {
                MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST)
            };
            (!monitor.is_null()).then_some(monitor)
        }
    }

    #[cfg(target_os = "windows")]
    fn monitor_info(
        &self,
        outer_rect: Option<Rect>,
    ) -> Option<windows_sys::Win32::Graphics::Gdi::MONITORINFO> {
        use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};

        let monitor = self.current_monitor(outer_rect)?;
        unsafe {
            let mut info: MONITORINFO = mem::zeroed();
            info.cbSize = mem::size_of::<MONITORINFO>() as u32;
            (GetMonitorInfoW(monitor, &mut info) != 0).then_some(info)
        }
    }

    fn monitor_key_from_outer_rect(&self, outer_rect: Option<Rect>) -> Option<MonitorKey> {
        #[cfg(target_os = "windows")]
        {
            self.monitor_info(outer_rect).map(|info| MonitorKey {
                left: info.rcMonitor.left,
                top: info.rcMonitor.top,
                right: info.rcMonitor.right,
                bottom: info.rcMonitor.bottom,
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = outer_rect;
            None
        }
    }

    pub(super) fn sync_windows_tiling(&mut self) {
        #[cfg(target_os = "windows")]
        {
            let outer_rect = self.ctx.input(|input| input.viewport().outer_rect);
            let outer_changed = !Self::same_rect(self.last_viewport_outer_rect, outer_rect);
            self.last_viewport_outer_rect = outer_rect;
            if !outer_changed && !self.window_dragging && !self.resizing {
                return;
            }

            if self.hwnd.is_null() || self.maximized {
                return;
            }

            if self.recover_window_bounds() {
                return;
            }

            let Some(window_rect) = self.window_rect() else {
                return;
            };
            let Some(work_area) = self.monitor_work_area() else {
                return;
            };

            if let Some(state) = self.tiled_state {
                let expected_rect = tile_rect(work_area, state.cell);
                if rect_matches(window_rect, expected_rect, TILE_RELEASE_TOLERANCE) {
                    self.snap_window_to_rect(expected_rect);
                    self.sync_tiled_size(expected_rect);
                    return;
                }

                if let Some(cell) = detect_tile_cell(window_rect, work_area) {
                    self.apply_tile(cell, tile_rect(work_area, cell));
                    return;
                }

                self.leave_tile(false);
                return;
            }

            if let Some(cell) = detect_tile_cell(window_rect, work_area) {
                self.apply_tile(cell, tile_rect(work_area, cell));
            }
        }
    }

    pub(super) fn update_window_drag(
        &mut self,
        resize_rect: Rect,
        drag_rect: Rect,
        allow_horizontal: bool,
    ) {
        let allow_resize = self.tiled_state.is_none();
        let bottom_y = resize_rect.max.y + BOTTOM_RESIZE_OFFSET;
        let top_left_rect = Rect::from_min_max(
            resize_rect.min,
            resize_rect.min + Vec2::splat(RESIZE_HANDLE_SIZE),
        );
        let top_right_rect = Rect::from_min_max(
            egui::pos2(resize_rect.max.x - RESIZE_HANDLE_SIZE, resize_rect.min.y),
            egui::pos2(resize_rect.max.x, resize_rect.min.y + RESIZE_HANDLE_SIZE),
        );
        let bottom_left_rect = Rect::from_min_max(
            egui::pos2(resize_rect.min.x, bottom_y - RESIZE_HANDLE_SIZE),
            egui::pos2(resize_rect.min.x + RESIZE_HANDLE_SIZE, bottom_y),
        );
        let bottom_right_rect = Rect::from_min_max(
            egui::pos2(
                resize_rect.max.x - RESIZE_HANDLE_SIZE,
                bottom_y - RESIZE_HANDLE_SIZE,
            ),
            egui::pos2(resize_rect.max.x, bottom_y),
        );
        let top_rect = Rect::from_min_max(
            egui::pos2(resize_rect.min.x + RESIZE_HANDLE_SIZE, resize_rect.min.y),
            egui::pos2(
                resize_rect.max.x - RESIZE_HANDLE_SIZE,
                resize_rect.min.y + RESIZE_HANDLE_SIZE,
            ),
        );
        let bottom_rect = Rect::from_min_max(
            egui::pos2(
                resize_rect.min.x + RESIZE_HANDLE_SIZE,
                bottom_y - RESIZE_HANDLE_SIZE,
            ),
            egui::pos2(resize_rect.max.x - RESIZE_HANDLE_SIZE, bottom_y),
        );
        let left_rect = Rect::from_min_max(
            egui::pos2(resize_rect.min.x, resize_rect.min.y + RESIZE_HANDLE_SIZE),
            egui::pos2(
                resize_rect.min.x + RESIZE_HANDLE_SIZE,
                resize_rect.max.y - RESIZE_HANDLE_SIZE,
            ),
        );
        let right_rect = Rect::from_min_max(
            egui::pos2(
                resize_rect.max.x - RESIZE_HANDLE_SIZE,
                resize_rect.min.y + RESIZE_HANDLE_SIZE,
            ),
            egui::pos2(resize_rect.max.x, resize_rect.max.y - RESIZE_HANDLE_SIZE),
        );

        let resize_zone = |pos| {
            if allow_resize && allow_horizontal {
                if top_left_rect.contains(pos) {
                    return Some((CursorIcon::ResizeNwSe, egui::ResizeDirection::NorthWest));
                } else if top_right_rect.contains(pos) {
                    return Some((CursorIcon::ResizeNeSw, egui::ResizeDirection::NorthEast));
                } else if bottom_left_rect.contains(pos) {
                    return Some((CursorIcon::ResizeNeSw, egui::ResizeDirection::SouthWest));
                } else if bottom_right_rect.contains(pos) {
                    return Some((CursorIcon::ResizeNwSe, egui::ResizeDirection::SouthEast));
                } else if left_rect.contains(pos) {
                    return Some((CursorIcon::ResizeHorizontal, egui::ResizeDirection::West));
                } else if right_rect.contains(pos) {
                    return Some((CursorIcon::ResizeHorizontal, egui::ResizeDirection::East));
                }
            }
            if allow_resize && top_rect.contains(pos) {
                Some((CursorIcon::ResizeVertical, egui::ResizeDirection::North))
            } else if allow_resize && bottom_rect.contains(pos) {
                Some((CursorIcon::ResizeVertical, egui::ResizeDirection::South))
            } else {
                None
            }
        };

        if let Some(cursor) = self.ctx.input(|input| {
            input
                .pointer
                .hover_pos()
                .and_then(resize_zone)
                .map(|zone| zone.0)
        }) {
            self.ctx.set_cursor_icon(cursor);
        }

        let begin_resize = self.ctx.input(|input| {
            input
                .pointer
                .primary_pressed()
                .then(|| {
                    input
                        .pointer
                        .interact_pos()
                        .and_then(resize_zone)
                        .map(|zone| zone.1)
                })
                .flatten()
        });

        if let Some(direction) = begin_resize {
            self.drag_armed = false;
            self.window_dragging = false;
            self.resizing = true;
            self.user_height_override = Some(self.ctx.screen_rect().height());
            self.last_inner_size = Some(self.ctx.screen_rect().size());
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
            return;
        }

        let text_cursor = self.ctx.output(|o| o.cursor_icon == CursorIcon::Text);
        let start_drag = self.ctx.input(|input| {
            if input.pointer.primary_pressed() {
                self.drag_armed = !text_cursor
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|pos| drag_rect.contains(pos) && resize_zone(pos).is_none());
                self.window_dragging = false;
            }

            if !input.pointer.primary_down() {
                self.drag_armed = false;
                self.window_dragging = false;
                return false;
            }

            self.drag_armed && !self.window_dragging && !input.pointer.could_any_button_be_click()
        });

        if start_drag {
            if self.tiled_state.is_some() {
                self.leave_tile(true);
            }
            self.window_dragging = true;
            self.ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    }

    fn current_restore_state(&self) -> WindowRestoreState {
        let inner_size = self
            .last_inner_size
            .unwrap_or_else(|| self.ctx.screen_rect().size());
        let outer_size = self.last_outer_size.unwrap_or(inner_size);
        WindowRestoreState {
            inner_size,
            outer_size,
            user_height_override: self.user_height_override,
        }
    }

    #[cfg(target_os = "windows")]
    fn window_rect(&self) -> Option<windows_sys::Win32::Foundation::RECT> {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

        let mut rect: RECT = unsafe { mem::zeroed() };
        (unsafe { GetWindowRect(self.hwnd, &mut rect) } != 0).then_some(rect)
    }

    #[cfg(target_os = "windows")]
    fn monitor_work_area(&self) -> Option<windows_sys::Win32::Foundation::RECT> {
        let outer_rect = self.ctx.input(|input| input.viewport().outer_rect);
        self.monitor_info(outer_rect).map(|info| info.rcWork)
    }

    #[cfg(target_os = "windows")]
    pub(super) fn auto_resize_height_limit(&self) -> Option<f32> {
        let rect = self.window_rect().or_else(|| {
            self.ctx
                .input(|input| input.viewport().outer_rect)
                .map(Self::outer_rect_to_native_rect)
        })?;
        let work_area = self.monitor_work_area()?;
        let outer_extra =
            ((rect.bottom - rect.top) as f32 - self.ctx.screen_rect().height()).max(0.0);
        let max_outer_height = (work_area.bottom - rect.top)
            .min(work_area.bottom - work_area.top)
            .max(1) as f32;
        Some((max_outer_height - outer_extra).max(self.min_inner_size().y))
    }

    #[cfg(not(target_os = "windows"))]
    pub(super) fn auto_resize_height_limit(&self) -> Option<f32> {
        None
    }

    #[cfg(target_os = "windows")]
    fn recover_window_bounds(&mut self) -> bool {
        let Some(rect) = self.window_rect() else {
            return false;
        };
        let Some(work_area) = self.monitor_work_area() else {
            return false;
        };

        let work_width = (work_area.right - work_area.left).max(1);
        let work_height = (work_area.bottom - work_area.top).max(1);
        let min_inner_size = self.min_inner_size();
        let min_width = min_inner_size.x.round() as i32;
        let min_height = min_inner_size.y.round() as i32;
        let width = (rect.right - rect.left).clamp(min_width.min(work_width), work_width);
        let height = (rect.bottom - rect.top).clamp(min_height.min(work_height), work_height);
        let left = rect.left.clamp(work_area.left, work_area.right - width);
        let top = rect.top.clamp(work_area.top, work_area.bottom - height);

        if left == rect.left
            && top == rect.top
            && width == rect.right - rect.left
            && height == rect.bottom - rect.top
        {
            return false;
        }

        if self.tiled_state.is_some() {
            self.leave_tile(false);
        }

        self.set_window_rect(left, top, width, height);
        self.last_inner_size = Some(Vec2::new(width as f32, height as f32));
        self.last_outer_size = Some(Vec2::new(width as f32, height as f32));
        self.ctx.request_repaint();
        true
    }

    #[cfg(target_os = "windows")]
    fn apply_tile(&mut self, cell: TileCell, rect: windows_sys::Win32::Foundation::RECT) {
        let restore = self
            .tiled_state
            .map(|state| state.restore)
            .unwrap_or_else(|| self.current_restore_state());
        self.tiled_state = Some(TiledWindowState { cell, restore });
        self.set_native_resizable(false);
        self.snap_window_to_rect(rect);
        self.sync_tiled_size(rect);
        self.ctx.request_repaint();
    }

    fn leave_tile(&mut self, restore_size: bool) {
        let Some(state) = self.tiled_state.take() else {
            return;
        };

        #[cfg(target_os = "windows")]
        self.set_native_resizable(true);

        if restore_size {
            self.user_height_override = state.restore.user_height_override;
            self.last_inner_size = Some(state.restore.inner_size);
            self.last_outer_size = Some(state.restore.outer_size);

            #[cfg(target_os = "windows")]
            if let Some(rect) = self.window_rect() {
                let width = state.restore.outer_size.x.round() as i32;
                let height = state.restore.outer_size.y.round() as i32;
                self.set_window_rect(rect.left, rect.top, width, height);
            }

            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(state.restore.inner_size));
        } else {
            self.user_height_override = None;
            self.invalidate_text_layout();
            self.resize_for_text();
        }

        self.ctx.request_repaint();
    }

    #[cfg(target_os = "windows")]
    fn sync_tiled_size(&mut self, rect: windows_sys::Win32::Foundation::RECT) {
        let size = Vec2::new(
            (rect.right - rect.left) as f32,
            (rect.bottom - rect.top) as f32,
        );
        self.user_height_override = Some(size.y);
        self.last_inner_size = Some(size);
        self.last_outer_size = Some(size);
        self.invalidate_text_layout();
    }

    #[cfg(target_os = "windows")]
    fn snap_window_to_rect(&mut self, rect: windows_sys::Win32::Foundation::RECT) {
        if self
            .window_rect()
            .is_some_and(|current| rect_matches(current, rect, TILE_RECT_TOLERANCE))
        {
            return;
        }

        self.set_window_rect(
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        );
    }

    #[cfg(target_os = "windows")]
    fn set_native_resizable(&self, resizable: bool) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_STYLE, GetWindowLongW, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
            SetWindowLongW, SetWindowPos, WS_THICKFRAME,
        };

        let style = unsafe { GetWindowLongW(self.hwnd, GWL_STYLE) } as u32;
        let next = if resizable {
            style | WS_THICKFRAME
        } else {
            style & !WS_THICKFRAME
        };
        if next == style {
            return;
        }

        unsafe {
            SetWindowLongW(self.hwnd, GWL_STYLE, next as i32);
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            );
        }
    }

    #[cfg(target_os = "windows")]
    fn set_window_rect(&self, left: i32, top: i32, width: i32, height: i32) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
        };

        let ok = unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                left,
                top,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
        };
        if ok == 0 {
            logging::error("SetWindowPos failed while updating tiled window");
        }
    }
}

#[cfg(target_os = "windows")]
fn detect_tile_cell(
    rect: windows_sys::Win32::Foundation::RECT,
    work_area: windows_sys::Win32::Foundation::RECT,
) -> Option<TileCell> {
    let mut best = None;
    let mut score = i32::MAX;

    for row in 0..TILE_GRID_DIMENSION {
        for col in 0..TILE_GRID_DIMENSION {
            let cell = TileCell { col, row };
            let tile = tile_rect(work_area, cell);
            let left_delta = (rect.left - tile.left).abs();
            let top_delta = (rect.top - tile.top).abs();
            let right_delta = (rect.right - tile.right).abs();
            let bottom_delta = (rect.bottom - tile.bottom).abs();
            if left_delta > TILE_SNAP_TOLERANCE
                || top_delta > TILE_SNAP_TOLERANCE
                || right_delta > TILE_SNAP_TOLERANCE
                || bottom_delta > TILE_SNAP_TOLERANCE
            {
                continue;
            }

            let next_score = left_delta + top_delta + right_delta + bottom_delta;
            if next_score < score {
                score = next_score;
                best = Some(cell);
            }
        }
    }

    best
}

#[cfg(target_os = "windows")]
fn tile_rect(
    work_area: windows_sys::Win32::Foundation::RECT,
    cell: TileCell,
) -> windows_sys::Win32::Foundation::RECT {
    let width = work_area.right - work_area.left;
    let height = work_area.bottom - work_area.top;
    let left = work_area.left + (width * cell.col) / TILE_GRID_DIMENSION;
    let right = work_area.left + (width * (cell.col + 1)) / TILE_GRID_DIMENSION;
    let top = work_area.top + (height * cell.row) / TILE_GRID_DIMENSION;
    let bottom = work_area.top + (height * (cell.row + 1)) / TILE_GRID_DIMENSION;

    windows_sys::Win32::Foundation::RECT {
        left,
        top,
        right,
        bottom,
    }
}

#[cfg(target_os = "windows")]
fn rect_matches(
    current: windows_sys::Win32::Foundation::RECT,
    expected: windows_sys::Win32::Foundation::RECT,
    tolerance: i32,
) -> bool {
    (current.left - expected.left).abs() <= tolerance
        && (current.top - expected.top).abs() <= tolerance
        && (current.right - expected.right).abs() <= tolerance
        && (current.bottom - expected.bottom).abs() <= tolerance
}
