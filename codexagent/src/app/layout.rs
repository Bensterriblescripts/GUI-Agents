use eframe::egui::{self, Vec2};

use crate::config::{
    AUTO_EXPAND_VISIBLE_ROWS, CARD_INNER_PADDING_X, LINE_HEIGHT, MAX_VISIBLE_ROWS,
    MAX_WINDOW_HEIGHT, MIN_TEXT_WRAP_WIDTH, TEXT_EDIT_MARGIN_X, WINDOW_BOTTOM_PADDING,
    WINDOW_PADDING,
};
use crate::logging;

use super::CodexAgentApp;
use super::render::{markdown_layout_job, response_separator_y};

impl CodexAgentApp {
    pub(super) fn visible_row_limit(&self) -> usize {
        if self.user_height_override.is_some() {
            MAX_VISIBLE_ROWS
        } else {
            AUTO_EXPAND_VISIBLE_ROWS
        }
    }

    pub(super) fn display_rows_for_width(&mut self, wrap_width: f32) -> (usize, usize) {
        if let Some(step) = self.pending_step() {
            if self.render_step != Some(step) || self.text_layout_dirty {
                self.sync_render_buffer(step);
                self.render_step = Some(step);
            }
        }
        if !self.text_layout_dirty && Self::same_width(self.display_rows_width, wrap_width) {
            return (self.output_rows_cache, self.input_rows_cache);
        }
        self.sync_output_galley(wrap_width);
        self.sync_input_galley(wrap_width);
        let (raw_output, raw_output_h) = if self.output_display_buffer.is_empty() {
            (0, 0.0)
        } else if let Some(galley) = self.output_galley.as_ref() {
            let rows = galley.rows.len().max(1);
            (rows, rows as f32 * LINE_HEIGHT)
        } else {
            logging::error("output galley missing during layout sync");
            (0, 0.0)
        };
        let (raw_input, raw_input_h) = if let Some(input_galley) = self.input_galley.as_ref() {
            let rows = input_galley.rows.len().max(1);
            (rows, rows as f32 * LINE_HEIGHT)
        } else {
            logging::error("input galley missing during layout sync");
            (1, LINE_HEIGHT)
        };
        let visible_row_limit = self.visible_row_limit();
        let max_input_h = visible_row_limit as f32 * LINE_HEIGHT;
        let (output_rows, input_rows, output_h, input_h) = if raw_output > 0 {
            let o = raw_output.min(visible_row_limit - 1);
            let o_h = raw_output_h.min((visible_row_limit - 1) as f32 * LINE_HEIGHT);
            let remaining = max_input_h - o_h;
            let i = raw_input.min(visible_row_limit - o).max(1);
            let i_h = raw_input_h.min(remaining).max(LINE_HEIGHT);
            (o, i, o_h, i_h)
        } else {
            let i = raw_input.min(visible_row_limit);
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

    pub(super) fn sync_output_galley(&mut self, wrap_width: f32) {
        self.sync_output_display_buffer();
        if Self::same_width(self.output_galley_width, wrap_width) && self.output_galley.is_some() {
            return;
        }
        let job = markdown_layout_job(
            &self.output_display_buffer,
            wrap_width,
            &self.output_display_prompt_ranges,
            self.output_display_response_start,
            &self.output_display_line_kinds,
        );
        let galley = self.ctx.fonts(|fonts| fonts.layout_job(job));
        self.output_separator_y = (self.output_display_response_start
            < self.output_display_buffer.len())
        .then(|| response_separator_y(&galley, self.output_display_response_chars))
        .flatten();
        self.output_galley = Some(galley);
        self.output_galley_width = Some(wrap_width);
    }

    pub(super) fn sync_input_galley(&mut self, wrap_width: f32) {
        if Self::same_width(self.input_galley_width, wrap_width) && self.input_galley.is_some() {
            return;
        }
        let job = markdown_layout_job(&self.input, wrap_width, &[], 0, &[]);
        self.input_galley = Some(self.ctx.fonts(|fonts| fonts.layout_job(job)));
        self.input_galley_width = Some(wrap_width);
    }

    pub(super) fn text_wrap_width(&self) -> f32 {
        (self.ctx.screen_rect().width()
            - WINDOW_PADDING * 2.0
            - CARD_INNER_PADDING_X
            - TEXT_EDIT_MARGIN_X)
            .max(MIN_TEXT_WRAP_WIDTH)
    }

    pub(super) fn resize_for_text(&mut self) {
        self.resize_for_text_with_width(self.text_wrap_width(), self.auto_resize_height_limit());
    }

    pub(super) fn resize_for_appended_output(&mut self) {
        self.resize_for_text();
    }

    pub(super) fn resize_for_text_with_width(&mut self, wrap_width: f32, max_height: Option<f32>) {
        if self.resizing || self.user_height_override.is_some() {
            return;
        }
        let width = self.ctx.screen_rect().width();
        let (output_rows, _input_rows) = self.display_rows_for_width(wrap_width);
        let separator = if output_rows > 0 { 9.0 } else { 0.0 };
        let mut height = (58.0
            + self.output_height_cache
            + self.input_height_cache
            + self.command_panel_height()
            + separator
            + WINDOW_PADDING
            + WINDOW_BOTTOM_PADDING)
            .clamp(self.min_inner_size().y, MAX_WINDOW_HEIGHT);
        if let Some(max_height) = max_height {
            height = height.min(max_height);
        }
        let size = Vec2::new(width, height);
        if Self::same_size(self.last_inner_size, size) {
            return;
        }
        self.last_inner_size = Some(size);
        self.apply_auto_resize(size);
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
        if !self.ctx.input(|input| input.focused) {
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
