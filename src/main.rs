#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod models;
use models::*;

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveTime, Timelike};
use eframe::egui::{self, Color32, CursorIcon, FontData, FontDefinitions, FontFamily, Pos2, Rect, RichText, Rounding, Sense, Stroke, Vec2, ViewportCommand};
use notify_rust::Notification;
use single_instance::SingleInstance;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::time::Instant;

#[derive(Clone)]
struct EditDialogState {
    is_plan: bool,
    is_new_plan: bool,
    is_periodic_instance: bool,
    target_uuid: String,
    title: String,
    priority: bool,
    has_due: bool,
    due_year: i32,
    due_month: u32,
    due_day: u32,
    has_time: bool,
    time_hour: u32,
    time_min: u32,
    repeat_rule: String,
    weekly_checked: [bool; 7],
    monthly_str: String,
}

#[derive(PartialEq, Clone, Copy)]
enum ViewTab { All, Quadrants, Plans, Forest }

struct EggDoneApp {
    config: Config,
    todos: Vec<TodoTask>,
    plans: Vec<Plan>,
    history: Vec<HistoryRecord>,
    is_expanded: bool,
    auto_collapse_enabled: bool,
    current_view: ViewTab,
    input_text: String,
    parsed_preview: Vec<String>,
    autostart: bool,
    pending_confirm: HashMap<String, Instant>,
    last_hover_leave: Option<Instant>,
    last_reminder_check: Instant,
    last_sync_date: NaiveDate,
    edit_modal: Option<EditDialogState>,
    delete_confirm_modal: Option<(bool, String, String)>,
    giveup_confirm_modal: bool,
    show_history_modal: bool,
    show_forest_stats_modal: bool,
    show_focus_success_modal: bool,
    toast: Option<(String, Instant)>,
    focus_duration_secs: u32,
    rest_duration_secs: u32,
    remaining_secs: u32,
    is_focus_running: bool,
    is_resting: bool,
    focus_task: Option<TodoTask>,
    last_timer_tick: Instant,
    withered_anim_until: Option<Instant>,
}

impl EggDoneApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_custom_fonts(&cc.egui_ctx);
        let config: Config = safe_json_load("config.json");
        let mut todos: Vec<TodoTask> = safe_json_load("todos.json");
        let mut plans: Vec<Plan> = safe_json_load("plans.json");
        let history: Vec<HistoryRecord> = safe_json_load("history.json");

        let mut needs_save = false;
        let today_s = today_str();
        for task in &mut todos {
            if task.uuid.is_empty() { task.uuid = gen_uuid(); needs_save = true; }
            if task.cycle_date.is_empty() { task.cycle_date = task.due_date.clone().unwrap_or_else(|| today_s.clone()); needs_save = true; }
            if let Some(ref rule) = task.repeat_rule {
                if task.plan_uuid.is_none() {
                    let p_uuid = gen_uuid();
                    task.plan_uuid = Some(p_uuid.clone());
                    plans.push(Plan {
                        plan_uuid: p_uuid,
                        text: task.text.clone(),
                        priority: task.priority,
                        due_time: task.due_time.clone(),
                        reminder_time: task.reminder_time.clone(),
                        repeat_rule: rule.clone(),
                        repeat_days: task.repeat_days.clone().unwrap_or_default(),
                        base_date: task.due_date.clone().unwrap_or_else(|| today_s.clone()),
                    });
                    needs_save = true;
                }
            }
        }

        let focus_mins = config.focus_minutes.clamp(1, 120);
        let mut app = Self {
            config, todos, plans, history,
            is_expanded: true,
            auto_collapse_enabled: true,
            current_view: ViewTab::All,
            input_text: String::new(),
            parsed_preview: Vec::new(),
            autostart: is_autostart_enabled(),
            pending_confirm: HashMap::new(),
            last_hover_leave: None,
            last_reminder_check: Instant::now(),
            last_sync_date: Local::now().date_naive(),
            edit_modal: None,
            delete_confirm_modal: None,
            giveup_confirm_modal: false,
            show_history_modal: false,
            show_forest_stats_modal: false,
            show_focus_success_modal: false,
            toast: None,
            focus_duration_secs: focus_mins * 60,
            rest_duration_secs: 5 * 60,
            remaining_secs: focus_mins * 60,
            is_focus_running: false,
            is_resting: false,
            focus_task: None,
            last_timer_tick: Instant::now(),
            withered_anim_until: None,
        };
        if needs_save { app.save_all_data(); }
        app.sync_repeat_plans();
        app.sort_todos();
        app
    }

    fn save_all_data(&self) {
        safe_json_dump("config.json", &self.config);
        safe_json_dump("todos.json", &self.todos);
        safe_json_dump("plans.json", &self.plans);
        safe_json_dump("history.json", &self.history);
    }

    fn sort_todos(&mut self) {
        self.todos.sort_by(|a, b| {
            let da = a.due_date.as_deref().unwrap_or("9999-12-31");
            let db = b.due_date.as_deref().unwrap_or("9999-12-31");
            let ta = a.due_time.as_deref().unwrap_or("23:59");
            let tb = b.due_time.as_deref().unwrap_or("23:59");
            (da, ta).cmp(&(db, tb))
        });
    }

    fn sync_repeat_plans(&mut self) -> bool {
        let today = Local::now().date_naive();
        self.last_sync_date = today;
        let eval_dates: Vec<NaiveDate> = (0..=7).rev().map(|i| today - Duration::days(i)).collect();
        let mut existing = HashSet::new();
        for t in &self.todos {
            if let Some(ref pu) = t.plan_uuid { existing.insert((pu.clone(), t.cycle_date.clone())); }
        }
        for h in self.history.iter().rev().take(500) {
            if let (Some(ref pu), Some(ref cd)) = (&h.plan_uuid, &h.cycle_date) { existing.insert((pu.clone(), cd.clone())); }
        }

        let mut new_tasks = Vec::new();
        for plan in &self.plans {
            let base = NaiveDate::parse_from_str(&plan.base_date, "%Y-%m-%d").unwrap_or(today);
            for &ed in &eval_dates {
                if ed < base { continue; }
                let hits = match plan.repeat_rule.as_str() {
                    "daily" => true,
                    "weekdays" => ed.weekday().num_days_from_monday() < 5,
                    "weekly" => {
                        let td = if plan.repeat_days.is_empty() { vec![base.weekday().num_days_from_monday()] } else { plan.repeat_days.clone() };
                        td.contains(&ed.weekday().num_days_from_monday())
                    }
                    "monthly" => {
                        let td = if plan.repeat_days.is_empty() { vec![base.day()] } else { plan.repeat_days.clone() };
                        let max_d = days_in_month(ed.year(), ed.month());
                        td.iter().any(|&d| d.min(max_d) == ed.day())
                    }
                    _ => false,
                };
                if hits {
                    let ed_s = ed.format("%Y-%m-%d").to_string();
                    let key = (plan.plan_uuid.clone(), ed_s.clone());
                    if !existing.contains(&key) {
                        existing.insert(key);
                        new_tasks.push(TodoTask {
                            uuid: gen_uuid(),
                            plan_uuid: Some(plan.plan_uuid.clone()),
                            cycle_date: ed_s.clone(),
                            text: plan.text.clone(),
                            priority: plan.priority,
                            due_date: Some(ed_s),
                            original_due: None,
                            due_time: plan.due_time.clone(),
                            reminder_time: plan.reminder_time.clone().or_else(|| plan.due_time.clone()),
                            repeat_rule: Some(plan.repeat_rule.clone()),
                            repeat_days: Some(plan.repeat_days.clone()),
                            created: now_iso(),
                            done: false,
                            hide_until: None,
                            notified: false,
                        });
                    }
                }
            }
        }
        if !new_tasks.is_empty() {
            self.todos.extend(new_tasks);
            self.sort_todos();
            self.save_all_data();
            return true;
        }
        false
    }

    fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now() + std::time::Duration::from_millis(2200)));
    }

    fn is_any_modal_open(&self) -> bool {
        self.edit_modal.is_some() || self.delete_confirm_modal.is_some() || self.giveup_confirm_modal
            || self.show_history_modal || self.show_forest_stats_modal || self.show_focus_success_modal
    }

    fn expand_window(&mut self, ctx: &egui::Context) {
        if !self.is_expanded {
            self.is_expanded = true;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(360.0, 640.0)));
            if let Some(ms) = ctx.input(|i| i.viewport().monitor_size) {
                if ms.x > 400.0 {
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(Pos2::new((ms.x - 380.0).max(20.0), 60.0)));
                }
            }
        }
    }

    fn collapse_window(&mut self, ctx: &egui::Context) {
        if self.is_expanded {
            self.is_expanded = false;
            self.input_text.clear();
            self.parsed_preview.clear();
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(64.0, 64.0)));
            if let Some(ms) = ctx.input(|i| i.viewport().monitor_size) {
                if ms.x > 100.0 {
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(Pos2::new((ms.x - 84.0).max(20.0), 60.0)));
                }
            }
        }
    }

    fn commit_new_task(&mut self) {
        let raw = self.input_text.trim().to_string();
        if raw.is_empty() { return; }
        if let Some(parsed) = QuickAddParser::parse(&raw) {
            let mut plan_uuid = None;
            if let Some(ref rule) = parsed.repeat_rule {
                let pu = gen_uuid();
                plan_uuid = Some(pu.clone());
                self.plans.push(Plan {
                    plan_uuid: pu,
                    text: parsed.title.clone(),
                    priority: parsed.priority,
                    due_time: parsed.due_time.clone(),
                    reminder_time: parsed.due_time.clone(),
                    repeat_rule: rule.clone(),
                    repeat_days: parsed.repeat_days.clone(),
                    base_date: parsed.base_date.clone().unwrap_or_else(today_str),
                });
            }
            let today = Local::now().date_naive();
            let today_s = today.format("%Y-%m-%d").to_string();
            let mut hide_until = None;
            if plan_uuid.is_some() {
                if let Some(Ok(d_obj)) = parsed.due_date.as_deref().map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d")) {
                    if d_obj > today {
                        let h_obj = d_obj - Duration::days(1);
                        hide_until = Some(if h_obj > today { h_obj.format("%Y-%m-%d").to_string() } else { today_s.clone() });
                    }
                }
            }
            self.todos.insert(0, TodoTask {
                uuid: gen_uuid(),
                plan_uuid,
                cycle_date: parsed.due_date.clone().unwrap_or_else(|| today_s.clone()),
                text: parsed.title,
                priority: parsed.priority,
                due_date: parsed.due_date,
                original_due: None,
                due_time: parsed.due_time.clone(),
                reminder_time: parsed.due_time,
                repeat_rule: parsed.repeat_rule,
                repeat_days: if parsed.repeat_days.is_empty() { None } else { Some(parsed.repeat_days) },
                created: now_iso(),
                done: false,
                hide_until,
                notified: false,
            });
            self.input_text.clear();
            self.parsed_preview.clear();
            self.sync_repeat_plans();
            self.sort_todos();
            self.save_all_data();
        }
    }

    fn complete_task(&mut self, task_uuid: &str) {
        if let Some(pos) = self.todos.iter().position(|t| t.uuid == task_uuid) {
            let task = self.todos.remove(pos);
            self.pending_confirm.remove(task_uuid);
            self.history.push(HistoryRecord {
                plan_uuid: task.plan_uuid,
                cycle_date: Some(task.cycle_date),
                text: task.text,
                due_date: task.due_date,
                created: Some(task.created),
                completed: now_iso(),
            });
            self.save_all_data();
        }
    }

    fn record_forest(&self, status: &str) {
        let mut records: Vec<ForestRecord> = safe_json_load("forest.json");
        let task_name = self.focus_task.as_ref().map(|t| t.text.clone()).unwrap_or_else(|| "自由专注".into());
        records.push(ForestRecord { timestamp: now_iso(), status: status.into(), task: task_name, minutes: self.focus_duration_secs / 60 });
        safe_json_dump("forest.json", &records);
    }

    fn reset_forest_session(&mut self) {
        self.is_focus_running = false;
        self.is_resting = false;
        self.remaining_secs = self.focus_duration_secs;
        self.withered_anim_until = None;
    }

    fn start_rest_session(&mut self) {
        self.is_resting = true;
        self.remaining_secs = self.rest_duration_secs;
        self.is_focus_running = true;
        self.last_timer_tick = Instant::now();
    }

    fn copy_handover(&mut self, ctx: &egui::Context) {
        let now_str = Local::now().format("%Y-%m-%d %H:%M").to_string();
        let tom_str = (Local::now().date_naive() + Duration::days(1)).format("%Y-%m-%d").to_string();
        let mut lines = vec!["【前台未完成待办交接】".to_string(), format!("交接时间：{}", now_str), "------------------------".to_string()];
        let list: Vec<&TodoTask> = self.todos.iter().filter(|t| t.due_date.as_deref().map(|d| d <= tom_str.as_str()).unwrap_or(true)).collect();
        if list.is_empty() {
            lines.push("当前无紧急未完成事项，辛苦了！🎉".to_string());
        } else {
            for (idx, task) in list.iter().enumerate() {
                let mut ti = Vec::new();
                if let Some(ref d) = task.due_date { if d.len() >= 5 { ti.push(d[5..].to_string()); } }
                if let Some(ref tm) = task.due_time { ti.push(tm.clone()); }
                let t_str = if ti.is_empty() { "不限期".to_string() } else { ti.join(" ") };
                if task.repeat_rule.is_some() {
                    let rn = get_rule_display_str(task.repeat_rule.as_deref(), task.repeat_days.as_ref());
                    lines.push(format!("{}. {} [⚠️{}任务：应于 {} 前完成，暂未处理]", idx + 1, task.text, rn, t_str));
                } else {
                    lines.push(format!("{}. {} （截止：{}）", idx + 1, task.text, t_str));
                }
            }
        }
        ctx.copy_text(lines.join("\n"));
        self.show_toast("✅ 交接事项已成功复制！");
    }

    fn open_edit_dialog_for_task(&mut self, task: &TodoTask) {
        let today = Local::now().date_naive();
        let (has_due, y, m, d) = task.due_date.as_deref()
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .map(|nd| (true, nd.year(), nd.month(), nd.day()))
            .unwrap_or((false, today.year(), today.month(), today.day()));
        let (has_time, hh, mm) = task.due_time.as_deref()
            .and_then(|s| NaiveTime::parse_from_str(s, "%H:%M").ok())
            .map(|nt| (true, nt.hour(), nt.minute()))
            .unwrap_or((false, 18, 0));
        let rule = task.repeat_rule.clone().unwrap_or_else(|| "none".into());
        let mut weekly_checked = [false; 7];
        let mut monthly_str = String::new();
        if let Some(ref days) = task.repeat_days {
            if rule == "weekly" { for &di in days { if di < 7 { weekly_checked[di as usize] = true; } } }
            else if rule == "monthly" { monthly_str = days.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", "); }
        }
        if !weekly_checked.iter().any(|&x| x) { weekly_checked[0] = true; }
        self.edit_modal = Some(EditDialogState {
            is_plan: false, is_new_plan: false, is_periodic_instance: task.plan_uuid.is_some(),
            target_uuid: task.uuid.clone(), title: task.text.clone(), priority: task.priority == 1,
            has_due, due_year: y, due_month: m, due_day: d, has_time, time_hour: hh, time_min: mm,
            repeat_rule: rule, weekly_checked, monthly_str,
        });
    }

    fn open_edit_dialog_for_plan(&mut self, plan: Option<&Plan>) {
        let today = Local::now().date_naive();
        if let Some(p) = plan {
            let (has_time, hh, mm) = p.due_time.as_deref()
                .and_then(|s| NaiveTime::parse_from_str(s, "%H:%M").ok())
                .map(|nt| (true, nt.hour(), nt.minute()))
                .unwrap_or((false, 18, 0));
            let mut weekly_checked = [false; 7];
            let mut monthly_str = String::new();
            if p.repeat_rule == "weekly" { for &d in &p.repeat_days { if d < 7 { weekly_checked[d as usize] = true; } } }
            else if p.repeat_rule == "monthly" { monthly_str = p.repeat_days.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", "); }
            if !weekly_checked.iter().any(|&x| x) { weekly_checked[0] = true; }
            self.edit_modal = Some(EditDialogState {
                is_plan: true, is_new_plan: false, is_periodic_instance: false,
                target_uuid: p.plan_uuid.clone(), title: p.text.clone(), priority: p.priority == 1,
                has_due: false, due_year: today.year(), due_month: today.month(), due_day: today.day(),
                has_time, time_hour: hh, time_min: mm, repeat_rule: p.repeat_rule.clone(), weekly_checked, monthly_str,
            });
        } else {
            let mut wc = [false; 7]; wc[0] = true;
            self.edit_modal = Some(EditDialogState {
                is_plan: true, is_new_plan: true, is_periodic_instance: false,
                target_uuid: gen_uuid(), title: String::new(), priority: false,
                has_due: false, due_year: today.year(), due_month: today.month(), due_day: today.day(),
                has_time: false, time_hour: 18, time_min: 0, repeat_rule: "daily".into(), weekly_checked: wc, monthly_str: "1, 15".into(),
            });
        }
    }

    fn background_tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.pending_confirm.retain(|_, &mut exp| now < exp);
        if self.toast.as_ref().map(|(_, exp)| now >= *exp).unwrap_or(false) { self.toast = None; }
        if self.withered_anim_until.map(|u| now >= u).unwrap_or(false) { self.reset_forest_session(); }

        if self.is_focus_running && now.duration_since(self.last_timer_tick).as_secs() >= 1 {
            self.last_timer_tick = now;
            self.remaining_secs = self.remaining_secs.saturating_sub(1);
            if self.remaining_secs == 0 {
                self.is_focus_running = false;
                if !self.is_resting {
                    self.record_forest("healthy");
                    if let Some(ft) = self.focus_task.clone() {
                        self.complete_task(&ft.uuid);
                        let _ = Notification::new().summary("🌲 待办达成！").body(&format!("专注结束，事项已标记完成：\n{}", ft.text)).show();
                    }
                    self.show_focus_success_modal = true;
                } else {
                    self.reset_forest_session();
                }
            }
        }

        if now.duration_since(self.last_reminder_check).as_secs() >= 10 {
            self.last_reminder_check = now;
            let cur_date = Local::now().date_naive();
            if cur_date != self.last_sync_date { self.sync_repeat_plans(); }
            let (now_d, now_t) = (cur_date.format("%Y-%m-%d").to_string(), Local::now().format("%H:%M").to_string());
            let mut changed = false;
            for task in &mut self.todos {
                if !task.notified {
                    let rt = task.reminder_time.as_ref().or(task.due_time.as_ref());
                    if let (Some(d), Some(t)) = (&task.due_date, rt) {
                        if d == &now_d && t == &now_t {
                            task.notified = true;
                            changed = true;
                            let _ = Notification::new().summary("⏰ 待办提醒").body(&task.text).show();
                        }
                    }
                }
            }
            if changed { self.save_all_data(); }
        }

        let pointer_in = ctx.input(|i| i.pointer.has_pointer());
        if self.auto_collapse_enabled {
            if self.is_expanded && !self.is_any_modal_open() {
                if pointer_in || ctx.memory(|m| m.focused().is_some()) {
                    self.last_hover_leave = None;
                } else if let Some(leave_t) = self.last_hover_leave {
                    if now.duration_since(leave_t).as_millis() > 650 {
                        self.collapse_window(ctx);
                        self.last_hover_leave = None;
                    }
                } else {
                    self.last_hover_leave = Some(now);
                }
            } else if !self.is_expanded && pointer_in {
                self.expand_window(ctx);
            }
        }

        if self.is_focus_running || !self.pending_confirm.is_empty() || self.toast.is_some() || self.withered_anim_until.is_some() || self.last_hover_leave.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs(10));
        }
    }

    fn current_forest_icon(&self) -> &'static str {
        if self.withered_anim_until.is_some() { return "🥀"; }
        if self.is_resting { return "☕"; }
        if !self.is_focus_running && self.remaining_secs == self.focus_duration_secs { return "🌰"; }
        let ratio = (self.focus_duration_secs.saturating_sub(self.remaining_secs) as f32) / (self.focus_duration_secs.max(1) as f32);
        if ratio < 0.25 { "🌰" } else if ratio < 0.60 { "🌱" } else if ratio < 0.90 { "🌿" } else { "🌲" }
    }

    fn render_collapsed_ball(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(60.0, 60.0));
            let resp = ui.allocate_rect(rect, Sense::click_and_drag());
            if resp.dragged() { ctx.send_viewport_cmd(ViewportCommand::StartDrag); }
            if resp.hovered() || resp.clicked() { self.expand_window(ctx); }

            let painter = ui.painter();
            painter.circle(rect.center(), 28.0, Color32::from_rgba_unmultiplied(30, 30, 30, 215), Stroke::new(2.0_f32, Color32::from_rgba_unmultiplied(255, 215, 0, 230)));

            let (icon, time_str) = if self.withered_anim_until.is_some() {
                ("🥀", "枯萎".to_string())
            } else if self.is_focus_running || self.is_resting || self.remaining_secs != self.focus_duration_secs {
                (self.current_forest_icon(), format!("{:02}:{:02}", self.remaining_secs / 60, self.remaining_secs % 60))
            } else {
                ("🥚", String::new())
            };

            if time_str.is_empty() {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(24.0), Color32::WHITE);
            } else {
                painter.text(rect.center() - Vec2::new(0.0, 9.0), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(17.0), Color32::WHITE);
                painter.text(rect.center() + Vec2::new(0.0, 10.0), egui::Align2::CENTER_CENTER, time_str, egui::FontId::proportional(12.5), Color32::from_rgb(255, 213, 79));
            }
        });
    }

    fn render_expanded_panel(&mut self, ctx: &egui::Context) {
        let panel_frame = egui::Frame::none()
            .fill(Color32::from_rgba_unmultiplied(250, 250, 252, 255))
            .rounding(Rounding::same(14.0))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(180, 180, 180, 220)))
            .inner_margin(egui::Margin::symmetric(12.0, 14.0));

        egui::CentralPanel::default().frame(panel_frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let tr = ui.add(egui::Label::new(RichText::new(&self.config.title_prefix).size(16.0).strong().color(Color32::from_rgb(34, 34, 34))).sense(Sense::click_and_drag()));
                if tr.dragged() { ctx.send_viewport_cmd(ViewportCommand::StartDrag); }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button(RichText::new("⚙️").size(14.0), |ui| {
                        ui.set_min_width(150.0);
                        ui.checkbox(&mut self.auto_collapse_enabled, "🧲 鼠标移出自动收缩");
                        if ui.checkbox(&mut self.autostart, "⚡ 开机自动启动").changed() {
                            if set_autostart(self.autostart) {
                                self.show_toast(if self.autostart { "✅ 已开启开机自启" } else { "🚫 已关闭开机自启" });
                            } else {
                                self.autostart = !self.autostart;
                                self.show_toast("❌ 权限不足或系统不支持");
                            }
                            ui.close_menu();
                        }
                        if ui.button("➖ 立即收缩为悬浮球").clicked() {
                            ui.close_menu();
                            self.collapse_window(ctx);
                        }
                        ui.separator();
                        if ui.button(RichText::new("✖ 彻底退出程序").color(Color32::from_rgb(211, 47, 47))).clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                    });
                    if ui.add(egui::Button::new(RichText::new("📜森林").size(12.5).strong().color(Color32::from_rgb(85, 85, 85))).frame(false)).on_hover_cursor(CursorIcon::PointingHand).clicked() { self.show_forest_stats_modal = true; }
                    if ui.add(egui::Button::new(RichText::new("📚历史").size(12.5).strong().color(Color32::from_rgb(85, 85, 85))).frame(false)).on_hover_cursor(CursorIcon::PointingHand).clicked() { self.show_history_modal = true; }
                    if ui.add(egui::Button::new(RichText::new("📋交接").size(12.5).strong().color(Color32::from_rgb(85, 85, 85))).frame(false)).on_hover_cursor(CursorIcon::PointingHand).clicked() { self.copy_handover(ctx); }
                    if ui.add(egui::Button::new(RichText::new("🔄刷新").size(12.5).strong().color(Color32::from_rgb(85, 85, 85))).frame(false)).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        self.sync_repeat_plans();
                        self.sort_todos();
                        self.show_toast("🔄 数据已同步刷新");
                    }
                });
            });

            ui.add_space(6.0);
            ui.columns(4, |cols| {
                for (i, (tab, label)) in [(ViewTab::All, "📋 当前"), (ViewTab::Quadrants, "🎯 象限"), (ViewTab::Plans, "🗓️ 周期"), (ViewTab::Forest, "🌲 森林")].iter().enumerate() {
                    let sel = self.current_view == *tab;
                    let bg = if sel { Color32::from_rgb(46, 125, 50) } else { Color32::from_rgb(224, 224, 224) };
                    let fg = if sel { Color32::WHITE } else { Color32::from_rgb(102, 102, 102) };
                    let btn = egui::Button::new(RichText::new(*label).size(12.5).strong().color(fg)).fill(bg).rounding(Rounding::same(6.0)).min_size(Vec2::new(cols[i].available_width(), 28.0));
                    if cols[i].add(btn).on_hover_cursor(CursorIcon::PointingHand).clicked() { self.current_view = *tab; }
                }
            });

            ui.add_space(6.0);
            let content_h = (ui.available_height() - 56.0).max(100.0);
            ui.allocate_ui(Vec2::new(ui.available_width(), content_h), |ui| {
                match self.current_view {
                    ViewTab::All => self.render_view_all(ui),
                    ViewTab::Quadrants => self.render_view_quadrants(ui),
                    ViewTab::Plans => self.render_view_plans(ui),
                    ViewTab::Forest => self.render_view_forest(ui),
                }
            });

            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                let er = ui.add_sized(Vec2::new(ui.available_width(), 30.0), egui::TextEdit::singleline(&mut self.input_text).hint_text("💡 快速新增 (如: ! 明天下午3点开会 / 每月1号交租)"));
                if er.changed() {
                    self.parsed_preview = QuickAddParser::parse(&self.input_text).map(|r| r.labels).unwrap_or_default();
                }
                if er.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.commit_new_task();
                    er.request_focus();
                }
                if !self.parsed_preview.is_empty() {
                    ui.label(RichText::new(format!("智能识别: {}", self.parsed_preview.join(" | "))).size(11.0).color(Color32::from_rgb(46, 125, 50)));
                }
            });
        });

        self.render_modals(ctx);
        self.render_toast(ctx);
    }

    fn render_view_all(&mut self, ui: &mut egui::Ui) {
        let (today_s, now_t) = (today_str(), Local::now().format("%H:%M").to_string());
        let visible: Vec<TodoTask> = self.todos.iter().filter(|t| t.hide_until.as_deref().map(|h| today_s.as_str() >= h).unwrap_or(true)).cloned().collect();
        if visible.is_empty() {
            ui.centered_and_justified(|ui| { ui.label(RichText::new("🎉 暂无待办事项，享受轻松一刻吧！").color(Color32::GRAY)); });
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for task in visible {
                let is_pending = self.pending_confirm.contains_key(&task.uuid);
                let rule_disp = get_rule_display_str(task.repeat_rule.as_deref(), task.repeat_days.as_ref());
                let is_overdue = task.due_date.as_ref().map(|d| d.as_str() < today_s.as_str() || (d == &today_s && task.due_time.as_deref().map(|tm| tm < now_t.as_str()).unwrap_or(false))).unwrap_or(false);
                let card_bg = if is_pending { Color32::from_rgb(227, 242, 253) } else { Color32::WHITE };

                let resp = egui::Frame::none().fill(card_bg).rounding(Rounding::same(6.0)).stroke(Stroke::new(1.0_f32, Color32::from_rgb(238, 238, 238))).inner_margin(egui::Margin::symmetric(8.0, 6.0)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if is_pending { ui.label(RichText::new("✔️").size(15.0).color(Color32::from_rgb(21, 101, 192)).strong()); }
                        else { ui.label(RichText::new("◯").size(15.0).color(Color32::GRAY)); }
                        ui.vertical(|ui| {
                            ui.set_max_width(185.0);
                            if is_pending { ui.label(RichText::new(format!("【再次点击确认完成】\n{}", task.text)).size(12.5).strong().color(Color32::from_rgb(21, 101, 192))); }
                            else { ui.label(RichText::new(&task.text).size(13.0).color(Color32::from_rgb(51, 51, 51))); }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    if let Some(ref d) = task.due_date {
                                        let sd = if d.len() >= 5 { &d[5..] } else { d.as_str() };
                                        let b_txt = match task.original_due.as_ref().filter(|&o| o != d && o.len() >= 5) {
                                            Some(orig) => format!("{}📅{}(延)", if is_overdue { "⚠️逾期 " } else { "" }, &orig[5..]),
                                            None => format!("{}📅{}", if is_overdue { "⚠️逾期 " } else { "" }, sd),
                                        };
                                        let (bg, fg) = if is_overdue { (Color32::from_rgb(255, 235, 238), Color32::from_rgb(211, 47, 47)) } else { (Color32::from_rgb(227, 242, 253), Color32::from_rgb(21, 101, 192)) };
                                        draw_badge(ui, &b_txt, bg, fg);
                                    }
                                    if task.priority == 1 { draw_badge(ui, "⭐", Color32::from_rgb(255, 248, 225), Color32::from_rgb(245, 127, 23)); }
                                });
                                ui.horizontal(|ui| {
                                    if !rule_disp.is_empty() { draw_badge(ui, &format!("🔁{}", rule_disp), Color32::from_rgb(232, 245, 233), Color32::from_rgb(46, 125, 50)); }
                                    if let Some(ref tm) = task.due_time { draw_badge(ui, &format!("🕒{}", tm), Color32::from_rgb(232, 245, 233), Color32::from_rgb(46, 125, 50)); }
                                });
                            });
                        });
                    });
                }).response;

                let interact = ui.interact(resp.rect, ui.make_persistent_id(&task.uuid), Sense::click());
                if interact.clicked() {
                    if self.pending_confirm.contains_key(&task.uuid) { self.complete_task(&task.uuid); }
                    else { self.pending_confirm.insert(task.uuid.clone(), Instant::now() + std::time::Duration::from_millis(2500)); }
                }
                interact.context_menu(|ui| { self.render_task_context_menu(ui, &task); });
                ui.add_space(4.0);
            }
        });
    }

    fn render_view_quadrants(&mut self, ui: &mut egui::Ui) {
        let today_s = today_str();
        let mut q_tasks: [Vec<TodoTask>; 4] = Default::default();
        for task in &self.todos {
            if task.hide_until.as_deref().map(|h| today_s.as_str() < h).unwrap_or(false) { continue; }
            q_tasks[(compute_quadrant(task, &today_s) - 1) as usize].push(task.clone());
        }
        let half_h = (ui.available_height() - 6.0) / 2.0;
        let configs = [
            ("🔥 重要且紧急", Color32::from_rgb(255, 205, 210), Color32::from_rgb(255, 235, 238)),
            ("⭐ 重要不紧急", Color32::from_rgb(200, 230, 201), Color32::from_rgb(232, 245, 233)),
            ("⚡ 紧急不重要", Color32::from_rgb(255, 224, 130), Color32::from_rgb(255, 248, 225)),
            ("🍃 不重要不紧急", Color32::from_rgb(224, 224, 224), Color32::from_rgb(245, 245, 245)),
        ];
        for row in 0..2 {
            ui.columns(2, |cols| {
                for col in 0..2 {
                    let idx = row * 2 + col;
                    let (title, h_bg, b_bg) = configs[idx];
                    cols[col].allocate_ui(Vec2::new(cols[col].available_width(), half_h), |ui| {
                        egui::Frame::none().fill(h_bg).rounding(Rounding::same(4.0)).inner_margin(egui::Margin::symmetric(6.0, 4.0)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(title).size(11.5).strong().color(Color32::from_rgb(40, 40, 40)));
                        });
                        ui.add_space(2.0);
                        egui::Frame::none().fill(b_bg).rounding(Rounding::same(6.0)).inner_margin(egui::Margin::same(4.0)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.set_height(ui.available_height());
                            egui::ScrollArea::vertical().id_source(format!("q_sc_{}", idx)).show(ui, |ui| {
                                for task in &q_tasks[idx] {
                                    let is_p = self.pending_confirm.contains_key(&task.uuid);
                                    let lbl = if is_p { format!("✔️ 待确认: {}...", task.text.chars().take(6).collect::<String>()) }
                                        else if let Some(ref tm) = task.due_time { format!("◯ {} 🕒{}", task.text, tm) }
                                        else { format!("◯ {}", task.text) };
                                    let fg = if is_p { Color32::from_rgb(21, 101, 192) } else { Color32::from_rgb(51, 51, 51) };
                                    let r = ui.add(egui::Label::new(RichText::new(lbl).size(12.0).color(fg)).sense(Sense::click())).on_hover_text(&task.text);
                                    if r.clicked() {
                                        if is_p { self.complete_task(&task.uuid); }
                                        else { self.pending_confirm.insert(task.uuid.clone(), Instant::now() + std::time::Duration::from_millis(2500)); }
                                    }
                                    r.context_menu(|ui| { self.render_task_context_menu(ui, task); });
                                }
                            });
                        });
                    });
                }
            });
            if row == 0 { ui.add_space(6.0); }
        }
    }

    fn render_view_plans(&mut self, ui: &mut egui::Ui) {
        let btn = egui::Button::new(RichText::new("➕ 添加周期计划").size(13.0).strong().color(Color32::from_rgb(46, 125, 50)))
            .fill(Color32::from_rgb(232, 245, 233)).stroke(Stroke::new(1.0_f32, Color32::from_rgb(200, 230, 201))).rounding(Rounding::same(6.0)).min_size(Vec2::new(ui.available_width(), 32.0));
        if ui.add(btn).on_hover_cursor(CursorIcon::PointingHand).clicked() { self.open_edit_dialog_for_plan(None); }
        ui.add_space(6.0);
        let snap = self.plans.clone();
        if snap.is_empty() {
            ui.centered_and_justified(|ui| { ui.label(RichText::new("暂无周期计划，点击上方按钮或输入“每天...”创建").color(Color32::GRAY)); });
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for plan in snap {
                let r_str = get_rule_display_str(Some(&plan.repeat_rule), Some(&plan.repeat_days));
                let resp = egui::Frame::none().fill(Color32::WHITE).rounding(Rounding::same(6.0)).stroke(Stroke::new(1.0_f32, Color32::from_rgb(238, 238, 238))).inner_margin(egui::Margin::symmetric(8.0, 8.0)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("🗓️").size(15.0));
                        ui.label(RichText::new(&plan.text).size(13.0).strong().color(Color32::from_rgb(51, 51, 51)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            draw_badge(ui, &format!("🔁{}", r_str), Color32::from_rgb(232, 245, 233), Color32::from_rgb(46, 125, 50));
                            if let Some(ref tm) = plan.due_time { draw_badge(ui, &format!("🕒{}", tm), Color32::from_rgb(232, 245, 233), Color32::from_rgb(46, 125, 50)); }
                            if plan.priority == 1 { draw_badge(ui, "⭐", Color32::from_rgb(255, 248, 225), Color32::from_rgb(245, 127, 23)); }
                        });
                    });
                }).response;
                ui.interact(resp.rect, ui.make_persistent_id(&plan.plan_uuid), Sense::click()).context_menu(|ui| {
                    if ui.button("✏️ 修改计划蓝图").clicked() { ui.close_menu(); self.open_edit_dialog_for_plan(Some(&plan)); }
                    if ui.button(RichText::new("🗑️ 停止并删除计划").color(Color32::from_rgb(211, 47, 47))).clicked() {
                        ui.close_menu();
                        self.delete_confirm_modal = Some((true, plan.plan_uuid.clone(), plan.text.clone()));
                    }
                });
                ui.add_space(4.0);
            }
        });
    }

    fn render_view_forest(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let t_title = self.focus_task.as_ref().map(|t| format!("专注: {}", t.text.chars().take(12).collect::<String>())).unwrap_or_else(|| "当前专注：自由沉浸".into());
            ui.label(RichText::new(t_title).size(13.0).strong().color(Color32::from_rgb(46, 125, 50)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut mins = self.config.focus_minutes;
                ui.add_enabled_ui(!self.is_focus_running && !self.is_resting, |ui| {
                    if ui.add(egui::DragValue::new(&mut mins).clamp_range(1..=120).suffix(" 分钟")).changed() {
                        self.config.focus_minutes = mins;
                        self.focus_duration_secs = mins * 60;
                        self.remaining_secs = self.focus_duration_secs;
                        self.save_all_data();
                    }
                    ui.label(RichText::new("时长:").size(12.0));
                });
            });
        });
        ui.add_space(12.0);
        egui::Frame::none().fill(Color32::from_rgba_unmultiplied(232, 245, 233, 220)).rounding(Rounding::same(16.0)).inner_margin(egui::Margin::symmetric(0.0, 28.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| { ui.label(RichText::new(self.current_forest_icon()).size(64.0)); });
        });
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(format!("{:02}:{:02}", self.remaining_secs / 60, self.remaining_secs % 60)).size(28.0).strong().color(Color32::from_rgb(27, 94, 32)));
        });
        ui.add_space(8.0);
        let total = if self.is_resting { self.rest_duration_secs } else { self.focus_duration_secs }.max(1);
        let prog = (total.saturating_sub(self.remaining_secs) as f32) / (total as f32);
        ui.add(egui::ProgressBar::new(prog).desired_height(6.0).fill(Color32::from_rgb(67, 160, 71)));
        ui.add_space(16.0);
        ui.columns(2, |cols| {
            let t_txt = if self.is_focus_running { "⏸️ 暂停" } else if self.remaining_secs != self.focus_duration_secs && !self.is_resting { "▶️ 继续" } else { "🌱 开始种植" };
            let btn_t = egui::Button::new(RichText::new(t_txt).size(13.5).strong().color(Color32::WHITE)).fill(Color32::from_rgb(76, 175, 80)).rounding(Rounding::same(6.0)).min_size(Vec2::new(cols[0].available_width(), 34.0));
            if cols[0].add(btn_t).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                self.is_focus_running = !self.is_focus_running;
                self.last_timer_tick = Instant::now();
            }
            let can_gu = self.is_focus_running || self.is_resting || self.remaining_secs != self.focus_duration_secs;
            let gu_txt = if self.is_resting { "⏭️ 跳过休息" } else { "⚠️ 放弃" };
            let bg_gu = if can_gu { Color32::from_rgb(229, 57, 53) } else { Color32::from_rgb(189, 189, 189) };
            cols[1].add_enabled_ui(can_gu, |ui| {
                let btn_gu = egui::Button::new(RichText::new(gu_txt).size(13.5).strong().color(Color32::WHITE)).fill(bg_gu).rounding(Rounding::same(6.0)).min_size(Vec2::new(ui.available_width(), 34.0));
                if ui.add(btn_gu).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    if self.is_resting { self.reset_forest_session(); } else { self.giveup_confirm_modal = true; }
                }
            });
        });
    }

    fn render_task_context_menu(&mut self, ui: &mut egui::Ui, task: &TodoTask) {
        if ui.button("🌲 对此事项开启专注").clicked() {
            ui.close_menu();
            self.focus_task = Some(task.clone());
            self.current_view = ViewTab::Forest;
        }
        if ui.button("➡️ 延后一天 (智能视线隐藏)").clicked() {
            ui.close_menu();
            let today_obj = Local::now().date_naive();
            let tom_obj = today_obj + Duration::days(1);
            let tom_s = tom_obj.format("%Y-%m-%d").to_string();
            if let Some(t) = self.todos.iter_mut().find(|x| x.uuid == task.uuid) {
                if t.due_date.is_some() && t.original_due.is_none() { t.original_due = t.due_date.clone(); }
                if let Some(ref ds) = t.due_date {
                    let nd = (NaiveDate::parse_from_str(ds, "%Y-%m-%d").unwrap_or(today_obj) + Duration::days(1)).max(tom_obj);
                    t.due_date = Some(nd.format("%Y-%m-%d").to_string());
                } else {
                    t.due_date = Some(tom_s.clone());
                }
                t.hide_until = Some(tom_s);
            }
            self.sort_todos();
            self.save_all_data();
            self.show_toast("➡️ 已延后至明天并暂时隐藏");
        }
        if ui.button("✏️ 编辑事项").clicked() { ui.close_menu(); self.open_edit_dialog_for_task(task); }
        if ui.button(RichText::new("🗑️ 删除事项").color(Color32::from_rgb(211, 47, 47))).clicked() {
            ui.close_menu();
            self.delete_confirm_modal = Some((false, task.uuid.clone(), task.text.clone()));
        }
    }

    fn render_modals(&mut self, ctx: &egui::Context) {
        if self.show_focus_success_modal {
            egui::Window::new("focus_success").title_bar(false).collapsible(false).resizable(false).fixed_size(Vec2::new(260.0, 250.0)).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .frame(egui::Frame::none().fill(Color32::from_rgb(241, 248, 233)).rounding(Rounding::same(16.0)).stroke(Stroke::new(2.0_f32, Color32::from_rgb(129, 199, 132))).inner_margin(egui::Margin::same(16.0)))
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("🌲").size(56.0));
                        ui.add_space(4.0);
                        ui.label(RichText::new("专注完成！").size(20.0).strong().color(Color32::from_rgb(46, 125, 50)));
                        ui.add_space(6.0);
                        ui.label(RichText::new("路漫漫其修远兮\n休息一下吧！").size(13.5).color(Color32::from_rgb(85, 85, 85)));
                        ui.add_space(16.0);
                        let btn = egui::Button::new(RichText::new("开始休息 ☕").size(15.0).strong().color(Color32::WHITE)).fill(Color32::from_rgb(76, 175, 80)).rounding(Rounding::same(8.0)).min_size(Vec2::new(180.0, 36.0));
                        if ui.add(btn).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            self.show_focus_success_modal = false;
                            self.start_rest_session();
                        }
                    });
                });
        }

        if self.giveup_confirm_modal {
            egui::Window::new("⚠️ 放弃专注？").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                ui.label("中途放弃会导致当前种植的树苗枯萎！");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("确认放弃").color(Color32::from_rgb(211, 47, 47))).clicked() {
                        self.giveup_confirm_modal = false;
                        self.is_focus_running = false;
                        self.record_forest("withered");
                        self.withered_anim_until = Some(Instant::now() + std::time::Duration::from_secs(2));
                    }
                    if ui.button("继续坚持").clicked() { self.giveup_confirm_modal = false; }
                });
            });
        }

        if let Some((is_plan, uuid, title)) = self.delete_confirm_modal.clone() {
            egui::Window::new("🗑️ 确认删除").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                if is_plan { ui.label(format!("彻底删除周期计划【{}】吗？\n(主列表中未完成的该任务也会被一并移除)", title)); }
                else { ui.label(format!("删除单次事项【{}】吗？\n(仅删本次实例，不影响原始周期计划运行)", title)); }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("确认删除").color(Color32::from_rgb(211, 47, 47))).clicked() {
                        if is_plan {
                            self.plans.retain(|p| p.plan_uuid != uuid);
                            self.todos.retain(|t| t.plan_uuid.as_deref() != Some(uuid.as_str()));
                        } else {
                            self.todos.retain(|t| t.uuid != uuid);
                        }
                        self.save_all_data();
                        self.delete_confirm_modal = None;
                    }
                    if ui.button("取消").clicked() { self.delete_confirm_modal = None; }
                });
            });
        }

        if let Some(mut state) = self.edit_modal.clone() {
            let (mut close_m, mut save_m) = (false, false);
            egui::Window::new(if state.is_plan { "✏️ 编辑周期计划" } else { "✏️ 编辑事项" }).collapsible(false).resizable(false).default_width(310.0).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                egui::Grid::new("edit_grid").num_columns(2).spacing([8.0, 8.0]).show(ui, |ui| {
                    ui.label("事项名称:"); ui.text_edit_singleline(&mut state.title); ui.end_row();
                    ui.label("优先级:"); ui.checkbox(&mut state.priority, "⭐ 标记为重要事项"); ui.end_row();
                    if !state.is_plan {
                        ui.label("截止日期:");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut state.has_due, "");
                            ui.add_enabled_ui(state.has_due, |ui| {
                                ui.add(egui::DragValue::new(&mut state.due_year).clamp_range(2020..=2099).suffix("年"));
                                ui.add(egui::DragValue::new(&mut state.due_month).clamp_range(1..=12).suffix("月"));
                                let max_d = days_in_month(state.due_year, state.due_month);
                                state.due_day = state.due_day.min(max_d);
                                ui.add(egui::DragValue::new(&mut state.due_day).clamp_range(1..=max_d).suffix("日"));
                            });
                        });
                        ui.end_row();
                    }
                    ui.label("截止时刻:");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut state.has_time, "");
                        ui.add_enabled_ui(state.has_time, |ui| {
                            ui.add(egui::DragValue::new(&mut state.time_hour).clamp_range(0..=23).suffix("时"));
                            ui.add(egui::DragValue::new(&mut state.time_min).clamp_range(0..=59).suffix("分"));
                        });
                    });
                    ui.end_row();

                    if state.is_periodic_instance {
                        ui.label("重复设置:");
                        ui.label(RichText::new("(周期实例，仅支持修改单次时间与名称)").size(11.0).color(Color32::from_rgb(230, 81, 0)));
                        ui.end_row();
                    } else {
                        ui.label("重复设置:");
                        egui::ComboBox::from_id_source("cb_repeat")
                            .selected_text(match state.repeat_rule.as_str() {
                                "daily" => "每天",
                                "weekdays" => "工作日 (周一至周五)",
                                "weekly" => "每周 (指定周几)",
                                "monthly" => "每月 (指定几号)",
                                _ => "不重复",
                            })
                            .show_ui(ui, |ui| {
                                if !state.is_plan {
                                    ui.selectable_value(&mut state.repeat_rule, "none".into(), "不重复");
                                }
                                ui.selectable_value(&mut state.repeat_rule, "daily".into(), "每天");
                                ui.selectable_value(&mut state.repeat_rule, "weekdays".into(), "工作日 (周一至周五)");
                                ui.selectable_value(&mut state.repeat_rule, "weekly".into(), "每周 (指定周几)");
                                ui.selectable_value(&mut state.repeat_rule, "monthly".into(), "每月 (指定几号)");
                            });
                        ui.end_row();

                        if state.repeat_rule == "weekly" {
                            ui.label("具体星期:");
                            ui.horizontal(|ui| {
                                for (i, w) in ["一", "二", "三", "四", "五", "六", "日"].iter().enumerate() {
                                    ui.checkbox(&mut state.weekly_checked[i], *w);
                                }
                            });
                            ui.end_row();
                        } else if state.repeat_rule == "monthly" {
                            ui.label("具体日期:");
                            ui.add(egui::TextEdit::singleline(&mut state.monthly_str).hint_text("如: 1, 15, 31"));
                            ui.end_row();
                        }
                    }
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("💾 保存").strong().color(Color32::from_rgb(46, 125, 50))).clicked() {
                        if state.title.trim().is_empty() {
                            self.show_toast("❌ 名称不能为空！");
                        } else {
                            save_m = true;
                        }
                    }
                    if ui.button("取消").clicked() {
                        close_m = true;
                    }
                });
            });

            if save_m {
                self.apply_edit_modal_save(&state);
                self.edit_modal = None;
            } else if close_m {
                self.edit_modal = None;
            } else {
                self.edit_modal = Some(state);
            }
        }

        if self.show_history_modal {
            let mut open = true;
            egui::Window::new("📚 历史复盘中心")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_size(Vec2::new(330.0, 440.0))
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    let mut grouped: BTreeMap<String, Vec<&HistoryRecord>> = BTreeMap::new();
                    for h in self.history.iter().rev() {
                        let dk = if h.completed.len() >= 10 { h.completed[..10].to_string() } else { "历史归档".to_string() };
                        grouped.entry(dk).or_default().push(h);
                    }
                    if grouped.is_empty() {
                        ui.centered_and_justified(|ui| { ui.label("暂无已完成的历史记录"); });
                    } else {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for (dk, list) in grouped.iter().rev() {
                                egui::CollapsingHeader::new(RichText::new(format!("📂 {} (完成 {} 项)", dk, list.len())).strong().color(Color32::from_rgb(46, 125, 50)))
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        for item in list {
                                            let ct = if item.completed.len() >= 16 { &item.completed[11..16] } else { "--:--" };
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new(format!("✔️ [{}]", ct)).size(11.5).color(Color32::GRAY));
                                                ui.label(RichText::new(&item.text).size(12.5));
                                            });
                                        }
                                    });
                            }
                        });
                    }
                });
            if !open { self.show_history_modal = false; }
        }

        if self.show_forest_stats_modal {
            let mut open = true;
            let records: Vec<ForestRecord> = safe_json_load("forest.json");
            let healthy = records.iter().filter(|r| r.status == "healthy").count();
            let withered = records.iter().filter(|r| r.status == "withered").count();
            let total_mins: u32 = records.iter().filter(|r| r.status == "healthy").map(|r| r.minutes).sum();

            egui::Window::new("🌳 个人专注森林")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_size(Vec2::new(300.0, 360.0))
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    egui::Frame::none().fill(Color32::from_rgb(232, 245, 233)).rounding(Rounding::same(8.0)).inner_margin(egui::Margin::same(10.0)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(format!("🌲 成活大树：{} 棵\n🥀 枯萎灌木：{} 棵\n⏳ 累计深度专注：{} 分钟", healthy, withered, total_mins)).strong().color(Color32::from_rgb(27, 94, 32)));
                    });
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for r in records.iter().rev().take(35) {
                            let ic = if r.status == "healthy" { "🌲" } else { "🥀" };
                            let ts = if r.timestamp.len() >= 16 { r.timestamp[5..16].replace('T', " ") } else { "未知时间".into() };
                            ui.label(format!("{} [{}] {} ({}m)", ic, ts, r.task, r.minutes));
                            ui.separator();
                        }
                    });
                });
            if !open { self.show_forest_stats_modal = false; }
        }
    }

    fn apply_edit_modal_save(&mut self, state: &EditDialogState) {
        let title = state.title.trim().to_string();
        let prio = if state.priority { 1 } else { 0 };
        let due_date = if state.has_due { Some(format!("{:04}-{:02}-{:02}", state.due_year, state.due_month, state.due_day)) } else { None };
        let due_time = if state.has_time { Some(format!("{:02}:{:02}", state.time_hour, state.time_min)) } else { None };

        let (rule_opt, repeat_days) = if state.repeat_rule == "none" {
            (None, Vec::new())
        } else if state.repeat_rule == "weekly" {
            let mut days: Vec<u32> = state.weekly_checked.iter().enumerate().filter_map(|(i, &chk)| if chk { Some(i as u32) } else { None }).collect();
            if days.is_empty() { days.push(0); }
            (Some("weekly".to_string()), days)
        } else if state.repeat_rule == "monthly" {
            let mut nums: Vec<u32> = state.monthly_str.replace('，', ",").split(',').filter_map(|s| s.trim().parse::<u32>().ok()).filter(|&n| (1..=31).contains(&n)).collect();
            nums.sort_unstable();
            nums.dedup();
            if nums.is_empty() { nums.push(1); }
            (Some("monthly".to_string()), nums)
        } else {
            (Some(state.repeat_rule.clone()), Vec::new())
        };

        if state.is_plan {
            let r_rule = rule_opt.unwrap_or_else(|| "daily".into());
            if state.is_new_plan {
                self.plans.push(Plan {
                    plan_uuid: state.target_uuid.clone(),
                    text: title,
                    priority: prio,
                    due_time: due_time.clone(),
                    reminder_time: due_time,
                    repeat_rule: r_rule,
                    repeat_days,
                    base_date: today_str(),
                });
            } else {
                if let Some(p) = self.plans.iter_mut().find(|x| x.plan_uuid == state.target_uuid) {
                    p.text = title.clone();
                    p.priority = prio;
                    p.due_time = due_time.clone();
                    p.repeat_rule = r_rule.clone();
                    p.repeat_days = repeat_days.clone();
                }
                for t in &mut self.todos {
                    if t.plan_uuid.as_deref() == Some(state.target_uuid.as_str()) {
                        t.text = title.clone();
                        t.priority = prio;
                        t.due_time = due_time.clone();
                        t.repeat_rule = Some(r_rule.clone());
                        t.repeat_days = Some(repeat_days.clone());
                    }
                }
            }
            self.sync_repeat_plans();
        } else if let Some(t) = self.todos.iter_mut().find(|x| x.uuid == state.target_uuid) {
            t.text = title;
            t.priority = prio;
            t.due_date = due_date;
            t.due_time = due_time;
            if !state.is_periodic_instance {
                t.repeat_rule = rule_opt;
                t.repeat_days = if repeat_days.is_empty() { None } else { Some(repeat_days) };
            }
        }
        self.sort_todos();
        self.save_all_data();
    }

    fn render_toast(&self, ctx: &egui::Context) {
        if let Some((ref msg, _)) = self.toast {
            egui::Area::new(egui::Id::new("toast_area"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -65.0))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::none().fill(Color32::from_rgba_unmultiplied(30, 30, 30, 220)).rounding(Rounding::same(8.0)).inner_margin(egui::Margin::symmetric(18.0, 10.0)).show(ui, |ui| {
                        ui.label(RichText::new(msg).size(13.0).strong().color(Color32::WHITE));
                    });
                });
        }
    }
}

impl eframe::App for EggDoneApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] { [0.0, 0.0, 0.0, 0.0] }
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.background_tick(ctx);
        if !self.is_expanded { self.render_collapsed_ball(ctx); } else { self.render_expanded_panel(ctx); }
    }
}

fn draw_badge(ui: &mut egui::Ui, text: &str, bg: Color32, fg: Color32) {
    egui::Frame::none().fill(bg).rounding(Rounding::same(4.0)).inner_margin(egui::Margin::symmetric(5.0, 2.0)).show(ui, |ui| {
        ui.label(RichText::new(text).size(10.0).strong().color(fg));
    });
}

fn is_valid_single_font(bytes: &[u8]) -> bool {
    if bytes.len() < 4 { return false; }
    let magic = &bytes[0..4];
    magic == [0x00, 0x01, 0x00, 0x00] || magic == b"OTTO" || magic == b"true"
}

fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let cjk_candidates = [
        "C:\\Windows\\Fonts\\simhei.ttf",
        "C:\\Windows\\Fonts\\msyh.ttf",
        "C:\\Windows\\Fonts\\simkai.ttf",
        "C:\\Windows\\Fonts\\simfang.ttf",
        "C:\\Windows\\Fonts\\Deng.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/Library/Fonts/Arial Unicode.ttf",
    ];
    for path in cjk_candidates {
        if let Ok(bytes) = fs::read(path) {
            if is_valid_single_font(&bytes) {
                fonts.font_data.insert("cjk".to_owned(), FontData::from_owned(bytes));
                fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "cjk".to_owned());
                fonts.families.get_mut(&FontFamily::Monospace).unwrap().push("cjk".to_owned());
                break;
            }
        }
    }
    let emoji_candidates = [
        "C:\\Windows\\Fonts\\seguiemj.ttf",
        "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    ];
    for path in emoji_candidates {
        if let Ok(bytes) = fs::read(path) {
            if is_valid_single_font(&bytes) {
                fonts.font_data.insert("sys_emoji".to_owned(), FontData::from_owned(bytes));
                fonts.families.get_mut(&FontFamily::Proportional).unwrap().push("sys_emoji".to_owned());
                break;
            }
        }
    }
    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        let log_path = get_base_dir().join("crash.log");
        let _ = fs::write(log_path, format!("Panic occurred:\n{}", info));
    }));

    if let Ok(instance) = SingleInstance::new("EggDone_Rust_Instance_Mutex") {
        if !instance.is_single() {
            let _ = Notification::new().summary("提示").body("待办程序已在运行中！").show();
            std::process::exit(0);
        }
        std::mem::forget(instance);
    }

    let viewport = egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(true)
        .with_inner_size([360.0, 640.0])
        .with_position([200.0, 80.0]);

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("EggDoneTodo", options, Box::new(|cc| Box::new(EggDoneApp::new(cc))))
}
// === main.rs 彻底结束 ===
