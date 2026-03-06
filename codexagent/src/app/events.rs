use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Instant;

use eframe::egui;

use crate::config::set_notifications_enabled;
use crate::events::{AppEvent, CodexCheckResult, PromptResult};
use crate::logging;
use crate::notify;
use crate::prompt::{
    append_cancelled_text, check_codex_availability, has_node, kill_prompt_process, prompt_codex,
    run_full_install,
};
use crate::runtime::{
    ContextMenuSelection, current_context_menu_selection, ensure_codex_files, install_context_menu,
    remove_context_menu, set_model,
};
use crate::status::current_usage_text;

use super::render::trim_string_in_place;
use super::{CodexAgentApp, ContextMenuState, SetupState};

impl CodexAgentApp {
    pub(super) fn submit(&mut self) {
        if self.busy || self.locked || !trim_string_in_place(&mut self.input) {
            return;
        }
        self.clear_picker_selection();
        let prompt = std::mem::take(&mut self.input);
        if self.try_run_local_command(&prompt) {
            return;
        }
        self.push_prompt_history(&prompt);

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
            self.stream_generation = stream.generation;
            self.stream_visible_len = 0;
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

    pub(super) fn show_status(&mut self) {
        if self.busy || self.locked {
            return;
        }
        self.append_status_output(false);
    }

    pub(super) fn select_slash_command(&mut self, name: &str) {
        self.clear_picker_selection();
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
        self.apply_model_selection(model);
    }

    pub(super) fn select_notification(&mut self, enabled: bool) {
        self.clear_picker_selection();
        if self.notifications_enabled == enabled {
            return;
        }
        match set_notifications_enabled(enabled) {
            Ok(enabled) => {
                self.notifications_enabled = enabled;
                self.push_settings_output(if enabled {
                    "Notification set to On"
                } else {
                    "Notification set to Off"
                });
                self.finish_local_success();
            }
            Err(error) => {
                logging::error(format!(
                    "failed to set notification {}: {}",
                    if enabled { "on" } else { "off" },
                    error
                ));
                self.push_local_error(&format!("Failed to set notification: {}", error));
            }
        }
        self.finish_local_change();
    }

    pub(super) fn select_context_menu(&mut self, enabled: bool) {
        self.clear_picker_selection();
        let result = if enabled {
            install_context_menu()
        } else {
            remove_context_menu()
        };
        match result {
            Ok(_) => {
                self.context_menu_state = if enabled {
                    ContextMenuState::Add
                } else {
                    ContextMenuState::Remove
                };
                self.context_menu_refresh_pending = false;
                self.refresh_context_menu_state_async();
                self.push_settings_output(if enabled {
                    "Context menu added"
                } else {
                    "Context menu removed"
                });
                self.finish_local_success();
            }
            Err(error) => {
                logging::error(format!(
                    "failed to {} context menu: {}",
                    if enabled { "add" } else { "remove" },
                    error
                ));
                self.push_local_error(&format!(
                    "Failed to {} context menu: {}",
                    if enabled { "add" } else { "remove" },
                    error
                ));
            }
        }
        self.finish_local_change();
    }

    fn apply_model_selection(&mut self, model: &str) {
        match set_model(model) {
            Ok(model) => {
                self.current_model = model.clone();
                let message = format!("Model set to {}", model);
                self.push_settings_output(&message);
                self.finish_local_success();
            }
            Err(error) => {
                logging::error(format!("failed to set model {}: {}", model, error));
                self.push_local_error(&format!("Failed to set model: {}", error));
            }
        }
        self.finish_local_change();
    }

    fn finish_local_change(&mut self) {
        self.persist_history();
        self.pending_input_focus = true;
        self.refresh_after_text_change();
    }

    fn finish_local_success(&mut self) {
        self.input.clear();
        self.reset_prompt_history_navigation();
    }

    fn push_local_error(&mut self, message: &str) {
        self.output.push('\x1D');
        self.output.push_str(message);
    }

    fn push_settings_output(&mut self, message: &str) {
        self.ensure_output_spacing();
        self.output.push('\x1C');
        self.output.push_str(message);
        self.output.push_str("\n\n");
    }

    fn ensure_output_spacing(&mut self) {
        if self.output.is_empty() {
            return;
        }
        if !self.output.ends_with('\n') {
            self.output.push_str("\n\n");
        } else if !self.output.ends_with("\n\n") {
            self.output.push('\n');
        }
    }

    fn finish_prompt(&mut self, prompt_id: u64) {
        self.active_prompt_id = None;
        self.pending_started_at = None;
        self.stream_notification_pending
            .store(false, Ordering::Relaxed);
        self.clear_render_buffer();
        self.reset_stream_progress();
        let mut stream = self.shared_stream.lock().unwrap_or_else(|e| e.into_inner());
        stream.clear(prompt_id);
    }

    fn start_install_flow(&mut self, node_available: bool) {
        self.setup_state = SetupState::Installing;
        self.clear_output_buffers();
        self.output.push_str("Installing Codex CLI...\n\n");
        self.refresh_after_text_change();
        self.spawn_install(node_available);
    }

    fn spawn_install(&self, node_available: bool) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let install_stdin = Arc::clone(&self.install_stdin);
        thread::spawn(move || {
            let result = run_full_install(node_available, &tx, &ctx, &install_stdin);
            if tx.send(AppEvent::CodexInstallDone(result)).is_err() {
                logging::error("failed to deliver install completion to app");
            }
            ctx.request_repaint();
        });
    }

    fn spawn_codex_check(&self) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let result = check_codex_availability();
            if tx.send(AppEvent::CodexCheck(result)).is_err() {
                logging::error("failed to deliver codex check result to app");
            }
            ctx.request_repaint();
        });
    }

    pub(super) fn refresh_context_menu_state_async(&mut self) {
        if self.context_menu_refresh_pending {
            return;
        }
        self.context_menu_refresh_pending = true;
        self.context_menu_state = ContextMenuState::Checking;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let result = current_context_menu_selection().map_err(|error| error.to_string());
            if tx.send(AppEvent::ContextMenuSelection(result)).is_err() {
                logging::error("failed to deliver context menu state to app");
            }
            ctx.request_repaint();
        });
    }

    fn try_run_local_command(&mut self, prompt: &str) -> bool {
        if prompt == "/status" {
            self.append_status_output(true);
            return true;
        }
        false
    }

    fn append_status_output(&mut self, add_to_history: bool) {
        if add_to_history {
            self.push_prompt_history("/status");
        } else {
            self.reset_prompt_history_navigation();
        }
        self.ensure_output_spacing();
        self.output_base = self.output.len();
        self.output.push_str(&current_usage_text());
        self.mark_output_for_rebuild();
        self.input.clear();
        self.finish_local_change();
    }

    fn push_prompt_output(&mut self, prompt: &str) {
        self.output.reserve(prompt.len() + 2);
        self.ensure_output_spacing();
        let prompt_start = self.output.len();
        self.output.push_str(prompt);
        self.prompt_ranges.push((prompt_start, self.output.len()));
        self.output.push_str("\n\n");
        self.output_base = self.output.len();
        self.mark_output_for_rebuild();
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
        self.reset_stream_progress();
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
                            let next_len = next.len();
                            let needs_replace = self.stream_generation != stream.generation
                                || self.stream_visible_len > next_len;
                            if needs_replace {
                                if self.output.get(self.output_base..) != Some(next) {
                                    self.output.truncate(self.output_base);
                                    self.output.push_str(next);
                                    self.output_display_can_append = false;
                                    updated = true;
                                }
                                self.stream_generation = stream.generation;
                                self.stream_visible_len = next_len;
                            } else if next_len > self.stream_visible_len {
                                self.output.push_str(&next[self.stream_visible_len..]);
                                self.stream_visible_len = next_len;
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
                self.mark_output_for_rebuild();
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
                        if self.setup_state == SetupState::Ready && is_codex_missing(&error) {
                            self.finish_prompt(prompt_id);
                            self.start_install_flow(has_node());
                            return;
                        }
                        self.output
                            .reserve(error.len() + error.lines().count().max(1));
                        for line in error.split_inclusive('\n') {
                            self.output.push('\x1D');
                            self.output.push_str(line);
                        }
                    }
                }
                self.finish_prompt(prompt_id);
                self.persist_history();
                self.refresh_after_output_change();
                if self.notifications_enabled {
                    notify::prompt_completed(self.hwnd);
                }
            }
            AppEvent::CodexCheck(result) => match result {
                CodexCheckResult::Ready => {
                    self.setup_state = SetupState::Ready;
                    self.locked = false;
                    self.clear_output_buffers();
                    self.pending_input_focus = true;
                    self.refresh_after_text_change();
                }
                CodexCheckResult::NotInstalled { node_available } => {
                    self.start_install_flow(node_available);
                }
            },
            AppEvent::CodexInstallOutput(line) => {
                if matches!(self.setup_state, SetupState::Installing) {
                    self.output.push_str(&line);
                    self.output.push('\n');
                    self.refresh_after_output_rewrite();
                }
            }
            AppEvent::CodexInstallDone(result) => {
                match result {
                    Ok(()) => {
                        self.output
                            .push_str("\nInstallation complete. Verifying...");
                        self.setup_state = SetupState::Checking;
                        self.spawn_codex_check();
                    }
                    Err(msg) => {
                        self.setup_state = SetupState::InstallFailed(msg.clone());
                        self.output.push_str("\nInstallation failed: ");
                        self.output.push_str(&msg);
                    }
                }
                self.refresh_after_output_rewrite();
            }
            AppEvent::ContextMenuSelection(result) => {
                self.context_menu_refresh_pending = false;
                match result {
                    Ok(ContextMenuSelection::Add) => {
                        self.context_menu_state = ContextMenuState::Add;
                    }
                    Ok(ContextMenuSelection::Remove) => {
                        self.context_menu_state = ContextMenuState::Remove;
                    }
                    Err(error) => {
                        self.context_menu_state = ContextMenuState::Error;
                        logging::error(format!("failed to refresh context menu state: {}", error));
                    }
                }
            }
        }
    }

    pub(super) fn poll(&mut self) {
        while let Ok(result) = self.rx.try_recv() {
            self.handle_event(result);
        }
    }

    pub(super) fn start_codex_install(&mut self) {
        self.start_install_flow(false);
    }

    pub(super) fn send_install_input(&mut self) {
        let input = std::mem::take(&mut self.input);
        {
            let mut guard = self.install_stdin.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(stdin) = guard.as_mut() {
                use std::io::Write;
                if let Err(error) = stdin.write_all(input.as_bytes()) {
                    logging::error(format!("failed to write installer input: {}", error));
                } else if let Err(error) = stdin.write_all(b"\n") {
                    logging::error(format!("failed to terminate installer input: {}", error));
                } else if let Err(error) = stdin.flush() {
                    logging::error(format!("failed to flush installer input: {}", error));
                }
            } else {
                logging::error("installer input unavailable");
            }
        }
        self.output.push_str(&input);
        self.output.push('\n');
        self.mark_output_for_rebuild();
        self.pending_input_focus = true;
        self.refresh_after_text_change();
    }
}

fn is_codex_missing(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("cannot find the file specified")
        || error.contains("file not found")
        || error.contains("program is not recognized")
        || error.contains("program not found")
        || error.contains("is not recognized as an internal or external command")
}
