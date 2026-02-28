use std::process::Child;
use std::sync::{Arc, Mutex};
use std::{io, thread};

use crate::logging;

#[derive(Clone, Copy)]
pub(crate) struct RunningPrompt {
    pub(crate) id: u64,
    pub(crate) pid: u32,
}

#[derive(Default)]
pub(crate) struct PromptStreamState {
    pub(crate) prompt_id: Option<u64>,
    pub(crate) text: String,
}

const RETAINED_STREAM_CAPACITY: usize = 1024;
const MAX_IDLE_STREAM_CAPACITY: usize = 16 * 1024;

impl PromptStreamState {
    pub(crate) fn start(&mut self, prompt_id: u64) {
        self.prompt_id = Some(prompt_id);
        self.text.clear();
    }

    pub(crate) fn update(&mut self, prompt_id: u64, text: &str) -> bool {
        if self.prompt_id != Some(prompt_id) {
            return false;
        }
        if text.starts_with(&self.text) {
            self.text.push_str(&text[self.text.len()..]);
        } else {
            self.text.clear();
            self.text.push_str(text);
        }
        true
    }

    pub(crate) fn clear(&mut self, prompt_id: u64) {
        if self.prompt_id == Some(prompt_id) {
            self.prompt_id = None;
            self.text.clear();
            if self.text.capacity() > MAX_IDLE_STREAM_CAPACITY {
                self.text.shrink_to(RETAINED_STREAM_CAPACITY);
            }
        }
    }
}

pub(super) struct RunningPromptGuard {
    pub(super) prompt_id: u64,
    pub(super) running_prompt: Arc<Mutex<Option<RunningPrompt>>>,
}

impl Drop for RunningPromptGuard {
    fn drop(&mut self) {
        clear_running_prompt(&self.running_prompt, self.prompt_id);
    }
}

pub(super) struct PromptProcessGuard {
    pub(super) child: Option<Child>,
    pub(super) stderr_handle: Option<thread::JoinHandle<io::Result<String>>>,
}

impl Drop for PromptProcessGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            match child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => {
                    if let Err(e) = child.kill() {
                        logging::error(format!("failed to kill child process: {}", e));
                    }
                    let _ = child.wait();
                }
                Err(e) => {
                    logging::error(format!("failed to check child process status: {}", e));
                    if let Err(e) = child.kill() {
                        logging::error(format!("failed to kill child process: {}", e));
                    }
                    let _ = child.wait();
                }
            }
        }
        if let Some(handle) = self.stderr_handle.take() {
            if handle.join().is_err() {
                logging::error("stderr reader thread panicked during cleanup");
            }
        }
    }
}

fn clear_running_prompt(running_prompt: &Arc<Mutex<Option<RunningPrompt>>>, prompt_id: u64) {
    let mut active = running_prompt.lock().unwrap_or_else(|e| e.into_inner());
    if active.as_ref().is_some_and(|prompt| prompt.id == prompt_id) {
        *active = None;
    }
}
