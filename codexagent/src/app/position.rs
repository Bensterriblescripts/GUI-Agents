use eframe::egui;

const DEFAULT_POSITION_RATIO: f32 = 0.10;

pub(super) fn startup_outer_position(monitor_size: egui::Vec2) -> egui::Pos2 {
    egui::pos2(
        monitor_size.x * DEFAULT_POSITION_RATIO,
        monitor_size.y * DEFAULT_POSITION_RATIO,
    )
}
