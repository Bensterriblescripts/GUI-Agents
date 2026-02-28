use std::time::Duration;

use eframe::egui::{
    self, Color32, CursorIcon, FontId, Key, KeyboardShortcut, Modifiers, RichText, TextEdit,
};

use crate::config::{
    CANCEL_BUTTON_HEIGHT, CANCEL_BUTTON_WIDTH, LINE_HEIGHT, PROMPT_SCROLL_ID, TEXT_FONT_SIZE,
    WINDOW_PADDING,
};

use super::AutoAgentApp;
use super::render::markdown_layout_job;

const TITLEBAR_BUTTON_SIZE: f32 = 24.0;
const TITLEBAR_BUTTON_SPACING: f32 = 2.0;

impl eframe::App for AutoAgentApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();

        if !self.positioned {
            if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                let x = monitor.x * 0.10;
                let y = monitor.y * 0.10;
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x, y)));
                self.positioned = true;
            }
        }

        if ctx.input(|input| input.viewport().close_requested()) {
            if self.busy {
                self.cancel_active_prompt();
            }
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
        self.was_focused = focused;

        if self.resizing && ctx.input(|input| !input.pointer.primary_down()) {
            self.resizing = false;
            self.user_height_override = Some(ctx.screen_rect().height());
            self.invalidate_text_layout();
        }

        if ctx.input(|input| input.viewport().minimized.unwrap_or(false)) {
            self.release_input_focus();
            return;
        }

        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        if !focused {
            self.release_input_focus();
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(WINDOW_PADDING as i8)))
            .show(ctx, |ui| {
                ui.set_min_size(ui.available_size());
                let card_response = egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(14, 18, 24, 204))
                    .stroke(egui::Stroke::new(
                        1.0,
                        Color32::from_rgba_unmultiplied(124, 189, 255, 92),
                    ))
                    .corner_radius(egui::CornerRadius::same(18))
                    .inner_margin(egui::Margin::symmetric(18, 8))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 0],
                        blur: 32,
                        spread: 3,
                        color: Color32::from_rgba_unmultiplied(96, 176, 255, 88),
                    })
                    .show(ui, |ui| {
                        ui.style_mut().spacing.item_spacing.y = 0.0;
                        let mut cancel = false;
                        let mut clear = false;
                        let mut minimize = false;
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
                            let titlebar_w = TITLEBAR_BUTTON_SIZE * 2.0 + TITLEBAR_BUTTON_SPACING;
                            let action_w = if self.busy || self.can_clear() {
                                CANCEL_BUTTON_WIDTH
                            } else {
                                0.0
                            };
                            ui.add_space((ui.available_width() - action_w - titlebar_w).max(0.0));
                            if self.busy {
                                cancel = egui::Frame::new()
                                    .corner_radius(egui::CornerRadius::same(255))
                                    .show(ui, |ui| {
                                        ui.spacing_mut().button_padding = egui::vec2(14.0, 4.0);
                                        let resp = ui.add(
                                            egui::Button::new(
                                                RichText::new("Cancel")
                                                    .strong()
                                                    .color(Color32::WHITE),
                                            )
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
                                                resp.rect,
                                                egui::CornerRadius::same(255),
                                                Color32::from_rgba_unmultiplied(255, 40, 40, 25),
                                            );
                                        }
                                        let rect = resp.rect;
                                        let painter = ui.painter();
                                        for i in 0..3 {
                                            let expand_x = (i as f32) * 1.792 + 0.896;
                                            let expand_y = (i as f32) * 2.304 + 1.152;
                                            let alpha = 10 - i * 3;
                                            painter.rect_filled(
                                                rect.expand2(egui::vec2(expand_x, expand_y)),
                                                egui::CornerRadius::same(255),
                                                Color32::from_rgba_unmultiplied(
                                                    255,
                                                    40,
                                                    40,
                                                    (alpha.max(0) as f32 * 0.64) as u8,
                                                ),
                                            );
                                        }
                                        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
                                            .clicked()
                                    })
                                    .inner;
                            } else if self.can_clear() {
                                let resp = ui.add(
                                    egui::Button::new(
                                        RichText::new("Clear").color(Color32::WHITE),
                                    )
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
                                        resp.rect,
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
                                [egui::pos2(c.x - 4.5, c.y - 4.5), egui::pos2(c.x + 4.5, c.y + 4.5)],
                                s,
                            );
                            ui.painter().line_segment(
                                [egui::pos2(c.x + 4.5, c.y - 4.5), egui::pos2(c.x - 4.5, c.y + 4.5)],
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
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        if close {
                            if self.busy {
                                self.cancel_active_prompt();
                            }
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        ui.add_space(2.0);
                        let (output_rows, input_rows) = self.display_rows();
                        let input_h = self.input_height_cache;
                        let output_h = self.output_height_cache;
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
                                    self.sync_output_display_buffer();
                                    let prompt_ranges = &self.prompt_ranges;
                                    let output_base = self.output_base;
                                    let output_display_buffer = &mut self.output_display_buffer;
                                    let mut layouter =
                                        |ui: &egui::Ui, text: &str, wrap_width: f32| {
                                            let job = markdown_layout_job(
                                                text,
                                                wrap_width,
                                                prompt_ranges,
                                                output_base,
                                            );
                                            ui.fonts(|fonts| fonts.layout_job(job))
                                        };
                                    TextEdit::multiline(output_display_buffer)
                                        .id_source("output-display")
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(output_rows)
                                        .layouter(&mut layouter)
                                        .frame(false)
                                        .show(ui);
                                });
                            });
                            ui.add_space(4.0);
                            let (sep_rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 1.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(
                                sep_rect,
                                0.0,
                                Color32::from_rgba_unmultiplied(124, 189, 255, 40),
                            );
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
                                        let mut layouter =
                                            |ui: &egui::Ui, text: &str, wrap_width: f32| {
                                                let job = markdown_layout_job(text, wrap_width, &[], 0);
                                                ui.fonts(|fonts| fonts.layout_job(job))
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
                            self.invalidate_text_layout();
                            self.resize_for_text();
                        }
                        self.sync_input_focus(&response);
                        let submit = response.has_focus()
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
                self.update_window_drag(resize_rect, drag_rect);
            });
    }
}
