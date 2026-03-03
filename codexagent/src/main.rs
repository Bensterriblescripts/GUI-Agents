#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod config;
mod events;
mod logging;
mod notify;
mod prompt;
mod runtime;
mod status;

use std::env;
use std::io;
use std::path::PathBuf;

use eframe::egui::{self, Vec2};

use crate::app::CodexAgentApp;
use crate::config::{
    APP_DISPLAY_NAME, APP_NAME, DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH, MIN_WINDOW_HEIGHT,
    MIN_WINDOW_WIDTH,
};
use crate::runtime::{ensure_app_identity, ensure_codex_files};

fn main() -> io::Result<()> {
    logging::init();
    logging::install_panic_hook();
    struct LogGuard;
    impl Drop for LogGuard {
        fn drop(&mut self) {
            logging::close();
        }
    }
    let _log_guard = LogGuard;

    apply_launch_args();
    ensure_app_identity();

    let result = logging::catch_panic("main thread", || -> io::Result<()> {
        logging::trace("process start");

        ensure_codex_files()?;

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size(Vec2::new(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT))
                .with_min_inner_size(Vec2::new(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT))
                .with_clamp_size_to_monitor_size(true)
                .with_resizable(true)
                .with_visible(true)
                .with_decorations(false)
                .with_transparent(true)
                .with_title(APP_DISPLAY_NAME),
            persist_window: false,
            ..Default::default()
        };

        logging::trace("starting native app runtime");
        eframe::run_native(
            APP_NAME,
            options,
            Box::new(move |cc| {
                CodexAgentApp::new(cc)
                    .map(|app| Box::new(app) as Box<dyn eframe::App>)
                    .map_err(|error| error.to_string().into())
            }),
        )
        .map_err(|error| io::Error::other(error.to_string()))?;

        logging::trace("process exit");
        Ok(())
    })
    .unwrap_or_else(|message| Err(io::Error::other(message)));

    if let Err(error) = &result {
        logging::error(format!("process failed: {}", error));
    }

    result
}

fn apply_launch_args() {
    let mut args = env::args_os().skip(1);

    while let Some(arg) = args.next() {
        if arg == "--show" {
            continue;
        }

        if arg == "--cwd" {
            if let Some(path) = args.next() {
                set_process_cwd(PathBuf::from(path));
            } else {
                logging::error("missing path after --cwd");
            }
            continue;
        }

        if let Some(value) = arg.to_str().and_then(|arg| arg.strip_prefix("--cwd=")) {
            set_process_cwd(PathBuf::from(value));
        }
    }
}

fn set_process_cwd(path: PathBuf) {
    match env::set_current_dir(&path) {
        Ok(()) => logging::trace(format!("set working directory to {}", path.display())),
        Err(error) => logging::error(format!(
            "failed to set working directory to {}: {}",
            path.display(),
            error
        )),
    }
}
