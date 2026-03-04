use eframe::egui::{
    self, Color32, CursorIcon, FontId, Key, KeyboardShortcut, Modifiers, RichText, TextEdit,
};

use crate::config::{
    CANCEL_BUTTON_HEIGHT, CANCEL_BUTTON_WIDTH, LINE_HEIGHT, PROMPT_SCROLL_ID, TEXT_FONT_SIZE,
    WINDOW_BOTTOM_PADDING, WINDOW_PADDING,
};
use crate::notify;

use super::position::startup_outer_position;
use super::render::markdown_layout_job;
use super::{
    CodexAgentApp, MODEL_OPTIONS, NOTIFICATION_OPTIONS, SLASH_COMMANDS, WindowRestoreState,
};

const TITLEBAR_BUTTON_SIZE: f32 = 24.0;
const TITLEBAR_BUTTON_SPACING: f32 = 2.0;
const CANCEL_BUSY_BUTTON_WIDTH: f32 = CANCEL_BUTTON_WIDTH * 0.8;

struct GlowPalette {
    stroke: Color32,
    shadow: Color32,
    separator: Color32,
}

fn show_picker_row(
    ui: &mut egui::Ui,
    name: &str,
    description: &str,
    selected: bool,
    active: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::click());
    let fill = if selected {
        Color32::from_rgba_unmultiplied(124, 189, 255, 28)
    } else if response.hovered() {
        Color32::from_rgba_unmultiplied(255, 255, 255, 12)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    if active {
        ui.painter().rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(124, 189, 255, 80)),
            egui::StrokeKind::Outside,
        );
    }
    let content_rect = rect.shrink2(egui::vec2(8.0, 5.0));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(content_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.add(
                egui::Label::new(RichText::new(name).monospace().color(if active {
                    Color32::from_rgb(160, 214, 255)
                } else {
                    Color32::from_rgb(124, 189, 255)
                }))
                .selectable(false)
                .sense(egui::Sense::empty()),
            );
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(RichText::new(description).color(if active {
                    Color32::from_rgba_unmultiplied(214, 224, 238, 190)
                } else {
                    Color32::from_rgba_unmultiplied(214, 224, 238, 150)
                }))
                .selectable(false)
                .sense(egui::Sense::empty()),
            );
            if active {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new("IN USE")
                                .size(11.0)
                                .color(Color32::from_rgb(160, 214, 255)),
                        )
                        .selectable(false)
                        .sense(egui::Sense::empty()),
                    );
                });
            }
        },
    );
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn show_picker(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(20, 26, 34, 214))
        .stroke(egui::Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(124, 189, 255, 36),
        ))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            ui.style_mut().spacing.item_spacing.y = 4.0;
            add_contents(ui);
        });
}

impl CodexAgentApp {
    fn glow_palette(&self) -> GlowPalette {
        if self.busy {
            return GlowPalette {
                stroke: Color32::from_rgba_unmultiplied(158, 164, 173, 84),
                shadow: Color32::from_rgba_unmultiplied(122, 128, 138, 64),
                separator: Color32::from_rgba_unmultiplied(158, 164, 173, 36),
            };
        }

        GlowPalette {
            stroke: Color32::from_rgba_unmultiplied(124, 189, 255, 92),
            shadow: Color32::from_rgba_unmultiplied(96, 176, 255, 88),
            separator: Color32::from_rgba_unmultiplied(124, 189, 255, 40),
        }
    }

    fn handle_picker_keys(&mut self, ctx: &egui::Context) -> bool {
        if self.picker_item_count() == 0 {
            return false;
        }
        if self.picker_selection().is_some()
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter))
        {
            return self.activate_picker_selection();
        }
        let mut moved = false;
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            moved |= self.move_picker_selection(1);
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            moved |= self.move_picker_selection(-1);
        }
        moved
    }

    fn handle_prompt_history_keys(&mut self, ctx: &egui::Context) -> bool {
        if self.prompt_history.is_empty() {
            return false;
        }
        let mut handled = false;
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            handled |= self.browse_prompt_history(false);
        }
        if self.prompt_history_index.is_some()
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown))
        {
            handled |= self.browse_prompt_history(true);
        }
        handled
    }

    fn show_settings_menu(&mut self, ui: &mut egui::Ui) {
        let button = egui::Button::new(
            RichText::new("Settings").color(Color32::from_rgba_unmultiplied(214, 224, 238, 170)),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE)
        .corner_radius(egui::CornerRadius::same(255));
        let response = egui::menu::menu_custom_button(ui, button, |ui| {
            self.refresh_current_model();
            self.refresh_notifications_enabled();
            ui.set_min_width(360.0);
            ui.scope(|ui| {
                ui.style_mut().interaction.selectable_labels = false;
                ui.menu_button(
                    RichText::new("Model").color(Color32::from_rgb(214, 224, 238)),
                    |ui| {
                        ui.set_min_width(360.0);
                        show_picker(ui, |ui| {
                            ui.set_min_width(340.0);
                            for option in MODEL_OPTIONS.iter() {
                                let active = option.name == self.current_model;
                                if show_picker_row(
                                    ui,
                                    option.name,
                                    option.description,
                                    false,
                                    active,
                                )
                                .clicked()
                                {
                                    if !active {
                                        self.select_model(option.name);
                                    }
                                    ui.close_menu();
                                }
                            }
                        });
                    },
                );
                ui.menu_button(
                    RichText::new("Notification").color(Color32::from_rgb(214, 224, 238)),
                    |ui| {
                        ui.set_min_width(360.0);
                        show_picker(ui, |ui| {
                            ui.set_min_width(340.0);
                            for option in NOTIFICATION_OPTIONS.iter() {
                                let active = option.enabled == self.notifications_enabled;
                                if show_picker_row(
                                    ui,
                                    option.name,
                                    option.description,
                                    false,
                                    active,
                                )
                                .clicked()
                                {
                                    if !active {
                                        self.select_notification(option.enabled);
                                    }
                                    ui.close_menu();
                                }
                            }
                        });
                    },
                );
            });
        })
        .response;
        if response.hovered() {
            ui.painter().rect_filled(
                response.rect.expand2(egui::vec2(1.4336, 2.304)),
                egui::CornerRadius::same(255),
                Color32::from_rgba_unmultiplied(255, 255, 255, 15),
            );
        }
    }
}

impl eframe::App for CodexAgentApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();

        if !self.positioned {
            if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                    startup_outer_position(monitor),
                ));
                self.positioned = true;
            }
        }

        if ctx.input(|input| input.viewport().close_requested()) {
            if self.busy {
                self.cancel_active_prompt();
            }
            notify::cleanup(self.hwnd);
            return;
        }

        if ctx.input(|input| input.key_pressed(Key::Escape)) {
            if self.busy {
                self.cancel_active_prompt();
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }

        let focused = ctx.input(|input| input.focused);
        if focused && !self.was_focused {
            self.pending_input_focus = true;
        }
        if focused {
            notify::try_capture_hwnd(&mut self.hwnd);
        }
        self.was_focused = focused;

        if self.resizing && ctx.input(|input| !input.pointer.primary_down()) {
            self.resizing = false;
            self.user_height_override = Some(ctx.screen_rect().height());
            self.invalidate_text_layout();
        }

        if self.sync_viewport_state() {
            self.release_input_focus();
            return;
        }

        self.sync_windows_tiling();

        if let Some(delay) = self.pending_repaint_delay() {
            ctx.request_repaint_after(delay);
        }

        if !focused {
            self.release_input_focus();
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                left: WINDOW_PADDING as i8,
                right: WINDOW_PADDING as i8,
                top: WINDOW_PADDING as i8,
                bottom: WINDOW_BOTTOM_PADDING as i8,
            }))
            .show(ctx, |ui| {
                let glow = self.glow_palette();
                ui.set_min_size(ui.available_size());
                let card_response = egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(14, 18, 24, 204))
                    .stroke(egui::Stroke::new(1.0, glow.stroke))
                    .corner_radius(egui::CornerRadius::same(18))
                    .inner_margin(egui::Margin::symmetric(18, 10))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 0],
                        blur: 32,
                        spread: 3,
                        color: glow.shadow,
                    })
                    .show(ui, |ui| {
                        ui.style_mut().spacing.item_spacing.y = 0.0;
                        let mut cancel = false;
                        let mut clear = false;
                        let mut minimize = false;
                        let mut maximize = false;
                        let mut close = false;
                        ui.horizontal(|ui| {
                            ui.set_min_height(CANCEL_BUTTON_HEIGHT);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(self.cwd_text.as_str())
                                        .color(Color32::from_rgba_unmultiplied(214, 224, 238, 150)),
                                )
                                .selectable(false),
                            );
                            ui.add_space(10.0);
                            self.show_settings_menu(ui);
                            let titlebar_w =
                                TITLEBAR_BUTTON_SIZE * 3.0 + TITLEBAR_BUTTON_SPACING * 2.0;
                            let action_w = if self.busy || self.can_clear() {
                                if self.busy {
                                    CANCEL_BUSY_BUTTON_WIDTH
                                } else {
                                    CANCEL_BUTTON_WIDTH
                                }
                            } else {
                                0.0
                            };
                            ui.add_space((ui.available_width() - action_w - titlebar_w).max(0.0));
                            if self.busy {
                                cancel = egui::Frame::new()
                                    .corner_radius(egui::CornerRadius::same(255))
                                    .shadow(egui::epaint::Shadow {
                                        offset: [0, 0],
                                        blur: 12,
                                        spread: 2,
                                        color: Color32::from_rgba_unmultiplied(255, 30, 30, 60),
                                    })
                                    .show(ui, |ui| {
                                        ui.spacing_mut().button_padding = egui::vec2(14.0, 4.0);
                                        let resp = ui.add(
                                            egui::Button::new(
                                                RichText::new("Cancel")
                                                    .strong()
                                                    .color(Color32::WHITE),
                                            )
                                            .min_size(egui::vec2(
                                                CANCEL_BUSY_BUTTON_WIDTH,
                                                CANCEL_BUTTON_HEIGHT,
                                            ))
                                            .fill(Color32::TRANSPARENT)
                                            .stroke(egui::Stroke::NONE)
                                            .corner_radius(egui::CornerRadius::same(255)),
                                        );
                                        if resp.hovered() {
                                            ui.painter().rect_filled(
                                                resp.rect.expand2(egui::vec2(1.4336, 2.304)),
                                                egui::CornerRadius::same(255),
                                                Color32::from_rgba_unmultiplied(255, 40, 40, 25),
                                            );
                                        }
                                        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
                                            .clicked()
                                    })
                                    .inner;
                            } else if self.can_clear() {
                                let resp = ui.add(
                                    egui::Button::new(RichText::new("Clear").color(Color32::WHITE))
                                        .min_size(egui::vec2(
                                            CANCEL_BUTTON_WIDTH,
                                            CANCEL_BUTTON_HEIGHT,
                                        ))
                                        .fill(Color32::TRANSPARENT)
                                        .stroke(egui::Stroke::NONE)
                                        .corner_radius(egui::CornerRadius::same(255)),
                                );
                                if resp.hovered() {
                                    ui.painter().rect_filled(
                                        resp.rect.expand2(egui::vec2(1.4336, 2.304)),
                                        egui::CornerRadius::same(255),
                                        Color32::from_rgba_unmultiplied(255, 255, 255, 15),
                                    );
                                }
                                clear = resp
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked();
                            }
                            let btn = egui::vec2(TITLEBAR_BUTTON_SIZE, TITLEBAR_BUTTON_SIZE);
                            let (min_rect, min_resp) =
                                ui.allocate_exact_size(btn, egui::Sense::click());
                            if min_resp.hovered() {
                                ui.painter().rect_filled(
                                    min_rect,
                                    4.0,
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 15),
                                );
                            }
                            let c = min_rect.center();
                            ui.painter().line_segment(
                                [egui::pos2(c.x - 5.0, c.y), egui::pos2(c.x + 5.0, c.y)],
                                egui::Stroke::new(
                                    1.5,
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 180),
                                ),
                            );
                            minimize = min_resp.on_hover_cursor(CursorIcon::PointingHand).clicked();
                            ui.add_space(TITLEBAR_BUTTON_SPACING);
                            let (max_rect, max_resp) =
                                ui.allocate_exact_size(btn, egui::Sense::click());
                            if max_resp.hovered() {
                                ui.painter().rect_filled(
                                    max_rect,
                                    4.0,
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 15),
                                );
                            }
                            let c = max_rect.center();
                            if self.maximized {
                                let s = egui::Stroke::new(
                                    1.5,
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 180),
                                );
                                ui.painter().rect_stroke(
                                    egui::Rect::from_min_size(
                                        egui::pos2(c.x - 3.0, c.y - 5.0),
                                        egui::vec2(8.0, 8.0),
                                    ),
                                    0.0,
                                    s,
                                    egui::StrokeKind::Outside,
                                );
                                ui.painter().rect_filled(
                                    egui::Rect::from_min_size(
                                        egui::pos2(c.x - 5.0, c.y - 3.0),
                                        egui::vec2(8.0, 8.0),
                                    ),
                                    0.0,
                                    Color32::from_rgba_unmultiplied(14, 18, 24, 255),
                                );
                                ui.painter().rect_stroke(
                                    egui::Rect::from_min_size(
                                        egui::pos2(c.x - 5.0, c.y - 3.0),
                                        egui::vec2(8.0, 8.0),
                                    ),
                                    0.0,
                                    s,
                                    egui::StrokeKind::Outside,
                                );
                            } else {
                                ui.painter().rect_stroke(
                                    egui::Rect::from_center_size(c, egui::vec2(10.0, 10.0)),
                                    0.0,
                                    egui::Stroke::new(
                                        1.5,
                                        Color32::from_rgba_unmultiplied(255, 255, 255, 180),
                                    ),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            maximize = max_resp.on_hover_cursor(CursorIcon::PointingHand).clicked();
                            ui.add_space(TITLEBAR_BUTTON_SPACING);
                            let (cls_rect, cls_resp) =
                                ui.allocate_exact_size(btn, egui::Sense::click());
                            if cls_resp.hovered() {
                                ui.painter().rect_filled(
                                    cls_rect,
                                    4.0,
                                    Color32::from_rgba_unmultiplied(255, 60, 60, 50),
                                );
                            }
                            let c = cls_rect.center();
                            let s = egui::Stroke::new(
                                1.5,
                                Color32::from_rgba_unmultiplied(255, 255, 255, 180),
                            );
                            ui.painter().line_segment(
                                [
                                    egui::pos2(c.x - 4.5, c.y - 4.5),
                                    egui::pos2(c.x + 4.5, c.y + 4.5),
                                ],
                                s,
                            );
                            ui.painter().line_segment(
                                [
                                    egui::pos2(c.x + 4.5, c.y - 4.5),
                                    egui::pos2(c.x - 4.5, c.y + 4.5),
                                ],
                                s,
                            );
                            close = cls_resp.on_hover_cursor(CursorIcon::PointingHand).clicked();
                        });
                        if cancel {
                            self.cancel_active_prompt();
                        }
                        if clear {
                            self.clear_session();
                        }
                        if minimize {
                            self.prepare_for_minimize_from_ctx();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        if maximize {
                            let next_maximized = !ctx.input(|input| {
                                input.viewport().maximized.unwrap_or(self.maximized)
                            });
                            if next_maximized {
                                let outer_size = ctx
                                    .input(|input| {
                                        input.viewport().outer_rect.map(|rect| rect.size())
                                    })
                                    .or(self.last_outer_size)
                                    .unwrap_or_else(|| ctx.screen_rect().size());
                                self.pre_maximize_state = self
                                    .last_inner_size
                                    .or(Some(ctx.screen_rect().size()))
                                    .map(|inner_size| WindowRestoreState {
                                        inner_size,
                                        outer_size,
                                        user_height_override: self.user_height_override,
                                    });
                            }
                            self.maximized = next_maximized;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(next_maximized));
                            if next_maximized {
                                self.user_height_override = Some(ctx.screen_rect().height());
                            } else if let Some(state) = self.pre_maximize_state.take() {
                                self.user_height_override = state.user_height_override;
                                self.last_inner_size = Some(state.inner_size);
                                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                                    state.inner_size,
                                ));
                                self.invalidate_text_layout();
                            } else {
                                self.user_height_override = None;
                                self.invalidate_text_layout();
                                self.resize_for_text();
                            }
                        }
                        if close {
                            if self.busy {
                                self.cancel_active_prompt();
                            }
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        ui.add_space(6.0);
                        let content_width = ui.available_width();
                        let (output_rows, input_rows) = self.display_rows_for_width(content_width);
                        self.resize_for_text_with_width(
                            content_width,
                            self.auto_resize_height_limit(),
                        );
                        let input_h = self.input_height_cache;
                        let output_h = self.output_height_cache.max(LINE_HEIGHT);
                        if output_rows > 0 {
                            let output_height = if self.user_height_override.is_some() {
                                let available = ui.available_height();
                                (available - input_h - 9.0).max(LINE_HEIGHT)
                            } else {
                                output_h
                            };
                            ui.scope(|ui| {
                                ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                                let mut scroll = egui::ScrollArea::vertical()
                                    .id_salt("output-scroll")
                                    .stick_to_bottom(true)
                                    .max_height(output_height);
                                if self.user_height_override.is_some() {
                                    scroll = scroll.auto_shrink([true, false]);
                                }
                                scroll.show(ui, |ui| {
                                    ui.style_mut().override_font_id =
                                        Some(FontId::proportional(TEXT_FONT_SIZE));
                                    let wrap_width = ui.available_width();
                                    self.sync_output_galley(wrap_width);
                                    let output_content_h = self
                                        .output_galley
                                        .as_ref()
                                        .map(|galley| galley.size().y.max(LINE_HEIGHT))
                                        .unwrap_or(output_h);
                                    let prompt_ranges = &self.output_display_prompt_ranges;
                                    let output_base = self.output_display_response_start;
                                    let line_kinds = &self.output_display_line_kinds;
                                    let output_galley = self.output_galley.clone();
                                    let output_galley_width = self.output_galley_width;
                                    let output_display_buffer = &mut self.output_display_buffer;
                                    let mut layouter =
                                        |ui: &egui::Ui, text: &str, wrap_width: f32| {
                                            if Self::same_width(output_galley_width, wrap_width) {
                                                if let Some(galley) = output_galley.clone() {
                                                    galley
                                                } else {
                                                    let job = markdown_layout_job(
                                                        text,
                                                        wrap_width,
                                                        prompt_ranges,
                                                        output_base,
                                                        line_kinds,
                                                    );
                                                    ui.fonts(|fonts| fonts.layout_job(job))
                                                }
                                            } else {
                                                let job = markdown_layout_job(
                                                    text,
                                                    wrap_width,
                                                    prompt_ranges,
                                                    output_base,
                                                    line_kinds,
                                                );
                                                ui.fonts(|fonts| fonts.layout_job(job))
                                            }
                                        };
                                    let output_response = ui.add_sized(
                                        egui::vec2(ui.available_width(), output_content_h),
                                        TextEdit::multiline(output_display_buffer)
                                            .id_source("output-display")
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(1)
                                            .layouter(&mut layouter)
                                            .frame(false),
                                    );
                                    if output_galley.is_some() {
                                        if let Some(y) = self.output_separator_y {
                                            let sep_rect = egui::Rect::from_min_size(
                                                egui::pos2(
                                                    output_response.rect.left(),
                                                    output_response.rect.top() + y,
                                                ),
                                                egui::vec2(output_response.rect.width(), 1.0),
                                            );
                                            ui.painter().rect_filled(sep_rect, 0.0, glow.separator);
                                        }
                                    }
                                });
                            });
                            ui.add_space(4.0);
                            let (sep_rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 1.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(sep_rect, 0.0, glow.separator);
                            ui.add_space(4.0);
                        }
                        let response = ui
                            .scope(|ui| {
                                ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                                egui::ScrollArea::vertical()
                                    .id_salt(PROMPT_SCROLL_ID)
                                    .stick_to_bottom(true)
                                    .max_height(input_h)
                                    .show(ui, |ui| {
                                        ui.style_mut().override_font_id =
                                            Some(FontId::proportional(TEXT_FONT_SIZE));
                                        let wrap_width = ui.available_width();
                                        self.sync_input_galley(wrap_width);
                                        let input_galley = self.input_galley.clone();
                                        let input_galley_width = self.input_galley_width;
                                        let mut layouter =
                                            |ui: &egui::Ui, text: &str, wrap_width: f32| {
                                                if Self::same_width(input_galley_width, wrap_width)
                                                {
                                                    if let Some(galley) = input_galley.clone() {
                                                        galley
                                                    } else {
                                                        let job = markdown_layout_job(
                                                            text,
                                                            wrap_width,
                                                            &[],
                                                            0,
                                                            &[],
                                                        );
                                                        ui.fonts(|fonts| fonts.layout_job(job))
                                                    }
                                                } else {
                                                    let job = markdown_layout_job(
                                                        text,
                                                        wrap_width,
                                                        &[],
                                                        0,
                                                        &[],
                                                    );
                                                    ui.fonts(|fonts| fonts.layout_job(job))
                                                }
                                            };
                                        TextEdit::multiline(&mut self.input)
                                            .id_source(Self::INPUT_ID)
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(input_rows)
                                            .interactive(!self.locked)
                                            .return_key(KeyboardShortcut::new(
                                                Modifiers::SHIFT,
                                                Key::Enter,
                                            ))
                                            .layouter(&mut layouter)
                                            .frame(false)
                                            .show(ui)
                                            .response
                                    })
                                    .inner
                            })
                            .inner;
                        if response.changed() {
                            self.clear_picker_selection();
                            self.reset_prompt_history_navigation();
                            self.refresh_after_input_change();
                        }
                        if self.slash_command_count() > 0 {
                            ui.add_space(6.0);
                            show_picker(ui, |ui| {
                                let selected = self.picker_selection();
                                let mut index = 0;
                                for command in SLASH_COMMANDS.iter() {
                                    if !self.should_show_slash_command(command) {
                                        continue;
                                    }
                                    if show_picker_row(
                                        ui,
                                        command.label,
                                        command.description,
                                        selected == Some(index),
                                        false,
                                    )
                                    .clicked()
                                    {
                                        self.clear_picker_selection();
                                        self.select_slash_command(command.name);
                                    }
                                    index += 1;
                                }
                            });
                        } else {
                            self.clear_picker_selection();
                        }
                        self.sync_input_focus(&response);
                        let picker_handled =
                            response.has_focus() && !self.locked && self.handle_picker_keys(ctx);
                        let history_handled = response.has_focus()
                            && !self.locked
                            && !picker_handled
                            && self.handle_prompt_history_keys(ctx);
                        let submit = response.has_focus()
                            && !picker_handled
                            && !history_handled
                            && ui.input(|input| {
                                input.key_pressed(Key::Enter) && !input.modifiers.shift
                            });

                        if submit && !self.busy && !self.locked {
                            self.submit();
                        }
                    });
                let card_rect = card_response.response.rect;
                let resize_rect = card_rect.translate(egui::vec2(0.0, -10.0));
                let drag_rect = card_rect.shrink2(egui::vec2(18.0, 8.0));
                self.update_window_drag(resize_rect, drag_rect, self.output_rows_cache > 0);
            });
    }
}
