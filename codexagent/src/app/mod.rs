mod events;
mod layout;
mod position;
mod render;
mod ui;
mod window;

use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::process::ChildStdin;
use std::sync::{Arc, Mutex, atomic::AtomicBool, mpsc};
use std::time::{Duration, Instant};

use eframe::egui::{self, Vec2};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::config::{
    APP_NAME, DEFAULT_NOTIFICATIONS_ENABLED, LINE_HEIGHT, PENDING_ANIMATION_INTERVAL,
    PromptHistory, load_notifications_enabled, load_prompt_history, save_prompt_history,
    save_prompt_history_prompts, trim_prompt_history,
};
use crate::events::AppEvent;
use crate::logging;
use crate::prompt::{PromptStreamState, RunningPrompt};
use crate::runtime::{current_cwd_text, current_model, set_window_app_id};

use self::render::{OutputLineKind, append_output_display, pending_dots, prepare_output_display};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum SetupState {
    Checking,
    Ready,
    Installing,
    InstallFailed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum ContextMenuState {
    Checking,
    Add,
    Remove,
    Error,
}

const RETAINED_RENDER_CAPACITY: usize = 1024;
const MAX_IDLE_RENDER_CAPACITY: usize = 16 * 1024;
const LAYOUT_EPSILON: f32 = 0.1;
const SLASH_COMMAND_PANEL_TOP_SPACING: f32 = 6.0;
const SLASH_COMMAND_PANEL_ROW_HEIGHT: f32 = 28.0;
const SLASH_COMMAND_PANEL_ROW_SPACING: f32 = 4.0;
const SLASH_COMMAND_PANEL_PADDING_Y: f32 = 8.0;
pub(super) struct SlashCommand {
    pub(super) label: &'static str,
    pub(super) name: &'static str,
    pub(super) description: &'static str,
}

pub(super) const SLASH_COMMANDS: [SlashCommand; 1] = [SlashCommand {
    label: "/status",
    name: "status",
    description: "Show local rate-limit status",
}];

pub(super) struct ModelOption {
    pub(super) name: &'static str,
    pub(super) description: &'static str,
}

pub(super) const MODEL_OPTIONS: [ModelOption; 4] = [
    ModelOption {
        name: "gpt-5.3-codex",
        description: "Default Codex model",
    },
    ModelOption {
        name: "gpt-5.2-codex",
        description: "Latest Codex model",
    },
    ModelOption {
        name: "gpt-5-codex",
        description: "Previous Codex model",
    },
    ModelOption {
        name: "gpt-5",
        description: "General-purpose GPT-5",
    },
];

pub(super) struct NotificationOption {
    pub(super) name: &'static str,
    pub(super) enabled: bool,
    pub(super) description: &'static str,
}

pub(super) const NOTIFICATION_OPTIONS: [NotificationOption; 2] = [
    NotificationOption {
        name: "On",
        enabled: true,
        description: "Show completion notifications (default)",
    },
    NotificationOption {
        name: "Off",
        enabled: false,
        description: "Hide completion notifications",
    },
];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct MonitorKey {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct WindowRestoreState {
    inner_size: Vec2,
    outer_size: Vec2,
    user_height_override: Option<f32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TileCell {
    col: i32,
    row: i32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TiledWindowState {
    cell: TileCell,
    restore: WindowRestoreState,
}

pub(crate) struct CodexAgentApp {
    input: String,
    prompt_history: Vec<String>,
    prompt_history_index: Option<usize>,
    prompt_history_draft: Option<String>,
    output: String,
    current_model: String,
    notifications_enabled: bool,
    context_menu_state: ContextMenuState,
    context_menu_refresh_pending: bool,
    render_buffer: String,
    output_display_buffer: String,
    output_display_prompt_ranges: Vec<(usize, usize)>,
    output_display_line_kinds: Vec<(usize, OutputLineKind)>,
    output_display_response_start: usize,
    output_display_response_chars: usize,
    output_display_base_len: usize,
    output_display_source_len: usize,
    output_display_can_append: bool,
    output_display_dirty: bool,
    output_display_busy: bool,
    output_galley: Option<Arc<egui::Galley>>,
    output_galley_width: Option<f32>,
    output_separator_y: Option<f32>,
    input_galley: Option<Arc<egui::Galley>>,
    input_galley_width: Option<f32>,
    cwd_text: String,
    output_rows_cache: usize,
    input_rows_cache: usize,
    output_height_cache: f32,
    input_height_cache: f32,
    display_rows_width: Option<f32>,
    text_layout_dirty: bool,
    render_step: Option<u128>,
    output_base: usize,
    prompt_ranges: Vec<(usize, usize)>,
    busy: bool,
    locked: bool,
    next_prompt_id: u64,
    active_prompt_id: Option<u64>,
    pending_input_focus: bool,
    picker_selection: Option<usize>,
    settings_menu_open: bool,
    was_focused: bool,
    drag_armed: bool,
    window_dragging: bool,
    was_minimized: bool,
    maximized: bool,
    resizing: bool,
    user_height_override: Option<f32>,
    min_inner_size: Option<Vec2>,
    last_inner_size: Option<Vec2>,
    last_outer_size: Option<Vec2>,
    last_viewport_outer_rect: Option<egui::Rect>,
    pre_maximize_state: Option<WindowRestoreState>,
    tiled_state: Option<TiledWindowState>,
    window_restore_states: HashMap<MonitorKey, WindowRestoreState>,
    minimized_restore_state: Option<WindowRestoreState>,
    minimized_monitor: Option<MonitorKey>,
    pending_started_at: Option<Instant>,
    ctx: egui::Context,
    tx: mpsc::Sender<AppEvent>,
    rx: mpsc::Receiver<AppEvent>,
    running_prompt: Arc<Mutex<Option<RunningPrompt>>>,
    shared_stream: Arc<Mutex<PromptStreamState>>,
    stream_notification_pending: Arc<AtomicBool>,
    stream_generation: u64,
    stream_visible_len: usize,
    session_id: Option<String>,
    cancelled_resume_context: Option<String>,
    setup_state: SetupState,
    install_stdin: Arc<Mutex<Option<ChildStdin>>>,
    positioned: bool,
    title_set: bool,
    hwnd: *mut c_void,
}

impl CodexAgentApp {
    const INPUT_ID: &'static str = "prompt-input";

    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let history = match load_prompt_history() {
            Ok(history) => history,
            Err(error) => {
                logging::error(format!("failed to load prompt history: {}", error));
                PromptHistory::default()
            }
        };
        if let Err(error) = save_prompt_history(&history) {
            logging::error(format!("failed to sanitize prompt history: {}", error));
        }
        logging::trace("app created");
        let notifications_enabled = match load_notifications_enabled() {
            Ok(enabled) => enabled,
            Err(error) => {
                logging::error(format!("failed to load notification setting: {}", error));
                DEFAULT_NOTIFICATIONS_ENABLED
            }
        };
        Ok(Self {
            input: String::new(),
            prompt_history: history.prompts,
            prompt_history_index: None,
            prompt_history_draft: None,
            output_base: 0,
            output: String::new(),
            current_model: current_model(),
            notifications_enabled,
            context_menu_state: ContextMenuState::Checking,
            context_menu_refresh_pending: false,
            render_buffer: String::new(),
            output_display_buffer: String::new(),
            output_display_prompt_ranges: Vec::new(),
            output_display_line_kinds: Vec::new(),
            output_display_response_start: 0,
            output_display_response_chars: 0,
            output_display_base_len: 0,
            output_display_source_len: 0,
            output_display_can_append: false,
            output_display_dirty: true,
            output_display_busy: false,
            output_galley: None,
            output_galley_width: None,
            output_separator_y: None,
            input_galley: None,
            input_galley_width: None,
            cwd_text: current_cwd_text(),
            output_rows_cache: 0,
            input_rows_cache: 1,
            output_height_cache: 0.0,
            input_height_cache: LINE_HEIGHT,
            display_rows_width: None,
            text_layout_dirty: true,
            render_step: None,
            prompt_ranges: Vec::new(),
            busy: false,
            locked: false,
            next_prompt_id: 1,
            active_prompt_id: None,
            pending_input_focus: true,
            picker_selection: None,
            settings_menu_open: false,
            was_focused: false,
            drag_armed: false,
            window_dragging: false,
            was_minimized: false,
            maximized: false,
            resizing: false,
            user_height_override: None,
            min_inner_size: None,
            last_inner_size: None,
            last_outer_size: None,
            last_viewport_outer_rect: None,
            pre_maximize_state: None,
            tiled_state: None,
            window_restore_states: HashMap::new(),
            minimized_restore_state: None,
            minimized_monitor: None,
            pending_started_at: None,
            ctx: cc.egui_ctx.clone(),
            tx,
            rx,
            running_prompt: Arc::new(Mutex::new(None)),
            shared_stream: Arc::new(Mutex::new(PromptStreamState::default())),
            stream_notification_pending: Arc::new(AtomicBool::new(false)),
            stream_generation: 0,
            stream_visible_len: 0,
            session_id: None,
            cancelled_resume_context: None,
            setup_state: SetupState::Ready,
            install_stdin: Arc::new(Mutex::new(None)),
            positioned: false,
            title_set: false,
            hwnd: {
                let h = creation_hwnd(cc);
                set_window_app_id(h);
                h
            },
        })
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
        self.output.clear();
        self.output_base = 0;
        self.prompt_ranges.clear();
        self.output_display_prompt_ranges.clear();
        self.output_display_line_kinds.clear();
        self.output_display_response_start = 0;
        self.output_display_response_chars = 0;
        self.output_display_base_len = 0;
        self.output_display_source_len = 0;
        self.output_display_can_append = false;
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
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.clear_render_buffer();
        {
            let mut stream = self.shared_stream.lock().unwrap_or_else(|e| e.into_inner());
            stream.reset();
        }
        self.reset_stream_progress();
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

    pub(super) fn slash_command_query(&self) -> Option<&str> {
        let input = self.input.trim();
        let query = input.strip_prefix('/')?;
        if query.contains(char::is_whitespace) {
            return None;
        }
        Some(query)
    }

    pub(super) fn should_show_slash_command(&self, command: &SlashCommand) -> bool {
        self.slash_command_query()
            .is_some_and(|query| command.name.starts_with(query))
    }

    pub(super) fn refresh_current_model(&mut self) {
        self.current_model = current_model();
    }

    pub(super) fn refresh_notifications_enabled(&mut self) {
        self.notifications_enabled = match load_notifications_enabled() {
            Ok(enabled) => enabled,
            Err(error) => {
                logging::error(format!("failed to refresh notification setting: {}", error));
                DEFAULT_NOTIFICATIONS_ENABLED
            }
        };
    }

    pub(super) fn slash_command_count(&self) -> usize {
        SLASH_COMMANDS
            .iter()
            .filter(|command| self.should_show_slash_command(command))
            .count()
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

    pub(super) fn clear_picker_selection(&mut self) {
        self.picker_selection = None;
    }

    pub(super) fn reset_stream_progress(&mut self) {
        self.stream_generation = 0;
        self.stream_visible_len = 0;
    }

    pub(super) fn reset_prompt_history_navigation(&mut self) {
        self.prompt_history_index = None;
        self.prompt_history_draft = None;
    }

    pub(super) fn push_prompt_history(&mut self, prompt: &str) {
        if prompt.is_empty() {
            return;
        }
        if self
            .prompt_history
            .last()
            .is_some_and(|entry| entry == prompt)
        {
            self.reset_prompt_history_navigation();
            return;
        }
        self.prompt_history.push(prompt.to_owned());
        trim_prompt_history(&mut self.prompt_history);
        self.reset_prompt_history_navigation();
    }

    pub(super) fn browse_prompt_history(&mut self, newer: bool) -> bool {
        if self.prompt_history.is_empty() {
            return false;
        }

        match self.prompt_history_index {
            Some(index) if newer => {
                if index + 1 >= self.prompt_history.len() {
                    self.prompt_history_index = None;
                    self.input = self.prompt_history_draft.take().unwrap_or_default();
                } else {
                    let next = index + 1;
                    self.prompt_history_index = Some(next);
                    self.input = self.prompt_history[next].clone();
                }
            }
            Some(index) => {
                let next = index.saturating_sub(1);
                self.prompt_history_index = Some(next);
                self.input = self.prompt_history[next].clone();
            }
            None if newer => return false,
            None => {
                self.prompt_history_draft = Some(self.input.clone());
                let next = self.prompt_history.len() - 1;
                self.prompt_history_index = Some(next);
                self.input = self.prompt_history[next].clone();
            }
        }

        self.pending_input_focus = true;
        self.refresh_after_input_change();
        true
    }

    pub(super) fn picker_selection(&self) -> Option<usize> {
        self.picker_selection
            .filter(|selection| *selection < self.picker_item_count())
    }

    pub(super) fn move_picker_selection(&mut self, offset: isize) -> bool {
        let count = self.picker_item_count();
        if count == 0 {
            self.picker_selection = None;
            return false;
        }
        let next = match self.picker_selection() {
            Some(selection) => {
                let last = count.saturating_sub(1) as isize;
                (selection as isize + offset).clamp(0, last) as usize
            }
            None if offset < 0 => count - 1,
            None => 0,
        };
        self.picker_selection = Some(next);
        true
    }

    pub(super) fn activate_picker_selection(&mut self) -> bool {
        let Some(selection) = self.picker_selection() else {
            return false;
        };
        let mut index = 0;
        for command in SLASH_COMMANDS.iter() {
            if !self.should_show_slash_command(command) {
                continue;
            }
            if index == selection {
                self.select_slash_command(command.name);
                return true;
            }
            index += 1;
        }
        false
    }

    fn picker_item_count(&self) -> usize {
        self.slash_command_count()
    }

    pub(super) fn command_panel_height(&self) -> f32 {
        let count = self.picker_item_count();
        if count == 0 {
            return 0.0;
        }
        SLASH_COMMAND_PANEL_TOP_SPACING
            + SLASH_COMMAND_PANEL_PADDING_Y * 2.0
            + SLASH_COMMAND_PANEL_ROW_HEIGHT * count as f32
            + SLASH_COMMAND_PANEL_ROW_SPACING * count.saturating_sub(1) as f32
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

fn creation_hwnd(cc: &eframe::CreationContext<'_>) -> *mut c_void {
    match cc.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get() as *mut c_void,
        _ => std::ptr::null_mut(),
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
            _ if line == crate::config::CANCELLED_TEXT => ("System", line),
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
