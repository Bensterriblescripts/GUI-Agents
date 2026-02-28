use std::sync::{Arc, atomic::Ordering};
use std::thread;
use std::time::Instant;

use crate::events::{AppEvent, PromptResult};
use crate::logging;
use crate::prompt::{append_cancelled_text, kill_prompt_process, prompt_codex};
use crate::runtime::ensure_codex_files;

use super::AutoAgentApp;
use super::render::trim_string_in_place;

impl AutoAgentApp {
    pub(super) fn submit(&mut self) {
        if self.busy || self.locked || !trim_string_in_place(&mut self.input) {
            return;
        }
        let prompt = std::mem::take(&mut self.input);

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
        if !self.output.is_empty() {
            if !self.output.ends_with('\n') {
                self.output.push_str("\n\n");
            } else if !self.output.ends_with("\n\n") {
                self.output.push('\n');
            }
        }
        let prompt_start = self.output.len();
        self.output.push_str(&prompt);
        self.prompt_ranges.push((prompt_start, self.output.len()));
        self.output.push_str("\n\n");
        self.output_base = self.output.len();
        self.invalidate_output_layout();
        self.resize_for_text();
        self.stream_notification_pending
            .store(false, Ordering::Relaxed);
        {
            let mut stream = self.shared_stream.lock().unwrap_or_else(|e| e.into_inner());
            stream.start(prompt_id);
        }

        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let running_prompt = Arc::clone(&self.running_prompt);
        let shared_stream = Arc::clone(&self.shared_stream);
        let stream_notification_pending = Arc::clone(&self.stream_notification_pending);
        let session_id = self.session_id.clone();
        thread::spawn(move || {
            let result = match prompt_codex(
                prompt_id,
                prompt,
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
            };
            if tx.send(result).is_err() {
                logging::error("failed to deliver prompt result to app");
            }
            ctx.request_repaint();
        });
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
        self.active_prompt_id = None;
        self.busy = false;
        self.locked = false;
        self.pending_started_at = None;
        self.stream_notification_pending
            .store(false, Ordering::Relaxed);
        self.clear_render_buffer();
        append_cancelled_text(&mut self.output);
        self.invalidate_output_layout();
        self.resize_for_text();
        {
            let mut stream = self.shared_stream.lock().unwrap_or_else(|e| e.into_inner());
            stream.clear(running_prompt.id);
        }

        thread::spawn(move || {
            if let Err(error) = kill_prompt_process(running_prompt.pid) {
                logging::error(format!(
                    "failed to cancel prompt pid {}: {}",
                    running_prompt.pid, error
                ));
            }
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
                            let new_output = &self.output[self.output_base..];
                            if stream.text.starts_with(new_output) {
                                self.output.push_str(&stream.text[new_output.len()..]);
                            } else {
                                self.output.truncate(self.output_base);
                                self.output.push_str(&stream.text);
                            }
                            updated = true;
                        }
                    }
                    if updated {
                        self.invalidate_output_layout();
                        self.resize_for_text();
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
                match result {
                    PromptResult::Ok(text, sid) => {
                        self.output.push_str(&text);
                        if sid.is_some() {
                            self.session_id = sid;
                        }
                    }
                    PromptResult::Err(error) => {
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
                self.invalidate_output_layout();
                self.resize_for_text();
            }
        }
    }

    pub(super) fn poll(&mut self) {
        while let Ok(result) = self.rx.try_recv() {
            self.handle_event(result);
        }
    }
}
