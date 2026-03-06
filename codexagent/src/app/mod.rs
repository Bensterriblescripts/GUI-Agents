mod events;
mod history;
mod init;
mod layout;
mod output;
mod position;
mod render;
mod ui;
mod window;

use std::collections::HashMap;
use std::ffi::c_void;
use std::process::ChildStdin;
use std::sync::{Arc, Mutex, atomic::AtomicBool, mpsc};
use std::time::Instant;

use eframe::egui::{self, Vec2};

use crate::events::AppEvent;
use crate::prompt::{PromptStreamState, RunningPrompt};

use self::render::OutputLineKind;

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
}

pub(super) const MODEL_OPTIONS: [ModelOption; 4] = [
    ModelOption {
        name: "gpt-5.3-codex",
    },
    ModelOption {
        name: "gpt-5.2-codex",
    },
    ModelOption {
        name: "gpt-5-codex",
    },
    ModelOption {
        name: "gpt-5",
    },
];

pub(super) struct NotificationOption {
    pub(super) name: &'static str,
    pub(super) enabled: bool,
}

pub(super) const NOTIFICATION_OPTIONS: [NotificationOption; 2] = [
    NotificationOption {
        name: "On",
        enabled: true,
    },
    NotificationOption {
        name: "Off",
        enabled: false,
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
