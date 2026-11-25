//! The app itself: folder picker, rules editor, dry-run preview, undo, and
//! the optional scheduled run. The footer with the auto-update toggle and
//! version is required on every factory app.

use crate::rules::{self, Action, Dest, Plan, Rule};
use crate::tidy::{self, RunLog};
use eframe::egui;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 560.0;

pub fn run(_args: Vec<String>) -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_title("Downloads Tidy")
        .with_inner_size([WIDTH, HEIGHT])
        .with_min_inner_size([520.0, 380.0]);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Downloads Tidy",
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Preview,
    Rules,
    Undo,
    Schedule,
}

enum Status {
    Info(String),
    Error(String),
}

pub struct App {
    settings: crate::settings::Settings,
    version: String,
    /// "What would make this better?" (src/feedback.rs).
    feedback: crate::feedback::Feedback,
    tab: Tab,
    /// Folder text box contents (committed to settings on change).
    folder_text: String,
    plan: Option<Result<Plan, String>>,
    status: Option<Status>,
    log: Option<RunLog>,
    next_auto: Option<Instant>,
}

impl App {
    fn new() -> Self {
        let mut settings = crate::settings::load();
        if settings.folder.trim().is_empty() {
            settings.folder = crate::settings::default_folder();
        }
        let mut app = Self {
            folder_text: settings.folder.clone(),
            settings,
            version: crate::update::current_version(),
            feedback: Default::default(),
            tab: Tab::Preview,
            plan: None,
            status: None,
            log: tidy::load_log(),
            next_auto: None,
        };
        app.refresh();
        app
    }

    fn folder(&self) -> PathBuf {
        PathBuf::from(self.folder_text.trim())
    }

    fn save(&self) {
        crate::settings::save(&self.settings);
    }

    fn compute_plan(&self) -> Result<Plan, String> {
        let folder = self.folder();
        if self.folder_text.trim().is_empty() {
            return Err("Choose a folder to tidy.".into());
        }
        let files = tidy::scan(&folder)?;
        Ok(rules::plan(
            &folder,
            &files,
            &self.settings.rules,
            SystemTime::now(),
            self.settings.quiet_minutes,
            &|p| p.exists(),
        ))
    }

    fn refresh(&mut self) {
        self.plan = Some(self.compute_plan());
    }

    /// Carry out the previewed plan and record it in the undo log.
    fn tidy_now(&mut self, scheduled: bool) {
        let plan = match self.compute_plan() {
            Ok(p) => p,
            Err(e) => {
                self.status = Some(Status::Error(e));
                return;
            }
        };
        if plan.moves.is_empty() {
            if !scheduled {
                self.status = Some(Status::Info("Nothing to do.".into()));
            }
            self.refresh();
            return;
        }
        let outcome = tidy::execute(&plan.moves);
        let done = outcome.entries.len();
        if done > 0 {
            let log = RunLog {
                time: tidy::now_secs(),
                entries: outcome.entries,
            };
            tidy::save_log(&log);
            self.log = Some(log);
        }
        let prefix = if scheduled { "Scheduled run: " } else { "" };
        self.status = Some(if outcome.errors.is_empty() {
            Status::Info(format!("{prefix}Tidied {done} file(s). You can undo this on the Undo tab."))
        } else {
            Status::Error(format!(
                "{prefix}Tidied {done} file(s); {} failed. {}",
                outcome.errors.len(),
                outcome.errors.first().cloned().unwrap_or_default()
            ))
        });
        self.refresh();
    }

    fn undo_all(&mut self) {
        let Some(log) = self.log.as_mut() else { return };
        let (mut ok, mut failed, mut bin) = (0, 0, 0);
        for e in log.entries.iter_mut().filter(|e| !e.restored) {
            if e.moved_to.is_none() {
                bin += 1;
                continue;
            }
            match tidy::restore(e) {
                Ok(_) => {
                    e.restored = true;
                    ok += 1;
                }
                Err(_) => failed += 1,
            }
        }
        tidy::save_log(log);
        let mut msg = format!("Restored {ok} file(s).");
        if failed > 0 {
            msg.push_str(&format!(" {failed} could not be restored."));
        }
        if bin > 0 {
            msg.push_str(&format!(" {bin} in the Recycle Bin must be restored there."));
        }
        self.status = Some(if failed > 0 { Status::Error(msg) } else { Status::Info(msg) });
        self.refresh();
    }

    fn restore_one(&mut self, index: usize) {
        let Some(log) = self.log.as_mut() else { return };
        match tidy::restore(&log.entries[index]) {
            Ok(path) => {
                log.entries[index].restored = true;
                self.status = Some(Status::Info(format!("Restored to {}", path.display())));
            }
            Err(e) => self.status = Some(Status::Error(e)),
        }
        tidy::save_log(log);
        self.refresh();
    }

    fn tick_schedule(&mut self, ctx: &egui::Context) {
        if !self.settings.schedule_enabled {
            self.next_auto = None;
            return;
        }
        let interval = Duration::from_secs(u64::from(self.settings.schedule_minutes.max(1)) * 60);
        let now = Instant::now();
        let due = *self.next_auto.get_or_insert(now + interval);
        if now >= due {
            self.next_auto = Some(now + interval);
            self.tidy_now(true);
        }
        ctx.request_repaint_after(Duration::from_secs(15));
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (f5, run, undo) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::F5),
                i.modifiers.command && i.key_pressed(egui::Key::Enter),
                i.modifiers.command && i.key_pressed(egui::Key::Z),
            )
        });
        if f5 {
            self.refresh();
        }
        if run {
            self.tidy_now(false);
        }
        if undo && !ctx.egui_wants_keyboard_input() {
            self.undo_all();
        }
    }

    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Folder:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.folder_text)
                    .desired_width(ui.available_width() - 190.0)
                    .hint_text("Path of the folder to tidy"),
            );
            if resp.changed() {
                self.settings.folder = self.folder_text.trim().to_string();
                self.save();
                self.refresh();
            }
            if ui
                .button("Use Downloads")
                .on_hover_text("Reset to the default Downloads folder")
                .clicked()
            {
                self.folder_text = crate::settings::default_folder();
                self.settings.folder = self.folder_text.clone();
                self.save();
                self.refresh();
            }
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Preview, "Preview");
            ui.selectable_value(&mut self.tab, Tab::Rules, "Rules");
            ui.selectable_value(&mut self.tab, Tab::Undo, "Undo");
            ui.selectable_value(&mut self.tab, Tab::Schedule, "Schedule");
        });
        ui.separator();

        if let Some(status) = &self.status {
            let (text, color) = match status {
                Status::Info(t) => (t, ui.visuals().text_color()),
                Status::Error(t) => (t, ui.visuals().error_fg_color),
            };
            ui.colored_label(color, text);
            ui.add_space(4.0);
        }

        match self.tab {
            Tab::Preview => self.preview_tab(ui),
            Tab::Rules => self.rules_tab(ui),
            Tab::Undo => self.undo_tab(ui),
            Tab::Schedule => self.schedule_tab(ui),
        }
    }

    fn preview_tab(&mut self, ui: &mut egui::Ui) {
        let folder = self.folder();
        let mut do_refresh = false;
        let mut do_run = false;

        match &self.plan {
            None => {}
            Some(Err(e)) => {
                ui.colored_label(ui.visuals().error_fg_color, e);
                do_refresh = ui.button("Try again").clicked();
            }
            Some(Ok(plan)) => {
                ui.horizontal(|ui| {
                    let summary = if plan.moves.is_empty() {
                        "Nothing to tidy right now.".to_string()
                    } else {
                        let mut s = format!("{} file(s) to move", plan.move_count());
                        if plan.recycle_count() > 0 {
                            s.push_str(&format!(", {} to the Recycle Bin", plan.recycle_count()));
                        }
                        format!("{s} ({})", tidy::format_size(plan.total_size()))
                    };
                    ui.strong(summary);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!plan.moves.is_empty(), egui::Button::new("Tidy now"))
                            .on_hover_text("Ctrl+Enter. Nothing changes until you click this.")
                            .clicked()
                        {
                            do_run = true;
                        }
                        do_refresh = ui.button("Refresh").on_hover_text("F5").clicked();
                    });
                });
                if plan.skipped_busy > 0 {
                    ui.weak(format!(
                        "{} file(s) skipped: modified in the last {} min, still downloading, or open in another program.",
                        plan.skipped_busy, self.settings.quiet_minutes
                    ));
                }
                if plan.unmatched > 0 {
                    ui.weak(format!("{} file(s) match no rule and stay where they are.", plan.unmatched));
                }
                ui.add_space(4.0);

                if plan.moves.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        if !self.settings.rules.iter().any(|r| r.enabled) {
                            ui.label("No rules are switched on. Open the Rules tab to enable some.");
                        } else if plan.skipped_busy + plan.unmatched == 0 {
                            ui.label("This folder is already tidy.");
                        } else {
                            ui.label("No file matches a rule yet. Check the Rules tab.");
                        }
                    });
                } else {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Grid::new("preview").striped(true).num_columns(4).spacing([14.0, 4.0]).show(ui, |ui| {
                            ui.strong("File");
                            ui.strong("Size");
                            ui.strong("Goes to");
                            ui.strong("Rule");
                            ui.end_row();
                            for m in &plan.moves {
                                ui.label(&m.name);
                                ui.label(tidy::format_size(m.size));
                                match &m.dest {
                                    Dest::Recycle => ui.colored_label(ui.visuals().warn_fg_color, "Recycle Bin"),
                                    Dest::Folder(p) => {
                                        let rel = p.strip_prefix(&folder).unwrap_or(p);
                                        ui.label(rel.display().to_string())
                                    }
                                };
                                ui.weak(&m.rule);
                                ui.end_row();
                            }
                        });
                    });
                }
            }
        }

        if do_run {
            self.tidy_now(false);
        } else if do_refresh {
            self.status = None;
            self.refresh();
        }
    }

    fn rules_tab(&mut self, ui: &mut egui::Ui) {
        let mut dirty = false;
        let mut remove = None;
        let mut swap: Option<(usize, usize)> = None;
        let count = self.settings.rules.len();

        ui.horizontal(|ui| {
            ui.weak("The first matching rule wins, so order matters.");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Reset to defaults").clicked() {
                    self.settings.rules = rules::default_rules();
                    dirty = true;
                }
                if ui.button("Add rule").clicked() {
                    self.settings.rules.push(Rule::new_move("New rule", "", "Sorted"));
                    dirty = true;
                }
            });
        });
        ui.add_space(4.0);

        if count == 0 {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.label("No rules yet. Add one to start sorting."));
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (i, rule) in self.settings.rules.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        dirty |= ui.checkbox(&mut rule.enabled, "").on_hover_text("Rule on/off").changed();
                        dirty |= ui
                            .add(egui::TextEdit::singleline(&mut rule.name).desired_width(200.0).hint_text("Rule name"))
                            .changed();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Remove").clicked() {
                                remove = Some(i);
                            }
                            if ui.add_enabled(i + 1 < count, egui::Button::new("Down").small()).clicked() {
                                swap = Some((i, i + 1));
                            }
                            if ui.add_enabled(i > 0, egui::Button::new("Up").small()).clicked() {
                                swap = Some((i, i - 1));
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("File types:");
                        dirty |= ui
                            .add(
                                egui::TextEdit::singleline(&mut rule.extensions)
                                    .desired_width(ui.available_width() - 130.0)
                                    .hint_text("jpg, png, pdf  (blank = any type)"),
                            )
                            .changed();
                        egui::ComboBox::from_id_salt(("group", i)).selected_text("Add group").show_ui(ui, |ui| {
                            for (name, exts) in rules::GROUPS {
                                if ui.selectable_label(false, *name).clicked() {
                                    rule.extensions = rules::add_group(&rule.extensions, exts);
                                    dirty = true;
                                }
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("Older than");
                        dirty |= ui
                            .add(egui::DragValue::new(&mut rule.min_age_days).range(0..=3650).suffix(" days"))
                            .changed();
                        ui.label("   Larger than");
                        dirty |= ui
                            .add(egui::DragValue::new(&mut rule.min_size_mb).range(0..=1_000_000).suffix(" MB"))
                            .changed();
                        ui.weak("(0 = any)");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Then");
                        let mut recycle = rule.action == Action::Recycle;
                        egui::ComboBox::from_id_salt(("action", i))
                            .selected_text(if recycle { "Send to Recycle Bin" } else { "Move to folder" })
                            .show_ui(ui, |ui| {
                                dirty |= ui.selectable_value(&mut recycle, false, "Move to folder").changed();
                                dirty |= ui.selectable_value(&mut recycle, true, "Send to Recycle Bin").changed();
                            });
                        match (&rule.action, recycle) {
                            (Action::MoveTo(_), true) => rule.action = Action::Recycle,
                            (Action::Recycle, false) => rule.action = Action::MoveTo(String::new()),
                            _ => {}
                        }
                        if let Action::MoveTo(folder) = &mut rule.action {
                            dirty |= ui
                                .add(egui::TextEdit::singleline(folder).desired_width(180.0).hint_text("Subfolder name"))
                                .changed();
                            if rules::sanitize_subfolder(folder).is_none() {
                                ui.colored_label(ui.visuals().error_fg_color, "Needs a plain subfolder name");
                            }
                        }
                    });
                });
            }
        });

        if let Some(i) = remove {
            self.settings.rules.remove(i);
            dirty = true;
        }
        if let Some((a, b)) = swap {
            self.settings.rules.swap(a, b);
            dirty = true;
        }
        if dirty {
            self.save();
            self.refresh();
        }
    }

    fn undo_tab(&mut self, ui: &mut egui::Ui) {
        let Some(log) = &self.log else {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.label("No tidy run yet. When you run one, you can undo it here."));
            return;
        };
        let pending = log.entries.iter().filter(|e| !e.restored && e.moved_to.is_some()).count();
        let in_bin = log.entries.iter().filter(|e| !e.restored && e.moved_to.is_none()).count();
        let ago = tidy::now_secs().saturating_sub(log.time);
        let mut undo_all = false;
        let mut restore = None;
        let mut open_bin = false;

        ui.horizontal(|ui| {
            ui.strong(format!("Last run: {} ({} file(s))", tidy::format_ago(ago), log.entries.len()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(pending > 0, egui::Button::new("Undo whole run"))
                    .on_hover_text("Ctrl+Z")
                    .clicked()
                {
                    undo_all = true;
                }
                if in_bin > 0 && ui.button("Open Recycle Bin").clicked() {
                    open_bin = true;
                }
            });
        });
        if pending == 0 && in_bin == 0 {
            ui.weak("Everything from this run has been put back.");
        }
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("undo").striped(true).num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
                for (i, e) in log.entries.iter().enumerate() {
                    let name = e.original.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(name);
                    match (&e.moved_to, e.restored) {
                        (_, true) => {
                            ui.weak("Restored");
                        }
                        (None, false) => {
                            ui.colored_label(ui.visuals().warn_fg_color, "In Recycle Bin (restore from there)");
                        }
                        (Some(p), false) => {
                            let dir = p.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned());
                            ui.weak(format!("Now in {}", dir.unwrap_or_default()));
                        }
                    }
                    if !e.restored && e.moved_to.is_some() && ui.small_button("Restore").clicked() {
                        restore = Some(i);
                    }
                    ui.end_row();
                }
            });
        });

        if open_bin {
            let _ = std::process::Command::new("explorer.exe").arg("shell:RecycleBinFolder").spawn();
        }
        if undo_all {
            self.undo_all();
        } else if let Some(i) = restore {
            self.restore_one(i);
        }
    }

    fn schedule_tab(&mut self, ui: &mut egui::Ui) {
        let mut dirty = false;
        ui.add_space(4.0);
        if ui
            .checkbox(&mut self.settings.schedule_enabled, "Tidy automatically while Downloads Tidy is open")
            .changed()
        {
            self.next_auto = None;
            dirty = true;
        }
        ui.add_enabled_ui(self.settings.schedule_enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Run every");
                if ui
                    .add(egui::DragValue::new(&mut self.settings.schedule_minutes).range(5..=10_080).suffix(" minutes"))
                    .changed()
                {
                    self.next_auto = None;
                    dirty = true;
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label("Leave files alone if modified in the last");
            dirty |= ui
                .add(egui::DragValue::new(&mut self.settings.quiet_minutes).range(0..=1440).suffix(" minutes"))
                .changed();
        });
        ui.add_space(8.0);
        ui.weak("Applies to previews and manual runs too. Files still being downloaded or open in another program are always skipped.");
        ui.weak("Scheduled runs happen only while this window is open (it can be minimised). The first one runs one interval after you switch it on.");
        ui.weak("Each run is logged on the Undo tab; a run that moves nothing leaves the previous log alone.");
        if dirty {
            self.save();
            self.refresh();
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut auto_update = self.settings.auto_update;
            if ui
                .checkbox(&mut auto_update, "Automatically update")
                .on_hover_text("Checks for new versions about once a day and installs them in the background.")
                .changed()
            {
                self.settings.auto_update = auto_update;
                crate::settings::save(&self.settings);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("v{}", self.version));
                ui.add_space(8.0);
                self.feedback.link(ui);
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui_root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui_root.ctx().clone();
        self.handle_shortcuts(&ctx);
        self.tick_schedule(&ctx);
        egui::Panel::bottom("footer").show(ui_root, |ui| self.footer(ui));
        egui::CentralPanel::default().show(ui_root, |ui| self.main_ui(ui));
            let feedback_ctx = ui_root.ctx().clone();
        self.feedback.show(&feedback_ctx, &self.version, false);
    }
}
