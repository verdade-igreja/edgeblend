//! "Config history" section of the panel: milestones, restore, export / import.
//! Dialogs from `rfd` are modal and block the UI thread while open (acceptable here; the
//! blend engine runs on its own thread and keeps going).

use eframe::egui;
use edgeblend_config::history::{self, Entry, Kind, Service, Source};
use edgeblend_config::Config;
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct HistoryPanel {
    entries: Vec<Entry>,
    refreshed: Option<Instant>,
    milestone_name: String,
    milestones_only: bool,
    confirm: Option<String>,
    message: Option<(bool, String)>, // (is_error, text)
}

impl HistoryPanel {
    fn refresh(&mut self, svc: &Service) {
        self.entries = svc.list();
        self.refreshed = Some(Instant::now());
    }

    /// Returns true when the live config was replaced (restore / import).
    pub fn show(&mut self, ui: &mut egui::Ui, svc: &Service, cfg: &Config) -> bool {
        let mut applied = false;
        egui::CollapsingHeader::new("History").show(ui, |ui| {
            // The body only runs while open, so this reads the disk only then.
            if self.refreshed.is_none_or(|t| t.elapsed() > REFRESH) {
                self.refresh(svc);
            }
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.milestone_name)
                        .hint_text("milestone name (optional)")
                        .desired_width(180.0),
                );
                if ui.button("Save milestone").clicked() {
                    let name = Some(std::mem::take(&mut self.milestone_name));
                    self.message = Some(match svc.save_milestone(name, Source::Desktop) {
                        Ok(_) => (false, "Milestone saved.".into()),
                        Err(e) => (true, format!("save failed: {e:#}")),
                    });
                    self.refresh(svc);
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Export…").clicked() {
                    self.message = export(cfg);
                }
                if ui.button("Import…").clicked() {
                    if let Some(m) = import(svc) {
                        applied = !m.0;
                        self.message = Some(m);
                        self.refresh(svc);
                    }
                }
            });
            if let Some((err, msg)) = &self.message {
                let c = if *err { egui::Color32::from_rgb(220, 70, 70) } else { egui::Color32::from_rgb(60, 190, 90) };
                ui.colored_label(c, msg);
            }
            ui.checkbox(&mut self.milestones_only, "milestones only");
            ui.separator();

            let mut restore: Option<String> = None;
            egui::ScrollArea::vertical().max_height(240.0).id_salt("history_list").show(ui, |ui| {
                for e in self.entries.iter().filter(|e| !self.milestones_only || e.kind == Kind::Milestone) {
                    ui.horizontal(|ui| {
                        let when = local_string(e.created_ms);
                        if e.broken.is_some() {
                            ui.weak(format!("{when}  (unreadable file {})", e.id));
                            return;
                        }
                        let label = match (&e.kind, &e.name) {
                            (Kind::Milestone, Some(n)) => format!("★ {n}"),
                            (Kind::Milestone, None) => "★ milestone".into(),
                            (_, _) => format!("auto ({})", source_label(e.source)),
                        };
                        ui.label(when);
                        if e.kind == Kind::Milestone {
                            ui.label(egui::RichText::new(label).strong().color(egui::Color32::from_rgb(230, 180, 50)));
                        } else {
                            ui.weak(label);
                        }
                        if self.confirm.as_deref() == Some(e.id.as_str()) {
                            if ui.button("Yes, restore").clicked() {
                                restore = Some(e.id.clone());
                            }
                            if ui.button("Cancel").clicked() {
                                self.confirm = None;
                            }
                        } else if ui.button("Restore").clicked() {
                            self.confirm = Some(e.id.clone());
                        }
                    });
                }
            });
            if let Some(id) = restore {
                self.confirm = None;
                self.message = Some(match svc.restore(&id) {
                    Ok(()) => {
                        applied = true;
                        (false, "Restored. Nothing was deleted.".into())
                    }
                    Err(e) => (true, format!("restore failed: {e:#}")),
                });
                self.refresh(svc);
            }
            ui.small(format!("Versions are kept in {}", svc.dir().display()));
        });
        applied
    }
}

fn source_label(s: Source) -> &'static str {
    match s {
        Source::Desktop => "desktop",
        Source::Remote => "remote",
        Source::Restore => "restore",
        Source::Import => "import",
        Source::Auto => "edit",
    }
}

fn export(cfg: &Config) -> Option<(bool, String)> {
    let (y, mo, d, h, mi) = local_parts(history::now_ms());
    let path = rfd::FileDialog::new()
        .set_title("Export edgeblend config")
        .set_file_name(format!("edgeblend-config-{y:04}{mo:02}{d:02}-{h:02}{mi:02}.toml"))
        .add_filter("TOML", &["toml"])
        .save_file()?;
    Some(match cfg.export(&path) {
        Ok(()) => (false, format!("Exported to {}", path.display())),
        Err(e) => (true, format!("export failed: {e:#}")),
    })
}

/// None = dialog cancelled. Some((is_error, text)) otherwise.
fn import(svc: &Service) -> Option<(bool, String)> {
    let path = rfd::FileDialog::new()
        .set_title("Import edgeblend config")
        .add_filter("TOML", &["toml"])
        .pick_file()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return Some((true, format!("import failed: reading {}: {e}", path.display()))),
    };
    Some(match svc.import_text(&text) {
        Ok(()) => (false, "Imported. The previous config was saved as milestone \"before import\".".into()),
        Err(e) => (true, format!("import failed, nothing changed: {e:#}")),
    })
}

/// (year, month, day, hour, minute) in local time.
fn local_parts(ms: i64) -> (i32, u32, u32, u32, u32) {
    let s = history::format_rfc3339(ms); // 2026-11-22T15:30:12.123Z
    let n = |r: std::ops::Range<usize>| s[r].parse::<u32>().unwrap_or(0);
    let utc = (n(0..4) as i32, n(5..7), n(8..10), n(11..13), n(14..16));
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::SYSTEMTIME;
        use windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime;
        let st = SYSTEMTIME {
            wYear: utc.0 as u16,
            wMonth: utc.1 as u16,
            wDay: utc.2 as u16,
            wHour: utc.3 as u16,
            wMinute: utc.4 as u16,
            ..Default::default()
        };
        let mut local = SYSTEMTIME::default();
        if unsafe { SystemTimeToTzSpecificLocalTime(None, &st, &mut local) }.is_ok() {
            return (local.wYear as i32, local.wMonth as u32, local.wDay as u32, local.wHour as u32, local.wMinute as u32);
        }
    }
    utc
}

fn local_string(ms: i64) -> String {
    let (y, mo, d, h, mi) = local_parts(ms);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}")
}
