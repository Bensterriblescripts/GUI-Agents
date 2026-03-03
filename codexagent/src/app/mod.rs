mod events;
mod layout;
mod position;
mod render;
mod ui;
mod window;

use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::sync::{Arc, Mutex, atomic::AtomicBool, mpsc};
use std::time::{Duration, Instant};

use eframe::egui::{self, Vec2};

use crate::config::{APP_NAME, LINE_HEIGHT, PENDING_ANIMATION_INTERVAL};
use crate::events::AppEvent;
use crate::logging;
use crate::prompt::{PromptStreamState, RunningPrompt};
use crate::runtime::{current_cwd_text, current_model};

use self::render::{OutputLineKind, pending_dots, prepare_output_display};

const RETAINED_RENDER_CAPACITY: usize = 1024;
const MAX_IDLE_RENDER_CAPACITY: usize = 16 * 1024;
const SLASH_COMMAND_PANEL_TOP_SPACING: f32 = 6.0;
const SLASH_COMMAND_PANEL_ROW_HEIGHT: f32 = 28.0;
const SLASH_COMMAND_PANEL_ROW_SPACING: f32 = 4.0;
const SLASH_COMMAND_PANEL_PADDING_Y: f32 = 8.0;
pub(super) struct SlashCommand {
    pub(super) label: &'static str,
    pub(super) name: &'static str,
    pub(super) description: &'static str,
}

pub(super) const SLASH_COMMANDS: [SlashCommand; 1] = [
    SlashCommand {
        label: "/status",
        name: "status",
        description: "Show rate-limit status",
    },
];

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
    output: String,
    current_model: String,
    render_buffer: String,
    output_display_buffer: String,
    output_display_prompt_ranges: Vec<(usize, usize)>,
    output_display_line_kinds: Vec<(usize, OutputLineKind)>,
    output_display_points: Vec<(usize, usize)>,
    output_display_mapped_points: Vec<usize>,
    output_display_response_start: usize,
    output_display_dirty: bool,
    output_display_busy: bool,
    output_galley: Option<Arc<egui::Galley>>,
    output_galley_width: Option<f32>,
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
    session_id: Option<String>,
    positioned: bool,
    title_set: bool,
    hwnd: *mut c_void,
}

impl CodexAgentApp {
    const INPUT_ID: &'static str = "prompt-input";

    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        logging::trace("app created");
        Ok(Self {
            input: String::new(),
            output: String::new(),
            current_model: current_model(),
            render_buffer: String::new(),
            output_display_buffer: String::new(),
            output_display_prompt_ranges: Vec::new(),
            output_display_line_kinds: Vec::new(),
            output_display_points: Vec::new(),
            output_display_mapped_points: Vec::new(),
            output_display_response_start: 0,
            output_display_dirty: true,
            output_display_busy: false,
            output_galley: None,
            output_galley_width: None,
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
            output_base: 0,
            prompt_ranges: Vec::new(),
            busy: false,
            locked: false,
            next_prompt_id: 1,
            active_prompt_id: None,
            pending_input_focus: true,
            picker_selection: None,
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
            session_id: None,
            positioned: false,
            title_set: false,
            hwnd: std::ptr::null_mut(),
        })
    }

    pub(super) fn invalidate_text_layout(&mut self) {
        self.text_layout_dirty = true;
        self.output_galley = None;
        self.output_galley_width = None;
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
        self.output_display_dirty = true;
    }

    pub(super) fn can_clear(&self) -> bool {
        !self.busy && (!self.output.is_empty() || self.session_id.is_some())
    }

    pub(super) fn clear_session(&mut self) {
        self.input.clear();
        self.output.clear();
        self.output_base = 0;
        self.prompt_ranges.clear();
        self.output_display_prompt_ranges.clear();
        self.output_display_line_kinds.clear();
        self.output_display_points.clear();
        self.output_display_mapped_points.clear();
        self.output_display_response_start = 0;
        self.session_id = None;
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
            stream.prompt_id = None;
            stream.text.clear();
        }
        self.invalidate_input_layout();
        self.invalidate_output_layout();
        self.resize_for_text();
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
            self.output_display_dirty = true;
            self.output_galley = None;
            return;
        }
        self.render_buffer
            .reserve(self.output.len() + dots.len() + 1);
        self.render_buffer.push_str(&self.output);
        if !self.output.ends_with('\n') {
            self.render_buffer.push('\n');
        }
        self.render_buffer.push_str(dots);
        self.output_display_dirty = true;
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

    pub(super) fn slash_command_count(&self) -> usize {
        SLASH_COMMANDS
            .iter()
            .filter(|command| self.should_show_slash_command(command))
            .count()
    }

    pub(super) fn clear_picker_selection(&mut self) {
        self.picker_selection = None;
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
        if !self.output_display_dirty && self.output_display_busy == self.busy {
            return;
        }
        let source = if self.busy {
            &self.render_buffer
        } else {
            &self.output
        };
        self.output_display_response_start = prepare_output_display(
            source,
            &self.prompt_ranges,
            self.output_base,
            &mut self.output_display_buffer,
            &mut self.output_display_prompt_ranges,
            &mut self.output_display_line_kinds,
            &mut self.output_display_points,
            &mut self.output_display_mapped_points,
        );
        self.output_display_dirty = false;
        self.output_display_busy = self.busy;
        self.output_galley = None;
    }
}
