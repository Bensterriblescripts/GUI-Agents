#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod config;
mod events;
mod logging;
mod notify;
mod prompt;
mod runtime;
mod status;

use std::io;
use std::path::PathBuf;

use eframe::egui::{self, Vec2};

use crate::app::CodexAgentApp;
use crate::config::{
    APP_DISPLAY_NAME, APP_NAME, DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH, MIN_WINDOW_HEIGHT,
    MIN_WINDOW_WIDTH,
};
use crate::runtime::{
    LaunchRequest, acquire_instance_mutex, apply_launch_request, ensure_app_identity,
    ensure_codex_files,
};

const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/app-icon.png");

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

    let _instance_mutex = acquire_instance_mutex();
    let launch_request = parse_launch_request();
    apply_launch_request(&launch_request);
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
                .with_icon(load_app_icon()?)
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

fn load_app_icon() -> io::Result<egui::IconData> {
    eframe::icon_data::from_png_bytes(APP_ICON_BYTES)
        .map_err(|error| io::Error::other(format!("failed to load app icon: {error}")))
}

fn parse_launch_request() -> LaunchRequest {
    let mut request = LaunchRequest::default();
    let mut args = std::env::args_os().skip(1);

    while let Some(arg) = args.next() {
        if arg == "--cwd" {
            if let Some(path) = args.next() {
                request.cwd = Some(PathBuf::from(path));
            } else {
                logging::error("missing path after --cwd");
            }
            continue;
        }

        if let Some(value) = arg.to_str().and_then(|arg| arg.strip_prefix("--cwd=")) {
            request.cwd = Some(PathBuf::from(value));
        }
    }

    request
}
