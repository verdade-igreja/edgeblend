//! Calibration editor: drag projector quads / corners / edges on a wall canvas.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use edgeblend_config::{Config, Projector};
use edgeblend_geom::even_split;

const VIEWPORT_ID: &str = "calibration_editor";
const SIDE_W: f32 = 260.0;
const CORNER_R: f32 = 7.0;
const EDGE_HALF: f32 = 3.0;
const HIT_PAD: f32 = 4.0;
const UNDO_MAX: usize = 100;
const NUDGE: f32 = 0.001;
const MARGIN: f32 = 0.15;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Handle {
    Corner(usize),
    Edge(usize),
    Body,
}

#[derive(Clone, Copy)]
struct Hit {
    proj: usize,
    handle: Handle,
}

struct Drag {
    handle: Handle,
    last_wall: [f32; 2],
}

const ARROWS_MOVE_PIC_ID: &str = "edgeblend.calib.arrows_move_picture";

/// Separate OS window for editing projector wall quads.
pub struct CalibrationEditor {
    pub open: bool,
    show_preview: bool,
    /// When true, arrow nudges move the picture on the wall (negated geometry delta).
    arrows_move_picture: bool,
    /// Canvas widget id (it may hold keyboard focus; arrows must still work then).
    canvas_id: Option<egui::Id>,
    zoom: f32,
    /// Screen-pixel pan of the wall rect centre within the canvas.
    pan: Vec2,
    selected: usize,
    selection: Handle,
    hover: Option<Hit>,
    drag: Option<Drag>,
    undo: Vec<Vec<Projector>>,
    redo: Vec<Vec<Projector>>,
}

impl Default for CalibrationEditor {
    fn default() -> Self {
        Self {
            open: false,
            show_preview: true,
            arrows_move_picture: true,
            canvas_id: None,
            zoom: 1.0,
            pan: Vec2::ZERO,
            selected: 0,
            selection: Handle::Body,
            hover: None,
            drag: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
}

impl CalibrationEditor {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        cfg: &mut Config,
        preview: Option<&egui::TextureHandle>,
    ) {
        if !self.open {
            return;
        }

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of(VIEWPORT_ID),
            egui::ViewportBuilder::default()
                .with_title("edgeblend — calibration")
                .with_inner_size([1600.0, 650.0])
                .with_min_inner_size([900.0, 400.0]),
            |ctx, class| {
                if class == egui::ViewportClass::Embedded {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.label("Calibration editor needs a native multi-viewport window.");
                    });
                    return;
                }

                if ctx.input(|i| i.viewport().close_requested()) {
                    self.open = false;
                }
                if self.open {
                    self.ui(ctx, cfg, preview);
                }
            },
        );

        ctx.request_repaint();
    }

    fn ui(&mut self, ctx: &egui::Context, cfg: &mut Config, preview: Option<&egui::TextureHandle>) {
        cfg.ensure_geometry();
        let n = cfg.geometry.len().max(1);
        if self.selected >= n {
            self.selected = n - 1;
        }

        let mem_id = egui::Id::new(ARROWS_MOVE_PIC_ID);
        if let Some(saved) = ctx.data_mut(|d| d.get_persisted::<bool>(mem_id)) {
            self.arrows_move_picture = saved;
        }

        self.handle_keys(ctx, cfg);

        egui::SidePanel::right("calib_side")
            .exact_width(SIDE_W)
            .resizable(false)
            .show(ctx, |ui| {
                self.side_panel(ui, cfg);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(40, 40, 42)))
            .show(ctx, |ui| {
                let (response, painter) =
                    ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
                let area = response.rect;
                let wall_rect = self.wall_rect(area, cfg.canvas);
                self.canvas_id = Some(response.id);
                // Clicking the canvas takes focus away from side-panel fields so arrows nudge.
                if response.clicked() || response.drag_started() {
                    if let Some(f) = ui.memory(|m| m.focused()) {
                        if f != response.id {
                            ui.memory_mut(|m| m.surrender_focus(f));
                        }
                    }
                }
                // If egui gave the canvas focus, keep arrows/Tab from moving focus away.
                if response.has_focus() {
                    ui.memory_mut(|m| {
                        m.set_focus_lock_filter(
                            response.id,
                            egui::EventFilter {
                                tab: true,
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                escape: false,
                            },
                        )
                    });
                }

                self.handle_canvas_input(ui, &response, cfg, wall_rect);
                self.paint_canvas(&painter, cfg, preview, area, wall_rect);
            });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui, cfg: &mut Config) {
        ui.heading("Calibration");
        ui.add_space(4.0);
        ui.label("Projectors on (untick to black one out):");
        crate::projector_switches(ui, cfg);
        ui.add_space(6.0);

        let changed = ui
            .checkbox(&mut self.arrows_move_picture, "Arrows move the picture")
            .changed();
        ui.small(
            "ON: Arrow keys move the image on the wall (like pushing it). OFF: Arrow keys move the projector shape (light footprint).",
        );
        if changed {
            ui.ctx().data_mut(|d| {
                d.insert_persisted(egui::Id::new(ARROWS_MOVE_PIC_ID), self.arrows_move_picture);
            });
        }
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("Projector");
            let n = cfg.geometry.len();
            egui::ComboBox::from_id_salt("calib_proj")
                .selected_text(format!("P{}", self.selected + 1))
                .width(80.0)
                .show_ui(ui, |ui| {
                    for i in 0..n {
                        if ui
                            .selectable_label(self.selected == i, format!("P{}", i + 1))
                            .clicked()
                        {
                            self.selected = i;
                            self.selection = Handle::Body;
                        }
                    }
                });
        });

        ui.add_space(6.0);
        ui.label("Corners (wall q)");
        let wall_m = cfg.wall_m;
        let selected = self.selected;
        let snap = cfg.geometry.clone();
        if let Some(proj) = cfg.geometry.get_mut(selected) {
            let labels = ["TL", "TR", "BR", "BL"];
            for (i, label) in labels.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.monospace(*label);
                    if self.selection == Handle::Corner(i) {
                        ui.strong("●");
                    }
                    let dx = ui.add(
                        egui::DragValue::new(&mut proj.corners[i][0])
                            .speed(0.0005)
                            .fixed_decimals(4)
                            .prefix("x "),
                    );
                    let dy = ui.add(
                        egui::DragValue::new(&mut proj.corners[i][1])
                            .speed(0.0005)
                            .fixed_decimals(4)
                            .prefix("y "),
                    );
                    let started = dx.drag_started() || dy.drag_started();
                    let typed = (dx.changed() || dy.changed()) && !dx.dragged() && !dy.dragged();
                    if started || typed {
                        self.undo.push(snap.clone());
                        if self.undo.len() > UNDO_MAX {
                            self.undo.remove(0);
                        }
                        self.redo.clear();
                    }
                    if dx.changed() || dy.changed() {
                        self.selection = Handle::Corner(i);
                    }
                });
                let [x, y] = proj.corners[i];
                ui.small(format!("    {:.3} m, {:.3} m", x * wall_m[0], y * wall_m[1]));
            }
        }

        ui.add_space(8.0);
        ui.label("Wall size (m)");
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut cfg.wall_m[0])
                    .speed(0.01)
                    .range(0.1..=100.0)
                    .prefix("W "),
            );
            ui.add(
                egui::DragValue::new(&mut cfg.wall_m[1])
                    .speed(0.01)
                    .range(0.1..=100.0)
                    .prefix("H "),
            );
        });

        ui.add_space(8.0);
        ui.checkbox(&mut self.show_preview, "Show preview");

        ui.add_space(6.0);
        if ui.button("Fit").clicked() {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
        }
        if ui.button("Reset this projector").clicked() {
            self.push_undo(cfg);
            let n = cfg.projectors.max(1) as usize;
            let split = even_split(n, cfg.overlap_frac);
            if let (Some(dst), Some(src)) =
                (cfg.geometry.get_mut(self.selected), split.get(self.selected))
            {
                dst.corners = *src;
            }
            self.redo.clear();
        }
        if ui.button("Reset all").clicked() {
            self.push_undo(cfg);
            cfg.reset_geometry();
            self.redo.clear();
        }

        ui.add_space(12.0);
        ui.separator();
        ui.small("Shapes = where each projector's light really lands (spill included).");
        ui.small("Areas outside the screen are masked black automatically.");
        if let Some(proj) = cfg.geometry.get(self.selected) {
            if let Some(text) = spill_summary(self.selected, &proj.corners, cfg.wall_m) {
                ui.small(text);
            }
        }
        ui.add_space(6.0);
        ui.small("Drag corner / edge / quad · wheel zoom · RMB/MMB pan");
        ui.small("Arrows: 1 projector pixel (Shift ×10, Ctrl ¼ px) · Tab cycle");
        ui.small("Ctrl+Z undo · Ctrl+Y redo · Fit resets view");
    }

    fn handle_keys(&mut self, ctx: &egui::Context, cfg: &mut Config) {
        // A focused widget (e.g. a corner DragValue) also reacts to arrows/Tab/Ctrl+Z:
        // don't act twice. Press Esc or click the canvas to return focus to the editor.
        let focused = ctx.memory(|m| m.focused());
        if focused.is_some() && focused != self.canvas_id {
            return;
        }
        let mut nudge = None;
        let mut undo = false;
        let mut redo = false;
        let mut tab = false;

        ctx.input(|i| {
            if i.key_pressed(egui::Key::Tab) {
                tab = true;
            }
            if i.modifiers.command && i.key_pressed(egui::Key::Z) {
                undo = true;
            }
            if i.modifiers.command && i.key_pressed(egui::Key::Y) {
                redo = true;
            }

            // Step = one projector signal pixel (Shift ×10, Ctrl ¼ px).
            let px = projector_pixel(cfg, self.selected);
            let mut mult = 1.0_f32;
            if i.modifiers.shift {
                mult *= 10.0;
            }
            if i.modifiers.ctrl {
                mult *= 0.25;
            }
            let (sx, sy) = (px[0] * mult, px[1] * mult);

            let mut d = [0.0_f32, 0.0];
            if i.key_pressed(egui::Key::ArrowLeft) {
                d[0] -= sx;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                d[0] += sx;
            }
            if i.key_pressed(egui::Key::ArrowUp) {
                d[1] -= sy;
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                d[1] += sy;
            }
            if d != [0.0, 0.0] {
                nudge = Some(d);
            }
        });

        if tab {
            let n = cfg.geometry.len().max(1);
            self.selected = (self.selected + 1) % n;
            self.selection = Handle::Body;
        }
        if undo {
            self.do_undo(cfg);
        }
        if redo {
            self.do_redo(cfg);
        }
        if let Some(mut d) = nudge {
            if self.arrows_move_picture {
                d[0] = -d[0];
                d[1] = -d[1];
            }
            self.push_undo(cfg);
            self.apply_delta(cfg, self.selected, self.selection, d);
            self.redo.clear();
        }
    }

    fn handle_canvas_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        cfg: &mut Config,
        wall_rect: Rect,
    ) {
        let pointer = response.interact_pointer_pos();

        // Hover (interact_pointer_pos is None unless a button is down; use hover_pos).
        self.hover = response.hover_pos().and_then(|p| self.hit_test(cfg, wall_rect, p));

        // Zoom around cursor
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            if let Some(cursor) = pointer.or_else(|| response.hover_pos()) {
                let before = screen_to_wall(cursor, wall_rect);
                let factor = if scroll > 0.0 { 1.1_f32 } else { 1.0 / 1.1 };
                self.zoom = (self.zoom * factor).clamp(0.5, 8.0);
                let new_rect = self.wall_rect(response.rect, cfg.canvas);
                let after = wall_to_screen(before, new_rect);
                self.pan += cursor - after;
            }
        }

        // Pan with right or middle drag
        if response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
        {
            self.pan += response.drag_delta();
            self.drag = None;
            return;
        }

        // Primary click selects a handle (corner / edge / body) for arrow nudging.
        if response.clicked_by(egui::PointerButton::Primary) {
            if let Some(pos) = pointer.or_else(|| response.hover_pos()) {
                if let Some(hit) = self.hit_test(cfg, wall_rect, pos) {
                    self.selected = hit.proj;
                    self.selection = hit.handle;
                }
            }
        }

        // Primary drag of handles / quads
        if response.drag_started_by(egui::PointerButton::Primary) {
            // drag_started fires only after the drag threshold; hit-test where the button
            // was pressed, not where the pointer is now.
            let origin = ui.input(|i| i.pointer.press_origin());
            if let Some(pos) = origin.or(pointer) {
                if let Some(hit) = self.hit_test(cfg, wall_rect, pos) {
                    self.selected = hit.proj;
                    self.selection = hit.handle;
                    self.push_undo(cfg);
                    self.redo.clear();
                    self.drag = Some(Drag {
                        handle: hit.handle,
                        last_wall: screen_to_wall(pos, wall_rect),
                    });
                }
            }
        }

        if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(pos) = pointer {
                if self.drag.is_some() {
                    let wall_rect = self.wall_rect(response.rect, cfg.canvas);
                    let now = screen_to_wall(pos, wall_rect);
                    let drag = self.drag.as_mut().unwrap();
                    let delta = [now[0] - drag.last_wall[0], now[1] - drag.last_wall[1]];
                    let handle = drag.handle;
                    drag.last_wall = now;
                    let proj = self.selected;
                    Self::apply_delta_static(cfg, proj, handle, delta);
                }
            }
        }

        if response.drag_stopped() {
            self.drag = None;
        }
    }

    fn paint_canvas(
        &self,
        painter: &egui::Painter,
        cfg: &Config,
        preview: Option<&egui::TextureHandle>,
        area: Rect,
        wall_rect: Rect,
    ) {
        painter.rect_filled(area, 0.0, Color32::from_rgb(40, 40, 42));

        // Wall interior
        if self.show_preview {
            if let Some(tex) = preview {
                painter.image(
                    tex.id(),
                    wall_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                painter.rect_filled(wall_rect, 0.0, Color32::from_rgb(28, 28, 30));
            }
        } else {
            painter.rect_filled(wall_rect, 0.0, Color32::from_rgb(28, 28, 30));
        }

        // Grid every 10%
        let grid = Color32::from_rgba_unmultiplied(255, 255, 255, 28);
        let label_c = Color32::from_rgba_unmultiplied(220, 220, 220, 140);
        for i in 0..=10 {
            let t = i as f32 * 0.1;
            let x = wall_rect.left() + t * wall_rect.width();
            let y = wall_rect.top() + t * wall_rect.height();
            painter.line_segment(
                [Pos2::new(x, wall_rect.top()), Pos2::new(x, wall_rect.bottom())],
                Stroke::new(1.0_f32, grid),
            );
            painter.line_segment(
                [Pos2::new(wall_rect.left(), y), Pos2::new(wall_rect.right(), y)],
                Stroke::new(1.0_f32, grid),
            );
            if i > 0 && i < 10 {
                painter.text(
                    Pos2::new(x, wall_rect.top() - 2.0),
                    egui::Align2::CENTER_BOTTOM,
                    format!("{}%", i * 10),
                    egui::FontId::proportional(10.0),
                    label_c,
                );
                painter.text(
                    Pos2::new(wall_rect.left() - 2.0, y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{}%", i * 10),
                    egui::FontId::proportional(10.0),
                    label_c,
                );
            }
        }

        painter.rect_stroke(wall_rect, 0.0, Stroke::new(1.5_f32, Color32::WHITE));

        // Projectors: non-selected first, selected on top
        let n = cfg.geometry.len();
        let order: Vec<usize> = (0..n).filter(|&i| i != self.selected).chain(std::iter::once(self.selected)).collect();

        // Fills first so off-wall hatch can shade spilled light
        for &i in &order {
            let Some(proj) = cfg.geometry.get(i) else { continue };
            let pts: [Pos2; 4] = std::array::from_fn(|c| wall_to_screen(proj.corners[c], wall_rect));
            let color = projector_color(i);
            let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 64);
            painter.add(egui::Shape::convex_polygon(pts.to_vec(), fill, Stroke::NONE));
        }

        paint_offwall_hatch(painter, area, wall_rect);
        painter.text(
            Pos2::new(area.left() + 8.0, area.top() + 6.0),
            egui::Align2::LEFT_TOP,
            "off-screen: projector light here is drawn black",
            egui::FontId::proportional(11.0),
            Color32::from_rgba_unmultiplied(190, 190, 190, 170),
        );
        let mode_label = if self.arrows_move_picture {
            "Arrows: move picture"
        } else {
            "Arrows: move projector shape"
        };
        painter.text(
            Pos2::new(area.left() + 8.0, area.top() + 22.0),
            egui::Align2::LEFT_TOP,
            mode_label,
            egui::FontId::proportional(11.0),
            Color32::from_rgba_unmultiplied(220, 220, 140, 200),
        );

        // Outlines, labels, handles on top of hatch
        for &i in &order {
            let Some(proj) = cfg.geometry.get(i) else { continue };
            let pts: [Pos2; 4] = std::array::from_fn(|c| wall_to_screen(proj.corners[c], wall_rect));
            let color = projector_color(i);
            let thick = if i == self.selected { 3.5_f32 } else { 2.0_f32 };

            painter.add(egui::Shape::convex_polygon(
                pts.to_vec(),
                Color32::TRANSPARENT,
                Stroke::new(thick, color),
            ));

            let centre = Pos2::new(
                (pts[0].x + pts[1].x + pts[2].x + pts[3].x) * 0.25,
                (pts[0].y + pts[1].y + pts[2].y + pts[3].y) * 0.25,
            );
            painter.text(
                centre,
                egui::Align2::CENTER_CENTER,
                format!("P{}", i + 1),
                egui::FontId::proportional(16.0),
                Color32::WHITE,
            );

            if i == self.selected {
                for c in 0..4 {
                    let hover = self.hover.is_some_and(|h| {
                        h.proj == i && h.handle == Handle::Corner(c)
                    });
                    let selected = self.selection == Handle::Corner(c);
                    let r = if hover || selected { CORNER_R + 2.0 } else { CORNER_R };
                    painter.circle_filled(pts[c], r, color);
                    painter.circle_stroke(pts[c], r, Stroke::new(1.0_f32, Color32::WHITE));
                }
                for e in 0..4 {
                    let mid = Pos2::new(
                        (pts[e].x + pts[(e + 1) % 4].x) * 0.5,
                        (pts[e].y + pts[(e + 1) % 4].y) * 0.5,
                    );
                    let hover = self.hover.is_some_and(|h| {
                        h.proj == i && h.handle == Handle::Edge(e)
                    });
                    let selected = self.selection == Handle::Edge(e);
                    let half = if hover || selected { EDGE_HALF + 1.5 } else { EDGE_HALF };
                    let rect = Rect::from_center_size(mid, Vec2::splat(half * 2.0));
                    painter.rect_filled(rect, 0.0, color);
                    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, Color32::WHITE));
                }
            } else {
                // Dimmer handles on non-selected
                for c in 0..4 {
                    let hover = self.hover.is_some_and(|h| {
                        h.proj == i && h.handle == Handle::Corner(c)
                    });
                    let r = if hover { CORNER_R + 1.0 } else { CORNER_R - 1.0 };
                    painter.circle_filled(pts[c], r.max(4.0), color.gamma_multiply(0.85));
                }
                for e in 0..4 {
                    let mid = Pos2::new(
                        (pts[e].x + pts[(e + 1) % 4].x) * 0.5,
                        (pts[e].y + pts[(e + 1) % 4].y) * 0.5,
                    );
                    let hover = self.hover.is_some_and(|h| {
                        h.proj == i && h.handle == Handle::Edge(e)
                    });
                    let half = if hover { EDGE_HALF + 1.0 } else { EDGE_HALF - 0.5 };
                    let rect = Rect::from_center_size(mid, Vec2::splat(half.max(2.0) * 2.0));
                    painter.rect_filled(rect, 0.0, color.gamma_multiply(0.85));
                }
            }
        }
    }

    fn wall_rect(&self, area: Rect, canvas: [u32; 2]) -> Rect {
        let aspect = canvas[0] as f32 / canvas[1].max(1) as f32;
        let factor = 1.0 + 2.0 * MARGIN;
        let max_w = area.width() / factor;
        let max_h = area.height() / factor;
        let (base_w, base_h) = if max_w / max_h > aspect {
            (max_h * aspect, max_h)
        } else {
            (max_w, max_w / aspect)
        };
        let w = base_w * self.zoom;
        let h = base_h * self.zoom;
        Rect::from_center_size(area.center() + self.pan, Vec2::new(w, h))
    }

    fn hit_test(&self, cfg: &Config, wall_rect: Rect, pos: Pos2) -> Option<Hit> {
        let n = cfg.geometry.len();
        if n == 0 {
            return None;
        }
        // Selected projector first, then others in reverse draw order (top-most first).
        let mut order: Vec<usize> = Vec::with_capacity(n);
        order.push(self.selected.min(n - 1));
        for i in (0..n).rev() {
            if i != order[0] {
                order.push(i);
            }
        }

        // Corners
        for &pi in &order {
            let proj = &cfg.geometry[pi];
            for c in 0..4 {
                let p = wall_to_screen(proj.corners[c], wall_rect);
                if (p - pos).length() <= CORNER_R + HIT_PAD {
                    return Some(Hit {
                        proj: pi,
                        handle: Handle::Corner(c),
                    });
                }
            }
        }
        // Edge midpoints
        for &pi in &order {
            let proj = &cfg.geometry[pi];
            let pts: [Pos2; 4] = std::array::from_fn(|c| wall_to_screen(proj.corners[c], wall_rect));
            for e in 0..4 {
                let mid = Pos2::new(
                    (pts[e].x + pts[(e + 1) % 4].x) * 0.5,
                    (pts[e].y + pts[(e + 1) % 4].y) * 0.5,
                );
                let rect = Rect::from_center_size(mid, Vec2::splat((EDGE_HALF + HIT_PAD) * 2.0));
                if rect.contains(pos) {
                    return Some(Hit {
                        proj: pi,
                        handle: Handle::Edge(e),
                    });
                }
            }
        }
        // Interiors
        for &pi in &order {
            let proj = &cfg.geometry[pi];
            let pts: [Pos2; 4] = std::array::from_fn(|c| wall_to_screen(proj.corners[c], wall_rect));
            if point_in_quad(pos, &pts) {
                return Some(Hit {
                    proj: pi,
                    handle: Handle::Body,
                });
            }
        }
        None
    }

    fn push_undo(&mut self, cfg: &Config) {
        self.undo.push(cfg.geometry.clone());
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
    }

    fn do_undo(&mut self, cfg: &mut Config) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(cfg.geometry.clone());
            cfg.geometry = prev;
            cfg.projectors = cfg.geometry.len().max(1) as u32;
        }
    }

    fn do_redo(&mut self, cfg: &mut Config) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(cfg.geometry.clone());
            if self.undo.len() > UNDO_MAX {
                self.undo.remove(0);
            }
            cfg.geometry = next;
            cfg.projectors = cfg.geometry.len().max(1) as u32;
        }
    }

    fn apply_delta(&mut self, cfg: &mut Config, proj: usize, handle: Handle, delta: [f32; 2]) {
        Self::apply_delta_static(cfg, proj, handle, delta);
    }

    fn apply_delta_static(cfg: &mut Config, proj: usize, handle: Handle, delta: [f32; 2]) {
        let Some(p) = cfg.geometry.get_mut(proj) else { return };
        match handle {
            Handle::Corner(c) => {
                p.corners[c][0] += delta[0];
                p.corners[c][1] += delta[1];
            }
            Handle::Edge(e) => {
                let a = e;
                let b = (e + 1) % 4;
                p.corners[a][0] += delta[0];
                p.corners[a][1] += delta[1];
                p.corners[b][0] += delta[0];
                p.corners[b][1] += delta[1];
            }
            Handle::Body => {
                for c in &mut p.corners {
                    c[0] += delta[0];
                    c[1] += delta[1];
                }
            }
        }
    }
}

/// Signal pixels each projector receives from the video-wall box, assuming a 1920x1080 signal
/// split into 3 equal tiles. Ideally derived from the output size and projector count.
const SIGNAL_TILE: [f32; 2] = [640.0, 1080.0];

/// Size of one projector signal pixel in wall units, from the projector's quad
/// (average of opposite edges). Falls back to the fixed NUDGE step.
fn projector_pixel(cfg: &Config, proj: usize) -> [f32; 2] {
    let Some(p) = cfg.geometry.get(proj) else { return [NUDGE, NUDGE] };
    let len = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let c = &p.corners;
    let w = 0.5 * (len(c[0], c[1]) + len(c[3], c[2]));
    let h = 0.5 * (len(c[0], c[3]) + len(c[1], c[2]));
    if w <= 0.0 || h <= 0.0 {
        return [NUDGE, NUDGE];
    }
    [w / SIGNAL_TILE[0], h / SIGNAL_TILE[1]]
}

fn projector_color(i: usize) -> Color32 {
    match i % 3 {
        0 => Color32::from_rgb(220, 55, 55),
        1 => Color32::from_rgb(55, 185, 70),
        _ => Color32::from_rgb(55, 110, 230),
    }
}

/// Diagonal hatch in the visible canvas outside the wall rect (off-screen / spill zone).
fn paint_offwall_hatch(painter: &egui::Painter, area: Rect, wall: Rect) {
    let stroke = Stroke::new(
        1.0_f32,
        Color32::from_rgba_unmultiplied(160, 160, 160, 45),
    );
    let step = 12.0_f32;

    let top = Rect::from_min_max(
        Pos2::new(area.left(), area.top()),
        Pos2::new(area.right(), wall.top().min(area.bottom()).max(area.top())),
    );
    let bottom = Rect::from_min_max(
        Pos2::new(area.left(), wall.bottom().max(area.top()).min(area.bottom())),
        Pos2::new(area.right(), area.bottom()),
    );
    let left = Rect::from_min_max(
        Pos2::new(area.left(), wall.top().max(area.top())),
        Pos2::new(wall.left().min(area.right()).max(area.left()), wall.bottom().min(area.bottom())),
    );
    let right = Rect::from_min_max(
        Pos2::new(wall.right().max(area.left()).min(area.right()), wall.top().max(area.top())),
        Pos2::new(area.right(), wall.bottom().min(area.bottom())),
    );

    for r in [top, bottom, left, right] {
        hatch_rect(painter, r, step, stroke);
    }
}

fn hatch_rect(painter: &egui::Painter, r: Rect, step: f32, stroke: Stroke) {
    if r.width() < 0.5 || r.height() < 0.5 {
        return;
    }
    // Diagonals of constant (x - y), spaced ~step px.
    let min_d = r.left() - r.bottom();
    let max_d = r.right() - r.top();
    let mut d = (min_d / step).floor() * step;
    while d <= max_d + 0.01 {
        let mut a: Option<Pos2> = None;
        let mut b: Option<Pos2> = None;
        let mut add = |p: Pos2| {
            match (a, b) {
                (None, _) => a = Some(p),
                (Some(q), None) if (q - p).length_sq() >= 0.25 => b = Some(p),
                _ => {}
            }
        };
        let y_l = r.left() - d;
        if (r.top()..=r.bottom()).contains(&y_l) {
            add(Pos2::new(r.left(), y_l));
        }
        let y_r = r.right() - d;
        if (r.top()..=r.bottom()).contains(&y_r) {
            add(Pos2::new(r.right(), y_r));
        }
        let x_t = r.top() + d;
        if (r.left()..=r.right()).contains(&x_t) {
            add(Pos2::new(x_t, r.top()));
        }
        let x_b = r.bottom() + d;
        if (r.left()..=r.right()).contains(&x_b) {
            add(Pos2::new(x_b, r.bottom()));
        }
        if let (Some(p0), Some(p1)) = (a, b) {
            painter.line_segment([p0, p1], stroke);
        }
        d += step;
    }
}

/// Spill distances past the wall for a projector, in cm (rounded), or None if fully on-screen.
fn spill_summary(proj_i: usize, corners: &[[f32; 2]; 4], wall_m: [f32; 2]) -> Option<String> {
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for c in corners {
        min_x = min_x.min(c[0]);
        max_x = max_x.max(c[0]);
        min_y = min_y.min(c[1]);
        max_y = max_y.max(c[1]);
    }
    let mut parts: Vec<String> = Vec::new();
    if min_y < 0.0 {
        let cm = ((-min_y) * wall_m[1] * 100.0).round() as i32;
        parts.push(format!("top {cm} cm"));
    }
    if max_y > 1.0 {
        let cm = ((max_y - 1.0) * wall_m[1] * 100.0).round() as i32;
        parts.push(format!("bottom {cm} cm"));
    }
    if min_x < 0.0 {
        let cm = ((-min_x) * wall_m[0] * 100.0).round() as i32;
        parts.push(format!("left {cm} cm"));
    }
    if max_x > 1.0 {
        let cm = ((max_x - 1.0) * wall_m[0] * 100.0).round() as i32;
        parts.push(format!("right {cm} cm"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("P{} spills: {}", proj_i + 1, parts.join(" / ")))
    }
}

fn wall_to_screen(q: [f32; 2], wall: Rect) -> Pos2 {
    Pos2::new(
        wall.left() + q[0] * wall.width(),
        wall.top() + q[1] * wall.height(),
    )
}

fn screen_to_wall(p: Pos2, wall: Rect) -> [f32; 2] {
    [
        (p.x - wall.left()) / wall.width().max(1e-6),
        (p.y - wall.top()) / wall.height().max(1e-6),
    ]
}

fn point_in_tri(p: Pos2, a: Pos2, b: Pos2, c: Pos2) -> bool {
    let sign = |p1: Pos2, p2: Pos2, p3: Pos2| {
        (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y)
    };
    let b1 = sign(p, a, b) < 0.0;
    let b2 = sign(p, b, c) < 0.0;
    let b3 = sign(p, c, a) < 0.0;
    b1 == b2 && b2 == b3
}

fn point_in_quad(p: Pos2, c: &[Pos2; 4]) -> bool {
    point_in_tri(p, c[0], c[1], c[2]) || point_in_tri(p, c[0], c[2], c[3])
}
