use std::ffi::c_void;
use std::io;
use std::sync::{Arc, Mutex, atomic::AtomicBool, mpsc};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::config::{
    DEFAULT_NOTIFICATIONS_ENABLED, LINE_HEIGHT, PromptHistory, load_notifications_enabled,
    load_prompt_history, save_prompt_history,
};
use crate::logging;
use crate::prompt::PromptStreamState;
use crate::runtime::{available_models, current_cwd_text, current_model, set_window_app_id};

use super::{CodexAgentApp, ContextMenuState, SetupState};

impl CodexAgentApp {
    pub(super) const INPUT_ID: &'static str = "prompt-input";

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
        let current_model = current_model();
        let model_options = available_models(&current_model);
        Ok(Self {
            input: String::new(),
            prompt_history: history.prompts,
            prompt_history_index: None,
            prompt_history_draft: None,
            output_base: 0,
            output: String::new(),
            current_model,
            model_options,
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
            window_restore_states: std::collections::HashMap::new(),
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
                let hwnd = creation_hwnd(cc);
                set_window_app_id(hwnd);
                hwnd
            },
        })
    }

    pub(super) fn refresh_current_model(&mut self) {
        self.current_model = current_model();
    }

    pub(super) fn refresh_model_options(&mut self) {
        self.model_options = available_models(&self.current_model);
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
}

fn creation_hwnd(cc: &eframe::CreationContext<'_>) -> *mut c_void {
    match cc.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get() as *mut c_void,
        _ => std::ptr::null_mut(),
    }
}
