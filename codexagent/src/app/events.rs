use std::sync::{Arc, atomic::Ordering};
use std::thread;
use std::time::Instant;

use eframe::egui;

use crate::config::set_notifications_enabled;
use crate::events::{AppEvent, PromptResult};
use crate::logging;
use crate::notify;
use crate::prompt::{append_cancelled_text, kill_prompt_process, prompt_codex};
use crate::runtime::{ensure_codex_files, set_model};
use crate::status::current_usage_text;

use super::CodexAgentApp;
use super::render::trim_string_in_place;

impl CodexAgentApp {
    pub(super) fn submit(&mut self) {
        if self.busy || self.locked || !trim_string_in_place(&mut self.input) {
            return;
        }
        self.clear_picker_selection();
        let prompt = std::mem::take(&mut self.input);
        self.push_prompt_history(&prompt);

        if self.handle_slash_command(&prompt) {
            return;
        }

        if !self.title_set {
            self.title_set = true;
            let title: String = prompt.chars().take(40).collect();
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        if let Err(error) = ensure_codex_files() {
            logging::error(format!("codex file check failed: {}", error));
        }

        logging::trace(format!(
            "submitting prompt with {} chars",
            prompt.chars().count()
        ));
        let prompt_id = self.next_prompt_id;
        self.next_prompt_id += 1;
        self.busy = true;
        self.locked = true;
        self.active_prompt_id = Some(prompt_id);
        self.pending_started_at = Some(Instant::now());
        self.push_prompt_output(&prompt);
        self.persist_history();
        self.refresh_after_text_change();
        self.stream_notification_pending
            .store(false, Ordering::Relaxed);
        {
            let mut stream = self.shared_stream.lock().unwrap_or_else(|e| e.into_inner());
            stream.start(prompt_id);
        }

        let request_prompt = self.build_request_prompt(prompt);
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let running_prompt = Arc::clone(&self.running_prompt);
        let shared_stream = Arc::clone(&self.shared_stream);
        let stream_notification_pending = Arc::clone(&self.stream_notification_pending);
        let session_id = self.session_id.clone();
        thread::spawn(move || {
            let result = match logging::catch_panic("prompt worker thread", || {
                match prompt_codex(
                    prompt_id,
                    request_prompt,
                    session_id,
                    running_prompt,
                    shared_stream,
                    stream_notification_pending,
                    &tx,
                    &ctx,
                ) {
                    Ok((output, sid)) => AppEvent::Prompt(prompt_id, PromptResult::Ok(output, sid)),
                    Err(error) => {
                        logging::error(format!("prompt execution failed: {}", error));
                        AppEvent::Prompt(prompt_id, PromptResult::Err(error.to_string()))
                    }
                }
            }) {
                Ok(result) => result,
                Err(message) => AppEvent::Prompt(prompt_id, PromptResult::Err(message)),
            };
            if tx.send(result).is_err() {
                logging::error("failed to deliver prompt result to app");
            }
            ctx.request_repaint();
        });
    }

    fn handle_slash_command(&mut self, prompt: &str) -> bool {
        match prompt {
            "/status" => {
                self.push_prompt_output(prompt);
                self.output.push_str(&current_usage_text());
                self.persist_history();
            }
            _ => return false,
        }
        self.pending_input_focus = true;
        self.refresh_after_text_change();
        true
    }

    pub(super) fn select_slash_command(&mut self, name: &str) {
        self.clear_picker_selection();
        if name == "status" {
            self.input.clear();
            self.reset_prompt_history_navigation();
            self.handle_slash_command("/status");
            return;
        }
        self.input.clear();
        self.input.push('/');
        self.input.push_str(name);
        self.reset_prompt_history_navigation();
        self.pending_input_focus = true;
        self.refresh_after_input_change();
    }

    pub(super) fn select_model(&mut self, model: &str) {
        self.clear_picker_selection();
        if self.current_model == model {
            return;
        }
        let prompt = format!("settings model {}", model);
        self.apply_model_selection(&prompt, model);
    }

    pub(super) fn select_notification(&mut self, enabled: bool) {
        self.clear_picker_selection();
        if self.notifications_enabled == enabled {
            return;
        }
        match set_notifications_enabled(enabled) {
            Ok(enabled) => {
                self.notifications_enabled = enabled;
                self.output.push_str("Notification set to ");
                self.output.push_str(if enabled { "On" } else { "Off" });
                self.input.clear();
                self.reset_prompt_history_navigation();
            }
            Err(error) => {
                logging::error(format!(
                    "failed to set notification {}: {}",
                    if enabled { "on" } else { "off" },
                    error
                ));
                self.output.push('\x1D');
                self.output.push_str("Failed to set notification: ");
                self.output.push_str(&error.to_string());
            }
        }
        self.persist_history();
        self.pending_input_focus = true;
        self.refresh_after_text_change();
    }

    fn apply_model_selection(&mut self, prompt: &str, model: &str) {
        self.clear_picker_selection();
        self.push_prompt_output(prompt);
        match set_model(model) {
            Ok(model) => {
                self.current_model = model.clone();
                self.output.push_str("Model set to ");
                self.output.push_str(&model);
                self.input.clear();
                self.reset_prompt_history_navigation();
            }
            Err(error) => {
                logging::error(format!("failed to set model {}: {}", model, error));
                self.output.push('\x1D');
                self.output.push_str("Failed to set model: ");
                self.output.push_str(&error.to_string());
            }
        }
        self.persist_history();
        self.pending_input_focus = true;
        self.refresh_after_text_change();
    }

    fn push_prompt_output(&mut self, prompt: &str) {
        self.output.reserve(prompt.len() + 2);
        if !self.output.is_empty() {
            if !self.output.ends_with('\n') {
                self.output.push_str("\n\n");
            } else if !self.output.ends_with("\n\n") {
                self.output.push('\n');
            }
        }
        let prompt_start = self.output.len();
        self.output.push_str(prompt);
        self.prompt_ranges.push((prompt_start, self.output.len()));
        self.output.push_str("\n\n");
        self.output_base = self.output.len();
        self.output_display_can_append = false;
    }

    pub(super) fn cancel_active_prompt(&mut self) {
        let running_prompt = {
            let mut active = self
                .running_prompt
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            active.take()
        };
        let Some(running_prompt) = running_prompt else {
            return;
        };

        logging::trace(format!("canceling prompt pid {}", running_prompt.pid));
        if let Some(session_id) = running_prompt.session_id {
            self.session_id = Some(session_id);
            self.cancelled_resume_context = None;
        } else {
            self.capture_cancelled_resume_context();
        }
        self.active_prompt_id = None;
        self.busy = false;
        self.locked = false;
        self.pending_started_at = None;
        self.stream_notification_pending
            .store(false, Ordering::Relaxed);
        self.clear_render_buffer();
        append_cancelled_text(&mut self.output);
        self.persist_history();
        self.refresh_after_output_change();
        {
            let mut stream = self.shared_stream.lock().unwrap_or_else(|e| e.into_inner());
            stream.clear(running_prompt.id);
        }

        thread::spawn(move || {
            let _ = logging::catch_panic("prompt cancel thread", || {
                if let Err(error) = kill_prompt_process(running_prompt.pid) {
                    logging::error(format!(
                        "failed to cancel prompt pid {}: {}",
                        running_prompt.pid, error
                    ));
                }
            });
        });
    }

    fn handle_event(&mut self, result: AppEvent) {
        match result {
            AppEvent::PromptStream(prompt_id) => {
                self.stream_notification_pending
                    .store(false, Ordering::Relaxed);
                if self.active_prompt_id == Some(prompt_id) {
                    let mut updated = false;
                    {
                        let stream = self.shared_stream.lock().unwrap_or_else(|e| e.into_inner());
                        if stream.prompt_id == Some(prompt_id) {
                            let next = stream.text.as_str();
                            if self.output.get(self.output_base..) != Some(next) {
                                let append_from = {
                                    let current = &self.output[self.output_base..];
                                    next.strip_prefix(current)
                                        .map(|suffix| next.len() - suffix.len())
                                };
                                if let Some(start) = append_from {
                                    self.output.push_str(&next[start..]);
                                } else {
                                    self.output.truncate(self.output_base);
                                    self.output.push_str(next);
                                    self.output_display_can_append = false;
                                }
                                updated = true;
                            }
                        }
                    }
                    if updated {
                        self.refresh_after_output_change();
                    }
                }
            }
            AppEvent::Prompt(prompt_id, result) => {
                if self.active_prompt_id != Some(prompt_id) {
                    return;
                }
                match &result {
                    PromptResult::Ok(output, _) => logging::trace(format!(
                        "prompt completed; {} chars returned",
                        output.chars().count()
                    )),
                    PromptResult::Err(error) => {
                        logging::error(format!("prompt completed with error: {}", error))
                    }
                }
                self.busy = false;
                self.locked = false;
                self.pending_input_focus = true;
                self.output.truncate(self.output_base);
                self.output_display_can_append = false;
                match result {
                    PromptResult::Ok(text, sid) => {
                        self.output.reserve(text.len());
                        self.output.push_str(&text);
                        if sid.is_some() {
                            self.session_id = sid;
                            self.cancelled_resume_context = None;
                        }
                    }
                    PromptResult::Err(error) => {
                        self.output
                            .reserve(error.len() + error.lines().count().max(1));
                        for line in error.split_inclusive('\n') {
                            self.output.push('\x1D');
                            self.output.push_str(line);
                        }
                    }
                }
                self.active_prompt_id = None;
                self.pending_started_at = None;
                self.stream_notification_pending
                    .store(false, Ordering::Relaxed);
                self.clear_render_buffer();
                {
                    let mut stream = self.shared_stream.lock().unwrap_or_else(|e| e.into_inner());
                    stream.clear(prompt_id);
                }
                self.persist_history();
                self.refresh_after_output_change();
                if self.notifications_enabled {
                    notify::prompt_completed(self.hwnd);
                }
            }
        }
    }

    pub(super) fn poll(&mut self) {
        while let Ok(result) = self.rx.try_recv() {
            self.handle_event(result);
        }
    }
}
