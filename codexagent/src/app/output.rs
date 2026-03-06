use std::sync::atomic::Ordering;
use std::time::Duration;

use eframe::egui::{self, Vec2};

use crate::config::{
    APP_NAME, CANCELLED_TEXT, PENDING_ANIMATION_INTERVAL, save_prompt_history_prompts,
};
use crate::logging;

use super::render::{append_output_display, pending_dots, prepare_output_display};
use super::{CodexAgentApp, SetupState};

const RETAINED_RENDER_CAPACITY: usize = 1024;
const MAX_IDLE_RENDER_CAPACITY: usize = 16 * 1024;
const LAYOUT_EPSILON: f32 = 0.1;

impl CodexAgentApp {
    pub(super) fn clear_output_buffers(&mut self) {
        self.output.clear();
        self.output_base = 0;
        self.prompt_ranges.clear();
        self.output_display_buffer.clear();
        self.output_display_prompt_ranges.clear();
        self.output_display_line_kinds.clear();
        self.output_display_response_start = 0;
        self.output_display_response_chars = 0;
        self.output_display_base_len = 0;
        self.output_display_source_len = 0;
        self.output_display_can_append = false;
        self.output_display_dirty = true;
        self.output_display_busy = false;
        self.output_galley = None;
        self.output_galley_width = None;
        self.output_separator_y = None;
        self.reset_stream_progress();
    }

    pub(super) fn mark_output_for_rebuild(&mut self) {
        self.output_display_can_append = false;
    }

    pub(super) fn refresh_after_output_rewrite(&mut self) {
        self.mark_output_for_rebuild();
        self.refresh_after_output_change();
    }

    pub(super) fn invalidate_text_layout(&mut self) {
        self.text_layout_dirty = true;
        self.output_galley = None;
        self.output_galley_width = None;
        self.output_separator_y = None;
        self.input_galley = None;
        self.input_galley_width = None;
    }

    pub(super) fn invalidate_input_layout(&mut self) {
        self.text_layout_dirty = true;
        self.input_galley = None;
        self.input_galley_width = None;
    }

    pub(super) fn invalidate_output_layout(&mut self) {
        self.text_layout_dirty = true;
        self.output_galley = None;
        self.output_galley_width = None;
        self.output_separator_y = None;
        self.output_display_dirty = true;
    }

    pub(super) fn can_clear(&self) -> bool {
        !self.busy
            && self.setup_state == SetupState::Ready
            && (!self.output.is_empty() || self.session_id.is_some())
    }

    pub(super) fn clear_session(&mut self) {
        self.input.clear();
        self.reset_prompt_history_navigation();
        self.clear_output_buffers();
        self.session_id = None;
        self.cancelled_resume_context = None;
        self.active_prompt_id = None;
        self.locked = false;
        self.pending_started_at = None;
        self.pending_input_focus = true;
        self.title_set = false;
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::Title(APP_NAME.to_owned()));
        self.stream_notification_pending
            .store(false, Ordering::Relaxed);
        self.clear_render_buffer();
        {
            let mut stream = self.shared_stream.lock().unwrap_or_else(|e| e.into_inner());
            stream.reset();
        }
        self.persist_history();
        self.refresh_after_text_change();
    }

    pub(super) fn build_request_prompt(&self, prompt: String) -> String {
        if self.session_id.is_some() {
            return prompt;
        }
        let Some(context) = self.cancelled_resume_context.as_deref() else {
            return prompt;
        };
        if context.is_empty() {
            return prompt;
        }
        let mut wrapped = String::with_capacity(context.len() + prompt.len() + 192);
        wrapped.push_str("Continue this conversation from the transcript below. The previous reply was cancelled before a resumable conversation id was available.\n\nConversation transcript:\n");
        wrapped.push_str(context);
        wrapped.push_str("\n\nNext user message:\n");
        wrapped.push_str(&prompt);
        wrapped
    }

    pub(super) fn capture_cancelled_resume_context(&mut self) {
        if self.session_id.is_some() {
            self.cancelled_resume_context = None;
            return;
        }
        let context = build_resume_context(&self.output, &self.prompt_ranges);
        self.cancelled_resume_context = (!context.is_empty()).then_some(context);
    }

    pub(super) fn pending_step(&self) -> Option<u128> {
        self.busy.then(|| {
            self.pending_started_at
                .map(|started| {
                    started.elapsed().as_millis() / PENDING_ANIMATION_INTERVAL.as_millis()
                })
                .unwrap_or(0)
        })
    }

    pub(super) fn pending_repaint_delay(&self) -> Option<Duration> {
        if !self.busy {
            return None;
        }
        let interval = PENDING_ANIMATION_INTERVAL;
        let interval_ms = interval.as_millis();
        if interval_ms == 0 {
            return Some(Duration::ZERO);
        }
        let elapsed_ms = self
            .pending_started_at
            .map(|started| started.elapsed().as_millis())
            .unwrap_or(0);
        let remaining_ms = interval_ms - (elapsed_ms % interval_ms);
        Some(Duration::from_millis(remaining_ms as u64))
    }

    pub(super) fn sync_render_buffer(&mut self, step: u128) {
        self.render_buffer.clear();
        let dots = pending_dots(step);
        if self.output.is_empty() {
            self.render_buffer.push_str(dots);
            self.output_galley = None;
            return;
        }
        if !self.output.ends_with('\n') {
            self.render_buffer.push('\n');
        }
        self.render_buffer.push_str(dots);
        self.output_galley = None;
    }

    pub(super) fn clear_render_buffer(&mut self) {
        self.render_buffer.clear();
        self.render_step = None;
        if self.render_buffer.capacity() > MAX_IDLE_RENDER_CAPACITY {
            self.render_buffer.shrink_to(RETAINED_RENDER_CAPACITY);
        }
    }

    pub(super) fn refresh_after_input_change(&mut self) {
        self.invalidate_input_layout();
        self.resize_for_text();
    }

    pub(super) fn refresh_after_output_change(&mut self) {
        self.invalidate_output_layout();
        self.resize_for_appended_output();
    }

    pub(super) fn refresh_after_text_change(&mut self) {
        self.invalidate_input_layout();
        self.refresh_after_output_change();
    }

    pub(super) fn reset_stream_progress(&mut self) {
        self.stream_generation = 0;
        self.stream_visible_len = 0;
    }

    pub(super) fn sync_output_display_buffer(&mut self) {
        if self.output_display_dirty {
            truncate_output_display_suffix(
                &mut self.output_display_buffer,
                self.output_display_base_len,
                &mut self.output_display_busy,
            );
            if self.output_display_can_append && self.output.len() >= self.output_display_source_len
            {
                let previous_len = self.output_display_source_len;
                let line_start = previous_len == 0
                    || self
                        .output
                        .as_bytes()
                        .get(previous_len.saturating_sub(1))
                        .is_some_and(|byte| *byte == b'\n');
                append_output_display(
                    &self.output[previous_len..],
                    line_start,
                    &mut self.output_display_buffer,
                    &mut self.output_display_line_kinds,
                );
                self.output_display_base_len = self.output_display_buffer.len();
                self.output_display_source_len = self.output.len();
            } else {
                self.output_display_response_start = prepare_output_display(
                    &self.output,
                    &self.prompt_ranges,
                    self.output_base,
                    &mut self.output_display_buffer,
                    &mut self.output_display_prompt_ranges,
                    &mut self.output_display_line_kinds,
                );
                self.output_display_response_chars = self.output_display_buffer
                    [..self.output_display_response_start]
                    .chars()
                    .count();
                self.output_display_base_len = self.output_display_buffer.len();
                self.output_display_source_len = self.output.len();
            }
            self.output_display_can_append = true;
            self.output_display_dirty = false;
            self.output_galley = None;
            self.output_separator_y = None;
        }
        if self.busy {
            let suffix = self.render_buffer.as_str();
            let display = self.output_display_buffer.as_str();
            let needs_suffix = !self.output_display_busy
                || !display
                    .get(self.output_display_base_len..)
                    .is_some_and(|current| current == suffix);
            if needs_suffix {
                self.output_display_buffer
                    .truncate(self.output_display_base_len);
                self.output_display_buffer.push_str(suffix);
                self.output_display_busy = true;
                self.output_galley = None;
                self.output_separator_y = None;
            }
            return;
        }
        if self.output_display_busy {
            truncate_output_display_suffix(
                &mut self.output_display_buffer,
                self.output_display_base_len,
                &mut self.output_display_busy,
            );
            self.output_galley = None;
            self.output_separator_y = None;
        }
    }

    pub(super) fn persist_history(&self) {
        if let Err(error) = save_prompt_history_prompts(&self.prompt_history) {
            logging::error(format!("failed to save prompt history: {}", error));
        }
    }

    pub(super) fn same_axis(lhs: f32, rhs: f32) -> bool {
        (lhs - rhs).abs() <= LAYOUT_EPSILON
    }

    pub(super) fn same_width(slot: Option<f32>, width: f32) -> bool {
        slot.is_some_and(|current| Self::same_axis(current, width))
    }

    pub(super) fn same_size(slot: Option<Vec2>, size: Vec2) -> bool {
        slot.is_some_and(|current| {
            Self::same_axis(current.x, size.x) && Self::same_axis(current.y, size.y)
        })
    }

    pub(super) fn same_rect(lhs: Option<egui::Rect>, rhs: Option<egui::Rect>) -> bool {
        match (lhs, rhs) {
            (Some(lhs), Some(rhs)) => {
                Self::same_axis(lhs.min.x, rhs.min.x)
                    && Self::same_axis(lhs.min.y, rhs.min.y)
                    && Self::same_axis(lhs.max.x, rhs.max.x)
                    && Self::same_axis(lhs.max.y, rhs.max.y)
            }
            (None, None) => true,
            _ => false,
        }
    }
}

fn truncate_output_display_suffix(buffer: &mut String, base_len: usize, busy: &mut bool) {
    if !*busy {
        return;
    }
    buffer.truncate(base_len);
    *busy = false;
}

fn build_resume_context(output: &str, prompt_ranges: &[(usize, usize)]) -> String {
    let mut transcript = String::new();
    for (index, &(start, end)) in prompt_ranges.iter().enumerate() {
        let Some(prompt) = output.get(start..end) else {
            continue;
        };
        let prompt = prompt.trim();
        if prompt.is_empty() {
            continue;
        }
        if !transcript.is_empty() {
            transcript.push_str("\n\n");
        }
        transcript.push_str("User:\n");
        transcript.push_str(prompt);
        let response_end = prompt_ranges
            .get(index + 1)
            .map(|(next_start, _)| *next_start)
            .unwrap_or(output.len());
        let Some(response) = output.get(end..response_end) else {
            continue;
        };
        append_resume_response(&mut transcript, response.trim_matches('\n'));
    }
    transcript
}

fn append_resume_response(transcript: &mut String, response: &str) {
    if response.is_empty() {
        return;
    }
    let mut current_label = "";
    for line in response.lines() {
        let (label, content) = match line.chars().next() {
            Some('\x1D') => ("System", &line[1..]),
            Some('\x1E') => ("Assistant reasoning", &line[1..]),
            Some('\x1F') => ("Assistant note", &line[1..]),
            _ if line == CANCELLED_TEXT => ("System", line),
            _ => ("Assistant", line),
        };
        if content.is_empty() && current_label.is_empty() {
            continue;
        }
        if label != current_label {
            transcript.push_str("\n\n");
            transcript.push_str(label);
            transcript.push_str(":\n");
            current_label = label;
        } else if !transcript.ends_with('\n') {
            transcript.push('\n');
        }
        transcript.push_str(content);
    }
}
