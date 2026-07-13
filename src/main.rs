//! DBHEditor — a Detroit: Become Human save editor.
//!
//! Edit the relationship/stability of a checkpoint before loading it: drop
//! Connor's stability to unlock a deviant choice, raise Connor <-> Hank so Hank
//! doesn't quit, etc. Includes automatic per-save backups plus full-folder
//! backup & restore.
#![cfg_attr(windows, windows_subsystem = "windows")]

use dbheditor::qdsav::{self, Scalar, ADVANCED_VARS, GAUGES};
use dbheditor::saves::{self, FullBackup, SaveEntry};
use eframe::egui;
use std::path::PathBuf;

/// Variable name of the flowchart instability value that actually gates
/// Connor's deviant choices (see ADVANCED_VARS / memory notes).
const DEVIANCE_VAR: &str = "GM 0000XJaugeConnorDeviance";

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1060.0, 720.0])
            .with_min_inner_size([820.0, 520.0])
            .with_title("DBHEditor — Detroit: Become Human Save Editor"),
        ..Default::default()
    };
    eframe::run_native(
        "DBHEditor",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            cc.egui_ctx.set_zoom_factor(1.1);
            Ok(Box::new(App::new()))
        }),
    )
}

#[derive(PartialEq, Clone, Copy)]
enum StatusKind {
    Info,
    Good,
    Warn,
    Error,
}

struct App {
    root: Option<PathBuf>,
    saves: Vec<SaveEntry>,
    edited: Vec<bool>,
    filter: String,
    selected: Option<usize>,
    payload: Option<Vec<u8>>,
    chosen: Vec<Option<usize>>,
    original: Vec<Option<usize>>,
    adv: Vec<Option<Scalar>>,
    adv_orig: Vec<Option<Scalar>>,
    status: String,
    status_kind: StatusKind,
    // Backup / restore UI
    show_restore: bool,
    full_backups: Vec<FullBackup>,
    pending_restore: Option<PathBuf>,
}

impl App {
    fn new() -> Self {
        let root = saves::default_save_root();
        let mut app = Self {
            root,
            saves: Vec::new(),
            edited: Vec::new(),
            filter: String::new(),
            selected: None,
            payload: None,
            chosen: Vec::new(),
            original: Vec::new(),
            adv: Vec::new(),
            adv_orig: Vec::new(),
            status: String::new(),
            status_kind: StatusKind::Info,
            show_restore: false,
            full_backups: Vec::new(),
            pending_restore: None,
        };
        app.rescan();
        match (&app.root, app.saves.is_empty()) {
            (Some(_), false) => app.set_status(StatusKind::Info, format!("Found {} checkpoints. Pick one to edit.", app.saves.len())),
            (Some(_), true) => app.set_status(StatusKind::Warn, "Save folder found, but no checkpoints yet."),
            (None, _) => app.set_status(StatusKind::Warn, "Couldn't auto-find your saves. Use “Open folder…”."),
        }
        app
    }

    fn set_status(&mut self, kind: StatusKind, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_kind = kind;
    }

    fn rescan(&mut self) {
        if let Some(root) = self.root.clone() {
            self.saves = saves::find_checkpoints(&root);
            self.edited = self.saves.iter().map(|s| saves::latest_file_backup(&s.path).is_some()).collect();
        }
    }

    fn set_root(&mut self, dir: PathBuf) {
        self.root = Some(dir);
        self.selected = None;
        self.payload = None;
        self.rescan();
        self.set_status(StatusKind::Info, format!("{} checkpoints found.", self.saves.len()));
    }

    fn load_selected(&mut self, index: usize) {
        let path = self.saves[index].path.clone();
        self.load_path(path, Some(index));
    }

    fn load_path(&mut self, path: PathBuf, list_index: Option<usize>) {
        match qdsav::read_qdsav(&path) {
            Ok(payload) => {
                let mut chosen = vec![None; GAUGES.len()];
                for r in qdsav::scan(&payload) {
                    if let Some(cur) = &r.current {
                        chosen[r.gauge_index] = GAUGES[r.gauge_index].levels.iter().position(|l| l == cur);
                    }
                }
                self.original = chosen.clone();
                self.chosen = chosen;
                let adv: Vec<Option<Scalar>> = ADVANCED_VARS.iter().map(|a| qdsav::read_scalar(&payload, a.var)).collect();
                self.adv_orig = adv.clone();
                self.adv = adv;
                self.payload = Some(payload);
                self.selected = list_index;
                let present = self.original.iter().filter(|c| c.is_some()).count();
                self.set_status(StatusKind::Good, format!("Loaded “{}” — {} gauges.", path.display(), present));
            }
            Err(e) => self.set_status(StatusKind::Error, format!("Couldn't read save: {e}")),
        }
    }

    fn changes(&self) -> Vec<(usize, usize)> {
        self.chosen
            .iter()
            .zip(&self.original)
            .enumerate()
            .filter_map(|(gi, (now, orig))| now.filter(|n| Some(*n) != *orig).map(|n| (gi, n)))
            .collect()
    }

    fn adv_changes(&self) -> Vec<(usize, Scalar)> {
        self.adv
            .iter()
            .zip(&self.adv_orig)
            .enumerate()
            .filter_map(|(i, (now, orig))| now.and_then(|n| (Some(n) != *orig).then_some((i, n))))
            .collect()
    }

    fn dirty(&self) -> bool {
        !self.changes().is_empty() || !self.adv_changes().is_empty()
    }

    fn revert(&mut self) {
        self.chosen = self.original.clone();
        self.adv = self.adv_orig.clone();
    }

    fn save(&mut self) {
        let Some(sel) = self.selected else { return };
        let path = self.saves[sel].path.clone();
        let changes = self.changes();
        let adv_changes = self.adv_changes();
        if changes.is_empty() && adv_changes.is_empty() {
            self.set_status(StatusKind::Info, "No changes to save.");
            return;
        }
        let Some(mut payload) = self.payload.clone() else { return };
        let backup = match saves::backup(&path) {
            Ok(b) => b,
            Err(e) => return self.set_status(StatusKind::Error, format!("Backup failed, nothing written: {e}")),
        };
        for (gi, lvl) in &changes {
            if let Err(e) = qdsav::set_level(&mut payload, *gi, GAUGES[*gi].levels[*lvl]) {
                return self.set_status(StatusKind::Error, format!("Edit failed ({}): {e}", GAUGES[*gi].display));
            }
        }
        for (ai, val) in &adv_changes {
            if let Err(e) = qdsav::set_scalar(&mut payload, ADVANCED_VARS[*ai].var, *val) {
                return self.set_status(StatusKind::Error, format!("Edit failed ({}): {e}", ADVANCED_VARS[*ai].label));
            }
        }
        match qdsav::write_qdsav(&path, &payload) {
            Ok(()) => {
                self.payload = Some(payload);
                self.original = self.chosen.clone();
                self.adv_orig = self.adv.clone();
                self.edited[sel] = true;
                let name = backup.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.set_status(StatusKind::Good, format!("Saved {} change(s). Backup: {name}. Load this checkpoint in-game.", changes.len() + adv_changes.len()));
            }
            Err(e) => self.set_status(StatusKind::Error, format!("Write failed: {e}")),
        }
    }

    /// One-click: queue Connor's instability to a deviant value (user still Saves).
    fn queue_unlock_deviant(&mut self) {
        if let Some(i) = ADVANCED_VARS.iter().position(|a| a.var == DEVIANCE_VAR) {
            if self.adv[i].is_some() {
                self.adv[i] = Some(Scalar::Float(100.0));
                self.set_status(StatusKind::Info, "Instability set to 100 (deviant). Click “Save changes” to write it.");
            } else {
                self.set_status(StatusKind::Warn, "This checkpoint has no Connor instability value.");
            }
        }
    }

    fn undo_last_save(&mut self) {
        let Some(sel) = self.selected else { return };
        let path = self.saves[sel].path.clone();
        match saves::restore_latest_file_backup(&path) {
            Ok(()) => {
                self.load_selected(sel);
                self.set_status(StatusKind::Good, "Reverted this checkpoint to its most recent backup.");
            }
            Err(e) => self.set_status(StatusKind::Warn, format!("Nothing to undo: {e}")),
        }
    }

    fn do_full_backup(&mut self) {
        let Some(root) = self.root.clone() else { return };
        match saves::create_full_backup(&root, None) {
            Ok(dir) => {
                let name = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.set_status(StatusKind::Good, format!("Full backup created: {name}"));
            }
            Err(e) => self.set_status(StatusKind::Error, format!("Full backup failed: {e}")),
        }
    }

    fn open_restore(&mut self) {
        if let Some(root) = &self.root {
            self.full_backups = saves::list_full_backups(root);
        }
        self.show_restore = true;
    }

    fn perform_restore(&mut self, backup: PathBuf) {
        let Some(root) = self.root.clone() else { return };
        match saves::restore_full_backup(&root, &backup) {
            Ok(safety) => {
                let s = safety.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                self.selected = None;
                self.payload = None;
                self.rescan();
                self.set_status(StatusKind::Good, format!("Restored backup. (Your prior state was saved as “{s}”.)"));
            }
            Err(e) => self.set_status(StatusKind::Error, format!("Restore failed: {e}")),
        }
        self.show_restore = false;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.top_bar(ctx);
        self.left_panel(ctx);
        self.status_bar(ctx);
        self.central(ctx);
        self.restore_window(ctx);
        self.confirm_window(ctx);
    }
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("DBHEditor");
                ui.label(egui::RichText::new("Detroit: Become Human save editor").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Open folder…").on_hover_text("Choose your save folder manually").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.set_root(dir);
                        }
                    }
                    if let Some(root) = self.root.clone() {
                        if ui.button("Reveal saves").on_hover_text("Open the save folder in Explorer").clicked() {
                            saves::reveal(&root);
                        }
                    }
                    ui.separator();
                    if ui.button("↩ Restore backup…").on_hover_text("Roll the whole save folder back to a full backup").clicked() {
                        self.open_restore();
                    }
                    if ui
                        .add_enabled(self.root.is_some(), egui::Button::new("💾 Full backup"))
                        .on_hover_text("Copy your entire save folder aside, so you can always roll back")
                        .clicked()
                    {
                        self.do_full_backup();
                    }
                });
            });
            ui.add_space(4.0);
        });
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("saves").resizable(true).default_width(340.0).show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Checkpoints").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("⟳").on_hover_text("Rescan folder").clicked() {
                        self.rescan();
                    }
                });
            });
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("filter, e.g. Crossroads").desired_width(f32::INFINITY));
            ui.separator();

            let filter = self.filter.to_lowercase();
            let mut to_load = None;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                // Group contiguous entries by chapter (saves are sorted by label).
                let mut i = 0;
                while i < self.saves.len() {
                    let chapter = self.saves[i].chapter.clone();
                    let group_end = self.saves[i..].iter().take_while(|e| e.chapter == chapter).count() + i;
                    let visible: Vec<usize> = (i..group_end)
                        .filter(|&j| filter.is_empty() || self.saves[j].label().to_lowercase().contains(&filter))
                        .collect();
                    if !visible.is_empty() {
                        let header = if chapter.is_empty() { "Other".to_string() } else { chapter };
                        egui::CollapsingHeader::new(egui::RichText::new(header).strong())
                            .default_open(true)
                            .id_salt(i)
                            .show(ui, |ui| {
                                for j in visible {
                                    let sel = self.selected == Some(j);
                                    let mut label = self.saves[j].checkpoint.clone();
                                    if self.edited[j] {
                                        label.push_str("  ✎");
                                    }
                                    if ui.selectable_label(sel, label).on_hover_text(if self.edited[j] { "Edited (has a backup)" } else { "" }).clicked() {
                                        to_load = Some(j);
                                    }
                                }
                            });
                    }
                    i = group_end;
                }
            });
            if let Some(j) = to_load {
                self.load_selected(j);
            }
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(3.0);
            let color = match self.status_kind {
                StatusKind::Info => egui::Color32::GRAY,
                StatusKind::Good => egui::Color32::from_rgb(120, 200, 120),
                StatusKind::Warn => egui::Color32::from_rgb(230, 190, 90),
                StatusKind::Error => egui::Color32::from_rgb(230, 110, 110),
            };
            ui.label(egui::RichText::new(&self.status).color(color));
            ui.add_space(3.0);
        });
    }

    fn central(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.payload.is_none() {
                self.welcome(ui);
                return;
            }
            let dirty = self.dirty();
            let title = self.selected.map(|s| self.saves[s].label()).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.heading(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save_btn = egui::Button::new(egui::RichText::new(if dirty { "  💾 Save changes  " } else { "  Saved  " }).strong());
                    if ui.add_enabled(dirty, save_btn).clicked() {
                        self.save();
                    }
                    if ui.add_enabled(dirty, egui::Button::new("Revert")).on_hover_text("Discard unsaved edits").clicked() {
                        self.revert();
                    }
                    if ui.button("↺ Undo last save").on_hover_text("Restore this checkpoint from its most recent backup").clicked() {
                        self.undo_last_save();
                    }
                });
            });
            ui.add_space(4.0);

            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                self.deviant_card(ui);
                ui.add_space(8.0);
                self.gauges_section(ui);
                ui.add_space(8.0);
                self.advanced_section(ui);
                ui.add_space(12.0);
            });
        });
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(70.0);
            ui.heading("Pick a checkpoint on the left");
            ui.add_space(10.0);
            ui.label("Choose the save you're about to load, adjust relationships or Connor's stability, then Save.");
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Tip: use “💾 Full backup” once before you start, so you can always roll everything back.").weak());
        });
    }

    /// Highlighted quick action for the headline use-case.
    fn deviant_card(&mut self, ui: &mut egui::Ui) {
        let has_var = ADVANCED_VARS
            .iter()
            .position(|a| a.var == DEVIANCE_VAR)
            .map(|i| self.adv[i].is_some())
            .unwrap_or(false);
        egui::Frame::group(ui.style())
            .fill(ui.visuals().faint_bg_color)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Connor's deviant path").strong().size(15.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(has_var, egui::Button::new("Unlock deviant option"))
                            .on_hover_text("Sets software instability to 100 so the deviant dialogue choice appears")
                            .clicked()
                        {
                            self.queue_unlock_deviant();
                        }
                    });
                });
                if has_var {
                    ui.label(egui::RichText::new("Raises Connor's software instability so scenes offer the “become deviant” choice (e.g. confronting North). Click, then Save.").weak().size(11.0));
                } else {
                    ui.label(egui::RichText::new("This checkpoint doesn't track Connor's instability.").weak().size(11.0));
                }
            });
    }

    fn gauges_section(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Relationships & stability").strong().size(15.0));
        ui.label(egui::RichText::new("These set the on-screen gauge. Click a level per character.").weak().size(11.0));
        ui.add_space(4.0);
        for (gi, g) in GAUGES.iter().enumerate() {
            let present = self.original[gi].is_some();
            if !present {
                continue; // hide characters not in this save to reduce clutter
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(g.display).strong());
                if Some(self.chosen[gi]) != Some(self.original[gi]) {
                    ui.label(egui::RichText::new("· changed").color(egui::Color32::from_rgb(230, 190, 90)));
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (li, level) in g.levels.iter().enumerate() {
                    let is_sel = self.chosen[gi] == Some(li);
                    if ui.selectable_label(is_sel, *level).on_hover_text(g.note).clicked() {
                        self.chosen[gi] = Some(li);
                    }
                }
            });
            ui.separator();
        }
    }

    fn advanced_section(&mut self, ui: &mut egui::Ui) {
        let any = self.adv_orig.iter().any(|v| v.is_some());
        egui::CollapsingHeader::new(egui::RichText::new("Advanced — Connor deviancy variables").strong())
            .default_open(false)
            .show(ui, |ui| {
                if !any {
                    ui.label(egui::RichText::new("None of these variables exist in this save.").weak());
                    return;
                }
                ui.label(egui::RichText::new("The “Software instability” value is the one scenes actually check. The rest are display/state copies.").weak().size(11.0));
                ui.add_space(4.0);
                for (i, a) in ADVANCED_VARS.iter().enumerate() {
                    let Some(orig) = self.adv_orig[i] else { continue };
                    let changed = self.adv[i] != Some(orig);
                    ui.horizontal(|ui| {
                        match self.adv[i].get_or_insert(orig) {
                            Scalar::Bool(b) => {
                                ui.checkbox(b, a.label);
                            }
                            Scalar::Int(n) => {
                                ui.add(egui::DragValue::new(n).speed(1));
                                ui.label(a.label);
                            }
                            Scalar::Float(f) => {
                                ui.add(egui::DragValue::new(f).speed(1.0));
                                if ui.small_button("100").clicked() {
                                    *f = 100.0;
                                }
                                if ui.small_button("-100").clicked() {
                                    *f = -100.0;
                                }
                                ui.label(a.label);
                            }
                        }
                        if changed {
                            ui.label(egui::RichText::new("· changed").color(egui::Color32::from_rgb(230, 190, 90)));
                        }
                    });
                    ui.label(egui::RichText::new(a.help).weak().size(11.0));
                    ui.add_space(4.0);
                }
            });
    }

    fn restore_window(&mut self, ctx: &egui::Context) {
        if !self.show_restore {
            return;
        }
        let mut open = true;
        let mut pick: Option<PathBuf> = None;
        egui::Window::new("Restore a full backup")
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Rolls your entire save folder back to a chosen full backup. Your current state is saved first.");
                ui.separator();
                if self.full_backups.is_empty() {
                    ui.label(egui::RichText::new("No full backups yet. Use “💾 Full backup” to create one.").weak());
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for b in &self.full_backups {
                        ui.horizontal(|ui| {
                            if ui.button("Restore").clicked() {
                                pick = Some(b.path.clone());
                            }
                            if ui.small_button("📁").on_hover_text("Reveal in Explorer").clicked() {
                                saves::reveal(&b.path);
                            }
                            ui.label(b.label());
                        });
                    }
                });
            });
        if let Some(p) = pick {
            self.pending_restore = Some(p);
        }
        if !open {
            self.show_restore = false;
        }
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(target) = self.pending_restore.clone() else { return };
        let mut decided: Option<bool> = None;
        egui::Window::new("Confirm restore")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Overwrite your current saves with this backup?");
                ui.label(egui::RichText::new("A safety backup of the current state is taken automatically.").weak().size(11.0));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Yes, restore").clicked() {
                        decided = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decided = Some(false);
                    }
                });
            });
        match decided {
            Some(true) => {
                self.pending_restore = None;
                self.perform_restore(target);
            }
            Some(false) => self.pending_restore = None,
            None => {}
        }
    }
}
