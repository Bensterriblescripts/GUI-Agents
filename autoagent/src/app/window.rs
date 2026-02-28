use eframe::egui::{self, CursorIcon, Rect, Vec2};

use crate::config::RESIZE_HANDLE_SIZE;

use super::AutoAgentApp;

const BOTTOM_RESIZE_OFFSET: f32 = 20.0;

impl AutoAgentApp {
    pub(super) fn update_window_drag(&mut self, resize_rect: Rect, drag_rect: Rect) {
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
            if top_left_rect.contains(pos) {
                Some((CursorIcon::ResizeNwSe, egui::ResizeDirection::NorthWest))
            } else if top_right_rect.contains(pos) {
                Some((CursorIcon::ResizeNeSw, egui::ResizeDirection::NorthEast))
            } else if bottom_left_rect.contains(pos) {
                Some((CursorIcon::ResizeNeSw, egui::ResizeDirection::SouthWest))
            } else if bottom_right_rect.contains(pos) {
                Some((CursorIcon::ResizeNwSe, egui::ResizeDirection::SouthEast))
            } else if left_rect.contains(pos) {
                Some((CursorIcon::ResizeHorizontal, egui::ResizeDirection::West))
            } else if right_rect.contains(pos) {
                Some((CursorIcon::ResizeHorizontal, egui::ResizeDirection::East))
            } else if top_rect.contains(pos) {
                Some((CursorIcon::ResizeVertical, egui::ResizeDirection::North))
            } else if bottom_rect.contains(pos) {
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
            self.window_dragging = true;
            self.ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    }
}
