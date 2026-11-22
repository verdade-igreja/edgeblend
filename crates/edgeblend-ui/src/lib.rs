//! Tiny control panel: start/stop, displays, blend parameters. Edits apply live
//! and are autosaved.

mod blender;
mod editor;
mod tray;

pub use tray::show_panel as show_existing_panel;

use anyhow::{anyhow, Result};
use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use blender::Blender;
use editor::CalibrationEditor;
use edgeblend_config::{Config, Pattern, Preview};
use edgeblend_engine::Status;
use edgeblend_gpu::{display, PreviewImage};

pub fn run(cfg_path: PathBuf, autostart: bool) -> Result<()> {
    let cfg = Config::load_or_default(&cfg_path);
    let blender = Blender::new(cfg_path.clone(), cfg);
    if autostart {
        blender.start();
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("edgeblend")
            .with_inner_size([560.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "edgeblend",
        opts,
        Box::new(move |cc| {
            let tray = tray::spawn(cc.egui_ctx.clone(), blender.clone());
            Ok(Box::new(App::new(cfg_path, blender, tray)))
        }),
    )
    .map_err(|e| anyhow!("UI error: {e}"))
}

struct App {
    path: PathBuf,
    blender: Arc<Blender>,
    shared: Arc<Mutex<Config>>,
    status: Arc<Mutex<Status>>,
    preview: Arc<Mutex<Option<PreviewImage>>>,
    preview_tex: Option<(egui::TextureHandle, u64)>,
    seen_generation: u64,
    /// Tray icon present: closing the window hides it instead of quitting.
    tray: bool,
    /// (device name, label) for the combo boxes.
    displays: Vec<(String, String)>,
    saved: Config,
    changed_at: Option<Instant>,
    save_error: Option<String>,
    editor: CalibrationEditor,
}

impl App {
    fn new(path: PathBuf, blender: Arc<Blender>, tray: bool) -> Self {
        let cfg = blender.shared.lock().unwrap().clone();
        Self {
            path,
            saved: cfg,
            shared: blender.shared.clone(),
            status: blender.status.clone(),
            preview: blender.preview.clone(),
            preview_tex: None,
            seen_generation: blender.generation(),
            tray,
            blender,
            displays: list_displays(),
            changed_at: None,
            save_error: None,
            editor: CalibrationEditor::default(),
        }
    }

    fn start(&mut self) {
        self.blender.start();
    }

    fn stop(&mut self) {
        self.blender.stop();
    }

    fn running(&self) -> bool {
        self.blender.running()
    }

    fn save(&mut self, cfg: &Config) {
        self.save_error = cfg.save(&self.path).err().map(|e| format!("save failed: {e:#}"));
        self.saved = cfg.clone();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.tray && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::hide_panel();
        }
        // Started elsewhere (tray): the old preview texture is stale.
        if self.seen_generation != self.blender.generation() {
            self.seen_generation = self.blender.generation();
            self.preview_tex = None;
        }
        // Work on a copy; hotkeys on the blend thread may also change the shared config.
        let mut cfg = self.shared.lock().unwrap().clone();
        let before = cfg.clone();
        let status = self.status.lock().unwrap().clone();
        let running = self.running();
        let (mut want_start, mut want_stop) = (false, false);

        // Snapshot latest preview for the UI (drop the lock before painting).
        let preview_snap = self.preview.lock().unwrap().clone();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("edgeblend");
                ui.add_space(12.0);
                if running {
                    if ui.add(egui::Button::new("⏹  Stop").min_size([90.0, 28.0].into())).clicked() {
                        want_stop = true;
                    }
                } else {
                    let ok = !cfg.source.eq_ignore_ascii_case(&cfg.output);
                    if ui.add_enabled(ok, egui::Button::new("▶  Start").min_size([90.0, 28.0].into())).clicked() {
                        want_start = true;
                    }
                }
                let dot = if running { egui::Color32::from_rgb(60, 190, 90) } else { egui::Color32::GRAY };
                ui.colored_label(dot, "●");
                ui.label(&status.info);
            });
            if let Some(err) = &status.error {
                if !running {
                    ui.colored_label(egui::Color32::from_rgb(220, 70, 70), err);
                }
            }
            ui.separator();

            egui::Grid::new("cfg").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                ui.label("Source (canvas)");
                display_combo(ui, "src", &mut cfg.source, &self.displays);
                ui.end_row();

                ui.label("Output (projectors)");
                display_combo(ui, "out", &mut cfg.output, &self.displays);
                ui.end_row();

                ui.label("Mirror output");
                ui.checkbox(&mut cfg.mirror, "flip left/right (box or projectors mirror the image)");
                ui.end_row();

                ui.label("Reverse order");
                ui.checkbox(&mut cfg.reverse_order, "box feeds projectors right-to-left");
                ui.end_row();

                ui.label("Projectors");
                ui.add(egui::Slider::new(&mut cfg.projectors, 1..=6));
                ui.end_row();

                ui.label("Overlap %");
                ui.add(
                    egui::Slider::new(&mut cfg.overlap_frac, 0.0..=0.8)
                        .step_by(0.001)
                        .custom_formatter(|v, _| format!("{:.1}%", v * 100.0))
                        .custom_parser(|s| {
                            s.trim()
                                .trim_end_matches('%')
                                .parse::<f64>()
                                .ok()
                                .map(|v| if v > 1.0 { v / 100.0 } else { v })
                        }),
                );
                ui.end_row();

                ui.label("Geometry");
                ui.horizontal(|ui| {
                    // Two-click confirm: this throws away the whole calibration.
                    let arm_id = egui::Id::new("edgeblend.reset_geometry_armed");
                    let armed = ui.ctx().data(|d| d.get_temp::<bool>(arm_id)).unwrap_or(false);
                    let label = if armed { "Click again: erase calibration" } else { "Reset geometry (even split)" };
                    let r = ui.button(label);
                    if r.clicked() {
                        if armed {
                            cfg.reset_geometry();
                        }
                        ui.ctx().data_mut(|d| d.insert_temp(arm_id, !armed));
                    } else if armed && ui.ctx().input(|i| i.pointer.any_click()) {
                        ui.ctx().data_mut(|d| d.insert_temp(arm_id, false));
                    }
                    if ui.button("Calibration editor…").clicked() {
                        self.editor.open = true;
                    }
                });
                ui.end_row();

                ui.label("Gamma");
                ui.add(egui::Slider::new(&mut cfg.gamma, 1.0..=3.0).step_by(0.05));
                ui.end_row();

                ui.label("Curve");
                ui.add(egui::Slider::new(&mut cfg.curve, 0.5..=4.0).step_by(0.1));
                ui.end_row();

                ui.label("Pattern");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("pattern")
                        .selected_text(cfg.pattern.label())
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for p in Pattern::ALL {
                                ui.selectable_value(&mut cfg.pattern, p, p.label());
                            }
                        });
                    ui.weak("Ctrl+Alt+T");
                });
                ui.end_row();

                ui.label("Blend ramps");
                ui.checkbox(&mut cfg.blend_ramps, "on (turn off to align raw overlap)");
                ui.end_row();

                ui.label("Projectors on");
                projector_switches(ui, &mut cfg);
                ui.end_row();

                ui.label("Draw cursor");
                ui.checkbox(&mut cfg.show_cursor, "Ctrl+Alt+C");
                ui.end_row();
            });

            if cfg.source.eq_ignore_ascii_case(&cfg.output) {
                ui.colored_label(egui::Color32::from_rgb(220, 150, 40), "Source and output must be different displays.");
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Reset blend settings").clicked() {
                    // Only blend/look settings; keep hardware setup and calibration.
                    cfg = Config {
                        source: cfg.source.clone(),
                        output: cfg.output.clone(),
                        projectors: cfg.projectors,
                        canvas: cfg.canvas,
                        overlap_frac: cfg.overlap_frac,
                        geometry: cfg.geometry.clone(),
                        wall_m: cfg.wall_m,
                        mirror: cfg.mirror,
                        reverse_order: cfg.reverse_order,
                        ..Config::default()
                    };
                }
                if ui.button("Refresh displays").clicked() {
                    self.displays = list_displays();
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Preview");
                egui::ComboBox::from_id_salt("preview")
                    .selected_text(cfg.preview.label())
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for p in Preview::ALL {
                            ui.selectable_value(&mut cfg.preview, p, p.label());
                        }
                    });
            });

            if cfg.preview == Preview::Off {
                // nothing below
            } else if !running {
                ui.weak("Start blending to see the preview.");
            } else if let Some(img) = &preview_snap {
                let need_new = self.preview_tex.as_ref().map_or(true, |(_, seq)| *seq != img.seq);
                if need_new {
                    let color = egui::ColorImage::from_rgba_unmultiplied(
                        [img.width as usize, img.height as usize],
                        &img.rgba,
                    );
                    match &mut self.preview_tex {
                        Some((tex, seq)) => {
                            tex.set(color, egui::TextureOptions::LINEAR);
                            *seq = img.seq;
                        }
                        None => {
                            let tex = ctx.load_texture("preview", color, egui::TextureOptions::LINEAR);
                            self.preview_tex = Some((tex, img.seq));
                        }
                    }
                }
                if let Some((tex, _)) = &self.preview_tex {
                    let avail = ui.available_width();
                    let aspect = img.height as f32 / img.width.max(1) as f32;
                    let size = egui::vec2(avail, avail * aspect);
                    ui.image((tex.id(), size));
                }
            } else {
                ui.weak("Waiting for first preview frame…");
            }

            ui.separator();
            if let Some(e) = &self.save_error {
                ui.colored_label(egui::Color32::from_rgb(220, 70, 70), e);
            }
            ui.small(format!("Settings autosave to {}", self.path.display()));
            ui.small("Ctrl+Alt+Q stops blending. Changes apply live.");
        });

        // Calibration editor viewport (separate OS window; edits the same cfg).
        let preview_handle = self.preview_tex.as_ref().map(|(t, _)| t.clone());
        self.editor
            .show(ctx, &mut cfg, preview_handle.as_ref());

        if cfg.projectors != before.projectors {
            cfg.ensure_geometry();
        }

        if cfg != before {
            let display_changed = cfg.source != before.source || cfg.output != before.output;
            *self.shared.lock().unwrap() = cfg.clone();
            if display_changed && running {
                self.start(); // window/capture are bound to displays: restart
            }
        }

        // Debounced autosave (also catches hotkey toggles).
        if cfg != self.saved {
            let t = *self.changed_at.get_or_insert_with(Instant::now);
            if t.elapsed() > Duration::from_millis(400) {
                self.save(&cfg);
                self.changed_at = None;
            }
        } else {
            self.changed_at = None;
        }

        if want_start {
            self.start();
        }
        if want_stop {
            self.stop();
        }
        if self.editor.open {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.blender.stop();
        let cfg = self.shared.lock().unwrap().clone();
        if cfg != self.saved {
            let _ = cfg.save(&self.path);
        }
    }
}

fn list_displays() -> Vec<(String, String)> {
    display::enumerate()
        .map(|ds| {
            ds.iter()
                .map(|d| {
                    let primary = if d.rect.left == 0 && d.rect.top == 0 { " · primary" } else { "" };
                    let label = format!("{} — {}x{}{}", d.name.trim_start_matches(r"\\.\"), d.width(), d.height(), primary);
                    (d.name.clone(), label)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// P1..Pn checkboxes: untick to black out that projector (diagnostics; not saved).
pub(crate) fn projector_switches(ui: &mut egui::Ui, cfg: &mut Config) {
    ui.horizontal(|ui| {
        let n = cfg.projectors.clamp(1, 6);
        for i in 0..n {
            let bit = 1u32 << i;
            let mut on = cfg.mute_mask & bit == 0;
            if ui.checkbox(&mut on, format!("P{}", i + 1)).changed() {
                if on {
                    cfg.mute_mask &= !bit;
                } else {
                    cfg.mute_mask |= bit;
                }
            }
        }
        if cfg.mute_mask != 0 {
            if ui.button("all on").clicked() {
                cfg.mute_mask = 0;
            }
            ui.colored_label(egui::Color32::from_rgb(230, 160, 40), "some projectors are OFF");
        }
    });
}

fn display_combo(ui: &mut egui::Ui, id: &str, value: &mut String, displays: &[(String, String)]) {
    let selected = displays
        .iter()
        .find(|d| d.0.eq_ignore_ascii_case(value))
        .map(|d| d.1.clone())
        .unwrap_or_else(|| format!("{value} (not connected)"));
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected)
        .width(260.0)
        .show_ui(ui, |ui| {
            for (name, label) in displays {
                ui.selectable_value(value, name.clone(), label);
            }
        });
}
