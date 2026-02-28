use eframe::egui::{self, Vec2};

use crate::config::{
    CARD_INNER_PADDING_X, LINE_HEIGHT, MAX_VISIBLE_ROWS, MAX_WINDOW_HEIGHT, MIN_TEXT_WRAP_WIDTH,
    MIN_WINDOW_HEIGHT, TEXT_EDIT_MARGIN_X, WINDOW_PADDING,
};

use super::AutoAgentApp;
use super::render::text_metrics;

impl AutoAgentApp {
    pub(super) fn display_rows(&mut self) -> (usize, usize) {
        let wrap_width = self.text_wrap_width();
        if let Some(step) = self.pending_step() {
            if self.render_step != Some(step) || self.text_layout_dirty {
                self.sync_render_buffer(step);
                self.render_step = Some(step);
            }
        }
        if !self.text_layout_dirty && self.display_rows_width == Some(wrap_width) {
            return (self.output_rows_cache, self.input_rows_cache);
        }
        let output_text = if self.busy {
            &self.render_buffer
        } else {
            &self.output
        };
        let (raw_output, raw_output_h) = if output_text.is_empty() {
            (0, 0.0)
        } else {
            text_metrics(output_text, wrap_width, &self.ctx)
        };
        let (raw_input, raw_input_h) = text_metrics(&self.input, wrap_width, &self.ctx);
        let max_input_h = MAX_VISIBLE_ROWS as f32 * LINE_HEIGHT;
        let (output_rows, input_rows, output_h, input_h) = if raw_output > 0 {
            let o = raw_output.min(MAX_VISIBLE_ROWS - 1);
            let o_h = raw_output_h.min((MAX_VISIBLE_ROWS - 1) as f32 * LINE_HEIGHT);
            let remaining = max_input_h - o_h;
            let i = raw_input.min(MAX_VISIBLE_ROWS - o).max(1);
            let i_h = raw_input_h.min(remaining).max(LINE_HEIGHT);
            (o, i, o_h, i_h)
        } else {
            let i = raw_input.min(MAX_VISIBLE_ROWS);
            let i_h = raw_input_h.min(max_input_h);
            (0, i, 0.0, i_h)
        };
        self.output_rows_cache = output_rows;
        self.input_rows_cache = input_rows;
        self.output_height_cache = output_h;
        self.input_height_cache = input_h;
        self.display_rows_width = Some(wrap_width);
        self.text_layout_dirty = false;
        (output_rows, input_rows)
    }

    pub(super) fn text_wrap_width(&self) -> f32 {
        (self.ctx.screen_rect().width()
            - WINDOW_PADDING * 2.0
            - CARD_INNER_PADDING_X
            - TEXT_EDIT_MARGIN_X)
            .max(MIN_TEXT_WRAP_WIDTH)
    }

    pub(super) fn resize_for_text(&mut self) {
        if self.resizing || self.user_height_override.is_some() {
            return;
        }
        let width = self.ctx.screen_rect().width();
        let (output_rows, _input_rows) = self.display_rows();
        let separator = if output_rows > 0 { 9.0 } else { 0.0 };
        let height = (58.0
            + self.output_height_cache
            + self.input_height_cache
            + separator
            + WINDOW_PADDING * 2.0)
            .clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT);
        let size = Vec2::new(width, height);
        if self.last_inner_size == Some(size) {
            return;
        }
        self.last_inner_size = Some(size);
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    }

    pub(super) fn release_input_focus(&self) {
        let id = egui::Id::new(Self::INPUT_ID);
        if !self.ctx.memory(|mem| mem.has_focus(id)) {
            return;
        }
        self.ctx.memory_mut(|mem| {
            mem.surrender_focus(id);
            mem.stop_text_input();
        });
    }

    pub(super) fn sync_input_focus(&mut self, response: &egui::Response) {
        if !self.pending_input_focus {
            return;
        }

        response.request_focus();
        if response.has_focus() {
            self.pending_input_focus = false;
        } else {
            self.ctx.request_repaint();
        }
    }
}
