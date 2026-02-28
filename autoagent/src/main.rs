#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod config;
mod events;
mod logging;
mod prompt;
mod runtime;

use std::io;

use eframe::egui::{self, Vec2};

use crate::app::AutoAgentApp;
use crate::config::{
    APP_NAME, DEFAULT_WINDOW_WIDTH, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, WINDOW_PADDING,
};
use crate::runtime::ensure_codex_files;

fn main() -> io::Result<()> {
    logging::init();
    struct LogGuard;
    impl Drop for LogGuard {
        fn drop(&mut self) {
            logging::close();
        }
    }
    let _log_guard = LogGuard;

    let result = (|| -> io::Result<()> {
        logging::trace("process start");

        ensure_codex_files()?;

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size(Vec2::new(
                    DEFAULT_WINDOW_WIDTH,
                    MIN_WINDOW_HEIGHT + WINDOW_PADDING * 2.0,
                ))
                .with_min_inner_size(Vec2::new(
                    MIN_WINDOW_WIDTH,
                    MIN_WINDOW_HEIGHT + WINDOW_PADDING * 2.0,
                ))
                .with_resizable(true)
                .with_visible(true)
                .with_decorations(false)
                .with_transparent(true)
                .with_title(APP_NAME),
            ..Default::default()
        };

        logging::trace("starting native app runtime");
        eframe::run_native(
            APP_NAME,
            options,
            Box::new(move |cc| {
                AutoAgentApp::new(cc)
                    .map(|app| Box::new(app) as Box<dyn eframe::App>)
                    .map_err(|error| error.to_string().into())
            }),
        )
        .map_err(|error| io::Error::other(error.to_string()))?;

        logging::trace("process exit");
        Ok(())
    })();

    if let Err(error) = &result {
        logging::error(format!("process failed: {}", error));
    }

    result
}
