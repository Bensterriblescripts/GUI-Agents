mod events;
mod layout;
mod render;
mod ui;
mod window;

use std::io;
use std::sync::{
    Arc, Mutex,
    atomic::AtomicBool,
    mpsc,
};
use std::time::Instant;

use eframe::egui::{self, Vec2};

use crate::config::{LINE_HEIGHT, PENDING_ANIMATION_INTERVAL};
use crate::events::AppEvent;
use crate::logging;
use crate::prompt::{PromptStreamState, RunningPrompt};
use crate::runtime::current_cwd_text;

use self::render::pending_dots;

const RETAINED_RENDER_CAPACITY: usize = 1024;
const MAX_IDLE_RENDER_CAPACITY: usize = 16 * 1024;

pub(crate) struct AutoAgentApp {
    input: String,
    output: String,
    render_buffer: String,
    output_display_buffer: String,
    output_display_dirty: bool,
    output_display_busy: bool,
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
    was_focused: bool,
    drag_armed: bool,
    window_dragging: bool,
    resizing: bool,
    user_height_override: Option<f32>,
    last_inner_size: Option<Vec2>,
    pending_started_at: Option<Instant>,
    ctx: egui::Context,
    tx: mpsc::Sender<AppEvent>,
    rx: mpsc::Receiver<AppEvent>,
    running_prompt: Arc<Mutex<Option<RunningPrompt>>>,
    shared_stream: Arc<Mutex<PromptStreamState>>,
    stream_notification_pending: Arc<AtomicBool>,
    session_id: Option<String>,
    positioned: bool,
}

impl AutoAgentApp {
    const INPUT_ID: &'static str = "prompt-input";

    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        logging::trace("app created");
        Ok(Self {
            input: String::new(),
            output: String::new(),
            render_buffer: String::new(),
            output_display_buffer: String::new(),
            output_display_dirty: true,
            output_display_busy: false,
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
            was_focused: false,
            drag_armed: false,
            window_dragging: false,
            resizing: false,
            user_height_override: None,
            last_inner_size: None,
            pending_started_at: None,
            ctx: cc.egui_ctx.clone(),
            tx,
            rx,
            running_prompt: Arc::new(Mutex::new(None)),
            shared_stream: Arc::new(Mutex::new(PromptStreamState::default())),
            stream_notification_pending: Arc::new(AtomicBool::new(false)),
            session_id: None,
            positioned: false,
        })
    }

    pub(super) fn invalidate_text_layout(&mut self) {
        self.text_layout_dirty = true;
    }

    pub(super) fn invalidate_output_layout(&mut self) {
        self.invalidate_text_layout();
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
        self.session_id = None;
        self.active_prompt_id = None;
        self.locked = false;
        self.pending_started_at = None;
        self.pending_input_focus = true;
        self.stream_notification_pending
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.clear_render_buffer();
        {
            let mut stream = self.shared_stream.lock().unwrap_or_else(|e| e.into_inner());
            stream.prompt_id = None;
            stream.text.clear();
        }
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

    pub(super) fn sync_render_buffer(&mut self, step: u128) {
        self.render_buffer.clear();
        let dots = pending_dots(step);
        if self.output.is_empty() {
            self.render_buffer.push_str(dots);
            self.output_display_dirty = true;
            return;
        }
        self.render_buffer.reserve(self.output.len() + dots.len() + 1);
        self.render_buffer.push_str(&self.output);
        if !self.output.ends_with('\n') {
            self.render_buffer.push('\n');
        }
        self.render_buffer.push_str(dots);
        self.output_display_dirty = true;
    }

    pub(super) fn clear_render_buffer(&mut self) {
        self.render_buffer.clear();
        self.render_step = None;
        if self.render_buffer.capacity() > MAX_IDLE_RENDER_CAPACITY {
            self.render_buffer.shrink_to(RETAINED_RENDER_CAPACITY);
        }
    }

    pub(super) fn sync_output_display_buffer(&mut self) {
        if !self.output_display_dirty && self.output_display_busy == self.busy {
            return;
        }
        self.output_display_buffer.clear();
        if self.busy {
            self.output_display_buffer.push_str(&self.render_buffer);
        } else {
            self.output_display_buffer.push_str(&self.output);
        }
        self.output_display_dirty = false;
        self.output_display_busy = self.busy;
    }
}
