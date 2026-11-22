//! "Phone remote" row and window: start/stop the LAN server, show the QR code and link.

use eframe::egui;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::blender::Blender;

#[derive(Default)]
pub struct RemotePanel {
    open: bool,
}

impl RemotePanel {
    /// One line in the main panel: state plus start / show / stop.
    pub fn row(&mut self, ui: &mut egui::Ui, blender: &Arc<Blender>) {
        if blender.remote_window.swap(false, Ordering::SeqCst) {
            self.open = true; // requested from the tray menu
        }
        let on = blender.remote_info().is_some();
        ui.horizontal(|ui| {
            ui.label("Phone remote");
            if on {
                ui.colored_label(egui::Color32::from_rgb(60, 190, 90), "on");
                if ui.button("Show QR…").clicked() {
                    self.open = true;
                }
                if ui.button("Stop").clicked() {
                    blender.stop_remote();
                    self.open = false;
                }
            } else {
                ui.weak("off");
                if ui.button("Start…").clicked() {
                    blender.start_remote(ui.ctx());
                    self.open = true;
                }
            }
        });
        if let Some(e) = blender.remote_error.lock().unwrap().as_ref() {
            ui.colored_label(egui::Color32::from_rgb(220, 70, 70), e);
        }
    }

    pub fn window(&mut self, ctx: &egui::Context, blender: &Arc<Blender>) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let mut stop = false;
        egui::Window::new("Phone remote")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(340.0)
            .show(ctx, |ui| match blender.remote_info() {
                None => {
                    ui.label("The remote is off. Nothing is listening on the network.");
                    if ui.button("Start").clicked() {
                        blender.start_remote(ctx);
                    }
                }
                Some(info) => {
                    ui.label("Scan with a phone on the same Wi-Fi / network:");
                    ui.add_space(4.0);
                    paint_qr(ui, &info.qr, 260.0);
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(&info.url).monospace()).selectable(true));
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(info.url.clone());
                        }
                    });
                    if info.urls.len() > 1 {
                        egui::CollapsingHeader::new("Other addresses of this PC").show(ui, |ui| {
                            for u in &info.urls[1..] {
                                ui.add(egui::Label::new(egui::RichText::new(u).monospace().small()).selectable(true));
                            }
                        });
                    }
                    ui.add_space(6.0);
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 160, 40),
                        "Plain HTTP on your local network: anyone who gets this link (or sees this QR) \
                         can change the projection until you stop the remote. Use only on a trusted network. \
                         The link changes every time the remote starts.",
                    );
                    ui.add_space(6.0);
                    if ui.add(egui::Button::new("⏹  Stop remote").min_size([120.0, 26.0].into())).clicked() {
                        stop = true;
                    }
                }
            });
        if stop {
            blender.stop_remote();
            open = false;
        }
        self.open = open;
    }
}

/// Black-on-white QR with a 4-module quiet zone, `size` points wide.
fn paint_qr(ui: &mut egui::Ui, modules: &[Vec<bool>], size: f32) {
    let n = modules.len();
    if n == 0 {
        ui.colored_label(egui::Color32::from_rgb(220, 70, 70), "could not make a QR code");
        return;
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::WHITE);
    let cell = size / (n + 8) as f32;
    for (y, row) in modules.iter().enumerate() {
        for (x, &dark) in row.iter().enumerate() {
            if dark {
                let min = rect.min + egui::vec2((x + 4) as f32 * cell, (y + 4) as f32 * cell);
                // Slight overlap avoids hairline gaps between modules.
                painter.rect_filled(egui::Rect::from_min_size(min, egui::vec2(cell + 0.5, cell + 0.5)), 0.0, egui::Color32::BLACK);
            }
        }
    }
}
