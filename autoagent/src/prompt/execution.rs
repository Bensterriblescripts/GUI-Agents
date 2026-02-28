use std::io::{self, BufRead, Read};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;

use eframe::egui;
use serde_json::Value;
use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, SetThreadExecutionState,
};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::config::CANCELLED_TEXT;
use crate::events::AppEvent;
use crate::logging;
use crate::runtime::current_cwd_text;

use super::buffers::{ResponseBuffers, collect_response_text};
use super::codex::build_codex_command;
use super::state::{PromptProcessGuard, PromptStreamState, RunningPrompt, RunningPromptGuard};

struct DisplayWakeGuard {
    active: bool,
}

impl DisplayWakeGuard {
    fn enable() -> Self {
        let state = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED) };
        if state == 0 {
            logging::error("SetThreadExecutionState failed to set display wake lock");
        }
        Self { active: state != 0 }
    }
}

impl Drop for DisplayWakeGuard {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
        }
    }
}

pub(crate) fn prompt_codex(
    prompt_id: u64,
    prompt: String,
    session_id: Option<String>,
    running_prompt: Arc<Mutex<Option<RunningPrompt>>>,
    shared_stream: Arc<Mutex<PromptStreamState>>,
    stream_notification_pending: Arc<AtomicBool>,
    tx: &mpsc::Sender<AppEvent>,
    ctx: &egui::Context,
) -> io::Result<(String, Option<String>)> {
    let _display_wake = DisplayWakeGuard::enable();
    logging::trace(format!(
        "starting codex exec from {} with {} chars",
        current_cwd_text(),
        prompt.chars().count()
    ));
    let child = build_codex_command(&prompt, session_id.as_deref())
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut process = PromptProcessGuard {
        child: Some(child),
        stderr_handle: None,
    };
    {
        let mut active = running_prompt.lock().unwrap_or_else(|e| e.into_inner());
        *active = Some(RunningPrompt {
            id: prompt_id,
            pid: process.child.as_ref().expect("child just spawned").id(),
        });
    }
    let _running_prompt_guard = RunningPromptGuard {
        prompt_id,
        running_prompt,
    };

    let stdout = process
        .child
        .as_mut()
        .expect("child just spawned")
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Missing stdout pipe"))?;
    let stderr = process
        .child
        .as_mut()
        .expect("child just spawned")
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("Missing stderr pipe"))?;
    let stderr_handle = thread::spawn(move || -> io::Result<String> {
        let mut stderr = io::BufReader::new(stderr);
        let mut collected = String::new();
        let mut buffer = [0u8; 4096];
        loop {
            let read = stderr.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let text = String::from_utf8_lossy(&buffer[..read]);
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                logging::trace(format!("codex stderr: {}", trimmed));
                if !collected.is_empty() {
                    collected.push('\n');
                }
                collected.push_str(trimmed);
            }
        }
        Ok(collected)
    });
    process.stderr_handle = Some(stderr_handle);

    let mut stdout = io::BufReader::new(stdout);
    let mut line_number = 0usize;
    let mut response = ResponseBuffers::default();
    let mut failure_message = None;
    let mut resolved_session_id = session_id;
    let mut line = String::new();

    loop {
        line.clear();
        if stdout.read_line(&mut line)? == 0 {
            break;
        }
        line_number += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let previous_visible_len = response.visible_len();
        let previous_has_deltas = response.has_deltas();
        let event: Value = serde_json::from_str(trimmed).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Invalid JSON event on line {}: {}; line: {}",
                    line_number, error, trimmed
                ),
            )
        })?;

        if let Some(kind) = event.get("type").and_then(Value::as_str) {
            if kind == "error" && failure_message.is_none() {
                failure_message = event
                    .get("message")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                if let Some(message) = failure_message.as_ref() {
                    logging::error(format!("codex reported error event: {}", message));
                }
            }
            if kind == "thread.started" && resolved_session_id.is_none() {
                if let Some(tid) = event.get("thread_id").and_then(Value::as_str) {
                    logging::trace(format!("captured session id: {}", tid));
                    resolved_session_id = Some(tid.to_owned());
                }
            }
        }
        collect_response_text(&event, &mut response);
        let visible_len = response.visible_len();
        let has_deltas = response.has_deltas();
        if visible_len != 0
            && (visible_len != previous_visible_len || has_deltas != previous_has_deltas)
        {
            let visible_text = response.visible_text();
            let updated = {
                let mut stream = shared_stream.lock().unwrap_or_else(|e| e.into_inner());
                stream.update(prompt_id, visible_text)
            };
            if updated {
                logging::trace(format!("stream update: {} visible chars", visible_len));
                if !stream_notification_pending.swap(true, Ordering::Relaxed) {
                    let _ = tx.send(AppEvent::PromptStream(prompt_id));
                }
                ctx.request_repaint();
            }
        }
    }

    let status = process.child.as_mut().expect("child is active").wait()?;
    logging::trace(format!("codex process exited with {}", status));
    let stderr_handle = process
        .stderr_handle
        .take()
        .expect("stderr reader should exist");
    let stderr_text = join_stderr_reader(stderr_handle)?;
    let _ = process.child.take();

    if !status.success() {
        let message = if !stderr_text.is_empty() {
            stderr_text
        } else {
            failure_message.unwrap_or_else(|| format!("codex exited with {}", status))
        };
        logging::error(format!("codex exec failed: {}", message));
        return Err(io::Error::other(message));
    }

    let response = response.into_response();
    if !response.is_empty() {
        logging::trace(format!(
            "codex exec completed with {} chars",
            response.chars().count()
        ));
        return Ok((response, resolved_session_id));
    }
    logging::trace("codex exec completed with empty output");
    Ok((response, resolved_session_id))
}

pub(crate) fn kill_prompt_process(pid: u32) -> io::Result<()> {
    let status = Command::new("taskkill")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()?;
    if status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!("taskkill exited with {}", status)))
}

pub(crate) fn append_cancelled_text(input: &mut String) {
    if input == "..." {
        input.clear();
    }
    if !input.is_empty() {
        if !input.ends_with('\n') {
            input.push('\n');
        }
        input.push('\n');
    }
    input.push_str(CANCELLED_TEXT);
}

fn join_stderr_reader(handle: thread::JoinHandle<io::Result<String>>) -> io::Result<String> {
    handle
        .join()
        .map_err(|_| io::Error::other("stderr reader thread panicked"))?
}
